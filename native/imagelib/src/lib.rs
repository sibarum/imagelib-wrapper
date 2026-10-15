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
use std::sync::Arc;

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::metadata::LoopCount;
use image::{AnimationDecoder, ImageDecoder, ImageFormat, ImageReader};
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

// --- streaming --------------------------------------------------------------------------------

/// A raster file opened to be played rather than decoded whole: its frames come one at a time, each
/// composited onto the one before, so however long the animation and however large its canvas, one
/// canvas is all that is held.
///
/// The handle owns a copy of the file's bytes, so the caller's buffer is free the moment
/// [`img_anim_open`] returns and [`img_anim_rewind`] can start the decoder again from them. Its
/// timeline (how many frames, each one's delay, how many plays) is read at open by walking the file's
/// chunks without decoding a pixel, so a player knows the length before it has shown frame 0.
pub struct ImgAnim {
    data: Arc<[u8]>,
    kind: Kind,
    width: u32,
    height: u32,
    loops: u32,
    delays: Vec<u32>,
    /// The decoder, positioned after the last frame handed out; None for a still, which has none.
    frames: Option<image::Frames<'static>>,
    /// The frame last handed out, empty before the first.
    current: Vec<u8>,
    /// How many frames have been handed out since open or the last rewind.
    taken: u32,
}

#[derive(Clone, Copy)]
enum Kind {
    Gif,
    WebP,
    Apng,
    Still,
}

type Bytes = Cursor<Arc<[u8]>>;

/// Open a raster file to be played frame by frame. A still opens as an animation of one frame, so a
/// player needs no second path for it. Null on failure — see [`img_last_error`].
///
/// # Safety
/// `data` must point to `len` readable bytes; they are copied, and need not outlive the call. The
/// result is owned by the caller and closed with [`img_anim_close`].
#[no_mangle]
pub unsafe extern "C" fn img_anim_open(data: *const u8, len: usize) -> *mut ImgAnim {
    guard(ptr::null_mut(), || {
        let d: Arc<[u8]> = Arc::from(bytes(data, len)?);
        Ok(Box::into_raw(Box::new(open(d)?)))
    })
}

fn open(data: Arc<[u8]>) -> Result<ImgAnim, String> {
    let format = ImageReader::new(Cursor::new(&data[..]))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .format()
        .ok_or("unrecognised image format")?;
    let kind = match format {
        ImageFormat::Gif => Kind::Gif,
        ImageFormat::WebP => {
            let dec = WebPDecoder::new(Cursor::new(&data[..])).map_err(|e| e.to_string())?;
            if dec.has_animation() { Kind::WebP } else { Kind::Still }
        }
        ImageFormat::Png => {
            let dec = PngDecoder::new(Cursor::new(&data[..])).map_err(|e| e.to_string())?;
            if dec.is_apng().map_err(|e| e.to_string())? { Kind::Apng } else { Kind::Still }
        }
        _ => Kind::Still,
    };
    let (width, height, loops, frames) = start(kind, &data)?;
    let delays = match kind {
        Kind::Gif => scan_gif(&data),
        Kind::WebP => scan_webp(&data),
        Kind::Apng => scan_apng(&data),
        Kind::Still => vec![0],
    };
    if delays.is_empty() {
        return Err("the file declared an animation with no frames".into());
    }
    Ok(ImgAnim { data, kind, width, height, loops, delays, frames, current: Vec::new(), taken: 0 })
}

/// A decoder at frame 0, with the canvas size and play count it reports. A still has no decoder: it
/// is decoded whole when its one frame is asked for.
#[allow(clippy::type_complexity)]
fn start(kind: Kind, data: &Arc<[u8]>) -> Result<(u32, u32, u32, Option<image::Frames<'static>>), String> {
    fn plays(count: LoopCount) -> u32 {
        match count {
            LoopCount::Infinite => 0,
            LoopCount::Finite(n) => n.get(),
        }
    }
    let cursor: Bytes = Cursor::new(Arc::clone(data));
    let e = |e: image::ImageError| e.to_string();
    Ok(match kind {
        Kind::Gif => {
            let dec = GifDecoder::new(cursor).map_err(e)?;
            let (w, h) = dec.dimensions();
            (w, h, plays(dec.loop_count()), Some(dec.into_frames()))
        }
        Kind::WebP => {
            let dec = WebPDecoder::new(cursor).map_err(e)?;
            let (w, h) = dec.dimensions();
            (w, h, plays(dec.loop_count()), Some(dec.into_frames()))
        }
        Kind::Apng => {
            let dec = PngDecoder::new(cursor).map_err(e)?;
            let (w, h) = dec.dimensions();
            let apng = dec.apng().map_err(e)?;
            (w, h, plays(apng.loop_count()), Some(apng.into_frames()))
        }
        Kind::Still => {
            let (w, h) = ImageReader::new(Cursor::new(&data[..]))
                .with_guessed_format()
                .map_err(|e| e.to_string())?
                .into_dimensions()
                .map_err(e)?;
            (w, h, 1, None)
        }
    })
}

/// # Safety
/// `h` must be a live handle from [`img_anim_open`].
#[no_mangle]
pub unsafe extern "C" fn img_anim_width(h: *const ImgAnim) -> u32 {
    h.as_ref().map_or(0, |a| a.width)
}

/// # Safety
/// `h` must be a live handle from [`img_anim_open`].
#[no_mangle]
pub unsafe extern "C" fn img_anim_height(h: *const ImgAnim) -> u32 {
    h.as_ref().map_or(0, |a| a.height)
}

/// How many frames the file holds, by its own account; 1 for a still. A damaged file may end sooner,
/// and [`img_anim_next`] is what says so.
///
/// # Safety
/// `h` must be a live handle from [`img_anim_open`].
#[no_mangle]
pub unsafe extern "C" fn img_anim_count(h: *const ImgAnim) -> u32 {
    h.as_ref().map_or(0, |a| a.delays.len() as u32)
}

/// Frame `i`'s display time in ms, or 0 if the file gave none, read without decoding it.
///
/// # Safety
/// `h` must be a live handle from [`img_anim_open`].
#[no_mangle]
pub unsafe extern "C" fn img_anim_delay_ms(h: *const ImgAnim, i: u32) -> u32 {
    match h.as_ref() {
        Some(a) => a.delays.get(i as usize).copied().unwrap_or(0),
        None => 0,
    }
}

/// How many times the file asks to be played through: 0 for forever, 1 for a still.
///
/// # Safety
/// `h` must be a live handle from [`img_anim_open`].
#[no_mangle]
pub unsafe extern "C" fn img_anim_loops(h: *const ImgAnim) -> u32 {
    h.as_ref().map_or(0, |a| a.loops)
}

/// Decode the next frame, composited onto the ones before it. 1 when there is one (read it with
/// [`img_anim_pixels`]), 0 when the run has ended, -1 on failure — see [`img_last_error`].
///
/// # Safety
/// `h` must be a live handle from [`img_anim_open`], used by one thread at a time.
#[no_mangle]
pub unsafe extern "C" fn img_anim_next(h: *mut ImgAnim) -> i32 {
    guard(-1, || {
        let a = h.as_mut().ok_or("null handle")?;
        let frame = match a.kind {
            Kind::Still => {
                if a.taken > 0 {
                    return Ok(0);
                }
                still(&a.data)?.planes.swap_remove(0)
            }
            _ => match a.frames.as_mut().and_then(Iterator::next) {
                None => return Ok(0),
                Some(Err(e)) => return Err(e.to_string()),
                Some(Ok(f)) => {
                    let buffer = f.into_buffer();
                    if buffer.width() != a.width || buffer.height() != a.height {
                        return Err(format!(
                            "frame {} is {}x{} on a {}x{} canvas",
                            a.taken,
                            buffer.width(),
                            buffer.height(),
                            a.width,
                            a.height
                        ));
                    }
                    buffer.into_raw()
                }
            },
        };
        a.current = frame;
        a.taken += 1;
        Ok(1)
    })
}

/// The frame [`img_anim_next`] last decoded: `width * height * 4` bytes of straight RGBA8, owned by
/// `h` and valid until its next call to `img_anim_next`, `img_anim_rewind` or `img_anim_close`. Null
/// before the first frame.
///
/// # Safety
/// `h` must be a live handle from [`img_anim_open`].
#[no_mangle]
pub unsafe extern "C" fn img_anim_pixels(h: *const ImgAnim) -> *const u8 {
    match h.as_ref() {
        Some(a) if !a.current.is_empty() => a.current.as_ptr(),
        _ => ptr::null(),
    }
}

/// Go back to before frame 0: the next [`img_anim_next`] decodes it again. A frame of GIF or APNG is
/// drawn over the ones before it, so going back is starting again; the bytes are already here, so it
/// costs a decoder and nothing else. 1 on success, 0 on failure.
///
/// # Safety
/// `h` must be a live handle from [`img_anim_open`], used by one thread at a time.
#[no_mangle]
pub unsafe extern "C" fn img_anim_rewind(h: *mut ImgAnim) -> i32 {
    guard(0, || {
        let a = h.as_mut().ok_or("null handle")?;
        a.frames = None;
        a.frames = start(a.kind, &a.data)?.3;
        a.current = Vec::new();
        a.taken = 0;
        Ok(1)
    })
}

/// Release a handle. Null is accepted and does nothing.
///
/// # Safety
/// `h` must be a handle from [`img_anim_open`] that has not already been closed.
#[no_mangle]
pub unsafe extern "C" fn img_anim_close(h: *mut ImgAnim) {
    if !h.is_null() {
        drop(Box::from_raw(h));
    }
}

// --- the timeline, without the pixels ---------------------------------------------------------
//
// Each scan walks its container's blocks and reads the delays the decoder will report, in the same
// units and with the same rounding, so the times a player is told at open are the times its frames
// later arrive with. A truncated or damaged file yields the frames found before the damage.

fn u16le(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(*d.get(at)?) | u32::from(*d.get(at + 1)?) << 8)
}

fn u24le(d: &[u8], at: usize) -> Option<u32> {
    Some(u16le(d, at)? | u32::from(*d.get(at + 2)?) << 16)
}

fn u32le(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn u16be(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?)))
}

fn u32be(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// GIF: each image descriptor is a frame, timed by the graphic control extension before it, in
/// hundredths of a second; one without is 0.
fn scan_gif(d: &[u8]) -> Vec<u32> {
    fn skip_sub_blocks(d: &[u8], mut at: usize) -> Option<usize> {
        loop {
            let n = *d.get(at)? as usize;
            at += 1 + n;
            if n == 0 {
                return Some(at);
            }
        }
    }
    fn walk(d: &[u8], delays: &mut Vec<u32>) -> Option<()> {
        let packed = *d.get(10)?;
        let mut at = 13;
        if packed & 0x80 != 0 {
            at += 3 << ((packed & 7) + 1);
        }
        let mut delay = 0;
        loop {
            match *d.get(at)? {
                0x21 => {
                    if *d.get(at + 1)? == 0xF9 {
                        delay = u16le(d, at + 4)? * 10;
                    }
                    at = skip_sub_blocks(d, at + 2)?;
                }
                0x2C => {
                    let local = *d.get(at + 9)?;
                    at += 10;
                    if local & 0x80 != 0 {
                        at += 3 << ((local & 7) + 1);
                    }
                    // The LZW minimum code size, then the image data.
                    at = skip_sub_blocks(d, at + 1)?;
                    delays.push(delay);
                    delay = 0;
                }
                _ => return Some(()),
            }
        }
    }
    let mut delays = Vec::new();
    walk(d, &mut delays);
    delays
}

/// WebP: each ANMF chunk is a frame, its duration in ms at bytes 12..15 of the chunk's payload.
fn scan_webp(d: &[u8]) -> Vec<u32> {
    let mut delays = Vec::new();
    let mut at = 12;
    while let (Some(fourcc), Some(size)) = (d.get(at..at + 4), u32le(d, at + 4)) {
        if fourcc == b"ANMF" {
            match u24le(d, at + 8 + 12) {
                Some(ms) => delays.push(ms),
                None => break,
            }
        }
        at += 8 + size as usize + (size as usize & 1);
    }
    delays
}

/// APNG: each fcTL chunk is a frame, its delay a fraction of a second at bytes 20..24 of the
/// payload, a denominator of 0 meaning 100.
fn scan_apng(d: &[u8]) -> Vec<u32> {
    let mut delays = Vec::new();
    let mut at = 8;
    while let (Some(size), Some(kind)) = (u32be(d, at), d.get(at + 4..at + 8)) {
        if kind == b"fcTL" {
            let (Some(num), Some(den)) = (u16be(d, at + 8 + 20), u16be(d, at + 8 + 22)) else {
                break;
            };
            delays.push(num * 1000 / if den == 0 { 100 } else { den });
        }
        if kind == b"IEND" {
            break;
        }
        at += 12 + size as usize;
    }
    delays
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

#[cfg(test)]
mod tests {
    //! The streaming handle against the eager decoder, on animations made here: the same frames in
    //! the same order with the same delays, and a scanned timeline that agrees with what decoding
    //! later reports. `write_fixtures` puts the same files where the Java tests read them.

    use super::*;
    use image::codecs::gif::GifEncoder;
    use image::codecs::webp::WebPEncoder;
    use image::{Delay, ExtendedColorType, Frame, RgbaImage};

    const W: u32 = 4;
    const H: u32 = 3;
    /// Three frames of one colour each, at 30, 50 and 70 ms: distinct enough that a frame out of
    /// order, or a delay read from the wrong frame, shows.
    const COLOURS: [[u8; 4]; 3] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];
    const DELAYS: [u32; 3] = [30, 50, 70];

    fn plane(c: [u8; 4]) -> Vec<u8> {
        c.repeat((W * H) as usize)
    }

    fn gif() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut out);
            enc.set_repeat(image::codecs::gif::Repeat::Infinite).unwrap();
            for (c, d) in COLOURS.iter().zip(DELAYS) {
                let img = RgbaImage::from_raw(W, H, plane(*c)).unwrap();
                enc.encode_frame(Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(d, 1))).unwrap();
            }
        }
        out
    }

    fn apng() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, W, H);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_animated(COLOURS.len() as u32, 2).unwrap();
            let mut w = enc.write_header().unwrap();
            for (c, d) in COLOURS.iter().zip(DELAYS) {
                w.set_frame_delay(d as u16, 1000).unwrap();
                w.write_image_data(&plane(*c)).unwrap();
            }
        }
        out
    }

    /// `image` writes only still WebP, so each frame is encoded as one and its VP8L chunk wrapped in
    /// an ANMF; the container around them is the extended format's, with the animation flag set.
    fn webp() -> Vec<u8> {
        fn chunk(fourcc: &[u8], payload: &[u8]) -> Vec<u8> {
            let mut c = fourcc.to_vec();
            c.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            c.extend_from_slice(payload);
            if payload.len() % 2 == 1 {
                c.push(0);
            }
            c
        }
        fn u24(v: u32) -> [u8; 3] {
            [v as u8, (v >> 8) as u8, (v >> 16) as u8]
        }
        let mut body = b"WEBP".to_vec();
        let mut vp8x = vec![0x12, 0, 0, 0];
        vp8x.extend_from_slice(&u24(W - 1));
        vp8x.extend_from_slice(&u24(H - 1));
        body.extend(chunk(b"VP8X", &vp8x));
        body.extend(chunk(b"ANIM", &[0, 0, 0, 0, 0, 0]));
        for (c, d) in COLOURS.iter().zip(DELAYS) {
            let mut still = Vec::new();
            WebPEncoder::new_lossless(&mut still).encode(&plane(*c), W, H, ExtendedColorType::Rgba8).unwrap();
            let vp8l = &still[12..];
            assert_eq!(&vp8l[..4], b"VP8L", "a lossless still is one VP8L chunk");
            let mut frame = Vec::new();
            frame.extend_from_slice(&[0; 6]);
            frame.extend_from_slice(&u24(W - 1));
            frame.extend_from_slice(&u24(H - 1));
            frame.extend_from_slice(&u24(d));
            frame.push(0b10);
            frame.extend_from_slice(vp8l);
            body.extend(chunk(b"ANMF", &frame));
        }
        chunk(b"RIFF", &body)
    }

    /// Every frame `h` gives, with its scanned delay, read through the C ABI as Java reads it.
    fn stream(d: &[u8]) -> (u32, Vec<(Vec<u8>, u32)>) {
        unsafe {
            let h = img_anim_open(d.as_ptr(), d.len());
            assert!(!h.is_null(), "open: {:?}", LAST_ERROR.with(|e| e.borrow().clone()));
            let size = (img_anim_width(h) * img_anim_height(h) * 4) as usize;
            let mut frames = Vec::new();
            for pass in 0..2 {
                let mut i = 0;
                while img_anim_next(h) == 1 {
                    let px = slice::from_raw_parts(img_anim_pixels(h), size).to_vec();
                    if pass == 0 {
                        frames.push((px, img_anim_delay_ms(h, i)));
                    } else {
                        assert_eq!(frames[i as usize].0, px, "frame {i} again after a rewind");
                    }
                    i += 1;
                }
                assert_eq!(i, img_anim_count(h), "the scan counted the frames the decoder gave");
                assert_eq!(img_anim_rewind(h), 1);
            }
            let loops = img_anim_loops(h);
            img_anim_close(h);
            (loops, frames)
        }
    }

    fn agrees_with_decode(d: &[u8]) -> u32 {
        let eager = decode(d).unwrap();
        let (loops, streamed) = stream(d);
        assert_eq!(streamed.len(), eager.planes.len());
        for (i, (px, delay)) in streamed.iter().enumerate() {
            assert_eq!(px, &eager.planes[i], "frame {i}'s pixels");
            assert_eq!(*delay, eager.delays[i], "frame {i}'s scanned delay is the decoded one");
            assert_eq!(*delay, DELAYS[i]);
            assert_eq!(px[..4], COLOURS[i]);
        }
        loops
    }

    #[test]
    fn a_gif_streams_as_it_decodes() {
        assert_eq!(agrees_with_decode(&gif()), 0, "repeat forever is 0");
    }

    #[test]
    fn an_apng_streams_as_it_decodes() {
        assert_eq!(agrees_with_decode(&apng()), 2);
    }

    #[test]
    fn a_webp_streams_as_it_decodes() {
        agrees_with_decode(&webp());
    }

    #[test]
    fn a_still_streams_as_one_frame() {
        let mut png = Vec::new();
        RgbaImage::from_raw(W, H, plane(COLOURS[0]))
            .unwrap()
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let (loops, frames) = stream(&png);
        assert_eq!((loops, frames.len(), frames[0].1), (1, 1, 0));
        assert_eq!(frames[0].0, plane(COLOURS[0]));
    }

    #[test]
    fn a_truncated_gif_scans_the_frames_before_the_damage() {
        let whole = gif();
        let cut = &whole[..whole.len() - 20];
        assert!(scan_gif(cut).len() < 3);
    }

    #[test]
    #[ignore = "run once to write the Java tests' fixtures: cargo test -- --ignored write_fixtures"]
    fn write_fixtures() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/test/resources/");
        std::fs::write(format!("{dir}anim3.gif"), gif()).unwrap();
        std::fs::write(format!("{dir}anim3.apng.png"), apng()).unwrap();
        std::fs::write(format!("{dir}anim3.webp"), webp()).unwrap();
    }
}
