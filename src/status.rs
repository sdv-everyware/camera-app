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

//! What the camera is doing, in the words the zones show, and when the tile takes its next
//! still.

use std::time::{Duration, Instant};

/// The camera's state as the app shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Looking for the camera and starting it (the app's first seconds).
    Starting,
    /// Pictures arrive.
    On,
    /// No pictures, and why.
    Off(Offline),
}

/// Why there are no pictures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offline {
    /// No camera is plugged in (no UVC capture device).
    NoCamera,
    /// Another process streams from it.
    Busy,
    /// The app may not open it.
    NoPermission,
    /// It was started but sends nothing (or only broken frames).
    NoPictures,
    /// It offers no format the app decodes (MJPEG, YUYV).
    Unsupported,
}

impl Status {
    /// Whether pictures arrive: the panel's LIVE chip.
    pub fn on(&self) -> bool {
        *self == Status::On
    }

    /// The headline of the offline state, or None while pictures arrive.
    pub fn title(&self) -> Option<&'static str> {
        match self {
            Status::On => None,
            Status::Starting => Some("Connecting to the camera"),
            Status::Off(_) => Some("Camera offline"),
        }
    }

    /// The line under the headline.
    pub fn detail(&self) -> &'static str {
        match self {
            Status::On => "",
            Status::Starting => "The picture appears in a moment",
            Status::Off(Offline::NoCamera) => "No camera is connected",
            Status::Off(Offline::Busy) => "Another app is using the camera",
            Status::Off(Offline::NoPermission) => "No permission to use the camera",
            Status::Off(Offline::NoPictures) => "The camera sends no pictures",
            Status::Off(Offline::Unsupported) => "This camera's picture format is not supported",
        }
    }
}

/// When the tile's next still is due: every `interval`, and as soon as possible after a still
/// that did not come out (a frame that failed to decode does not wait a whole interval).
#[derive(Debug)]
pub struct Stills {
    interval: Duration,
    last: Option<Instant>,
}

impl Stills {
    pub fn new(interval: Duration) -> Self {
        Stills { interval, last: None }
    }

    pub fn due(&self, now: Instant) -> bool {
        self.last.is_none_or(|t| now.saturating_duration_since(t) >= self.interval)
    }

    /// A still was shown at `now`.
    pub fn taken(&mut self, now: Instant) {
        self.last = Some(now);
    }

    /// The camera started again: its first frame is the next still.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// Whether frames decode: a window of broken frames that starts at the first one and ends
/// with a good one. One broken frame now and then (a still's frame cut short on the bus) is not
/// the camera failing.
#[derive(Debug, Default)]
pub struct Health {
    broken_since: Option<Instant>,
}

impl Health {
    /// A frame decoded.
    pub fn good(&mut self) {
        self.broken_since = None;
    }

    /// A frame did not decode at `now`: true when none has for `limit`.
    pub fn bad(&mut self, now: Instant, limit: Duration) -> bool {
        let since = *self.broken_since.get_or_insert(now);
        now.saturating_duration_since(since) >= limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_still_every_interval_and_the_first_at_once() {
        let t0 = Instant::now();
        let mut s = Stills::new(Duration::from_secs(5));
        assert!(s.due(t0));
        s.taken(t0);
        assert!(!s.due(t0 + Duration::from_millis(4999)));
        assert!(s.due(t0 + Duration::from_secs(5)));
        s.reset();
        assert!(s.due(t0));
    }

    #[test]
    fn one_broken_frame_is_not_a_broken_camera() {
        let t0 = Instant::now();
        let limit = Duration::from_secs(5);
        let mut h = Health::default();
        // a still's frame fails 5 s after the last good one: the window starts now
        assert!(!h.bad(t0 + Duration::from_secs(5), limit));
        h.good();
        // frames that keep failing for the whole limit
        let mut h = Health::default();
        assert!(!h.bad(t0, limit));
        assert!(!h.bad(t0 + Duration::from_millis(4999), limit));
        assert!(h.bad(t0 + limit, limit));
        h.good();
        assert!(!h.bad(t0 + Duration::from_secs(20), limit));
    }

    #[test]
    fn the_words_of_each_state() {
        assert!(Status::On.on());
        assert!(!Status::Starting.on());
        assert_eq!(Status::On.title(), None);
        assert_eq!(Status::Off(Offline::NoCamera).title(), Some("Camera offline"));
        assert_eq!(Status::Off(Offline::NoCamera).detail(), "No camera is connected");
        assert_eq!(Status::Starting.title(), Some("Connecting to the camera"));
    }
}
