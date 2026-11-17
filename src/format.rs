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

//! Which of the camera's formats the app takes, and at which frame rates. A UVC webcam offers
//! MJPEG (compressed: large pictures at full rate over USB 2) and YUYV (raw: small pictures at
//! full rate, large ones slowly). The app takes the largest picture up to a limit that comes at
//! a live rate, MJPEG before YUYV at the same size; the still rate is the slowest the camera
//! offers from STILL_FPS up, the live rate the fastest up to the asked one.

/// A live view is taken from a format that delivers at least this rate.
pub const MIN_LIVE_FPS: u32 = 15;
/// The tile's stills come from a stream at about this rate: fewer frames, the same exposure.
pub const STILL_FPS: u32 = 5;

/// The pixel formats the app decodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pixels {
    /// Motion JPEG: one JPEG per frame (libjpeg-turbo, jpeg_shim.c).
    Mjpeg,
    /// YUV 4:2:2, Y0 U Y1 V (yuyv.rs).
    Yuyv,
}

impl Pixels {
    pub fn from_fourcc(fourcc: [u8; 4]) -> Option<Pixels> {
        match &fourcc {
            b"MJPG" => Some(Pixels::Mjpeg),
            b"YUYV" => Some(Pixels::Yuyv),
            _ => None,
        }
    }

    pub fn fourcc(self) -> [u8; 4] {
        match self {
            Pixels::Mjpeg => *b"MJPG",
            Pixels::Yuyv => *b"YUYV",
        }
    }
}

/// A frame interval, `num / den` seconds (V4L2's v4l2_fract).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub num: u32,
    pub den: u32,
}

impl Interval {
    /// Frames a second, rounded down.
    pub fn fps(self) -> u32 {
        if self.num == 0 {
            0
        } else {
            self.den / self.num
        }
    }

    /// The interval of `fps` frames a second.
    pub fn of_fps(fps: u32) -> Interval {
        Interval { num: 1, den: fps.max(1) }
    }

    /// Frames a second as a float, for comparing intervals like 1001/30000.
    fn rate(self) -> f64 {
        if self.num == 0 {
            0.0
        } else {
            f64::from(self.den) / f64::from(self.num)
        }
    }
}

/// One way the camera can send pictures: a format, a size and the intervals it offers at it.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub pixels: Pixels,
    pub width: u32,
    pub height: u32,
    pub intervals: Vec<Interval>,
}

/// What the app asks the camera for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub pixels: Pixels,
    pub width: u32,
    pub height: u32,
    /// The interval while the full view is shown.
    pub live: Interval,
    /// The interval while only the tile is (a still every few seconds).
    pub still: Interval,
}

/// The best offer for a live view of at most `max` (width, height) at up to `live_fps`, or None
/// when the camera offers no format the app decodes. An offer larger than `max` is taken only
/// when there is no smaller one (the smallest then).
pub fn choose(offers: &[Offer], max: (u32, u32), live_fps: u32) -> Option<Choice> {
    let usable: Vec<&Offer> = offers.iter().filter(|o| o.width > 0 && o.height > 0 && !o.intervals.is_empty()).collect();
    let fits: Vec<&Offer> = usable.iter().copied().filter(|o| o.width <= max.0 && o.height <= max.1).collect();
    let pool = if fits.is_empty() {
        let smallest = usable.iter().map(|o| area(o)).min()?;
        usable.into_iter().filter(|o| area(o) == smallest).collect()
    } else {
        fits
    };
    let best = pool.into_iter().max_by_key(|o| {
        let fast = fastest(&o.intervals).fps() >= MIN_LIVE_FPS;
        (fast, area(o), o.pixels == Pixels::Mjpeg)
    })?;
    Some(Choice {
        pixels: best.pixels,
        width: best.width,
        height: best.height,
        live: at_or_below(&best.intervals, live_fps),
        still: still(&best.intervals),
    })
}

fn area(o: &Offer) -> u64 {
    u64::from(o.width) * u64::from(o.height)
}

/// The fastest of `intervals` (not empty).
fn fastest(intervals: &[Interval]) -> Interval {
    *intervals.iter().max_by(|a, b| a.rate().total_cmp(&b.rate())).expect("intervals")
}

/// The slowest of `intervals` (not empty).
fn slowest(intervals: &[Interval]) -> Interval {
    *intervals.iter().min_by(|a, b| a.rate().total_cmp(&b.rate())).expect("intervals")
}

/// The fastest interval at or below `fps` frames a second; when all are faster, the slowest.
pub fn at_or_below(intervals: &[Interval], fps: u32) -> Interval {
    let limit = f64::from(fps) + 0.01;
    intervals
        .iter()
        .filter(|i| i.rate() <= limit)
        .max_by(|a, b| a.rate().total_cmp(&b.rate()))
        .copied()
        .unwrap_or_else(|| slowest(intervals))
}

/// The still stream's interval: the slowest at or above STILL_FPS; when all are slower, the
/// fastest of them.
fn still(intervals: &[Interval]) -> Interval {
    let floor = f64::from(STILL_FPS) - 0.01;
    intervals
        .iter()
        .filter(|i| i.rate() >= floor)
        .min_by(|a, b| a.rate().total_cmp(&b.rate()))
        .copied()
        .unwrap_or_else(|| fastest(intervals))
}

/// The intervals of a stepwise or continuous range (V4L2_FRMIVAL_TYPE_STEPWISE), as a list:
/// its ends and the usual rates between them.
pub fn stepwise(min: Interval, max: Interval) -> Vec<Interval> {
    let (fast, slow) = (min.rate(), max.rate());
    let mut out = vec![min, max];
    for fps in [5, 10, 15, 20, 25, 30] {
        let rate = f64::from(fps);
        if rate < fast && rate > slow {
            out.push(Interval::of_fps(fps));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(pixels: Pixels, w: u32, h: u32, fps: &[u32]) -> Offer {
        Offer { pixels, width: w, height: h, intervals: fps.iter().map(|f| Interval::of_fps(*f)).collect() }
    }

    #[test]
    fn mjpeg_720p_wins_over_slow_yuyv_of_the_same_size() {
        let offers = [
            offer(Pixels::Yuyv, 640, 480, &[30, 15, 5]),
            offer(Pixels::Yuyv, 1280, 720, &[10, 5]),
            offer(Pixels::Mjpeg, 640, 480, &[30, 15, 5]),
            offer(Pixels::Mjpeg, 1280, 720, &[30, 25, 20, 15, 10, 5]),
            offer(Pixels::Mjpeg, 1920, 1080, &[30, 5]),
        ];
        let c = choose(&offers, (1280, 720), 30).unwrap();
        assert_eq!((c.pixels, c.width, c.height), (Pixels::Mjpeg, 1280, 720));
        assert_eq!((c.live.fps(), c.still.fps()), (30, 5));
    }

    #[test]
    fn without_mjpeg_the_largest_yuyv_at_a_live_rate() {
        let offers = [offer(Pixels::Yuyv, 640, 480, &[30, 15]), offer(Pixels::Yuyv, 1280, 720, &[10, 5])];
        let c = choose(&offers, (1280, 720), 30).unwrap();
        assert_eq!((c.pixels, c.width, c.height), (Pixels::Yuyv, 640, 480));
        // no rate from 5 up but 15 and 30: the slowest of them
        assert_eq!(c.still.fps(), 15);
    }

    #[test]
    fn a_camera_with_only_slow_or_large_offers_still_gets_one() {
        let slow = [offer(Pixels::Yuyv, 1280, 720, &[10, 5])];
        assert_eq!(choose(&slow, (1280, 720), 30).unwrap().live.fps(), 10);
        let large = [offer(Pixels::Mjpeg, 3840, 2160, &[30]), offer(Pixels::Mjpeg, 1920, 1080, &[30])];
        let c = choose(&large, (1280, 720), 30).unwrap();
        assert_eq!((c.width, c.height), (1920, 1080));
        assert!(choose(&[], (1280, 720), 30).is_none());
    }

    #[test]
    fn the_live_rate_is_the_fastest_at_or_below_the_asked_one() {
        let i: Vec<Interval> = [30, 25, 20, 15, 10, 5].iter().map(|f| Interval::of_fps(*f)).collect();
        assert_eq!(at_or_below(&i, 30).fps(), 30);
        assert_eq!(at_or_below(&i, 24).fps(), 20);
        assert_eq!(at_or_below(&i, 1).fps(), 5);
        // NTSC: 1001/30000 is 29.97 frames a second, at or below 30
        let ntsc = [Interval { num: 1001, den: 30000 }, Interval::of_fps(60)];
        assert_eq!(at_or_below(&ntsc, 30), Interval { num: 1001, den: 30000 });
    }

    #[test]
    fn a_range_becomes_its_ends_and_the_usual_rates() {
        let list = stepwise(Interval::of_fps(30), Interval::of_fps(1));
        let mut fps: Vec<u32> = list.iter().map(|i| i.fps()).collect();
        fps.sort();
        assert_eq!(fps, vec![1, 5, 10, 15, 20, 25, 30]);
        assert_eq!(Pixels::from_fourcc(*b"MJPG"), Some(Pixels::Mjpeg));
        assert_eq!(Pixels::from_fourcc(*b"H264"), None);
        assert_eq!(Pixels::Yuyv.fourcc(), *b"YUYV");
    }
}
