package sibarum.imagelib;

import org.junit.jupiter.api.Test;

import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.UncheckedIOException;
import java.nio.charset.StandardCharsets;

import static org.junit.jupiter.api.Assertions.assertAll;
import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * What the boundary promises, checked on real files rather than on mocks — there is nothing here to
 * mock, since the whole library is the crossing.
 *
 * <p>Raster fixtures are encoded by ImageIO at test time, so the bytes under test are a real PNG and
 * a real JPEG and nothing is checked in that could drift from what it claims to be. The one committed
 * fixture is {@code anim.gif}, because an animation is the one thing ImageIO cannot write in a line.
 *
 * <p>These tests need the native library. They fail rather than skip when it is missing: a green run
 * of a binding suite that never crossed the boundary would say nothing at all.
 */
class ImagelibTest {

    // --- fixtures -------------------------------------------------------------------------------

    private static final String SVG_100_50 = """
            <svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" viewBox="0 0 100 50">
              <rect x="0" y="0" width="100" height="50" fill="#00ff00"/>
            </svg>
            """;

    /** Half-opaque red on nothing — the premultiplication check has to have a soft pixel to look at. */
    private static final String SVG_HALF_ALPHA = """
            <svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10">
              <rect x="0" y="0" width="10" height="10" fill="#ff0000" fill-opacity="0.5"/>
            </svg>
            """;

    /** No width, no height, no viewBox — legal, and a document saying the caller decides. */
    private static final String SVG_NO_SIZE = """
            <svg xmlns="http://www.w3.org/2000/svg">
              <rect x="0" y="0" width="9999" height="9999" fill="#00ff00"/>
            </svg>
            """;

    /** A symbol library, not a picture: an explicit zero size, with everything inside defs. */
    private static final String SVG_SPRITE = """
            <svg xmlns="http://www.w3.org/2000/svg" width="0" height="0" style="position:absolute">
              <defs><symbol id="a" viewBox="0 0 96 96"><rect width="96" height="96" fill="#f00"/></symbol></defs>
            </svg>
            """;

    private static byte[] svg(String source) {
        return source.getBytes(StandardCharsets.UTF_8);
    }

    private static byte[] encode(String format, int w, int h, int argb) {
        BufferedImage img = new BufferedImage(w, h, BufferedImage.TYPE_INT_ARGB);
        for (int y = 0; y < h; y++) {
            for (int x = 0; x < w; x++) {
                img.setRGB(x, y, argb);
            }
        }
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        try {
            // JPEG has no alpha channel and the JDK's BMP writer declines one, so both are handed an
            // opaque image. Only the PNG fixtures carry alpha, which is all the alpha tests need.
            BufferedImage source = format.equals("PNG") ? img : opaque(img);
            assertTrue(ImageIO.write(source, format, out), format + " has no ImageIO writer here");
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        return out.toByteArray();
    }

    private static BufferedImage opaque(BufferedImage src) {
        BufferedImage out = new BufferedImage(src.getWidth(), src.getHeight(), BufferedImage.TYPE_INT_RGB);
        out.getGraphics().drawImage(src, 0, 0, null);
        return out;
    }

    private static byte[] resource(String name) {
        try (InputStream in = ImagelibTest.class.getResourceAsStream("/" + name)) {
            assertNotNull(in, "test fixture missing: " + name);
            return in.readAllBytes();
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
    }

    /** The four channel bytes at {@code (x, y)} of frame {@code i}, as unsigned values. */
    private static int[] pixel(Frames frames, int i, int x, int y) {
        byte[] rgba = frames.rgba(i);
        int at = (y * frames.width() + x) * 4;
        return new int[]{rgba[at] & 0xFF, rgba[at + 1] & 0xFF, rgba[at + 2] & 0xFF, rgba[at + 3] & 0xFF};
    }

    // --- probing --------------------------------------------------------------------------------

    @Test
    void probeSaysWhichVerbBytesWant() {
        assertAll(
                () -> assertEquals(Imagelib.Kind.RASTER, Imagelib.probe(encode("PNG", 2, 2, 0xFF00FF00))),
                () -> assertEquals(Imagelib.Kind.RASTER, Imagelib.probe(encode("JPEG", 8, 8, 0xFF112233))),
                () -> assertEquals(Imagelib.Kind.RASTER, Imagelib.probe(resource("anim.gif"))),
                () -> assertEquals(Imagelib.Kind.VECTOR, Imagelib.probe(svg(SVG_100_50))),
                () -> assertEquals(Imagelib.Kind.UNKNOWN, Imagelib.probe("not an image".getBytes(StandardCharsets.UTF_8))),
                () -> assertEquals(Imagelib.Kind.UNKNOWN, Imagelib.probe(new byte[0])),
                () -> assertEquals(Imagelib.Kind.UNKNOWN, Imagelib.probe(null)));
    }

    // --- raster ---------------------------------------------------------------------------------

    @Test
    void decodesAPngAtItsOwnSizeAndInRgbaOrder() {
        Frames frames = Imagelib.decode(encode("PNG", 7, 5, 0xFF00FF00));   // opaque green

        assertEquals(7, frames.width());
        assertEquals(5, frames.height());
        assertEquals(7 * 5 * 4, frames.rgba().length, "one tightly packed RGBA8 plane");
        assertArrayEquals(new int[]{0, 255, 0, 255}, pixel(frames, 0, 3, 2),
                "green must arrive as R=0 G=255 B=0 A=255, not as some other channel order");
    }

    @Test
    void aStillIsAnAnimationOfOne() {
        Frames frames = Imagelib.decode(encode("PNG", 4, 4, 0xFF808080));

        assertAll(
                () -> assertEquals(1, frames.count()),
                () -> assertFalse(frames.animated()),
                () -> assertEquals(0, frames.delayMillis(0), "a still declares no delay rather than a made-up one"),
                () -> assertEquals(0, frames.durationMillis()),
                () -> assertSame(frames.rgba(), frames.rgba(0), "frame 0 is the still"));
    }

    @Test
    void decodesTheOtherRasterFormatsTheSameWay() {
        Frames jpeg = Imagelib.decode(encode("JPEG", 16, 9, 0xFF3366CC));
        Frames bmp = Imagelib.decode(encode("BMP", 6, 3, 0xFF00FF00));

        assertAll(
                () -> assertEquals(16, jpeg.width()),
                () -> assertEquals(9, jpeg.height()),
                () -> assertEquals(1, jpeg.count()),
                () -> assertEquals(255, pixel(jpeg, 0, 8, 4)[3], "a format without alpha decodes opaque"),
                () -> assertEquals(6, bmp.width()),
                () -> assertEquals(3, bmp.height()),
                () -> assertArrayEquals(new int[]{0, 255, 0, 255}, pixel(bmp, 0, 2, 1)));
    }

    @Test
    void decodesAnAnimatedGifIntoCompositedFrames() {
        Frames frames = Imagelib.decode(resource("anim.gif"));

        assertAll(
                () -> assertEquals(4, frames.width()),
                () -> assertEquals(3, frames.height(), "width and height must not be transposed"),
                () -> assertEquals(2, frames.count()),
                () -> assertTrue(frames.animated()),
                () -> assertEquals(30, frames.delayMillis(0), "3 GIF hundredths is 30 ms"),
                () -> assertEquals(30, frames.delayMillis(1)),
                () -> assertEquals(60, frames.durationMillis()),
                () -> assertArrayEquals(new int[]{255, 0, 0, 255}, pixel(frames, 0, 0, 0), "frame 0 is red"),
                () -> assertArrayEquals(new int[]{0, 0, 255, 255}, pixel(frames, 1, 3, 2),
                        "frame 1 is blue across the whole canvas, not just where it changed"),
                () -> assertEquals(4L * 3 * 4 * 2, frames.byteSize()));
    }

    // --- vector ---------------------------------------------------------------------------------

    @Test
    void svgSizeReadsWhatTheDocumentAsksFor() {
        Imagelib.Size size = Imagelib.svgSize(svg(SVG_100_50));

        assertEquals(100f, size.width(), 0.01f);
        assertEquals(50f, size.height(), 0.01f);
    }

    @Test
    void rasterizesAtTheBoxSizeRatherThanTheDocumentSize() {
        Frames frames = Imagelib.rasterize(svg(SVG_100_50), 20, 10);

        assertAll(
                () -> assertEquals(20, frames.width(), "the box decides, not the viewBox"),
                () -> assertEquals(10, frames.height()),
                () -> assertEquals(1, frames.count()),
                () -> assertEquals(20 * 10 * 4, frames.rgba().length),
                () -> assertArrayEquals(new int[]{0, 255, 0, 255}, pixel(frames, 0, 10, 5)));
    }

    @Test
    void theSameDocumentRastersAtAnySizeWithoutReparsing() {
        byte[] document = svg(SVG_100_50);

        assertEquals(400, Imagelib.rasterize(document, 400, 200).width());
        assertEquals(4, Imagelib.rasterize(document, 4, 2).width());
    }

    @Test
    void aDocumentDeclaringNoSizeRastersAtTheBoxItWasGiven() {
        Frames frames = Imagelib.rasterize(svg(SVG_NO_SIZE), 20, 10);

        assertAll(
                () -> assertEquals(20, frames.width()),
                () -> assertEquals(10, frames.height()),
                () -> assertArrayEquals(new int[]{0, 255, 0, 255}, pixel(frames, 0, 19, 9),
                        "the box became the viewport, so the fill reaches its far corner rather than "
                                + "being scaled up from an arbitrary 100x100 default"));
    }

    @Test
    void aZeroSizedSymbolSpriteIsRefusedRatherThanDrawnBlank() {
        // vexelray-icons ships one of these: width="0", everything inside <defs>, meant to be inlined
        // and <use>d. Rendering it would produce an empty texture and call it success.
        assertThrows(ImageException.class, () -> Imagelib.rasterize(svg(SVG_SPRITE), 32, 32));
    }

    @Test
    void alphaArrivesStraightRatherThanPremultiplied() {
        int[] px = pixel(Imagelib.rasterize(svg(SVG_HALF_ALPHA), 8, 8), 0, 4, 4);

        // Premultiplied, this pixel would read (128, 0, 0, 128) and every soft edge in the GUI would
        // darken. Straight, the colour survives its own transparency.
        assertEquals(128, px[3], 2, "half-opaque");
        assertTrue(px[0] >= 250, "red must still be full red at half alpha, was " + px[0]);
    }

    // --- failure --------------------------------------------------------------------------------

    @Test
    void unreadableBytesFailWithTheDecodersOwnReason() {
        byte[] garbage = "certainly not an image".getBytes(StandardCharsets.UTF_8);

        ImageException e = assertThrows(ImageException.class, () -> Imagelib.decode(garbage));
        assertTrue(e.getMessage().contains("decode failed"), e.getMessage());
        assertFalse(e.getMessage().contains("gave no reason"),
                "the native side must say why, not merely that");
    }

    @Test
    void malformedSvgFails() {
        assertThrows(ImageException.class,
                () -> Imagelib.rasterize(svg("<svg><this is not markup"), 10, 10));
        assertThrows(ImageException.class, () -> Imagelib.svgSize(svg("not a document")));
    }

    @Test
    void emptyAndZeroSizedRequestsAreRefusedBeforeTheyCross() {
        assertAll(
                () -> assertThrows(ImageException.class, () -> Imagelib.decode(new byte[0])),
                () -> assertThrows(ImageException.class, () -> Imagelib.decode((byte[]) null)),
                () -> assertThrows(ImageException.class, () -> Imagelib.rasterize(svg(SVG_100_50), 0, 10)),
                () -> assertThrows(ImageException.class, () -> Imagelib.rasterize(svg(SVG_100_50), 10, -1)));
    }

    @Test
    void manyDecodesInARowDoNotLeakOrTrampleEachOther() {
        // The handle is freed inside every call, so a thousand decodes hold one image's worth of
        // native memory at a time. A regression here shows up as a heap that only grows.
        for (int i = 0; i < 1000; i++) {
            Frames frames = Imagelib.decode(encode("PNG", 32, 32, 0xFF00FF00));
            assertEquals(32, frames.width());
        }
        assertArrayEquals(new int[]{0, 255, 0, 255},
                pixel(Imagelib.decode(encode("PNG", 2, 2, 0xFF00FF00)), 0, 1, 1));
    }
}
