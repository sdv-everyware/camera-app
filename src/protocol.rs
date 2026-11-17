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

//! The lines between the panel and the camera app on the control socket (control.rs). One word
//! a line, both ways:
//!
//! ```text
//! panel ──"live\n" / "still\n"──► camera_app    the full view opened / closed
//! panel ◄──"on\n" / "off\n"────── camera_app    the camera delivers / does not (at connect, at
//!                                               every change): the tile's LIVE chip
//! ```
//!
//! A connection that asked for live keeps the camera live until it says still or closes: a
//! panel that dies does not leave the camera streaming at the live rate. Anything else than
//! the two words, or a line longer than MAX_LINE, ends the connection.

use std::collections::HashSet;

/// The longest line taken, without its newline.
pub const MAX_LINE: usize = 16;

/// What a client asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Live,
    Still,
}

/// The word of one line (without the newline), or None for anything else.
pub fn parse(line: &[u8]) -> Option<Request> {
    match line {
        b"live" => Some(Request::Live),
        b"still" => Some(Request::Still),
        _ => None,
    }
}

/// The line that tells a client whether the camera delivers.
pub fn status_line(on: bool) -> &'static [u8] {
    if on {
        b"on\n"
    } else {
        b"off\n"
    }
}

/// A line longer than MAX_LINE.
#[derive(Debug, PartialEq, Eq)]
pub struct TooLong;

/// The bytes of one connection as lines: what came in, cut at the newlines.
#[derive(Default)]
pub struct Lines {
    partial: Vec<u8>,
}

impl Lines {
    /// The complete lines in `bytes` (with what came before), without their newlines.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, TooLong> {
        let mut lines = Vec::new();
        for &b in bytes {
            if b == b'\n' {
                lines.push(std::mem::take(&mut self.partial));
            } else if self.partial.len() == MAX_LINE {
                return Err(TooLong);
            } else {
                self.partial.push(b);
            }
        }
        Ok(lines)
    }
}

/// Which connections want the live view. The camera is live while one does.
#[derive(Debug, Default)]
pub struct Demand {
    live: HashSet<u64>,
}

impl Demand {
    /// Connection `client` asked for `request`. True when the camera's mode changes with it.
    pub fn ask(&mut self, client: u64, request: Request) -> bool {
        let was = self.live();
        match request {
            Request::Live => self.live.insert(client),
            Request::Still => self.live.remove(&client),
        };
        was != self.live()
    }

    /// Connection `client` closed. True when the camera's mode changes with it.
    pub fn gone(&mut self, client: u64) -> bool {
        self.ask(client, Request::Still)
    }

    pub fn live(&self) -> bool {
        !self.live.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_words_and_nothing_else() {
        assert_eq!(parse(b"live"), Some(Request::Live));
        assert_eq!(parse(b"still"), Some(Request::Still));
        assert_eq!(parse(b"LIVE"), None);
        assert_eq!(parse(b"live "), None);
        assert_eq!(parse(b""), None);
        assert_eq!(status_line(true), b"on\n");
        assert_eq!(status_line(false), b"off\n");
    }

    #[test]
    fn lines_come_whole_across_reads_and_a_long_one_ends_it() {
        let mut lines = Lines::default();
        assert_eq!(lines.push(b"li").unwrap(), Vec::<Vec<u8>>::new());
        assert_eq!(lines.push(b"ve\nstill\nli").unwrap(), vec![b"live".to_vec(), b"still".to_vec()]);
        assert_eq!(lines.push(b"ve\n").unwrap(), vec![b"live".to_vec()]);
        assert_eq!(lines.push(&[b'x'; MAX_LINE]).unwrap(), Vec::<Vec<u8>>::new());
        assert_eq!(lines.push(b"x"), Err(TooLong));
    }

    #[test]
    fn live_while_one_connection_wants_it() {
        let mut d = Demand::default();
        assert!(!d.live());
        assert!(d.ask(1, Request::Live), "still -> live");
        assert!(!d.ask(2, Request::Live), "live already");
        assert!(!d.ask(1, Request::Still), "2 still wants live");
        assert!(d.gone(2), "the last one went: back to stills");
        assert!(!d.live());
        assert!(!d.gone(3), "a connection that never asked");
        assert!(!d.ask(4, Request::Still));
    }
}
