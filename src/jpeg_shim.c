/*
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

/*
 * One MJPEG frame of a webcam to RGBA, with libjpeg-turbo (external/libjpeg-turbo).
 *
 * libjpeg reports a fatal error by calling error_exit, which must not return: the shim jumps
 * back here with longjmp. The jump never crosses a Rust frame: setjmp and every libjpeg call
 * are in this file, and Rust only sees the return code (jpeg.rs).
 *
 * A webcam's MJPEG frames often leave out the Huffman tables; libjpeg-turbo then takes the
 * standard ones (jstdhuff.c). A frame cut short on the bus decodes with warnings (grey rows at
 * the bottom): those are reported, and the caller drops the frame.
 */

#include <setjmp.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

#include "jpeglib.h"

/* Return codes, mirrored in jpeg.rs. */
#define CAMERA_JPEG_OK 0
#define CAMERA_JPEG_DAMAGED 1    /* decoded, but libjpeg warned: corrupt data */
#define CAMERA_JPEG_ERROR (-1)   /* not a JPEG, or a fatal error while decoding */
#define CAMERA_JPEG_TOO_LARGE (-2) /* the picture does not fit into the caller's buffer */

/* The largest side taken: no webcam sends more, and the caller's buffer is sized by it. */
#define CAMERA_JPEG_MAX_SIDE 4096

struct camera_jpeg_error {
    struct jpeg_error_mgr pub;
    jmp_buf jump;
};

static void camera_jpeg_error_exit(j_common_ptr cinfo) {
    struct camera_jpeg_error* err = (struct camera_jpeg_error*)cinfo->err;
    longjmp(err->jump, 1);
}

/* Warnings (level -1) are counted, not printed: the caller sees them in the return code. */
static void camera_jpeg_emit_message(j_common_ptr cinfo, int msg_level) {
    if (msg_level < 0) cinfo->err->num_warnings++;
}

static void camera_jpeg_output_message(j_common_ptr cinfo) { (void)cinfo; }

/*
 * Decodes the JPEG `src` (`src_len` bytes) into `dst` as RGBA rows of `width * 4` bytes and
 * writes the picture's size into `width` and `height`. `dst_len` is the size of `dst`: a
 * picture that needs more is not decoded (CAMERA_JPEG_TOO_LARGE, with its size written).
 */
int camera_jpeg_decode_rgba(const uint8_t* src, size_t src_len, uint8_t* dst, size_t dst_len,
                            uint32_t* width, uint32_t* height) {
    struct jpeg_decompress_struct cinfo;
    struct camera_jpeg_error err;
    int rc;

    *width = 0;
    *height = 0;
    if (src == NULL || src_len < 4 || dst == NULL) return CAMERA_JPEG_ERROR;

    cinfo.err = jpeg_std_error(&err.pub);
    err.pub.error_exit = camera_jpeg_error_exit;
    err.pub.emit_message = camera_jpeg_emit_message;
    err.pub.output_message = camera_jpeg_output_message;
    if (setjmp(err.jump)) {
        /* a fatal error inside libjpeg: what it allocated goes with the object */
        jpeg_destroy_decompress(&cinfo);
        return CAMERA_JPEG_ERROR;
    }
    jpeg_create_decompress(&cinfo);
    jpeg_mem_src(&cinfo, src, (unsigned long)src_len);
    if (jpeg_read_header(&cinfo, TRUE) != JPEG_HEADER_OK) {
        jpeg_destroy_decompress(&cinfo);
        return CAMERA_JPEG_ERROR;
    }
    if (cinfo.image_width == 0 || cinfo.image_height == 0 ||
        cinfo.image_width > CAMERA_JPEG_MAX_SIDE || cinfo.image_height > CAMERA_JPEG_MAX_SIDE) {
        jpeg_destroy_decompress(&cinfo);
        return CAMERA_JPEG_ERROR;
    }
    *width = cinfo.image_width;
    *height = cinfo.image_height;
    if ((size_t)cinfo.image_width * cinfo.image_height * 4 > dst_len) {
        jpeg_destroy_decompress(&cinfo);
        return CAMERA_JPEG_TOO_LARGE;
    }
    cinfo.out_color_space = JCS_EXT_RGBA;
    /* the fast integer IDCT and the plain chroma upsampling: a webcam frame needs neither the
       slow IDCT's last bit nor the smoothed chroma, and a live frame has 33 ms */
    cinfo.dct_method = JDCT_IFAST;
    cinfo.do_fancy_upsampling = FALSE;
    jpeg_start_decompress(&cinfo);
    const size_t stride = (size_t)cinfo.output_width * 4;
    while (cinfo.output_scanline < cinfo.output_height) {
        JSAMPROW row = dst + (size_t)cinfo.output_scanline * stride;
        if (jpeg_read_scanlines(&cinfo, &row, 1) != 1) break;
    }
    rc = cinfo.output_scanline == cinfo.output_height ? CAMERA_JPEG_OK : CAMERA_JPEG_DAMAGED;
    jpeg_finish_decompress(&cinfo);
    if (err.pub.num_warnings > 0) rc = CAMERA_JPEG_DAMAGED;
    jpeg_destroy_decompress(&cinfo);
    return rc;
}
