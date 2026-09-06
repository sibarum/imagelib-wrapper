package sibarum.imagelib;

import java.io.IOException;
import java.io.UncheckedIOException;
import java.lang.foreign.Arena;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.ValueLayout;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * Pixels, from anything.
 *
 * <p>Two verbs, and they differ in <b>who decides the size</b> — which is the one distinction that
 * cannot be papered over, and so the one this API keeps:
 *
 * <ul>
 *   <li>{@link #decode} — a raster file dictates its own pixel size. PNG, JPEG, GIF, WebP, BMP, ICO,
 *       TIFF, TGA, PNM, QOI, still or animated.</li>
 *   <li>{@link #rasterize} — a vector document has no size until a layout gives it one, so the
 *       caller passes the box. Re-rasterise when the box changes; that is what resolution
 *       independence costs and all that it costs.</li>
 * </ul>
 *
 * <p>Everything else is collapsed. Both verbs return {@link Frames}, so a JPEG is a one-frame
 * animation and no consumer branches on stillness to find its pixels. {@link #probe} answers which
 * verb some bytes want, so even the dispatch needs no knowledge of formats:
 *
 * <pre>{@code
 * byte[] bytes = Files.readAllBytes(path);
 * Frames frames = switch (Imagelib.probe(bytes)) {
 *     case RASTER -> Imagelib.decode(bytes);
 *     case VECTOR -> Imagelib.rasterize(bytes, box.width(), box.height());
 *     case UNKNOWN -> placeholder();
 * };
 * }</pre>
 *
 * <p>Thread-safe: the native side holds no state across calls beyond a thread-local error string, so
 * decodes on different threads do not interact. Every call copies the pixels out and frees the
 * native handle before returning, so nothing here outlives its own frame.
 *
 * <p>Callers need {@code --enable-native-access=ALL-UNNAMED} (or their module) on the command line.
 */
public final class Imagelib {

    /** Which verb a run of bytes wants — see {@link #probe}. */
    public enum Kind {
        /** A raster file: hand it to {@link #decode}. */
        RASTER,
        /** An SVG document: hand it to {@link #rasterize} with a box. */
        VECTOR,
        /** Nothing recognised the bytes. */
        UNKNOWN
    }

    /** A vector document's intrinsic size in px — what it would like to be, before layout decides. */
    public record Size(float width, float height) {
    }

    // --- classification -------------------------------------------------------------------------

    /**
     * Which verb {@code bytes} wants. Raster formats are identified by magic number; SVG by a text
     * sniff of the first kilobyte, which runs only once no raster format has claimed the bytes.
     * Never throws — unrecognised input is {@link Kind#UNKNOWN}, because "what is this" is a
     * question, not a failure.
     */
    public static Kind probe(byte[] bytes) {
        if (bytes == null || bytes.length == 0) {
            return Kind.UNKNOWN;
        }
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment in = arena.allocateFrom(ValueLayout.JAVA_BYTE, bytes);
            int kind;
            try {
                kind = (int) CApi.img_probe.invokeExact(in, (long) bytes.length);
            } catch (Throwable t) {
                throw downcallFailed("img_probe", t);
            }
            return switch (kind) {
                case 1 -> Kind.RASTER;
                case 2 -> Kind.VECTOR;
                default -> Kind.UNKNOWN;
            };
        }
    }

    /** The intrinsic px size of an SVG document. Throws {@link ImageException} if it will not parse. */
    public static Size svgSize(byte[] svg) {
        requireBytes(svg);
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment in = arena.allocateFrom(ValueLayout.JAVA_BYTE, svg);
            MemorySegment w = arena.allocate(ValueLayout.JAVA_FLOAT);
            MemorySegment h = arena.allocate(ValueLayout.JAVA_FLOAT);
            int ok;
            try {
                ok = (int) CApi.img_svg_size.invokeExact(in, (long) svg.length, w, h);
            } catch (Throwable t) {
                throw downcallFailed("img_svg_size", t);
            }
            if (ok == 0) {
                throw new ImageException("could not read the SVG's size: " + lastError());
            }
            return new Size(w.get(ValueLayout.JAVA_FLOAT, 0), h.get(ValueLayout.JAVA_FLOAT, 0));
        }
    }

    // --- the two verbs --------------------------------------------------------------------------

    /**
     * Decode a raster file at its own size, animated or not. Animation frames arrive fully
     * composited — GIF disposal and WebP blending are applied by the decoder, so a consumer draws
     * frame {@code i} and never reasons about frame {@code i - 1}.
     */
    public static Frames decode(byte[] bytes) {
        requireBytes(bytes);
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment in = arena.allocateFrom(ValueLayout.JAVA_BYTE, bytes);
            MemorySegment handle;
            try {
                handle = (MemorySegment) CApi.img_decode.invokeExact(in, (long) bytes.length);
            } catch (Throwable t) {
                throw downcallFailed("img_decode", t);
            }
            return collect(handle, "decode failed");
        }
    }

    /** {@link #decode(byte[])} of a file's contents. */
    public static Frames decode(Path file) {
        return decode(readAll(file));
    }

    /**
     * Rasterise an SVG document at exactly {@code width} x {@code height} px, scaling each axis to
     * fill the box. A caller that wants the aspect ratio kept asks {@link #svgSize} and chooses a box
     * that has it — this library will not guess at a layout it cannot see.
     */
    public static Frames rasterize(byte[] svg, int width, int height) {
        requireBytes(svg);
        if (width <= 0 || height <= 0) {
            throw new ImageException("a zero-sized raster was asked for: " + width + "x" + height);
        }
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment in = arena.allocateFrom(ValueLayout.JAVA_BYTE, svg);
            MemorySegment handle;
            try {
                handle = (MemorySegment) CApi.img_render_svg.invokeExact(in, (long) svg.length, width, height);
            } catch (Throwable t) {
                throw downcallFailed("img_render_svg", t);
            }
            return collect(handle, "SVG rasterisation failed");
        }
    }

    /** {@link #rasterize(byte[], int, int)} of a file's contents. */
    public static Frames rasterize(Path file, int width, int height) {
        return rasterize(readAll(file), width, height);
    }

    // --- taking the pixels out ------------------------------------------------------------------

    /**
     * Copy every frame out of a native handle and free it, so nothing this class returns depends on
     * native memory staying alive. The copy is the price of not handing a {@code MemorySegment} whose
     * lifetime a caller would have to honour — and the caller was going to copy into a staging buffer
     * anyway.
     */
    private static Frames collect(MemorySegment handle, String what) {
        if (handle == null || handle.address() == 0) {
            throw new ImageException(what + ": " + lastError());
        }
        try {
            int width = call(() -> (int) CApi.img_width.invokeExact(handle), "img_width");
            int height = call(() -> (int) CApi.img_height.invokeExact(handle), "img_height");
            int count = call(() -> (int) CApi.img_count.invokeExact(handle), "img_count");
            if (width <= 0 || height <= 0 || count <= 0) {
                throw new ImageException(what + ": the decoder returned an empty image ("
                        + width + "x" + height + ", " + count + " frames)");
            }
            long planeBytes = (long) width * height * 4L;
            int[] delays = new int[count];
            byte[][] planes = new byte[count][];
            for (int i = 0; i < count; i++) {
                int frame = i;
                delays[i] = call(() -> (int) CApi.img_delay_ms.invokeExact(handle, frame), "img_delay_ms");
                MemorySegment pixels =
                        call(() -> (MemorySegment) CApi.img_pixels.invokeExact(handle, frame), "img_pixels");
                if (pixels.address() == 0) {
                    throw new ImageException(what + ": frame " + frame + " had no pixels");
                }
                planes[i] = pixels.reinterpret(planeBytes).toArray(ValueLayout.JAVA_BYTE);
            }
            return new Frames(width, height, delays, planes);
        } finally {
            try {
                CApi.img_free.invokeExact(handle);
            } catch (Throwable t) {
                throw downcallFailed("img_free", t);
            }
        }
    }

    /** The native decoder's own account of the last failure on this thread, or a stand-in. */
    private static String lastError() {
        MemorySegment message;
        try {
            message = (MemorySegment) CApi.img_last_error.invokeExact();
        } catch (Throwable t) {
            return "(the error string could not be read: " + t + ")";
        }
        if (message == null || message.address() == 0) {
            return "(the native library gave no reason)";
        }
        return message.reinterpret(Long.MAX_VALUE).getString(0);
    }

    // --- plumbing -------------------------------------------------------------------------------

    private interface Call<T> {
        T get() throws Throwable;
    }

    private static <T> T call(Call<T> call, String name) {
        try {
            return call.get();
        } catch (Throwable t) {
            throw downcallFailed(name, t);
        }
    }

    private static ImageException downcallFailed(String name, Throwable cause) {
        if (cause instanceof ImageException e) {
            return e;
        }
        return new ImageException("the downcall to " + name + " failed", cause);
    }

    private static void requireBytes(byte[] bytes) {
        if (bytes == null || bytes.length == 0) {
            throw new ImageException("no bytes to read");
        }
    }

    private static byte[] readAll(Path file) {
        try {
            return Files.readAllBytes(file);
        } catch (IOException e) {
            throw new UncheckedIOException("could not read " + file, e);
        }
    }

    private Imagelib() {
    }
}
