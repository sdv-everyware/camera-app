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

//! Where the pictures come from: the camera, or a clip while there is none (clip.rs). The clip
//! is for a demo and for the emulator, which has no camera; the app is started with `--clip`
//! for it. A camera that is there is always taken first, and one that is plugged in while the
//! clip runs takes over at the next look (every CAMERA_LOOK); when it goes away again the clip
//! comes back.
//!
//! ```text
//! open: the camera ──no camera──► the clip (--clip) ──no clip──► the camera's error (offline)
//! next: the clip's picture for now, at the clip's rate; every 2 s: is there a camera?
//! ```

use crate::capture::{Camera, CaptureError, Frame};
use crate::clip::{Pace, Reel};
use crate::format::Pixels;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How often a camera is looked for while the clip runs.
const CAMERA_LOOK: Duration = Duration::from_secs(2);
/// The largest clip read into memory.
const CLIP_MAX_BYTES: u64 = 64 << 20;
/// A clip that cannot be used is reported once, not at every look for the camera.
static CLIP_REPORTED: AtomicBool = AtomicBool::new(false);

/// What to open.
#[derive(Clone, Debug)]
pub struct Wanted {
    /// The camera's device, or None: the first UVC capture device.
    pub device: Option<PathBuf>,
    /// The largest frame taken from the camera.
    pub max: (u32, u32),
    pub live_fps: u32,
    /// The clip shown while there is no camera, or None: the offline state.
    pub clip: Option<PathBuf>,
    pub clip_fps: u32,
}

pub enum Source {
    Camera(Camera),
    Clip(Box<Clip>),
}

/// The clip, running since it was opened.
pub struct Clip {
    path: PathBuf,
    reel: Reel,
    pace: Pace,
    wanted: Wanted,
    live: bool,
    started: Instant,
    /// When the last picture was handed out, and when a camera was last looked for.
    last: Instant,
    looked: Instant,
}

impl Clip {
    fn open(path: &Path, wanted: &Wanted, live: bool) -> Result<Clip, String> {
        let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        if size > CLIP_MAX_BYTES {
            return Err(format!("{size} bytes, more than {CLIP_MAX_BYTES}"));
        }
        let reel = Reel::parse(std::fs::read(path).map_err(|e| e.to_string())?).ok_or("no JPEG picture in it")?;
        let now = Instant::now();
        let pace = Pace::new(wanted.clip_fps, reel.len());
        log::info!(
            "camera: no camera, the clip {} stands in: {} pictures of {}x{} at {} fps",
            path.display(),
            reel.len(),
            reel.width,
            reel.height,
            wanted.clip_fps
        );
        Ok(Clip { path: path.to_path_buf(), reel, pace, wanted: wanted.clone(), live, started: now, last: now, looked: now })
    }

    /// The clip's picture for now, after the wait to the next one.
    fn next(&mut self) -> Frame {
        let due = self.last + self.pace.period(self.live);
        std::thread::sleep(due.saturating_duration_since(Instant::now()));
        self.last = Instant::now();
        let (index, sequence) = self.pace.at(self.started.elapsed());
        Frame { index, len: self.reel.frame(index).len(), sequence, stale: 0 }
    }

    /// A camera, when it is time to look for one and one opens.
    fn camera(&mut self) -> Option<Camera> {
        if self.looked.elapsed() < CAMERA_LOOK {
            return None;
        }
        self.looked = Instant::now();
        Camera::open(self.wanted.device.as_deref(), self.wanted.max, self.wanted.live_fps, self.live).ok()
    }
}

impl Source {
    /// The camera; without one the clip, when the app has one; else the camera's error.
    pub fn open(wanted: &Wanted, live: bool) -> Result<Source, CaptureError> {
        let error = match Camera::open(wanted.device.as_deref(), wanted.max, wanted.live_fps, live) {
            Ok(camera) => return Ok(Source::Camera(camera)),
            Err(e) => e,
        };
        let Some(path) = &wanted.clip else { return Err(error) };
        match Clip::open(path, wanted, live) {
            Ok(clip) => Ok(Source::Clip(Box::new(clip))),
            Err(why) => {
                if !CLIP_REPORTED.swap(true, Ordering::Relaxed) {
                    log::warn!("camera: the clip {} cannot be shown: {why}", path.display());
                }
                Err(error)
            }
        }
    }

    /// The device, or the clip's file.
    pub fn path(&self) -> &Path {
        match self {
            Source::Camera(camera) => camera.path(),
            Source::Clip(clip) => &clip.path,
        }
    }

    /// How the frames are encoded: the camera's choice; a clip's are JPEG.
    pub fn pixels(&self) -> Pixels {
        match self {
            Source::Camera(camera) => camera.choice.pixels,
            Source::Clip(_) => Pixels::Mjpeg,
        }
    }

    /// The frames' width, height and row stride (the stride of YUYV frames only).
    pub fn geometry(&self) -> (u32, u32, u32) {
        match self {
            Source::Camera(camera) => (camera.width, camera.height, camera.stride),
            Source::Clip(clip) => (clip.reel.width, clip.reel.height, 0),
        }
    }

    pub fn set_live(&mut self, live: bool) -> Result<(), CaptureError> {
        match self {
            Source::Camera(camera) => camera.set_live(live),
            Source::Clip(clip) => {
                clip.live = live;
                Ok(())
            }
        }
    }

    /// The newest frame (capture.rs). While the clip runs, a camera that is there now takes
    /// over first: the frame is its first one then.
    pub fn next(&mut self, timeout: Duration) -> Result<Frame, CaptureError> {
        if let Source::Clip(clip) = self {
            match clip.camera() {
                Some(camera) => {
                    log::info!("camera: {} is there, the clip ends", camera.path().display());
                    *self = Source::Camera(camera);
                }
                None => return Ok(clip.next()),
            }
        }
        match self {
            Source::Camera(camera) => camera.next(timeout),
            Source::Clip(clip) => Ok(clip.next()),
        }
    }

    /// The bytes of `frame`.
    pub fn bytes(&self, frame: &Frame) -> &[u8] {
        match self {
            Source::Camera(camera) => camera.bytes(frame),
            Source::Clip(clip) => clip.reel.frame(frame.index),
        }
    }

    /// Gives `frame` back for the next picture: to the camera's driver; a clip keeps its own.
    pub fn give_back(&self, frame: Frame) -> Result<(), CaptureError> {
        match self {
            Source::Camera(camera) => camera.give_back(frame),
            Source::Clip(_) => Ok(()),
        }
    }
}
