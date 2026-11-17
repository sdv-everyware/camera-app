// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The control socket: the panel says when the full view is shown (live) and hears whether the
//! camera delivers (its LIVE chip). A unix stream socket next to the panel's Wayland socket,
//! rw for this user and its group only, in the panel's runtime directory; the lines are
//! protocol.rs's.
//!
//! ```text
//! panel ──connect──► $XDG_RUNTIME_DIR/har-camera (this app listens)
//!   ◄── "on" | "off"          at once, then at every change   (Board::set)
//!   ──► "live" | "still"      the full view opened / closed   (protocol::Demand -> camera.rs)
//!   closed by the panel: its live request ends; closed by this app: the camera app is gone
//! ```

use crate::protocol::{self, Demand, Lines};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Connections at once: the panel needs one; more are refused.
const MAX_CLIENTS: usize = 4;
/// A client that does not read its status lines does not hold up the camera.
const WRITE_TIMEOUT: Duration = Duration::from_millis(100);

/// The connected clients and what they were told last.
pub struct Board {
    clients: Mutex<Vec<(u64, UnixStream)>>,
    on: AtomicBool,
}

impl Board {
    fn new() -> Self {
        Board { clients: Mutex::new(Vec::new()), on: AtomicBool::new(false) }
    }

    /// A board without a socket (the control socket could not be made): nobody to tell.
    pub fn none() -> Arc<Board> {
        Arc::new(Board::new())
    }

    /// Whether the camera delivers; every client hears a change.
    pub fn set(&self, on: bool) {
        if self.on.swap(on, Ordering::SeqCst) == on {
            return;
        }
        let Ok(mut clients) = self.clients.lock() else { return };
        // a client that cannot take the line is dropped; its reader sees the end
        clients.retain_mut(|(id, s)| match s.write_all(protocol::status_line(on)) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("camera: control client {id}: {e}, dropped");
                let _ = s.shutdown(std::net::Shutdown::Both);
                false
            }
        });
    }

    fn add(&self, id: u64, mut stream: UnixStream) -> std::io::Result<()> {
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        // under the lock: a change that `set` makes meanwhile reaches this client after its
        // first line, not before it or never
        let mut clients = self.clients.lock().map_err(|_| std::io::Error::other("lock"))?;
        stream.write_all(protocol::status_line(self.on.load(Ordering::SeqCst)))?;
        clients.push((id, stream));
        Ok(())
    }

    fn remove(&self, id: u64) {
        if let Ok(mut clients) = self.clients.lock() {
            clients.retain(|(i, _)| *i != id);
        }
    }

    fn count(&self) -> usize {
        self.clients.lock().map(|c| c.len()).unwrap_or(MAX_CLIENTS)
    }
}

/// Listens on `path` and hands the panel's mode (true: live) to `modes`. The board says the
/// camera's state to the clients. AddrInUse when another camera app serves the socket.
pub fn start(path: &Path, modes: Sender<bool>) -> std::io::Result<Arc<Board>> {
    // a socket someone answers on is another camera app's: not taken over
    if UnixStream::connect(path).is_ok() {
        return Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "another camera app serves it"));
    }
    remove_stale(path)?;
    let listener = UnixListener::bind(path)?;
    // the runtime directory (0750) keeps others out already; the socket is the user's and its
    // group's only
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
    log::info!("camera: control socket {}", path.display());
    let board = Arc::new(Board::new());
    let demand = Arc::new(Mutex::new(Demand::default()));
    let accept_board = board.clone();
    std::thread::Builder::new().name("camera-control".to_string()).spawn(move || {
        let next = AtomicU64::new(1);
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("camera: control socket: {e}");
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
            };
            let id = next.fetch_add(1, Ordering::SeqCst);
            let (uid, pid) = peer(&stream);
            if accept_board.count() >= MAX_CLIENTS {
                log::warn!("camera: control client {id} (uid {uid}, pid {pid}) refused: {MAX_CLIENTS} connected");
                continue;
            }
            log::info!("camera: control client {id} (uid {uid}, pid {pid})");
            // the board writes the status lines on a clone, this client's thread reads
            if let Err(e) = stream.try_clone().and_then(|w| accept_board.add(id, w)) {
                log::warn!("camera: control client {id}: {e}");
                continue;
            }
            let (board, demand, modes) = (accept_board.clone(), demand.clone(), modes.clone());
            let spawned = std::thread::Builder::new()
                .name(format!("camera-client-{id}"))
                .spawn(move || serve(id, stream, &board, &demand, &modes));
            if let Err(e) = spawned {
                log::warn!("camera: control client {id}: no thread ({e})");
                accept_board.remove(id);
            }
        }
    })?;
    Ok(board)
}

/// One client: its lines until it closes or says something else than the protocol's words.
fn serve(id: u64, mut stream: UnixStream, board: &Board, demand: &Mutex<Demand>, modes: &Sender<bool>) {
    let mut lines = Lines::default();
    let mut buf = [0u8; 64];
    'read: loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let Ok(got) = lines.push(&buf[..n]) else {
            log::warn!("camera: control client {id}: line too long, closed");
            break;
        };
        for line in got {
            let Some(request) = protocol::parse(&line) else {
                log::warn!("camera: control client {id}: unknown line ({} bytes), closed", line.len());
                break 'read;
            };
            log::info!("camera: control client {id}: {request:?}");
            if let Ok(mut d) = demand.lock() {
                if d.ask(id, request) {
                    let _ = modes.send(d.live());
                }
            }
        }
    }
    board.remove(id);
    let _ = stream.shutdown(std::net::Shutdown::Both);
    if let Ok(mut d) = demand.lock() {
        if d.gone(id) {
            let _ = modes.send(d.live());
        }
    }
    log::info!("camera: control client {id} gone");
}

/// The socket a run before left (nobody answers on it): removed when it is a socket, not a
/// file or a link someone put there.
fn remove_stale(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_socket() => std::fs::remove_file(path),
        Ok(_) => Err(std::io::Error::other(format!("{} exists and is not a socket", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// The client's uid and pid (SO_PEERCRED), for the log.
fn peer(stream: &UnixStream) -> (i64, i64) {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` and `len` are valid out pointers of the sizes passed.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if rc == 0 {
        (i64::from(cred.uid), i64::from(cred.pid))
    } else {
        (-1, -1)
    }
}
