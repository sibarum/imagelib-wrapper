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
if [ -n "$TARGET" ]; then
    cargo build --release --manifest-path imagelib/Cargo.toml --target "$TARGET"
    OUT="imagelib/target/$TARGET/release"
    TRIPLE="$TARGET"
else
    cargo build --release --manifest-path imagelib/Cargo.toml
    OUT="imagelib/target/release"
    TRIPLE=$(rustc -vV | awk '/^host:/ {print $2}')
fi

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
