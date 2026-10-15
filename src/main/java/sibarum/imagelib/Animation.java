package sibarum.imagelib;

import java.lang.foreign.MemorySegment;
import java.lang.foreign.ValueLayout;

/**
 * A raster file opened to be played rather than decoded whole: frames come one at a time, each composited onto the
 * one before it, so however long the animation and however large its canvas, one canvas is all that is held. What
 * {@link Imagelib#animate} returns; {@link Frames} is the same file decoded all at once.
 *
 * <p><b>The timeline is known at open.</b> {@link #count}, {@link #delayMillis} and {@link #loops} are read from the
 * file's blocks without decoding a pixel, so a player can lay out a seek bar before frame 0 has arrived. A damaged
 * file may end before the count it declared; {@link #next} returning false is the end, whatever the count said.
 *
 * <p><b>Forward only.</b> A GIF or APNG frame is drawn over the ones before it, so frame {@code i} can only be made by
 * making every frame before it. {@link #rewind} starts again from frame 0; the bytes are already held, so that costs
 * a decoder and nothing else. Going back further than one frame is a rewind and a walk forward, and a player that
 * wants it fast keeps the frames it has made.
 *
 * <p><b>A still is a run of one</b>, as in {@link Frames}: a JPEG opens here too, with one frame and no delay.
 *
 * <p>One thread at a time, as the decoder under it is. Not closing it leaks the file's bytes and a canvas in native
 * memory; closing twice does nothing.
 */
public final class Animation implements AutoCloseable {

    private MemorySegment handle;
    private final int width;
    private final int height;
    private final int[] delaysMillis;
    private final int loops;
    private final long frameBytes;
    private int index = -1;

    Animation(MemorySegment handle) {
        this.handle = handle;
        this.width = Imagelib.call(() -> (int) CApi.img_anim_width.invokeExact(handle), "img_anim_width");
        this.height = Imagelib.call(() -> (int) CApi.img_anim_height.invokeExact(handle), "img_anim_height");
        int count = Imagelib.call(() -> (int) CApi.img_anim_count.invokeExact(handle), "img_anim_count");
        this.loops = Imagelib.call(() -> (int) CApi.img_anim_loops.invokeExact(handle), "img_anim_loops");
        this.delaysMillis = new int[count];
        for (int i = 0; i < count; i++) {
            int frame = i;
            delaysMillis[i] = Imagelib.call(() -> (int) CApi.img_anim_delay_ms.invokeExact(handle, frame),
                    "img_anim_delay_ms");
        }
        this.frameBytes = (long) width * height * 4L;
    }

    /** The canvas's width in px, shared by every frame. */
    public int width() {
        return width;
    }

    /** The canvas's height in px, shared by every frame. */
    public int height() {
        return height;
    }

    /** How many frames the file declares; 1 for a still. */
    public int count() {
        return delaysMillis.length;
    }

    /** Whether there is more than one frame. */
    public boolean animated() {
        return delaysMillis.length > 1;
    }

    /**
     * Frame {@code i}'s display time in ms, or 0 where the file gave none, including a still's. Reported as found, as
     * {@link Frames#delayMillis} is: a player applies its own floor.
     */
    public int delayMillis(int i) {
        return delaysMillis[i];
    }

    /** One pass through the whole run in ms, as the file times it; 0 for a still. */
    public long durationMillis() {
        long total = 0;
        for (int d : delaysMillis) {
            total += d;
        }
        return total;
    }

    /** How many times the file asks to be played through: 0 for forever, 1 for a still. */
    public int loops() {
        return loops;
    }

    /** One frame's size in bytes: {@code width * height * 4}. */
    public long frameBytes() {
        return frameBytes;
    }

    /**
     * Decode the next frame. True when there is one, readable with {@link #pixels} or {@link #rgba}; false when the
     * run has ended, after which {@link #rewind} starts it again. Throws {@link ImageException} on a damaged frame.
     */
    public boolean next() {
        MemorySegment h = live();
        int got = Imagelib.call(() -> (int) CApi.img_anim_next.invokeExact(h), "img_anim_next");
        if (got < 0) {
            throw new ImageException("frame " + (index + 1) + " could not be decoded: " + Imagelib.lastError());
        }
        if (got == 0) {
            return false;
        }
        index++;
        return true;
    }

    /** Which frame {@link #next} last decoded, from 0; -1 before the first, and after a rewind. */
    public int index() {
        return index;
    }

    /**
     * The frame {@link #next} last decoded, in native memory: {@link #frameBytes} bytes of straight RGBA8, sRGB, top
     * row first. Valid only until the next call to {@code next}, {@code rewind} or {@code close}; read it, scale it
     * or copy it before then. Here so a caller that scales the frame down reads the canvas where it lies instead of
     * copying it first.
     */
    public MemorySegment pixels() {
        MemorySegment h = live();
        MemorySegment p = Imagelib.call(() -> (MemorySegment) CApi.img_anim_pixels.invokeExact(h), "img_anim_pixels");
        if (p.address() == 0) {
            throw new IllegalStateException("no frame has been decoded yet: call next() first");
        }
        return p.reinterpret(frameBytes).asReadOnly();
    }

    /** A copy of the frame {@link #next} last decoded, into a new array. */
    public byte[] rgba() {
        return pixels().toArray(ValueLayout.JAVA_BYTE);
    }

    /** Go back to before frame 0: the next {@link #next} decodes it again. */
    public void rewind() {
        MemorySegment h = live();
        int ok = Imagelib.call(() -> (int) CApi.img_anim_rewind.invokeExact(h), "img_anim_rewind");
        if (ok == 0) {
            throw new ImageException("could not start the animation again: " + Imagelib.lastError());
        }
        index = -1;
    }

    public boolean closed() {
        return handle == null;
    }

    /** Release the file's bytes and the canvas. Closing twice does nothing. */
    @Override
    public void close() {
        MemorySegment h = handle;
        if (h == null) {
            return;
        }
        handle = null;
        Imagelib.call(() -> {
            CApi.img_anim_close.invokeExact(h);
            return null;
        }, "img_anim_close");
    }

    private MemorySegment live() {
        if (handle == null) {
            throw new IllegalStateException("the animation has been closed");
        }
        return handle;
    }

    @Override
    public String toString() {
        return "Animation[" + width + "x" + height + ", " + count() + " frame" + (count() == 1 ? "" : "s")
                + (animated() ? ", " + durationMillis() + "ms" : "") + (closed() ? ", closed" : "") + "]";
    }
}
