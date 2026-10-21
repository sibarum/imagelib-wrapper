#!/usr/bin/env bash
# Build imagelib for Windows and stage it into native/dist/<platform>, which this module's jar bundles.
#
#   ./native/build.sh          (from Git Bash; Rust's default Windows toolchain links with the VS Build Tools)
#
# Also stages, apart from what the jar carries, the static library for a GraalVM native image that links imagelib
# in rather than extracting the DLL (README, "In a native image"): native/static/<platform>/imagelib.lib, and beside
# it imagelib-link.args, the native-image arguments that link it — every header function exported from the
# executable, since FFM's loader lookup finds a symbol in a native image through its export table and the linker
# keeps nothing that is not asked for, and the system libraries rustc named.
set -euo pipefail
cd "$(dirname "$0")"
. ../../native/common.sh

case "$(triple)" in
    x86_64-pc-windows-msvc) PLATFORM=windows-x86_64 ;;
    *) echo "$(triple) is not the Windows (MSVC) target" >&2; exit 1 ;;
esac
build_crate
DEST="dist/$PLATFORM"
mkdir -p "$DEST"
cp "$OUT/imagelib.dll" "$DEST/"
echo "staged: $DEST/imagelib.dll"

STATIC="static/$PLATFORM"
mkdir -p "$STATIC"
cp "$OUT/imagelib.lib" "$STATIC/"
ARGS="$STATIC/imagelib-link.args"
: > "$ARGS"
for symbol in $(grep -o 'img_[a-z_]*(' "$COMMON/include/imagelib.h" | tr -d '(' | sort -u); do
    echo "-H:NativeLinkerOption=/EXPORT:$symbol" >> "$ARGS"
done
for lib in $SYSTEM_LIBS; do
    case "$lib" in
        /defaultlib:*) ;;   # the C runtime, which native-image's own link already names
        *) echo "-H:NativeLinkerOption=$lib" >> "$ARGS" ;;
    esac
done
echo "staged: $STATIC/imagelib.lib and $(basename "$ARGS") ($(wc -l < "$ARGS") arguments)"
