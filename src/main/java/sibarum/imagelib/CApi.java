package sibarum.imagelib;

import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.SymbolLookup;
import java.lang.invoke.MethodHandle;

import static java.lang.foreign.ValueLayout.ADDRESS;
import static java.lang.foreign.ValueLayout.JAVA_INT;
import static java.lang.foreign.ValueLayout.JAVA_LONG;

/**
 * Raw FFM downcalls onto {@code native/include/imagelib.h}. Hand-written, one line per function, in
 * the header's order — eleven of them, which is the whole of the boundary.
 *
 * <p>{@code size_t} is bound as {@code JAVA_LONG} and every {@code uint32_t} as {@code JAVA_INT}; a
 * frame count or a pixel dimension that overflows a signed int is a file no decoder would have
 * accepted. Lifetimes belong to {@link Imagelib}, not here: nothing in this class frees anything.
 */
final class CApi {

    private static final Linker LINKER = Linker.nativeLinker();
    private static final SymbolLookup LOOKUP;

    static {
        NativeLib.ensureLoaded();
        LOOKUP = SymbolLookup.loaderLookup();
    }

    // classification
    static final MethodHandle img_probe =
            h("img_probe", FunctionDescriptor.of(JAVA_INT, ADDRESS, JAVA_LONG));
    static final MethodHandle img_svg_size =
            h("img_svg_size", FunctionDescriptor.of(JAVA_INT, ADDRESS, JAVA_LONG, ADDRESS, ADDRESS));

    // the two verbs
    static final MethodHandle img_decode =
            h("img_decode", FunctionDescriptor.of(ADDRESS, ADDRESS, JAVA_LONG));
    static final MethodHandle img_render_svg =
            h("img_render_svg", FunctionDescriptor.of(ADDRESS, ADDRESS, JAVA_LONG, JAVA_INT, JAVA_INT));

    // reading the result
    static final MethodHandle img_width = h("img_width", FunctionDescriptor.of(JAVA_INT, ADDRESS));
    static final MethodHandle img_height = h("img_height", FunctionDescriptor.of(JAVA_INT, ADDRESS));
    static final MethodHandle img_count = h("img_count", FunctionDescriptor.of(JAVA_INT, ADDRESS));
    static final MethodHandle img_delay_ms =
            h("img_delay_ms", FunctionDescriptor.of(JAVA_INT, ADDRESS, JAVA_INT));
    static final MethodHandle img_pixels =
            h("img_pixels", FunctionDescriptor.of(ADDRESS, ADDRESS, JAVA_INT));
    static final MethodHandle img_free = h("img_free", FunctionDescriptor.ofVoid(ADDRESS));

    static final MethodHandle img_last_error = h("img_last_error", FunctionDescriptor.of(ADDRESS));

    private static MethodHandle h(String name, FunctionDescriptor desc) {
        return LINKER.downcallHandle(
                LOOKUP.find(name).orElseThrow(() -> new ImageException("symbol not found: " + name)),
                desc);
    }

    private CApi() {
    }
}
