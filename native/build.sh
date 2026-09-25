#!/usr/bin/env bash
# Build the imagelib cdylib and stage it into native/dist/bin, which the Maven build bundles into
# the jar. Requires only a Rust toolchain — https://rustup.rs — because both decoders are pure Rust:
# there is no C dependency to find, no cmake, no MSYS2.
#
#   ./native/build.sh                      # host platform
#   TARGET=aarch64-apple-darwin ./build.sh # cross-compile (rustup target add <triple> first)
set -euo pipefail
cd "$(dirname "$0")"

command -v cargo >/dev/null || {
    echo "cargo not found — install Rust from https://rustup.rs and re-run" >&2
    exit 1
}

TARGET=${TARGET:-}
# `cargo rustc` rather than `cargo build`, only so rustc can say which system libraries the static library needs;
# the static linking below passes them on.
BUILD_LOG=$(mktemp)
if [ -n "$TARGET" ]; then
    cargo rustc --release --manifest-path imagelib/Cargo.toml --target "$TARGET" -- --print native-static-libs 2>&1 | tee "$BUILD_LOG"
    OUT="imagelib/target/$TARGET/release"
    TRIPLE="$TARGET"
else
    cargo rustc --release --manifest-path imagelib/Cargo.toml -- --print native-static-libs 2>&1 | tee "$BUILD_LOG"
    OUT="imagelib/target/release"
    TRIPLE=$(rustc -vV | awk '/^host:/ {print $2}')
fi
SYSTEM_LIBS=$(sed -n 's/^note: native-static-libs: //p' "$BUILD_LOG" | tail -1)
rm -f "$BUILD_LOG"

# The jar keeps one directory per platform, named the way NativeLib.java asks for it at runtime.
case "$TRIPLE" in
    *windows*)        PLATFORM=windows-x86_64; LIB=imagelib.dll ;;
    x86_64-apple-*)   PLATFORM=macos-x86_64;   LIB=libimagelib.dylib ;;
    aarch64-apple-*)  PLATFORM=macos-aarch64;  LIB=libimagelib.dylib ;;
    x86_64-*-linux-*) PLATFORM=linux-x86_64;   LIB=libimagelib.so ;;
    aarch64-*-linux-*) PLATFORM=linux-aarch64; LIB=libimagelib.so ;;
    *) echo "unmapped target triple $TRIPLE — add it here and to NativeLib.java" >&2; exit 1 ;;
esac

DEST="dist/bin/$PLATFORM"
mkdir -p "$DEST"
cp "$OUT/$LIB" "$DEST/"
echo "staged: $DEST/$LIB"

# The static library, for a GraalVM native image that links imagelib in rather than extracting the DLL (README,
# "In a native image"). Staged apart from dist/bin, which is what goes into the jar: a JVM user never needs it.
# Beside it, the native-image arguments that link it: every header function exported from the executable, since
# FFM's loader lookup finds a symbol in a native image through its export table and the linker keeps nothing that
# is not asked for; and the system libraries rustc named. MSVC only, so far.
case "$TRIPLE" in
    *windows-msvc*)
        STATIC="dist/static/$PLATFORM"
        mkdir -p "$STATIC"
        cp "$OUT/imagelib.lib" "$STATIC/"
        ARGS="$STATIC/imagelib-link.args"
        : > "$ARGS"
        for symbol in $(grep -o 'img_[a-z_]*(' include/imagelib.h | tr -d '(' | sort -u); do
            echo "-H:NativeLinkerOption=/EXPORT:$symbol" >> "$ARGS"
        done
        for lib in $SYSTEM_LIBS; do
            case "$lib" in
                /defaultlib:*) ;;   # the C runtime, which native-image's own link already names
                *) echo "-H:NativeLinkerOption=$lib" >> "$ARGS" ;;
            esac
        done
        echo "staged: $STATIC/imagelib.lib and $(basename "$ARGS") ($(wc -l < "$ARGS") arguments)"
        ;;
    *) echo "no static staging for $TRIPLE yet: only MSVC has been linked into a native image" ;;
esac
