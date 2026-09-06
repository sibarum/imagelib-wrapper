/**
 * Pixels, from anything — see {@link sibarum.imagelib.Imagelib}.
 *
 * <p>Four public types and no fifth: {@code Imagelib} (two verbs and a probe), {@link
 * sibarum.imagelib.Frames} (what every format decodes to), {@link sibarum.imagelib.Imagelib.Kind}
 * and {@link sibarum.imagelib.ImageException}. Nothing here names a format in a type, and nothing
 * above this package should have to.
 *
 * <p>The boundary itself is {@code native/include/imagelib.h}: eleven C functions, one opaque
 * handle, no struct crossing it. {@code CApi} binds them one line each and {@code NativeLib} loads
 * the library; both are package-private, because the ABI is an implementation detail of these four
 * types rather than a second API.
 */
package sibarum.imagelib;
