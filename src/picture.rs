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

//! A camera frame as the picture of a zone. harry stretches a picture over its frame, so the
//! app cuts it to the frame's aspect first: the largest centred rectangle (CSS object-fit:
//! cover), never a distorted face. YUYV is converted to RGBA here, only the rows and columns
//! of that rectangle.

/// A rectangle of a picture, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The largest centred rectangle of a `src` picture with the aspect of `dst`. Its x and width
/// are even, so that it starts and ends on a YUYV pixel pair.
pub fn cover(src: (u32, u32), dst: (u32, u32)) -> Rect {
    let (sw, sh) = (u64::from(src.0), u64::from(src.1));
    let (dw, dh) = (u64::from(dst.0.max(1)), u64::from(dst.1.max(1)));
    let (w, h) = if sw * dh > sh * dw {
        // the source is wider: full height, the width of the aspect
        ((sh * dw / dh) & !1, sh)
    } else {
        (sw & !1, sw * dh / dw)
    };
    let (w, h) = (w.max(2).min(sw), h.max(1).min(sh));
    Rect { x: (((sw - w) / 2) & !1) as u32, y: ((sh - h) / 2) as u32, width: w as u32, height: h as u32 }
}

/// A copy of the rectangle `r` of an RGBA picture `width` pixels wide (rows of `width * 4`
/// bytes). The caller keeps a whole picture as it is instead (camera.rs).
pub fn crop_rgba(rgba: &[u8], width: u32, r: Rect) -> Vec<u8> {
    let stride = width as usize * 4;
    let mut out = Vec::with_capacity(r.width as usize * r.height as usize * 4);
    for row in r.y..r.y + r.height {
        let start = row as usize * stride + r.x as usize * 4;
        out.extend_from_slice(&rgba[start..start + r.width as usize * 4]);
    }
    out
}

/// The rectangle `r` of a YUYV frame with rows of `stride` bytes, as RGBA (BT.601, limited
/// range, as UVC cameras send it). None when the frame is shorter than the rectangle needs (a
/// frame cut short on the bus).
pub fn yuyv_to_rgba(yuyv: &[u8], stride: usize, r: Rect) -> Option<Vec<u8>> {
    let last = (r.y + r.height) as usize;
    let needed = (last.checked_sub(1)?) * stride + (r.x + r.width) as usize * 2;
    if !r.width.is_multiple_of(2) || !r.x.is_multiple_of(2) || yuyv.len() < needed {
        return None;
    }
    let mut out = vec![0u8; r.width as usize * r.height as usize * 4];
    let row_out = r.width as usize * 4;
    for (i, row) in (r.y as usize..last).enumerate() {
        let src = &yuyv[row * stride + r.x as usize * 2..row * stride + (r.x + r.width) as usize * 2];
        let dst = &mut out[i * row_out..(i + 1) * row_out];
        for (pair, px) in src.chunks_exact(4).zip(dst.chunks_exact_mut(8)) {
            let (u, v) = (i32::from(pair[1]) - 128, i32::from(pair[3]) - 128);
            rgb(i32::from(pair[0]), u, v, &mut px[..4]);
            rgb(i32::from(pair[2]), u, v, &mut px[4..]);
        }
    }
    Some(out)
}

/// One pixel: Y with the pair's U and V (both less 128) into RGBA.
fn rgb(y: i32, u: i32, v: i32, px: &mut [u8]) {
    let c = 298 * (y - 16);
    px[0] = clamp((c + 409 * v + 128) >> 8);
    px[1] = clamp((c - 100 * u - 208 * v + 128) >> 8);
    px[2] = clamp((c + 516 * u + 128) >> 8);
    px[3] = 255;
}

fn clamp(x: i32) -> u8 {
    x.clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_tile_takes_the_middle_rows_of_a_16_9_frame() {
        // the panel's tile is 904 x 360
        assert_eq!(cover((1280, 720), (904, 360)), Rect { x: 0, y: 105, width: 1280, height: 509 });
        // a 16:9 frame in a 16:9 zone: all of it
        assert_eq!(cover((1280, 720), (1920, 1080)), Rect { x: 0, y: 0, width: 1280, height: 720 });
        // a 4:3 frame in a 16:9 zone: the middle rows
        assert_eq!(cover((640, 480), (1920, 1080)), Rect { x: 0, y: 60, width: 640, height: 360 });
        // a 16:9 frame in a square: the middle columns, an even start and width
        let r = cover((1280, 720), (500, 500));
        assert_eq!((r.x, r.width, r.height), (280, 720, 720));
    }

    #[test]
    fn a_crop_copies_the_rows_of_the_rectangle() {
        let rgba: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8).collect(); // 4 x 3
        assert_eq!(crop_rgba(&rgba, 4, Rect { x: 0, y: 0, width: 4, height: 3 }), rgba);
        let mid = crop_rgba(&rgba, 4, Rect { x: 1, y: 1, width: 2, height: 1 });
        assert_eq!(mid, vec![20, 21, 22, 23, 24, 25, 26, 27]);
    }

    #[test]
    fn yuyv_black_white_and_grey_come_out_as_such() {
        // two pixel pairs in a row: black and white, then mid grey twice (U = V = 128)
        let yuyv = [16, 128, 235, 128, 126, 128, 126, 128];
        let rgba = yuyv_to_rgba(&yuyv, 8, Rect { x: 0, y: 0, width: 4, height: 1 }).unwrap();
        assert_eq!(&rgba[0..4], &[0, 0, 0, 255]);
        assert_eq!(&rgba[4..8], &[255, 255, 255, 255]);
        assert_eq!(&rgba[8..12], &[128, 128, 128, 255]);
        // pure red in BT.601 limited range: Y 81, U 90, V 240
        let red = yuyv_to_rgba(&[81, 90, 81, 240], 4, Rect { x: 0, y: 0, width: 2, height: 1 }).unwrap();
        assert!(red[0] > 250 && red[1] < 5 && red[2] < 5, "{:?}", &red[..4]);
    }

    #[test]
    fn a_short_or_odd_yuyv_frame_is_refused() {
        let yuyv = [0u8; 16]; // 4 x 2 at a stride of 8
        assert!(yuyv_to_rgba(&yuyv, 8, Rect { x: 0, y: 0, width: 4, height: 2 }).is_some());
        assert!(yuyv_to_rgba(&yuyv, 8, Rect { x: 0, y: 0, width: 4, height: 3 }).is_none());
        assert!(yuyv_to_rgba(&yuyv, 8, Rect { x: 1, y: 0, width: 2, height: 1 }).is_none());
        // the middle row of three, at its stride
        let rows = [[16u8, 128, 16, 128], [235, 128, 235, 128], [16, 128, 16, 128]].concat();
        let mid = yuyv_to_rgba(&rows, 4, Rect { x: 0, y: 1, width: 2, height: 1 }).unwrap();
        assert_eq!(mid, vec![255, 255, 255, 255, 255, 255, 255, 255]);
    }
}
