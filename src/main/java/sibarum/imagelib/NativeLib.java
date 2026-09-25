package sibarum.imagelib;

import java.io.IOException;
import java.io.InputStream;
import java.lang.foreign.SymbolLookup;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Locale;

/**
 * Locates and loads the {@code imagelib} native library.
 *
 * <p>Search order: the library already linked into the executable, then the {@code imagelib.native.dir}
 * system property (a directory holding the library), then the bundled classpath resource under
 * {@code /natives/<platform>/}, extracted to a temp directory. Loading is idempotent and thread-safe.
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
        String name = libraryName();
        String dir = System.getProperty("imagelib.native.dir");
        try {
            Path base = dir != null ? Path.of(dir) : extractBundled(name);
            System.load(base.resolve(name).toAbsolutePath().toString());
        } catch (IOException | UnsatisfiedLinkError e) {
            throw new ImageException("failed to load the imagelib native library for " + platform(), e);
        }
        loaded = true;
    }

    /**
     * The directory name a build stages into and this loader reads back — {@code native/build.sh}
     * writes exactly these, so a new target is added in both places or in neither.
     */
    static String platform() {
        String os = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
        String arch = System.getProperty("os.arch", "").toLowerCase(Locale.ROOT);
        String family = os.contains("win") ? "windows"
                : os.contains("mac") || os.contains("darwin") ? "macos"
                : os.contains("linux") ? "linux"
                : os.isEmpty() ? "unknown" : os.split("\\s+")[0];
        String cpu = switch (arch) {
            case "amd64", "x86_64" -> "x86_64";
            case "aarch64", "arm64" -> "aarch64";
            default -> arch;
        };
        return family + "-" + cpu;
    }

    private static String libraryName() {
        String os = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
        if (os.contains("win")) {
            return "imagelib.dll";
        }
        return os.contains("mac") || os.contains("darwin") ? "libimagelib.dylib" : "libimagelib.so";
    }

    private static Path extractBundled(String name) throws IOException {
        String resource = "/natives/" + platform() + "/" + name;
        try (InputStream in = NativeLib.class.getResourceAsStream(resource)) {
            if (in == null) {
                throw new IOException("bundled native missing: " + resource
                        + " (build it first — see native/build.sh — or set -Dimagelib.native.dir)");
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
