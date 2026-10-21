package sibarum.imagelib;

import sibarum.imagelib.spi.ImagelibNatives;

import java.io.IOException;
import java.io.InputStream;
import java.lang.foreign.SymbolLookup;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ServiceLoader;

/**
 * Locates and loads the {@code imagelib} native library.
 *
 * <p>Search order: the library already linked into the executable, then the {@code imagelib.native.dir}
 * system property (a directory holding the library), then the classpath resource the OS module on the class
 * path names ({@link ImagelibNatives}), extracted to a temp directory. Loading is idempotent and thread-safe.
 * This class holds no platform knowledge of its own.
 *
 * <p>Linked in is the GraalVM native image built with the static library (README, "In a native
 * image"): its symbols are in the executable, where the loader lookup finds them, and there is no DLL
 * in the image to extract. Anywhere else nothing is found there, and loading goes on as before.
 *
 * <p>One library and no dependency order to get right — the whole reason the native side is Rust.
 * Both decoders compile in, so there is nothing beside it to stage and nothing on the host to find.
 */
final class NativeLib {

    private static volatile boolean loaded;

    static synchronized void ensureLoaded() {
        if (loaded) {
            return;
        }
        if (SymbolLookup.loaderLookup().find("img_probe").isPresent()) {
            loaded = true;   // linked into this executable
            return;
        }
        String dir = System.getProperty("imagelib.native.dir");
        try {
            String name = libraryName();
            Path base = dir != null ? Path.of(dir) : extractBundled(name);
            System.load(base.resolve(name).toAbsolutePath().toString());
        } catch (IOException | UnsatisfiedLinkError e) {
            throw new ImageException("failed to load the imagelib native library for " + platform(), e);
        }
        loaded = true;
    }

    /** What the OS module calls this platform, for messages. */
    static String platform() {
        ImagelibNatives natives = natives();
        return natives == null ? "unknown (no imagelib-wrapper OS module on the class path)" : natives.platform();
    }

    /** The one OS module on the class path, or null when there is none. */
    private static ImagelibNatives natives() {
        return ServiceLoader.load(ImagelibNatives.class).findFirst().orElse(null);
    }

    private static String libraryName() throws IOException {
        ImagelibNatives natives = natives();
        if (natives == null) {
            throw new IOException("no imagelib for " + platform() + " (add the imagelib-wrapper OS module for it)");
        }
        return natives.fileName();
    }

    private static Path extractBundled(String name) throws IOException {
        ImagelibNatives natives = natives();
        String resource = natives == null ? null : natives.resource();
        if (resource == null) {
            throw new IOException("no bundled imagelib for " + platform() + " (set -Dimagelib.native.dir)");
        }
        try (InputStream in = NativeLib.class.getResourceAsStream(resource)) {
            if (in == null) {
                throw new IOException("bundled native missing: " + resource
                        + " (build it first with the OS module's native/build.sh, or set -Dimagelib.native.dir)");
            }
            Path tmp = Files.createTempDirectory("imagelib");
            tmp.toFile().deleteOnExit();
            Path out = tmp.resolve(name);
            Files.copy(in, out, StandardCopyOption.REPLACE_EXISTING);
            out.toFile().deleteOnExit();
            return tmp;
        }
    }

    private NativeLib() {
    }
}
