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

//! The camera's thread: opens the camera, keeps it streaming and turns frames into the zones'
//! pictures. In stills (the panel shows only the tile) the stream runs at the still rate and
//! one frame every `still_interval` becomes the tile's picture; live (the panel shows the full
//! view, control.rs) the stream runs at the live rate and every frame becomes the full view's
//! picture, the tile still getting its still. Without a camera, or when it stops, the zones
//! show the offline state and the thread looks for the camera again every RETRY.
//!
//! ```text
//! V4L2 ──frame──► decode (MJPEG: libjpeg-turbo, YUYV: picture.rs) ──cut to the zone's aspect──►
//!   pictures.rs (the looper's resource client) ──► harry redraws the zone that shows it
//! status changes ──► CameraAction (the offline state) and control::Board (the panel's chip)
//! ```

use crate::actions::CameraAction;
use crate::capture::{Camera, CaptureError, Frame};
use crate::config::Zone;
use crate::control::Board;
use crate::format::Pixels;
use crate::jpeg;
use crate::meter::Live;
use crate::picture::{self, Rect};
use crate::pictures::Pictures;
use crate::status::{Health, Offline, Status, Stills};
use harry::reducer::Actions;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often the camera is looked for while there is none, and the pause after a stream that
/// failed right after it started.
const RETRY: Duration = Duration::from_secs(2);
/// The longest wait for one frame; three in a row and the camera is opened again.
const FRAME_WAIT: Duration = Duration::from_secs(2);
const STALLS_BEFORE_REOPEN: u32 = 3;
/// Frames that do not decode for this long: the camera counts as sending no pictures.
const BROKEN_FOR: Duration = Duration::from_secs(5);
/// The first frames after the stream starts are often not exposed yet: no still from them.
const WARM_UP: Duration = Duration::from_millis(500);
/// How often the live view's numbers are logged while live (meter.rs).
const LIVE_LOG: Duration = Duration::from_secs(5);
/// How long a decoded live frame waits for the looper to take the last one (its frame on the
/// panel); after that it is dropped. About a frame of the camera.
const READY_WAIT: Duration = Duration::from_millis(30);
/// The least capacity the camera thread asks the scheduler for (uclamp.min, of 1024): a live
/// frame's decode (libjpeg-turbo, 720p) takes ~25 ms on a little core at its idle clock and a
/// few ms on a big one, and the camera sends one every 33 ms.
const UCLAMP_MIN: u32 = 512;

pub struct Settings {
    /// The device, or None: the first UVC capture device.
    pub device: Option<PathBuf>,
    /// The largest frame taken from the camera.
    pub max: (u32, u32),
    pub live_fps: u32,
    pub still_interval: Duration,
}

/// Starts the thread. `modes` says live (true) or stills; `pictures` takes the zones' pictures,
/// `actions` the zones' state, `board` the state for the panel.
pub fn spawn(
    settings: Settings,
    modes: Receiver<bool>,
    pictures: Pictures,
    actions: Sender<Actions<CameraAction>>,
    board: Arc<Board>,
) {
    std::thread::Builder::new()
        .name("camera".to_string())
        .spawn(move || Worker::new(settings, modes, pictures, actions, board).run())
        .expect("spawn the camera thread");
}

struct Worker {
    settings: Settings,
    modes: Receiver<bool>,
    pictures: Pictures,
    actions: Sender<Actions<CameraAction>>,
    board: Arc<Board>,
    live: bool,
    status: Status,
    stills: Stills,
    /// Since when frames do not decode.
    health: Health,
    /// When the stream started: WARM_UP.
    started: Instant,
    /// The live view's numbers since `meter_since`, for the log; when the last live frame was
    /// shown.
    meter: Live,
    meter_since: Instant,
    last_shown: Option<Instant>,
}

impl Worker {
    fn new(
        settings: Settings,
        modes: Receiver<bool>,
        pictures: Pictures,
        actions: Sender<Actions<CameraAction>>,
        board: Arc<Board>,
    ) -> Self {
        let stills = Stills::new(settings.still_interval);
        Worker {
            settings,
            modes,
            pictures,
            actions,
            board,
            live: false,
            status: Status::Starting,
            stills,
            health: Health::default(),
            started: Instant::now(),
            meter: Live::default(),
            meter_since: Instant::now(),
            last_shown: None,
        }
    }

    fn run(mut self) {
        ask_for_capacity(UCLAMP_MIN);
        self.publish(Status::Starting);
        loop {
            self.take_modes();
            match Camera::open(self.settings.device.as_deref(), self.settings.max, self.settings.live_fps, self.live) {
                Ok(mut camera) => {
                    self.stills.reset();
                    self.health = Health::default();
                    self.started = Instant::now();
                    let e = self.stream(&mut camera);
                    log::warn!("camera: {} stopped: {e:?}", camera.path().display());
                    self.publish(Status::Off(e.offline()));
                    drop(camera);
                    // a camera that opens and fails at once is not opened again at once
                    if self.started.elapsed() < RETRY {
                        self.wait(RETRY);
                    }
                }
                Err(e) => {
                    if self.status != Status::Off(e.offline()) {
                        log::warn!("camera: not available ({e:?}), looking again every {} s", RETRY.as_secs());
                    }
                    self.publish(Status::Off(e.offline()));
                    self.wait(RETRY);
                }
            }
        }
    }

    /// Streams until the camera fails; the error says why.
    fn stream(&mut self, camera: &mut Camera) -> CaptureError {
        let mut stalls = 0;
        loop {
            let was_live = self.live;
            self.take_modes();
            if self.live != was_live {
                if let Err(e) = camera.set_live(self.live) {
                    return e;
                }
                self.meter = Live::default();
                self.meter_since = Instant::now();
                self.last_shown = None;
                if !self.live {
                    // the full view is closed: it opens dark next time, not on an old frame
                    match self.pictures.darken(Zone::Live) {
                        Ok(told) => self.tell(Zone::Live, told),
                        Err(e) => log::warn!("camera: the full view not darkened: {e}"),
                    }
                }
            }
            let frame = match camera.next(FRAME_WAIT) {
                Ok(frame) => frame,
                Err(CaptureError::Stalled) => {
                    stalls += 1;
                    log::warn!("camera: no frame for {} s", FRAME_WAIT.as_secs() * u64::from(stalls));
                    self.publish(Status::Off(Offline::NoPictures));
                    if stalls >= STALLS_BEFORE_REOPEN {
                        return CaptureError::Stalled;
                    }
                    continue;
                }
                Err(e) => return e,
            };
            stalls = 0;
            self.on_frame(camera, &frame);
            if let Err(e) = camera.give_back(frame) {
                return e;
            }
        }
    }

    /// One frame: the full view's picture when live, the tile's when a still is due. A live
    /// frame is decoded whatever the looper does and then waits for it (READY_WAIT): the decode
    /// and the panel's frame overlap. A tile whose last still the looper has not taken skips it.
    fn on_frame(&mut self, camera: &Camera, frame: &Frame) {
        let now = Instant::now();
        let still = self.stills.due(now) && now.duration_since(self.started) >= WARM_UP;
        let mut zones = Vec::new();
        if still && self.pictures.ready(Zone::Tile) {
            zones.push(Zone::Tile);
        }
        if self.live {
            zones.push(Zone::Live);
            self.meter.frame(frame.sequence, frame.stale);
        }
        if zones.is_empty() {
            return;
        }
        let spare = if self.live { self.pictures.spare(Zone::Live) } else { None };
        match pictures(camera, frame, &zones, spare) {
            Some(pictures) => {
                if self.live {
                    self.meter.decode.add(now.elapsed());
                }
                self.health.good();
                for (zone, (rgba, w, h)) in zones.iter().zip(pictures) {
                    if *zone == Zone::Live && !self.wait_ready(Zone::Live) {
                        self.meter.busy += 1;
                        continue;
                    }
                    match self.pictures.send(*zone, rgba, w, h) {
                        Ok(told) => self.tell(*zone, told),
                        Err(e) => log::warn!("camera: picture of {zone:?} not shown: {e}"),
                    }
                    if *zone == Zone::Live {
                        self.count_live();
                    }
                }
                if zones.contains(&Zone::Tile) {
                    self.stills.taken(now);
                    let _ = self.actions.send(Actions::CustomAction(CameraAction::Still(clock_now())));
                }
                self.publish(Status::On);
            }
            None => {
                log::debug!("camera: frame {} did not decode", frame.sequence);
                if self.health.bad(now, BROKEN_FOR) {
                    self.publish(Status::Off(Offline::NoPictures));
                }
            }
        }
    }

    /// Waits until the looper took the last picture of `zone`, at most READY_WAIT.
    fn wait_ready(&self, zone: Zone) -> bool {
        let until = Instant::now() + READY_WAIT;
        while !self.pictures.ready(zone) {
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        true
    }

    /// A zone's frame shows another picture (its first, another size): the reducer hears it.
    fn tell(&self, zone: Zone, told: Option<customizations::ImageContentData>) {
        if let Some(content) = told {
            let _ = self.actions.send(Actions::CustomAction(CameraAction::Picture(zone, Some(content))));
        }
    }

    /// One live frame shown; every LIVE_LOG the live view's numbers in the log.
    fn count_live(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_shown.replace(now) {
            self.meter.interval.add(now.duration_since(last));
        }
        self.meter.shown += 1;
        let since = self.meter_since.elapsed();
        if since >= LIVE_LOG {
            log::info!("{}", self.meter.report(since));
            self.meter_since = now;
        }
    }

    fn publish(&mut self, status: Status) {
        if status == self.status && status != Status::Starting {
            return;
        }
        if status != self.status {
            log::info!("camera: {status:?}");
        }
        self.status = status.clone();
        self.board.set(status.on());
        let _ = self.actions.send(Actions::CustomAction(CameraAction::Status(status)));
    }

    /// The last mode the panel asked for, without waiting.
    fn take_modes(&mut self) {
        while let Ok(live) = self.modes.try_recv() {
            self.live = live;
        }
    }

    /// Waits `d`, taking the panel's modes as they come.
    fn wait(&mut self, d: Duration) {
        let until = Instant::now() + d;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            match self.modes.recv_timeout(left) {
                Ok(live) => self.live = live,
                Err(RecvTimeoutError::Timeout) => return,
                // the control socket is gone: the app is ending, or it never had one
                Err(RecvTimeoutError::Disconnected) => {
                    std::thread::sleep(left);
                    return;
                }
            }
        }
    }
}

/// The frame as the pictures of `zones`, each cut to its zone's aspect: RGBA with its size.
/// None when the frame does not decode.
fn pictures(camera: &Camera, frame: &Frame, zones: &[Zone], spare: Option<Vec<u8>>) -> Option<Vec<(Vec<u8>, u32, u32)>> {
    let bytes = camera.bytes(frame);
    let (w, h) = (camera.width, camera.height);
    match camera.choice.pixels {
        Pixels::Mjpeg => {
            let (rgba, dw, dh) = match jpeg::decode_rgba(bytes, w, h, spare) {
                Ok(d) => d,
                Err(e) => {
                    log::debug!("camera: JPEG frame: {e:?}");
                    return None;
                }
            };
            let rects: Vec<Rect> = zones.iter().map(|z| picture::cover((dw, dh), z.size())).collect();
            let mut out = Vec::with_capacity(zones.len());
            let mut rgba = Some(rgba);
            for (i, r) in rects.iter().enumerate() {
                let whole = *r == Rect { x: 0, y: 0, width: dw, height: dh };
                // the last zone takes the decoded buffer itself when it shows all of it
                let px = if whole && i + 1 == rects.len() {
                    rgba.take()?
                } else {
                    picture::crop_rgba(rgba.as_deref()?, dw, *r)
                };
                out.push((px, r.width, r.height));
            }
            Some(out)
        }
        Pixels::Yuyv => zones
            .iter()
            .map(|z| {
                let r = picture::cover((w, h), z.size());
                picture::yuyv_to_rgba(bytes, camera.stride as usize, r).map(|px| (px, r.width, r.height))
            })
            .collect(),
    }
}

/// The local time of day, "HH:MM:SS".
fn clock_now() -> String {
    // SAFETY: time with a null pointer only returns the time; localtime_r writes the struct
    // passed and keeps no pointer.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return String::new();
        }
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

/// sched_attr of sched_setattr(2), up to the utilization clamps (SCHED_ATTR_SIZE_VER1).
#[repr(C)]
#[derive(Default)]
struct SchedAttr {
    size: u32,
    sched_policy: u32,
    sched_flags: u64,
    sched_nice: i32,
    sched_priority: u32,
    sched_runtime: u64,
    sched_deadline: u64,
    sched_period: u64,
    sched_util_min: u32,
    sched_util_max: u32,
}

const SCHED_FLAG_KEEP_POLICY: u64 = 0x08;
const SCHED_FLAG_KEEP_PARAMS: u64 = 0x10;
const SCHED_FLAG_UTIL_CLAMP_MIN: u64 = 0x20;

/// Asks the scheduler to run the calling thread with at least `min` of 1024 of a core's
/// capacity (uclamp.min): on a big.LITTLE SoC it then runs on a big core, or a little one at a
/// high clock. Only a request: without uclamp in the kernel, or without the permission
/// (SELinux process setsched), the thread runs as before and the log says so.
fn ask_for_capacity(min: u32) {
    let attr = SchedAttr {
        size: std::mem::size_of::<SchedAttr>() as u32,
        sched_flags: SCHED_FLAG_KEEP_POLICY | SCHED_FLAG_KEEP_PARAMS | SCHED_FLAG_UTIL_CLAMP_MIN,
        sched_util_min: min,
        ..Default::default()
    };
    // SAFETY: sched_setattr reads `attr` (its size in its first field) for the calling thread
    // (pid 0) and keeps no pointer.
    let rc = unsafe { libc::syscall(libc::SYS_sched_setattr, 0, &attr as *const SchedAttr, 0) };
    if rc != 0 {
        log::warn!("camera: no uclamp.min for the camera thread ({}): it may run on a little core", std::io::Error::last_os_error());
    }
}
