# What every OS module's native/build.sh shares: building the imagelib crate (native/imagelib, this folder) with
# cargo. Sourced, never run; the OS modules' scripts are what run. Requires only a Rust toolchain
# (https://rustup.rs): both decoders are pure Rust, so there is no C dependency to find.

COMMON="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# build_crate: build the cdylib (and static library) for $TARGET, or the host when it is unset. Sets OUT (where
# cargo put them) and SYSTEM_LIBS (the system libraries the static library needs, as rustc names them).
# CARGO_TARGET_DIR is honoured: cargo's build tree is large, and on a VM's shared folder slow.
build_crate() {
    command -v cargo >/dev/null || {
        echo "cargo not found — install Rust from https://rustup.rs and re-run" >&2
        exit 1
    }
    local target_dir=${CARGO_TARGET_DIR:-$COMMON/imagelib/target}
    local log
    log=$(mktemp)
    # `cargo rustc` rather than `cargo build`, only so rustc says which system libraries the static library needs.
    if [ -n "${TARGET:-}" ]; then
        cargo rustc --release --manifest-path "$COMMON/imagelib/Cargo.toml" --target "$TARGET" \
            -- --print native-static-libs 2>&1 | tee "$log"
        OUT="$target_dir/$TARGET/release"
    else
        cargo rustc --release --manifest-path "$COMMON/imagelib/Cargo.toml" \
            -- --print native-static-libs 2>&1 | tee "$log"
        OUT="$target_dir/release"
    fi
    SYSTEM_LIBS=$(sed -n 's/^note: native-static-libs: //p' "$log" | tail -1)
    rm -f "$log"
}

# triple: the target triple being built for.
triple() {
    if [ -n "${TARGET:-}" ]; then echo "$TARGET"; else rustc -vV | awk '/^host:/ {print $2}'; fi
}
