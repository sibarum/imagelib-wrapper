package sibarum.imagelib;

/**
 * What every format decodes to: an ordered run of same-sized RGBA8 planes, each with a display time.
 *
 * <p><b>A still is a run of one.</b> That is the point of the type — a JPEG, an SVG and an animated
 * GIF all arrive here, so nothing above this library branches on "is it animated" to find out how to
 * get at its pixels. A consumer that ignores {@link #count()} draws frame 0 and is correct for every
 * still ever written.
 *
 * <p>Pixels are straight (non-premultiplied) RGBA8, sRGB, tightly packed, top row first —
 * {@code width * height * 4} bytes per frame, in the byte order a {@code VK_FORMAT_R8G8B8A8_UNORM}
 * image wants uploaded.
 *
 * <p>Immutable in intent and cheap by design: {@link #rgba(int)} hands back the live array rather
 * than a copy, because the caller's next move is to push those bytes at a staging buffer and a
 * defensive copy of a 4K frame is 33 MB of nothing. Do not write to it.
 */
public final class Frames {

    private final int width;
    private final int height;
    private final int[] delaysMillis;
    private final byte[][] planes;

    Frames(int width, int height, int[] delaysMillis, byte[][] planes) {
        this.width = width;
        this.height = height;
        this.delaysMillis = delaysMillis;
        this.planes = planes;
    }

    /** Pixel width, shared by every frame. */
    public int width() {
        return width;
    }

    /** Pixel height, shared by every frame. */
    public int height() {
        return height;
    }

    /** How many frames; 1 for a still. */
    public int count() {
        return planes.length;
    }

    /** Whether there is more than one frame — a convenience over {@link #count()}, never a branch a
     *  consumer of the pixels needs to take. */
    public boolean animated() {
        return planes.length > 1;
    }

    /** Frame {@code i}'s pixels: the live array, {@code width * height * 4} bytes. Do not write to it. */
    public byte[] rgba(int i) {
        return planes[i];
    }

    /** Frame 0's pixels — what a still has, and what an animation starts on. */
    public byte[] rgba() {
        return planes[0];
    }

    /**
     * Frame {@code i}'s display time in ms, or 0 where the file gave none — including every still.
     * A zero is reported as it was found rather than replaced with a guess, so a player applies its
     * own floor (browsers use 100 ms for GIF's degenerate delays) and knows that it did.
     */
    public int delayMillis(int i) {
        return delaysMillis[i];
    }

    /** One pass through the whole run in ms; 0 for a still. */
    public long durationMillis() {
        long total = 0;
        for (int d : delaysMillis) {
            total += d;
        }
        return total;
    }

    /** Total pixel bytes held, across every frame — what a caller budgets a texture upload against. */
    public long byteSize() {
        return (long) width * height * 4L * planes.length;
    }

    @Override
    public String toString() {
        return "Frames[" + width + "x" + height + ", " + planes.length + " frame"
                + (planes.length == 1 ? "" : "s")
                + (animated() ? ", " + durationMillis() + "ms" : "") + "]";
    }
}
