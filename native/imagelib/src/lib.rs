//! One C ABI over every image format the GUI can show.
//!
//! The whole point of this crate is that the caller never learns a format. Two verbs come out of it,
//! and they differ in who decides the pixel size:
//!
//! * [`img_decode`] — a raster file dictates its own size. PNG, JPEG, GIF, WebP, BMP, ICO, TIFF,
//!   TGA, PNM, QOI.
//! * [`img_render_svg`] — a vector file has no size until a layout gives it one, so the caller
//!   passes the box.
//!
//! Both return the same value: `ImgFrames`, an ordered run of RGBA8 planes with a delay each. A JPEG
//! is a one-frame animation, which is what keeps "still" and "animated" from being two code paths
//! anywhere above this library.
//!
//! Pixels are **straight (non-premultiplied) RGBA8, sRGB, tightly packed, top row first** — exactly
//! what a `VkImage` in `VK_FORMAT_R8G8B8A8_UNORM` wants uploaded, and exactly the byte order the Java
//! side already uses for the font atlas.

use std::cell::RefCell;
use std::ffi::{c_char, CString};
use std::io::Cursor;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::slice;

use image::{AnimationDecoder, ImageFormat, ImageReader};
use resvg::{tiny_skia, usvg};

// --- errors -----------------------------------------------------------------------------------

thread_local! {
    /// The last failure on this thread. Lives until the next failed call on this thread, which is
    /// the contract `img_last_error` documents — a caller reads it immediately or not at all.
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn fail(message: impl Into<String>) {
    let text = message.into();
    let c = CString::new(text).unwrap_or_else(|_| CString::new("error contained a NUL").unwrap());
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(c));
}

/// Why the last call on this thread returned null or 0, or null if it did not. UTF-8, NUL-terminated,
/// valid until the next failing call on this thread.
#[no_mangle]
pub extern "C" fn img_last_error() -> *const c_char {
    LAST_ERROR.with(|e| match &*e.borrow() {
        Some(c) => c.as_ptr(),
        None => ptr::null(),
    })
}

/// Run `f`, turning a panic into the null/0 failure the ABI already has. A panic unwinding into C is
/// undefined behaviour, and a decoder handed a corrupt file is exactly where one would come from.
fn guard<T>(fallback: T, f: impl FnOnce() -> Result<T, String>) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(value)) => value,
        Ok(Err(message)) => {
            fail(message);
            fallback
        }
        Err(_) => {
            fail("panic inside the native decoder");
            fallback
        }
    }
}

/// # Safety
/// `data` must point to `len` readable bytes.
unsafe fn bytes<'a>(data: *const u8, len: usize) -> Result<&'a [u8], String> {
    if data.is_null() {
        return Err("null input pointer".into());
    }
    if len == 0 {
        return Err("empty input".into());
    }
    Ok(slice::from_raw_parts(data, len))
}

// --- the value --------------------------------------------------------------------------------

/// An opaque run of frames. All frames share one size; every plane is `width * height * 4` bytes.
pub struct ImgFrames {
    width: u32,
    height: u32,
    /// Per-frame display time in milliseconds. 0 on a still, and 0 on an animated frame whose file
    /// gave no delay — a caller substitutes its own floor rather than being handed a made-up one.
    delays: Vec<u32>,
    planes: Vec<Vec<u8>>,
}

const PROBE_UNKNOWN: i32 = 0;
const PROBE_RASTER: i32 = 1;
const PROBE_VECTOR: i32 = 2;

// --- probing ----------------------------------------------------------------------------------

/// Which verb these bytes want: 1 raster, 2 vector, 0 unknown.
///
/// Magic numbers first, because they are decisive; the SVG sniff is a text scan and only runs once no
/// raster format has claimed the bytes. That ordering is what stops SVG-shaped text inside some other
/// container from being mistaken for a document.
///
/// # Safety
/// `data` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn img_probe(data: *const u8, len: usize) -> i32 {
    guard(PROBE_UNKNOWN, || {
        let d = bytes(data, len)?;
        if image::guess_format(d).is_ok() {
            return Ok(PROBE_RASTER);
        }
        if looks_like_svg(d) {
            return Ok(PROBE_VECTOR);
        }
        Err("unrecognised image format".into())
    })
}

fn looks_like_svg(d: &[u8]) -> bool {
    // .svgz is a gzipped document and usvg decompresses it for us. Nothing else we accept is gzip.
    if d.len() >= 2 && d[0] == 0x1f && d[1] == 0x8b {
        return true;
    }
    let head = &d[..d.len().min(1024)];
    String::from_utf8_lossy(head).to_ascii_lowercase().contains("<svg")
}

/// The intrinsic size of an SVG in px. Writes `w_out`/`h_out` and returns 1; returns 0 and sets the
/// error on failure.
///
/// Here so a layout can ask what size a document wants to be *before* deciding what size to give it.
///
/// # Safety
/// `data` must point to `len` readable bytes; `w_out` and `h_out` must be writable `f32`s.
#[no_mangle]
pub unsafe extern "C" fn img_svg_size(
    data: *const u8,
    len: usize,
    w_out: *mut f32,
    h_out: *mut f32,
) -> i32 {
    guard(0, || {
        let d = bytes(data, len)?;
        if w_out.is_null() || h_out.is_null() {
            return Err("null output pointer".into());
        }
        let size = parse_svg(d, None)?.size();
        *w_out = size.width();
        *h_out = size.height();
        Ok(1)
    })
}

// --- raster -----------------------------------------------------------------------------------

/// Decode a raster file, animated or not. Null on failure — see [`img_last_error`].
///
/// # Safety
/// `data` must point to `len` readable bytes. The result is owned by the caller and freed with
/// [`img_free`].
#[no_mangle]
pub unsafe extern "C" fn img_decode(data: *const u8, len: usize) -> *mut ImgFrames {
    guard(ptr::null_mut(), || {
        let d = bytes(data, len)?;
        Ok(Box::into_raw(Box::new(decode(d)?)))
    })
}

fn decode(d: &[u8]) -> Result<ImgFrames, String> {
    let reader = ImageReader::new(Cursor::new(d))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    // Only three of our formats can hold more than one frame, and each has to be asked differently.
    // Everything else is a still by construction, so it takes the one path that cannot animate.
    match reader.format() {
        Some(ImageFormat::Gif) => {
            let dec = image::codecs::gif::GifDecoder::new(Cursor::new(d)).map_err(|e| e.to_string())?;
            animation(dec)
        }
        Some(ImageFormat::WebP) => {
            let dec = image::codecs::webp::WebPDecoder::new(Cursor::new(d)).map_err(|e| e.to_string())?;
            if dec.has_animation() {
                animation(dec)
            } else {
                still(d)
            }
        }
        Some(ImageFormat::Png) => {
            let dec = image::codecs::png::PngDecoder::new(Cursor::new(d)).map_err(|e| e.to_string())?;
            if dec.is_apng().map_err(|e| e.to_string())? {
                animation(dec.apng().map_err(|e| e.to_string())?)
            } else {
                still(d)
            }
        }
        Some(_) => still(d),
        None => Err("unrecognised image format".into()),
    }
}

/// Frames as the decoder composites them: full-canvas RGBA, disposal and blending already applied.
///
/// That compositing is the whole reason animation lives down here rather than in the GUI. GIF's
/// restore-to-previous and WebP's blend rules are format knowledge, and format knowledge stops here.
fn animation<'a, D: AnimationDecoder<'a>>(dec: D) -> Result<ImgFrames, String> {
    let frames = dec.into_frames().collect_frames().map_err(|e| e.to_string())?;
    let first = frames.first().ok_or("the file declared an animation with no frames")?;
    let (width, height) = (first.buffer().width(), first.buffer().height());
    let mut delays = Vec::with_capacity(frames.len());
    let mut planes = Vec::with_capacity(frames.len());
    for frame in frames {
        let (numer, denom) = frame.delay().numer_denom_ms();
        delays.push(if denom == 0 { 0 } else { numer / denom });
        planes.push(frame.into_buffer().into_raw());
    }
    Ok(ImgFrames { width, height, delays, planes })
}

fn still(d: &[u8]) -> Result<ImgFrames, String> {
    let img = ImageReader::new(Cursor::new(d))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?
        .to_rgba8();
    let (width, height) = (img.width(), img.height());
    Ok(ImgFrames { width, height, delays: vec![0], planes: vec![img.into_raw()] })
}

// --- vector -----------------------------------------------------------------------------------

/// Rasterise an SVG at exactly `width` x `height` px. Null on failure — see [`img_last_error`].
///
/// The document is scaled to fill the box on each axis independently. A caller that wants the aspect
/// ratio kept asks [`img_svg_size`] and picks a box that has it; deciding that here would be this
/// library guessing at a layout it cannot see.
///
/// # Safety
/// `data` must point to `len` readable bytes. The result is owned by the caller and freed with
/// [`img_free`].
#[no_mangle]
pub unsafe extern "C" fn img_render_svg(
    data: *const u8,
    len: usize,
    width: u32,
    height: u32,
) -> *mut ImgFrames {
    guard(ptr::null_mut(), || {
        let d = bytes(data, len)?;
        if width == 0 || height == 0 {
            return Err(format!("a zero-sized raster was asked for: {width}x{height}"));
        }
        Ok(Box::into_raw(Box::new(render_svg(d, width, height)?)))
    })
}

/// Parse a document. `viewport` is the size to assume when the document declares none — a document
/// with no `width`/`height` and no `viewBox` is saying the caller decides, and when rasterising, the
/// caller has: it passed a box. Falling back to usvg's arbitrary 100x100 and then scaling that to the
/// box would resample for no reason. `None` leaves the fallback alone, which is what asking a
/// document its own size wants.
///
/// Note this cannot rescue a document that declares a size of *zero* — `width="0"` is an explicit
/// answer, not an absent one, and a symbol sprite that says it is zero-sized is a library to `use`
/// rather than a picture to draw. That stays an error, because drawing it would produce a blank
/// texture and call it success.
fn parse_svg(d: &[u8], viewport: Option<(u32, u32)>) -> Result<usvg::Tree, String> {
    let mut options = usvg::Options::default();
    if let Some(size) = viewport.and_then(|(w, h)| usvg::Size::from_wh(w as f32, h as f32)) {
        options.default_size = size;
    }
    // Text in an SVG is set with the reader's fonts, since a document names families rather than
    // carrying them. A document with no text pays nothing for this.
    options.fontdb_mut().load_system_fonts();
    usvg::Tree::from_data(d, &options).map_err(|e| e.to_string())
}

fn render_svg(d: &[u8], width: u32, height: u32) -> Result<ImgFrames, String> {
    let tree = parse_svg(d, Some((width, height)))?;
    let size = tree.size();
    if size.width() <= 0.0 || size.height() <= 0.0 {
        return Err("the document has no intrinsic size to scale from".into());
    }
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| format!("could not allocate a {width}x{height} pixmap"))?;
    let scale = tiny_skia::Transform::from_scale(
        width as f32 / size.width(),
        height as f32 / size.height(),
    );
    resvg::render(&tree, scale, &mut pixmap.as_mut());
    let mut plane = pixmap.take();
    demultiply(&mut plane);
    Ok(ImgFrames { width, height, delays: vec![0], planes: vec![plane] })
}

/// tiny-skia composites premultiplied; a sampler reading a premultiplied texel as straight alpha
/// darkens every soft edge. Undoing it here keeps one pixel convention across both verbs.
fn demultiply(plane: &mut [u8]) {
    for px in plane.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a == 0 {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
        } else if a < 255 {
            for c in &mut px[..3] {
                *c = (((*c as u32) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

// --- reading the value ------------------------------------------------------------------------

/// # Safety
/// `h` must be a live handle from [`img_decode`] or [`img_render_svg`].
#[no_mangle]
pub unsafe extern "C" fn img_width(h: *const ImgFrames) -> u32 {
    h.as_ref().map_or(0, |f| f.width)
}

/// # Safety
/// `h` must be a live handle from [`img_decode`] or [`img_render_svg`].
#[no_mangle]
pub unsafe extern "C" fn img_height(h: *const ImgFrames) -> u32 {
    h.as_ref().map_or(0, |f| f.height)
}

/// How many frames — 1 for a still, so nothing above needs to ask which it has.
///
/// # Safety
/// `h` must be a live handle from [`img_decode`] or [`img_render_svg`].
#[no_mangle]
pub unsafe extern "C" fn img_count(h: *const ImgFrames) -> u32 {
    h.as_ref().map_or(0, |f| f.planes.len() as u32)
}

/// Frame `i`'s display time in ms, or 0 if it has none.
///
/// # Safety
/// `h` must be a live handle from [`img_decode`] or [`img_render_svg`].
#[no_mangle]
pub unsafe extern "C" fn img_delay_ms(h: *const ImgFrames, i: u32) -> u32 {
    match h.as_ref() {
        Some(f) => f.delays.get(i as usize).copied().unwrap_or(0),
        None => 0,
    }
}

/// Frame `i`'s pixels: `width * height * 4` bytes of straight RGBA8. Null if `i` is out of range.
/// The pointer is owned by `h` and dies with it.
///
/// # Safety
/// `h` must be a live handle from [`img_decode`] or [`img_render_svg`].
#[no_mangle]
pub unsafe extern "C" fn img_pixels(h: *const ImgFrames, i: u32) -> *const u8 {
    match h.as_ref() {
        Some(f) => f.planes.get(i as usize).map_or(ptr::null(), |p| p.as_ptr()),
        None => ptr::null(),
    }
}

/// Release a handle. Null is accepted and does nothing.
///
/// # Safety
/// `h` must be a handle from [`img_decode`] or [`img_render_svg`] that has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn img_free(h: *mut ImgFrames) {
    if !h.is_null() {
        drop(Box::from_raw(h));
    }
}
