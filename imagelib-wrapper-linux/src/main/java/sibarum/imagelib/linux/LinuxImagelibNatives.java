package sibarum.imagelib.linux;

import sibarum.imagelib.spi.ImagelibNatives;

import java.util.Locale;

/** Where this module bundles imagelib for Linux: {@code /natives/<platform>/libimagelib.so}, as native/build.sh stages it. */
public final class LinuxImagelibNatives implements ImagelibNatives {

    private static final String ARCH = System.getProperty("os.arch", "").toLowerCase(Locale.ROOT);

    @Override
    public String platform() {
        return ARCH.equals("aarch64") ? "linux-aarch64" : "linux-x86_64";
    }

    @Override
    public String fileName() {
        return "libimagelib.so";
    }

    /** Only the architectures native/build.sh stages for; anything else has no bundled build. */
    @Override
    public String resource() {
        return (ARCH.equals("amd64") || ARCH.equals("x86_64") || ARCH.equals("aarch64")) ? "/natives/" + platform() + "/" + fileName() : null;
    }
}
