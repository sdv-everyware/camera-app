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

//! The webcam over V4L2 (v4l2r's ioctls, as harry's reference camera uses them): find a UVC
//! capture device, choose its format (format.rs), stream with memory-mapped buffers, hand out
//! the newest frame. Every failure is an error the caller turns into the offline state; an
//! unplugged camera ends the stream and the caller opens it again when it is back.

use crate::format::{self, Choice, Interval, Offer, Pixels};
use crate::status::Offline;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use v4l2r::bindings;
use v4l2r::ioctl::{self, BufferFlags, Capabilities, IntoErrno, MemoryConsistency, PlaneMapping};
use v4l2r::memory::{MemoryType, MmapHandle};
use v4l2r::{Format, PixelFormat, QueueType};

const QUEUE: QueueType = QueueType::VideoCapture;
/// Buffers the driver fills: one with the app, the others in the driver's queue.
const BUFFERS: u32 = 4;
/// Where the kernel lists the video devices (with their drivers).
const SYSFS_VIDEO: &str = "/sys/class/video4linux";

/// Why the camera cannot be used (now).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureError {
    /// No device, or it went away (unplugged).
    Gone,
    /// Another process streams from it.
    Busy,
    NoPermission,
    /// Not a capture device, or no format the app decodes.
    Unsupported,
    /// No frame within the wait.
    Stalled,
    /// Another error of the device (an errno).
    Failed(i32),
}

impl CaptureError {
    fn from_errno(errno: i32) -> CaptureError {
        match errno {
            libc::ENOENT | libc::ENODEV | libc::ENXIO | libc::ESHUTDOWN => CaptureError::Gone,
            libc::EBUSY => CaptureError::Busy,
            libc::EACCES | libc::EPERM => CaptureError::NoPermission,
            other => CaptureError::Failed(other),
        }
    }

    fn of(e: impl IntoErrno) -> CaptureError {
        CaptureError::from_errno(e.into_errno())
    }

    fn of_io(e: std::io::Error) -> CaptureError {
        CaptureError::from_errno(e.raw_os_error().unwrap_or(libc::EIO))
    }

    /// The offline state to show for it.
    pub fn offline(self) -> Offline {
        match self {
            CaptureError::Gone => Offline::NoCamera,
            CaptureError::Busy => Offline::Busy,
            CaptureError::NoPermission => Offline::NoPermission,
            CaptureError::Unsupported => Offline::Unsupported,
            CaptureError::Stalled | CaptureError::Failed(_) => Offline::NoPictures,
        }
    }
}

/// The UVC devices that may be the camera, /dev/videoN by N: those whose driver is uvcvideo.
/// A UVC camera has a second node for its metadata, which `open` refuses (no video capture);
/// the SoC's codecs (venus) are not opened at all.
pub fn uvc_devices() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(SYSFS_VIDEO) else { return Vec::new() };
    let mut found: Vec<(u32, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let index: u32 = name.strip_prefix("video")?.parse().ok()?;
            let driver = std::fs::read_link(e.path().join("device/driver")).ok()?;
            (driver.file_name()? == "uvcvideo").then(|| (index, PathBuf::from(format!("/dev/{name}"))))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, p)| p).collect()
}

/// A device that is a camera: it captures video and streams.
fn is_capture(file: &File) -> Result<ioctl::Capability, CaptureError> {
    let cap: ioctl::Capability = ioctl::querycap(file).map_err(CaptureError::of)?;
    let caps = cap.device_caps();
    if caps.contains(Capabilities::VIDEO_CAPTURE) && caps.contains(Capabilities::STREAMING) {
        Ok(cap)
    } else {
        Err(CaptureError::Unsupported)
    }
}

fn open_file(path: &Path) -> Result<File, CaptureError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(CaptureError::of_io)
}

/// Every format, size and frame interval the device offers, of the formats the app decodes.
pub fn offers(file: &File) -> Vec<Offer> {
    let mut out = Vec::new();
    for desc in ioctl::FormatIterator::new(file, QUEUE) {
        let Some(pixels) = Pixels::from_fourcc(desc.pixelformat.to_fourcc()) else { continue };
        for (width, height) in sizes(file, desc.pixelformat) {
            let intervals = intervals(file, desc.pixelformat, width, height);
            if !intervals.is_empty() {
                out.push(Offer { pixels, width, height, intervals });
            }
        }
    }
    out
}

/// The sizes of `pixelformat`: the discrete ones, or of a range its ends.
fn sizes(file: &File, pixelformat: PixelFormat) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for index in 0..64 {
        let Ok(e) = ioctl::enum_frame_sizes::<bindings::v4l2_frmsizeenum>(file, index, pixelformat) else { break };
        match e.size() {
            Some(ioctl::FrmSizeTypes::Discrete(d)) => out.push((d.width, d.height)),
            Some(ioctl::FrmSizeTypes::StepWise(s)) => {
                out.push((s.min_width, s.min_height));
                out.push((s.max_width, s.max_height));
                break;
            }
            None => break,
        }
    }
    out
}

/// The frame intervals of `pixelformat` at a size.
fn intervals(file: &File, pixelformat: PixelFormat, width: u32, height: u32) -> Vec<Interval> {
    let mut out = Vec::new();
    for index in 0..64 {
        let Ok(e) = ioctl::enum_frame_intervals::<bindings::v4l2_frmivalenum>(file, index, pixelformat, width, height)
        else {
            break;
        };
        match e.intervals() {
            Some(ioctl::FrmIvalTypes::Discrete(f)) => out.push(Interval { num: f.numerator, den: f.denominator }),
            Some(ioctl::FrmIvalTypes::StepWise(s)) => {
                out.extend(format::stepwise(
                    Interval { num: s.min.numerator, den: s.min.denominator },
                    Interval { num: s.max.numerator, den: s.max.denominator },
                ));
                break;
            }
            None => break,
        }
    }
    out
}

/// What `camera_app --probe` prints: each UVC device with its formats.
pub fn probe(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    if paths.is_empty() {
        out.push_str("no UVC video device\n");
    }
    for path in paths {
        let file = match open_file(path) {
            Ok(f) => f,
            Err(e) => {
                out.push_str(&format!("{}: cannot open ({e:?})\n", path.display()));
                continue;
            }
        };
        match is_capture(&file) {
            Ok(cap) => {
                out.push_str(&format!("{}: {} ({}, {})\n", path.display(), cap.card, cap.driver, cap.bus_info));
                let all = offers(&file);
                if all.is_empty() {
                    out.push_str("  no MJPEG or YUYV format\n");
                }
                for o in &all {
                    let fps: Vec<String> = o.intervals.iter().map(|i| format!("{}/{}", i.den, i.num)).collect();
                    out.push_str(&format!("  {:?} {}x{} fps {}\n", o.pixels, o.width, o.height, fps.join(" ")));
                }
            }
            Err(_) => out.push_str(&format!("{}: not a video capture device (metadata)\n", path.display())),
        }
    }
    out
}

/// A frame the app holds: its bytes stay valid until `Camera::give_back`.
pub struct Frame {
    pub index: u32,
    pub len: usize,
    /// The driver's frame counter.
    pub sequence: u32,
    /// Older frames that were ready and given back for this one (the app was behind).
    pub stale: u32,
}

/// The camera, streaming.
pub struct Camera {
    file: File,
    path: PathBuf,
    pub choice: Choice,
    /// The format the driver took: its size and row stride.
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    maps: Vec<PlaneMapping>,
    interval: Interval,
}

impl Camera {
    /// Opens the camera at `path` (None: the first UVC capture device), chooses its format for
    /// a live view of at most `max` at up to `live_fps`, and starts it at the live or the still
    /// rate.
    pub fn open(path: Option<&Path>, max: (u32, u32), live_fps: u32, live: bool) -> Result<Camera, CaptureError> {
        let (path, file) = match path {
            Some(p) => {
                let f = open_file(p)?;
                is_capture(&f)?;
                (p.to_path_buf(), f)
            }
            None => find()?,
        };
        let choice = format::choose(&offers(&file), max, live_fps).ok_or(CaptureError::Unsupported)?;
        let mut camera = Camera {
            file,
            path,
            choice,
            width: 0,
            height: 0,
            stride: 0,
            maps: Vec::new(),
            interval: if live { choice.live } else { choice.still },
        };
        camera.set_format()?;
        camera.set_interval(camera.interval)?;
        camera.map_buffers()?;
        camera.start()?;
        log::info!(
            "camera: {} {:?} {}x{} (stride {}), live {} fps, stills from {} fps, started at {} fps",
            camera.path.display(),
            choice.pixels,
            camera.width,
            camera.height,
            camera.stride,
            choice.live.fps(),
            choice.still.fps(),
            camera.interval.fps()
        );
        Ok(camera)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn set_format(&mut self) -> Result<(), CaptureError> {
        let mut fmt: Format = ioctl::g_fmt(&self.file, QUEUE).map_err(CaptureError::of)?;
        fmt.width = self.choice.width;
        fmt.height = self.choice.height;
        fmt.pixelformat = PixelFormat::from_fourcc(&self.choice.pixels.fourcc());
        let fmt: Format = ioctl::s_fmt(&mut self.file, (QUEUE, &fmt)).map_err(CaptureError::of)?;
        // the driver may take another size or format than asked: what it took is what comes
        if Pixels::from_fourcc(fmt.pixelformat.to_fourcc()) != Some(self.choice.pixels) || fmt.plane_fmt.is_empty() {
            log::warn!("camera: the driver took {:?} instead of {:?}", fmt.pixelformat, self.choice.pixels);
            return Err(CaptureError::Unsupported);
        }
        self.width = fmt.width;
        self.height = fmt.height;
        self.stride = fmt.plane_fmt[0].bytesperline;
        if self.choice.pixels == Pixels::Yuyv && self.stride < self.width * 2 {
            self.stride = self.width * 2;
        }
        Ok(())
    }

    /// Asks for frames every `interval` (VIDIOC_S_PARM). The driver rounds to what it has.
    fn set_interval(&mut self, interval: Interval) -> Result<(), CaptureError> {
        let mut parm = bindings::v4l2_streamparm { type_: QUEUE as u32, ..Default::default() };
        // the capture member is the one of a capture queue
        parm.parm.capture.timeperframe = bindings::v4l2_fract { numerator: interval.num, denominator: interval.den };
        let got: bindings::v4l2_streamparm = ioctl::s_parm(&self.file, parm).map_err(CaptureError::of)?;
        // SAFETY: the driver filled the capture member: the queue is a capture queue.
        let tpf = unsafe { got.parm.capture.timeperframe };
        self.interval = Interval { num: tpf.numerator, den: tpf.denominator };
        Ok(())
    }

    fn map_buffers(&mut self) -> Result<(), CaptureError> {
        let n: usize = ioctl::reqbufs(&self.file, QUEUE, MemoryType::Mmap, BUFFERS, MemoryConsistency::empty())
            .map_err(CaptureError::of)?;
        for i in 0..n {
            let q: ioctl::QueryBuffer = ioctl::querybuf(&self.file, QUEUE, i).map_err(CaptureError::of)?;
            let plane = q.planes.first().ok_or(CaptureError::Unsupported)?;
            self.maps.push(ioctl::mmap(&self.file, plane.mem_offset, plane.length).map_err(CaptureError::of)?);
        }
        if self.maps.is_empty() {
            return Err(CaptureError::Unsupported);
        }
        Ok(())
    }

    fn queue(&self, index: u32) -> Result<(), CaptureError> {
        let mut buf = ioctl::QBuffer::<MmapHandle>::new(QUEUE, index);
        buf.planes = vec![ioctl::QBufPlane::new(0)];
        ioctl::qbuf::<_, ()>(&self.file, buf).map_err(CaptureError::of)
    }

    /// Queues every buffer and starts the stream.
    fn start(&self) -> Result<(), CaptureError> {
        for i in 0..self.maps.len() {
            self.queue(i as u32)?;
        }
        ioctl::streamon(&self.file, QUEUE).map_err(CaptureError::of)
    }

    /// The live rate (`live`) or the still rate: the stream stops, takes the interval and
    /// starts again (UVC takes no new interval while it streams). Nothing when it has it.
    pub fn set_live(&mut self, live: bool) -> Result<(), CaptureError> {
        let want = if live { self.choice.live } else { self.choice.still };
        if want == self.interval {
            return Ok(());
        }
        // STREAMOFF gives every buffer back to the app
        ioctl::streamoff(&self.file, QUEUE).map_err(CaptureError::of)?;
        self.set_interval(want)?;
        self.start()?;
        log::info!("camera: {} fps ({})", self.interval.fps(), if live { "live" } else { "stills" });
        Ok(())
    }

    /// The newest frame, waiting up to `timeout` for one. Older frames that are ready are given
    /// back on the way: the app always shows the latest picture. A frame the driver marks as
    /// broken, or an empty one, is given back and waited past.
    pub fn next(&mut self, timeout: Duration) -> Result<Frame, CaptureError> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(CaptureError::Stalled);
            }
            self.wait(left)?;
            let mut newest: Option<Frame> = None;
            let mut stale = 0;
            loop {
                match ioctl::dqbuf::<ioctl::V4l2Buffer>(&self.file, QUEUE) {
                    Ok(buf) => {
                        let len = (*buf.get_first_plane().bytesused as usize).min(self.maps[buf.index() as usize].len());
                        let frame = Frame { index: buf.index(), len, sequence: buf.sequence(), stale: 0 };
                        if buf.flags().contains(BufferFlags::ERROR) || len == 0 {
                            self.give_back(frame)?;
                            continue;
                        }
                        if let Some(older) = newest.replace(frame) {
                            self.give_back(older)?;
                            stale += 1;
                        }
                    }
                    Err(e) => {
                        let errno = e.into_errno();
                        if errno == libc::EAGAIN {
                            break;
                        }
                        // the stream's end: unplugged (ENODEV, EIO after a disconnect)
                        return Err(if errno == libc::EIO { CaptureError::Gone } else { CaptureError::from_errno(errno) });
                    }
                }
            }
            if let Some(mut frame) = newest {
                frame.stale = stale;
                return Ok(frame);
            }
        }
    }

    /// Waits until a frame is ready or `timeout` passed; an error when the device is gone.
    fn wait(&self, timeout: Duration) -> Result<(), CaptureError> {
        let mut fds = [libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: one valid pollfd for the call.
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 1, ms) };
        if rc < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINTR) {
                return Ok(());
            }
            return Err(CaptureError::of_io(e));
        }
        if rc == 0 {
            return Err(CaptureError::Stalled);
        }
        if fds[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 && fds[0].revents & libc::POLLIN == 0 {
            return Err(CaptureError::Gone);
        }
        Ok(())
    }

    /// The bytes of `frame`.
    pub fn bytes(&self, frame: &Frame) -> &[u8] {
        &self.maps[frame.index as usize][..frame.len]
    }

    /// Gives `frame` back to the driver for the next picture.
    pub fn give_back(&self, frame: Frame) -> Result<(), CaptureError> {
        self.queue(frame.index)
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        // the mappings go first (their Drop), then the file closes; the driver stops the
        // stream with the last close
        let _ = ioctl::streamoff(&self.file, QUEUE);
        self.maps.clear();
    }
}

/// The first UVC device that captures video.
fn find() -> Result<(PathBuf, File), CaptureError> {
    let mut last = CaptureError::Gone;
    for path in uvc_devices() {
        match open_file(&path).and_then(|f| is_capture(&f).map(|_| f)) {
            Ok(f) => return Ok((path, f)),
            // a metadata node: not the camera, look on
            Err(CaptureError::Unsupported) => {}
            Err(e) => last = e,
        }
    }
    Err(last)
}
