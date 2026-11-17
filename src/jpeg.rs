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

//! An MJPEG frame to RGBA: libjpeg-turbo through jpeg_shim.c, which keeps libjpeg's error
//! jumps in C.

/// Why a frame did not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JpegError {
    /// Not a JPEG, or libjpeg stopped on it.
    Broken,
    /// Decoded with warnings: corrupt data, a frame cut short on the bus.
    Damaged,
    /// Larger than the buffer for the camera's format (width x height); the picture's size.
    Unexpected(u32, u32),
}

extern "C" {
    fn camera_jpeg_decode_rgba(
        src: *const u8,
        src_len: usize,
        dst: *mut u8,
        dst_len: usize,
        width: *mut u32,
        height: *mut u32,
    ) -> i32;
}

/// The JPEG `frame` as RGBA, at most `max_width` x `max_height` (the camera's format): the
/// pixels with their width and height. The pixels go into `dst`, a buffer of an earlier frame
/// when the caller has one (no allocation; its old pixels are overwritten), else a new one.
pub fn decode_rgba(frame: &[u8], max_width: u32, max_height: u32, dst: Option<Vec<u8>>) -> Result<(Vec<u8>, u32, u32), JpegError> {
    let len = max_width as usize * max_height as usize * 4;
    let mut dst = dst.unwrap_or_default();
    // a buffer of this size already has its length: resize then neither allocates nor writes
    dst.resize(len, 0);
    let (mut width, mut height) = (0u32, 0u32);
    // SAFETY: `frame` and `dst` are valid for the lengths passed; the shim writes only into
    // `dst` (at most `dst.len()` bytes), `width` and `height`, and keeps no pointer.
    let rc = unsafe {
        camera_jpeg_decode_rgba(frame.as_ptr(), frame.len(), dst.as_mut_ptr(), dst.len(), &mut width, &mut height)
    };
    match rc {
        0 => {
            dst.truncate(width as usize * height as usize * 4);
            Ok((dst, width, height))
        }
        1 => Err(JpegError::Damaged),
        -2 => Err(JpegError::Unexpected(width, height)),
        _ => Err(JpegError::Broken),
    }
}
