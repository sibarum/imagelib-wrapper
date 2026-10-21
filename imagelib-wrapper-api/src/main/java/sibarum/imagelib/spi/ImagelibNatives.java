package sibarum.imagelib.spi;

/**
 * Where this operating system's build of the imagelib library is bundled: the one fact {@code imagelib-wrapper}
 * needs that differs per OS. An OS module ({@code imagelib-wrapper-windows}, {@code -linux}) provides it through
 * {@link java.util.ServiceLoader}, and the build puts exactly one of them on the class path.
 */
public interface ImagelibNatives {

    /** What this is for in a message: {@code windows-x86_64}, {@code linux-x86_64}. */
    String platform();

    /** What this OS calls the library's file: {@code imagelib.dll}, {@code libimagelib.so}. */
    String fileName();

    /** The class-path resource holding this machine's build, or null when the module has none for its architecture. */
    String resource();
}
