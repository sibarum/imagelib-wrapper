/*
 * imagelib — one C ABI over every image format the GUI can show.
 *
 * Hand-written, not generated: this header is the contract, and the Rust in ../imagelib/src/lib.rs
 * and the Java downcalls in ../../src/main/java/sibarum/imagelib/CApi.java are both written against
 * it. Twenty-one functions, no structs crossing the boundary, two opaque handles.
 *
 * Pixels are always straight (non-premultiplied) RGBA8, sRGB, tightly packed, top row first.
 *
 * Failure is uniform: a function returning a pointer returns NULL, one returning a status returns 0,
 * and img_last_error() then says why on this thread.
 */
#ifndef IMAGELIB_H
#define IMAGELIB_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* An ordered run of frames of one size. A still is a run of one, which is what keeps "still" and
 * "animated" from being two code paths in the caller. */
typedef struct ImgFrames ImgFrames;

#define IMG_UNKNOWN 0
#define IMG_RASTER  1
#define IMG_VECTOR  2

/* Which verb these bytes want. Magic numbers decide raster; an SVG text sniff runs only after. */
int32_t img_probe(const uint8_t *data, size_t len);

/* Intrinsic px size of an SVG, for a layout that wants to know before it chooses a box.
 * Returns 1 on success, 0 on failure. */
int32_t img_svg_size(const uint8_t *data, size_t len, float *w_out, float *h_out);

/* Decode a raster file — PNG, JPEG, GIF, WebP, BMP, ICO, TIFF, TGA, PNM, QOI — animated or not.
 * Animation frames arrive fully composited. NULL on failure. Free with img_free. */
ImgFrames *img_decode(const uint8_t *data, size_t len);

/* Rasterise an SVG at exactly width x height px, scaling each axis to fill.
 * NULL on failure. Free with img_free. */
ImgFrames *img_render_svg(const uint8_t *data, size_t len, uint32_t width, uint32_t height);

uint32_t img_width(const ImgFrames *frames);
uint32_t img_height(const ImgFrames *frames);

/* Frame count; 1 for a still. */
uint32_t img_count(const ImgFrames *frames);

/* Frame i's display time in ms, or 0 if the file gave none. */
uint32_t img_delay_ms(const ImgFrames *frames, uint32_t i);

/* Frame i's width*height*4 bytes of RGBA8, owned by `frames`. NULL if i is out of range. */
const uint8_t *img_pixels(const ImgFrames *frames, uint32_t i);

void img_free(ImgFrames *frames);

/* --- streaming ---------------------------------------------------------------------------------
 *
 * A raster file played rather than decoded whole: frames come one at a time, each composited onto the
 * one before, so one canvas is held however long the animation. The timeline (count, delays, plays)
 * is read at open without decoding a pixel. A still opens as a run of one. */
typedef struct ImgAnim ImgAnim;

/* Open a raster file to play. The bytes are copied. NULL on failure. Close with img_anim_close. */
ImgAnim *img_anim_open(const uint8_t *data, size_t len);

uint32_t img_anim_width(const ImgAnim *anim);
uint32_t img_anim_height(const ImgAnim *anim);

/* Frames by the file's own account (1 for a still), and frame i's delay in ms, 0 if it gave none. */
uint32_t img_anim_count(const ImgAnim *anim);
uint32_t img_anim_delay_ms(const ImgAnim *anim, uint32_t i);

/* Plays the file asks for: 0 forever, 1 for a still. */
uint32_t img_anim_loops(const ImgAnim *anim);

/* Decode the next frame: 1 there is one, 0 the run has ended, -1 failed. */
int32_t img_anim_next(ImgAnim *anim);

/* The frame img_anim_next last decoded, width*height*4 bytes of RGBA8, owned by `anim` and valid
 * until its next img_anim_next, img_anim_rewind or img_anim_close. NULL before the first. */
const uint8_t *img_anim_pixels(const ImgAnim *anim);

/* Back to before frame 0. Returns 1 on success, 0 on failure. */
int32_t img_anim_rewind(ImgAnim *anim);

void img_anim_close(ImgAnim *anim);

/* Why the last call on this thread failed, or NULL. Valid until the next failing call on it. */
const char *img_last_error(void);

#ifdef __cplusplus
}
#endif

#endif /* IMAGELIB_H */
