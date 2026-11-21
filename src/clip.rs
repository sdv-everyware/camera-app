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

//! A clip that stands in for the camera: a Motion-JPEG file, which is JPEG pictures one after
//! the other, as a UVC camera sends them. `Reel` finds the pictures in the file and their size;
//! `Pace` says which picture shows at which time, so the clip runs at its own rate whatever the
//! app asks it for, and starts again after its last picture.
//!
//! Made on a computer, once (the device needs no ffmpeg):
//!
//! ```text
//! ffmpeg -i clip.webm -an -vf fps=25,scale=1280:720 -c:v mjpeg -q:v 8 -f mjpeg clip.mjpeg
//! ```

use std::ops::Range;
use std::time::Duration;

/// How often a picture is handed out while only stills are shown: the tile takes one every few
/// seconds, and the panel's modes are looked at between two pictures.
const STILL_PERIOD: Duration = Duration::from_millis(250);

const SOI: u8 = 0xD8;
const EOI: u8 = 0xD9;
const SOS: u8 = 0xDA;

/// The pictures of a clip.
pub struct Reel {
    data: Vec<u8>,
    frames: Vec<Range<usize>>,
    pub width: u32,
    pub height: u32,
}

impl Reel {
    /// The clip in `data`, or None when it holds no whole JPEG picture with a size.
    pub fn parse(data: Vec<u8>) -> Option<Reel> {
        let frames = frames(&data);
        let (width, height) = size(&data[frames.first()?.clone()])?;
        Some(Reel { data, frames, width, height })
    }

    pub fn len(&self) -> u32 {
        self.frames.len() as u32
    }

    /// The bytes of picture `index` (counted around the clip).
    pub fn frame(&self, index: u32) -> &[u8] {
        &self.data[self.frames[index as usize % self.frames.len()].clone()]
    }
}

/// Where the JPEG pictures of `data` are: each from its start marker to the end of its end
/// marker. Bytes between two pictures are skipped; a picture cut short at the end is left out.
pub fn frames(data: &[u8]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < data.len() {
        if data[i] != 0xFF || data[i + 1] != SOI {
            i += 1;
            continue;
        }
        match frame_end(data, i + 2) {
            Some(end) => {
                out.push(i..end);
                i = end;
            }
            None => break,
        }
    }
    out
}

/// The end of the picture whose segments begin at `i`: after its end marker. None when the
/// data ends first or is not a JPEG's.
fn frame_end(data: &[u8], mut i: usize) -> Option<usize> {
    loop {
        if *data.get(i)? != 0xFF {
            return None;
        }
        // a marker may come after fill bytes (FF)
        let mut code = *data.get(i + 1)?;
        while code == 0xFF {
            i += 1;
            code = *data.get(i + 1)?;
        }
        i += 2;
        match code {
            EOI => return Some(i),
            // markers without a length: TEM and the restart markers
            0x01 | 0xD0..=0xD7 => {}
            SOS => {
                i += segment_len(data, i)?;
                // the compressed data: up to the next marker that is neither a stuffed FF (FF 00)
                // nor a restart marker
                loop {
                    if *data.get(i)? != 0xFF {
                        i += 1;
                        continue;
                    }
                    match *data.get(i + 1)? {
                        0x00 | 0xD0..=0xD7 => i += 2,
                        0xFF => i += 1,
                        _ => break,
                    }
                }
            }
            _ => i += segment_len(data, i)?,
        }
    }
}

/// The length of the segment at `i` (its two length bytes count).
fn segment_len(data: &[u8], i: usize) -> Option<usize> {
    let n = usize::from(u16::from_be_bytes([*data.get(i)?, *data.get(i + 1)?]));
    (n >= 2).then_some(n)
}

/// The width and height of the JPEG picture `frame`, from its frame header (SOF0 to SOF2:
/// baseline, extended, progressive).
pub fn size(frame: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    loop {
        if *frame.get(i)? != 0xFF {
            return None;
        }
        let code = *frame.get(i + 1)?;
        i += 2;
        match code {
            0xFF => i -= 1,
            0x01 | 0xD0..=0xD7 => {}
            0xC0..=0xC2 => {
                let height = u16::from_be_bytes([*frame.get(i + 3)?, *frame.get(i + 4)?]);
                let width = u16::from_be_bytes([*frame.get(i + 5)?, *frame.get(i + 6)?]);
                return (width > 0 && height > 0).then_some((u32::from(width), u32::from(height)));
            }
            EOI | SOS => return None,
            _ => i += segment_len(frame, i)?,
        }
    }
}

/// Which picture of a clip shows when.
#[derive(Clone, Copy, Debug)]
pub struct Pace {
    /// Pictures a second.
    fps: u32,
    /// Pictures in the clip.
    count: u32,
}

impl Pace {
    pub fn new(fps: u32, count: u32) -> Pace {
        Pace { fps: fps.max(1), count: count.max(1) }
    }

    /// The picture `elapsed` after the clip started: its place in the clip, and how many
    /// pictures went by since the start (a camera's frame counter).
    pub fn at(&self, elapsed: Duration) -> (u32, u32) {
        let n = elapsed.as_micros() * u128::from(self.fps) / 1_000_000;
        ((n % u128::from(self.count)) as u32, n as u32)
    }

    /// The time between two pictures handed out: the clip's rate live, a slow one for stills.
    pub fn period(&self, live: bool) -> Duration {
        if live {
            Duration::from_micros(1_000_000 / u64::from(self.fps))
        } else {
            STILL_PERIOD
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JPEG picture as small as the parser needs: a frame header with the size, a scan with
    /// `scan` as its compressed data, the end marker.
    fn picture(width: u16, height: u16, scan: &[u8]) -> Vec<u8> {
        let mut p = vec![0xFF, SOI];
        // APP0 with four bytes
        p.extend([0xFF, 0xE0, 0x00, 0x06, b'J', b'F', b'I', b'F']);
        p.extend([0xFF, 0xC0, 0x00, 0x0B, 8]);
        p.extend(height.to_be_bytes());
        p.extend(width.to_be_bytes());
        p.extend([1, 1, 0x11, 0]);
        p.extend([0xFF, SOS, 0x00, 0x08, 1, 1, 0, 0, 63, 0]);
        p.extend(scan);
        p.extend([0xFF, EOI]);
        p
    }

    #[test]
    fn the_pictures_of_a_clip_are_found_one_after_the_other() {
        let a = picture(1280, 720, &[1, 2, 3]);
        let b = picture(1280, 720, &[4, 5]);
        let mut data = a.clone();
        data.extend(&b);
        assert_eq!(frames(&data), vec![0..a.len(), a.len()..a.len() + b.len()]);
    }

    #[test]
    fn an_end_marker_inside_the_compressed_data_does_not_end_the_picture() {
        // FF 00 is a stuffed FF, FF D0 a restart marker; D9 after the stuffed FF is data
        let a = picture(64, 48, &[0xFF, 0x00, 0xD9, 7, 0xFF, 0xD0, 9]);
        assert_eq!(frames(&a), vec![0..a.len()]);
    }

    #[test]
    fn bytes_between_pictures_are_skipped_and_a_cut_picture_is_left_out() {
        let a = picture(64, 48, &[1]);
        let mut data = vec![0, 0xFF, 0x12];
        data.extend(&a);
        data.extend([0, 0]);
        let mut cut = picture(64, 48, &[1, 2, 3]);
        cut.truncate(cut.len() - 3);
        data.extend(&cut);
        assert_eq!(frames(&data), vec![3..3 + a.len()]);
        assert!(frames(&[]).is_empty());
        assert!(frames(b"not a clip").is_empty());
    }

    #[test]
    fn a_picture_has_the_size_of_its_frame_header() {
        assert_eq!(size(&picture(1280, 720, &[1])), Some((1280, 720)));
        assert_eq!(size(&picture(0, 720, &[1])), None);
        assert_eq!(size(&[0xFF, SOI, 0xFF, EOI]), None);
    }

    #[test]
    fn a_reel_hands_out_its_pictures_around_the_clip() {
        let a = picture(64, 48, &[1]);
        let b = picture(64, 48, &[2, 2]);
        let mut data = a.clone();
        data.extend(&b);
        let reel = Reel::parse(data).unwrap();
        assert_eq!((reel.len(), reel.width, reel.height), (2, 64, 48));
        assert_eq!(reel.frame(0), &a[..]);
        assert_eq!(reel.frame(1), &b[..]);
        assert_eq!(reel.frame(2), &a[..]);
        assert!(Reel::parse(b"nothing".to_vec()).is_none());
    }

    #[test]
    fn the_clip_runs_at_its_rate_and_starts_again() {
        let pace = Pace::new(25, 225);
        assert_eq!(pace.at(Duration::ZERO), (0, 0));
        assert_eq!(pace.at(Duration::from_millis(39)), (0, 0));
        assert_eq!(pace.at(Duration::from_millis(40)), (1, 1));
        assert_eq!(pace.at(Duration::from_secs(9)), (0, 225));
        assert_eq!(pace.at(Duration::from_millis(9040)), (1, 226));
        assert_eq!(pace.period(true), Duration::from_millis(40));
        assert_eq!(pace.period(false), STILL_PERIOD);
        // a rate or a length of nothing does not divide by zero
        assert_eq!(Pace::new(0, 0).at(Duration::from_secs(3)), (0, 3));
    }
}
