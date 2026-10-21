#!/usr/bin/env bash
# Build imagelib for Linux and stage it into native/dist/<platform>, which this module's jar bundles.
#
#   ./native/build.sh                                    # this machine
#   TARGET=aarch64-unknown-linux-gnu ./native/build.sh   # cross-compile (rustup target add <triple> first)
set -euo pipefail
cd "$(dirname "$0")"
# cargo's build tree of its own, as Maven's is (target-linux/): a Windows host building the same checkout uses
# native/imagelib/target, and the two would otherwise rebuild over each other.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$(cd ../../native && pwd)/imagelib/target-linux}"
. ../../native/common.sh

case "$(triple)" in
    x86_64-*-linux-*)  PLATFORM=linux-x86_64 ;;
    aarch64-*-linux-*) PLATFORM=linux-aarch64 ;;
    *) echo "$(triple) is not a Linux target" >&2; exit 1 ;;
esac
build_crate
DEST="dist/$PLATFORM"
mkdir -p "$DEST"
cp "$OUT/libimagelib.so" "$DEST/"
echo "staged: $DEST/libimagelib.so"
