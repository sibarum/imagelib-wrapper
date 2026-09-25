# imagelib-wrapper

Panama (FFM) bindings for one C ABI over every image format a GUI can show. Hand-written downcalls
over an eleven-function header — no jextract, no JNI, no code generation.

```java
byte[] bytes = Files.readAllBytes(path);
Frames frames = switch (Imagelib.probe(bytes)) {
    case RASTER  -> Imagelib.decode(bytes);                       // the file decides its size
    case VECTOR  -> Imagelib.rasterize(bytes, box.w(), box.h());  // the box decides its size
    case UNKNOWN -> placeholder();
};

frames.width();          // px, shared by every frame
frames.count();          // 1 for a still — an animation of one
frames.rgba(0);          // width * height * 4 bytes, straight RGBA8, ready to upload
frames.delayMillis(0);   // ms, 0 where the file gave none
```

## The one idea

**The caller never learns a format.** Two verbs come out of this library, and they differ only in who
decides the pixel size — a raster file dictates its own, a vector document has none until a layout
gives it one. That is the single distinction that cannot be papered over, so it is the only one the
API keeps.

Everything else collapses. Both verbs return `Frames`, an ordered run of RGBA8 planes with a delay
each, so **a JPEG is a one-frame animation** and nothing above this library branches on stillness to
find its pixels. `probe` answers which verb some bytes want, so even the dispatch stays format-blind.

Animation frames arrive **fully composited** — GIF disposal methods and WebP blend rules are applied
by the decoder. A consumer draws frame *i* and never reasons about frame *i-1*. Format knowledge
stops at this boundary; that is what the boundary is for.

## Formats

| | |
| --- | --- |
| Raster, still | PNG, JPEG, BMP, ICO, TIFF, TGA, PNM, QOI, WebP, GIF |
| Raster, animated | GIF, animated WebP, APNG |
| Vector | SVG, SVGZ |

Pixels are always **straight (non-premultiplied) RGBA8, sRGB, tightly packed, top row first** — the
byte order a `VK_FORMAT_R8G8B8A8_UNORM` image wants uploaded, and the order the VexelRay font atlas
already uses. tiny-skia renders premultiplied, so the SVG path demultiplies before returning; without
that, every soft edge in the GUI would darken.

A vector document that declares **no** size — no `width`/`height`, no `viewBox` — is saying the caller
decides, so `rasterize` makes the box its viewport rather than scaling up from usvg's arbitrary
100x100 fallback. `svgSize` reports that fallback, since asking a sizeless document its size has no
better answer.

A document that declares a size of **zero** is refused, because that is an explicit answer rather than
an absent one. `vexelray-icons/vexelray-marks-sprite.svg` is exactly this: `width="0"` with every mark
inside `<defs>`, meant to be inlined into HTML and `<use>`d. It is a symbol library, not a picture, and
rendering it would produce an empty texture and call it success. The standalone `svg/<name>.svg` files
are the ones to rasterize.

**AVIF is deliberately out.** The `image` crate can only decode it through libdav1d, a C library, and
that would put a system dependency back into a build which currently has none. If it becomes worth
the cost, it is one Cargo feature and one line in `build.sh`.

**Lottie is out too**, and for a different reason: it is a designer-handoff format, and VexelRay
authors animation in code (`krono`'s `Transitions`, `Cue`). Choosing `image` + `resvg` over ThorVG
was exactly that trade — no Lottie, but no C toolchain either.

## Layout

```
native/include/imagelib.h    the contract: 11 functions, 1 opaque handle, no struct crosses
native/imagelib/src/lib.rs   the Rust behind it — `image` for raster, `resvg` for vector
native/build.sh              cargo build + stage into native/dist/bin/<platform>/
src/main/java/sibarum/imagelib/
    Imagelib.java            the two verbs, the probe, and taking the pixels out
    Frames.java              what every format decodes to
    CApi.java                one line per header function
    NativeLib.java           find, extract, load
```

`CApi` and `NativeLib` are package-private on purpose: the ABI is an implementation detail of four
public types, not a second API.

## Building

The native side needs **only a Rust toolchain** — <https://rustup.rs>. Both decoders are pure Rust,
so there is no C dependency to find, no cmake, no MSYS2, and cross-compiling is one `cargo` flag.
(On Windows, `rustup`'s default MSVC toolchain wants the Visual Studio Build Tools; if you would
rather not have those, `rustup toolchain install stable-x86_64-pc-windows-gnu` brings its own linker.)

```bash
./native/build.sh    # stages native/dist/bin/<platform>/
mvn install          # bundles it into the jar; tests need --enable-native-access (surefire is set up)
```

Cross-compile with `rustup target add <triple>` and `TARGET=<triple> ./native/build.sh`. Add the
triple to the `case` in `build.sh` and to `NativeLib.platform()` — in both places or in neither.

At runtime the library is extracted from the jar to a temp dir and loaded. Override with
`-Dimagelib.native.dir=<dir>`. Callers need `--enable-native-access=ALL-UNNAMED` (or their module).

### In a native image

A GraalVM native image can link imagelib in rather than extract the DLL, so the executable is the only file.
`build.sh` also stages the static library and the arguments that link it (MSVC so far):

```
native/dist/static/windows-x86_64/imagelib.lib
native/dist/static/windows-x86_64/imagelib-link.args
```

Pass both to `native-image`, beside `-H:+ForeignAPISupport`:

```
-H:NativeLinkerOption=<path>/imagelib.lib  @<path>/imagelib-link.args
```

Nothing changes on the Java side. `NativeLib` finds the symbols already in the executable and skips the
extraction, so leave `natives/` out of the image's resources. The args file does two things. First, it exports
every header function from the executable: FFM's loader lookup finds a symbol in a native image through the export
table, and without the export the linker drops imagelib entirely, since nothing in the image calls it by name.
Second, it names the system libraries rustc reports. Tried on Windows with GraalVM 25: `Imagelib.decode` returns
the same pixels as the DLL, from a folder holding only the executable.

`resvg` is pinned: its `usvg::Options`/`fontdb` surface has moved between releases, so a version bump
is a deliberate act with a compile behind it, not a range that drifts.

## Tests

`mvn test` **fails** rather than skips when the native library is missing. A green run of a bindings
suite that never crossed the boundary would say nothing at all.

Raster fixtures are encoded by ImageIO at test time, so the bytes under test are a real PNG and a real
JPEG and nothing checked in can drift from what it claims to be. The one committed fixture is
`src/test/resources/anim.gif` (4x3, two frames, 30 ms each), because an animation is the one thing
ImageIO cannot write in a line. Animated WebP and APNG take the same `AnimationDecoder` path in Rust
as GIF, so they are covered by construction rather than by a fixture of their own.

Beyond the suite, the binding has been run over the real `vexelray-icons` assets: both PNG sheets
decode at 640x256 and 1280x512, and all ten standalone SVGs read their 96x96 intrinsic size and
rasterize with masks, strokes and hues intact at both 96 px and 24 px from the same document.

## Using it from VexelRay-GUI

The GPU side already exists: `Canvas.image(...)` is a `KIND_IMAGE` primitive with the same rounded-box
SDF as every other shape, `Canvas.Run` splits the vertex buffer where the bound image changes, and
`Node.image(SampledImage)` puts one on a node. Two things are missing there, and neither is in this
repo:

1. **`GuiApp.texture(byte[] rgba, int w, int h)`** — the device is deliberately private, so an
   application currently cannot turn its own pixels into a `SampledImage`. `viewport(w, h)` is the
   same shape of method and shows the way.
2. **A UV region on the image prop.** `Canvas.image` already takes `(u0, v0, u1, v1)`;
   `TreeRenderer` calls the whole-texture overload. Pass the rect through and an animation becomes
   one sprite-sheet texture with moving UVs — no per-frame upload, no new engine capability.

With those, a still is `decode` → `texture` → `Node.image`, and an animation is `decode` → pack the
planes into one sheet → `texture` → advance the UV rect on a `krono` clock.
