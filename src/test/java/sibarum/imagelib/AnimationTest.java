package sibarum.imagelib;

import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.io.InputStream;
import java.io.UncheckedIOException;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * {@link Animation}: the same frames {@link Imagelib#decode} gives, one at a time, with the timeline known before the
 * first. The {@code anim3} fixtures are three frames, red, green and blue, at 30, 50 and 70 ms, written by the native
 * crate's {@code write_fixtures} test in each of the three formats that animate.
 */
class AnimationTest {

    private static final int[][] COLOURS = {{255, 0, 0, 255}, {0, 255, 0, 255}, {0, 0, 255, 255}};

    @ParameterizedTest
    @ValueSource(strings = {"anim3.gif", "anim3.webp", "anim3.apng.png"})
    void streamsTheFramesDecodeGivesWithTheTimelineKnownAtOpen(String name) {
        byte[] bytes = resource(name);
        Frames whole = Imagelib.decode(bytes);
        try (Animation a = Imagelib.animate(bytes)) {
            assertEquals(4, a.width());
            assertEquals(3, a.height());
            assertEquals(3, a.count());
            assertTrue(a.animated());
            assertEquals(150, a.durationMillis(), "the delays are read before a frame is decoded");
            assertEquals(-1, a.index());
            for (int i = 0; i < 3; i++) {
                assertEquals(whole.delayMillis(i), a.delayMillis(i), "frame " + i + "'s delay");
                assertTrue(a.next());
                assertEquals(i, a.index());
                assertArrayEquals(whole.rgba(i), a.rgba(), "frame " + i);
                assertEquals(COLOURS[i][2], a.pixels().get(java.lang.foreign.ValueLayout.JAVA_BYTE, 2) & 0xFF);
            }
            assertFalse(a.next(), "the run ends after its last frame");
        }
    }

    @Test
    void rewindingPlaysTheSameFramesAgain() {
        try (Animation a = Imagelib.animate(resource("anim3.gif"))) {
            a.next();
            a.next();
            byte[] second = a.rgba();
            a.rewind();
            assertEquals(-1, a.index());
            a.next();
            assertArrayEquals(new byte[] {(byte) 255, 0, 0, (byte) 255}, java.util.Arrays.copyOf(a.rgba(), 4));
            a.next();
            assertArrayEquals(second, a.rgba());
        }
    }

    @Test
    void reportsHowManyTimesTheFileAsksToPlay() {
        try (Animation gif = Imagelib.animate(resource("anim3.gif"));
             Animation apng = Imagelib.animate(resource("anim3.apng.png"))) {
            assertEquals(0, gif.loops(), "forever");
            assertEquals(2, apng.loops());
        }
    }

    @Test
    void aStillOpensAsARunOfOne() {
        try (Animation a = Imagelib.animate(ImagelibTest.encode("PNG", 5, 2, 0xFF00FF00))) {
            assertEquals(1, a.count());
            assertFalse(a.animated());
            assertEquals(0, a.delayMillis(0));
            assertEquals(1, a.loops());
            assertTrue(a.next());
            assertEquals(5 * 2 * 4, a.rgba().length);
            assertFalse(a.next());
        }
    }

    @Test
    void aClosedAnimationRefusesAndClosingTwiceDoesNothing() {
        Animation a = Imagelib.animate(resource("anim3.webp"));
        a.close();
        a.close();
        assertTrue(a.closed());
        assertThrows(IllegalStateException.class, a::next);
    }

    @Test
    void pixelsBeforeTheFirstFrameIsAMistakeNotACrash() {
        try (Animation a = Imagelib.animate(resource("anim3.gif"))) {
            assertThrows(IllegalStateException.class, a::pixels);
        }
    }

    @Test
    void bytesNoDecoderKnowsAreRefused() {
        assertThrows(ImageException.class, () -> Imagelib.animate(new byte[] {1, 2, 3, 4, 5, 6, 7, 8}));
    }

    private static byte[] resource(String name) {
        try (InputStream in = AnimationTest.class.getResourceAsStream("/" + name)) {
            if (in == null) {
                throw new IllegalStateException("missing test resource " + name);
            }
            return in.readAllBytes();
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
    }
}
