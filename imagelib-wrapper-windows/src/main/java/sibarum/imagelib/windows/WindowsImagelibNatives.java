package sibarum.imagelib.windows;

import sibarum.imagelib.spi.ImagelibNatives;

import java.util.Locale;

/** Where this module bundles imagelib for Windows: {@code /natives/<platform>/imagelib.dll}, as native/build.sh stages it. */
public final class WindowsImagelibNatives implements ImagelibNatives {

    private static final String ARCH = System.getProperty("os.arch", "").toLowerCase(Locale.ROOT);

    @Override
    public String platform() {
        return "windows-x86_64";
    }

    @Override
    public String fileName() {
        return "imagelib.dll";
    }

    /** Only the architectures native/build.sh stages for; anything else has no bundled build. */
    @Override
    public String resource() {
        return (ARCH.equals("amd64") || ARCH.equals("x86_64")) ? "/natives/" + platform() + "/" + fileName() : null;
    }
}
