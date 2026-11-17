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

//! What the live view costs, measured while it runs and logged every few seconds: how many
//! frames the camera sent and how many the app showed, how long a frame took to decode, and the
//! time between two frames shown (min / avg / max: a steady 33 ms is fluent, a long max is a
//! stutter).

use std::fmt;
use std::time::Duration;

/// Min, average and max of durations.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spread {
    n: u32,
    sum: Duration,
    min: Duration,
    max: Duration,
}

impl Spread {
    pub fn add(&mut self, d: Duration) {
        if self.n == 0 || d < self.min {
            self.min = d;
        }
        if d > self.max {
            self.max = d;
        }
        self.n += 1;
        self.sum += d;
    }

    pub fn avg(&self) -> Duration {
        if self.n == 0 {
            Duration::ZERO
        } else {
            self.sum / self.n
        }
    }
}

impl fmt::Display for Spread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        write!(f, "{:.1}/{:.1}/{:.1} ms", ms(self.min), ms(self.avg()), ms(self.max))
    }
}

/// One window of the live view's numbers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    /// Frames the camera sent (the driver's sequence numbers), and the ones it lost for want of
    /// a free buffer (gaps in them).
    pub sent: u32,
    pub lost: u32,
    /// Frames the app took from the driver but left for a newer one (it was behind).
    pub stale: u32,
    /// Frames the app skipped because the looper had not taken the last one yet.
    pub busy: u32,
    /// Frames shown.
    pub shown: u32,
    /// Decoding a frame (MJPEG to RGBA, the cut).
    pub decode: Spread,
    /// Between two frames shown.
    pub interval: Spread,
    last_sequence: Option<u32>,
}

impl Live {
    /// A frame with the driver's `sequence` came from the camera; `stale` older ones were left.
    pub fn frame(&mut self, sequence: u32, stale: u32) {
        match self.last_sequence {
            Some(last) => {
                let step = sequence.wrapping_sub(last);
                self.sent += step;
                self.lost += step.saturating_sub(1 + stale);
            }
            None => self.sent += 1,
        }
        self.stale += stale;
        self.last_sequence = Some(sequence);
    }

    /// The numbers for the log over `window`, and a new window that goes on from the last frame.
    pub fn report(&mut self, window: Duration) -> String {
        let secs = window.as_secs_f64().max(0.001);
        let line = format!(
            "camera: live {:.1} fps shown of {:.1} sent ({} lost by the driver, {} stale, {} skipped for the looper), \
             between frames {}, decode {}",
            f64::from(self.shown) / secs,
            f64::from(self.sent) / secs,
            self.lost,
            self.stale,
            self.busy,
            self.interval,
            self.decode
        );
        *self = Live { last_sequence: self.last_sequence, ..Live::default() };
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spread_has_its_min_avg_and_max() {
        let mut s = Spread::default();
        assert_eq!(s.avg(), Duration::ZERO);
        for ms in [30, 33, 36] {
            s.add(Duration::from_millis(ms));
        }
        assert_eq!(s.avg(), Duration::from_millis(33));
        assert_eq!(s.to_string(), "30.0/33.0/36.0 ms");
    }

    #[test]
    fn gaps_in_the_sequence_are_lost_frames_but_stale_ones_are_not() {
        let mut l = Live::default();
        l.frame(10, 0);
        l.frame(11, 0);
        // 12 left as stale for 13
        l.frame(13, 1);
        // 14 and 15 never came out of the driver
        l.frame(16, 0);
        assert_eq!((l.sent, l.lost, l.stale), (7, 2, 1));
        let line = l.report(Duration::from_secs(1));
        assert!(line.contains("2 lost by the driver, 1 stale"), "{line}");
        // the next window counts from 16
        l.frame(17, 0);
        assert_eq!((l.sent, l.lost), (1, 0));
    }
}
