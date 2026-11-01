#!/bin/sh
# Builds, lints and tests the two small helper programs that ship next to blockforge in every Linux package:
#
#   packaging/linux/helpers.sh build     builds both in release mode (what the packages are made from)
#   packaging/linux/helpers.sh clippy    lints both, warnings are errors
#   packaging/linux/helpers.sh test      runs their tests
#
#   blockforge-bedrock-auth   (crates/bedrock-auth)     the Microsoft sign-in window for Bedrock Edition. It links GTK 3,
#                              WebKitGTK 4.1 and libsoup 3, which is why only this helper needs WebKit: BlockForge itself
#                              starts without it. Needs libwebkit2gtk-4.1-dev, libgtk-3-dev and libsoup-3.0-dev to build.
#   bedrock-runtime           (crates/bedrock-runtime)  runs inside the Steam Linux Runtime container, whose libraries are
#                              not the host's, so it is built static (musl, static-pie) and needs nothing there. Needs the
#                              Rust target x86_64-unknown-linux-musl (rustup target add x86_64-unknown-linux-musl); it has no
#                              C code, so no musl-gcc.
#
# Where the files end up (build.sh packs exactly these):
#   $CARGO_TARGET_DIR (default: target/ next to Cargo.toml)
#     release/blockforge-bedrock-auth
#     x86_64-unknown-linux-musl/release/bedrock-runtime
#
# Both crates are members of the workspace in Cargo.toml when they are listed there (or are a path dependency of the
# program); otherwise they are built from their own Cargo.toml and Cargo.lock. Either way works, and nothing here edits
# Cargo.toml. Linux only: Windows and macOS never build these crates.
set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
MUSL=x86_64-unknown-linux-musl
cd "$ROOT"

die() { echo "helpers.sh: $*" >&2; exit 1; }
[ "$(uname -s)" = Linux ] || die "the helpers are built on Linux only"

# crate <package name> <folder> <cargo arguments>: runs cargo for one crate, as a workspace member when it is one. The
# arguments after "--" (for clippy) are given in $AFTER, because the crate and target folder must come before them.
crate() {
    name="$1"; dir="$2"; shift 2
    if cargo pkgid -p "$name" > /dev/null 2>&1; then
        sel="-p $name"
    else
        sel="--manifest-path $ROOT/$dir/Cargo.toml"
    fi
    # shellcheck disable=SC2086  # $sel is two words on purpose
    if [ -n "${AFTER:-}" ]; then
        # shellcheck disable=SC2086
        cargo "$@" $sel --target-dir "$TARGET_DIR" -- $AFTER
    else
        cargo "$@" $sel --target-dir "$TARGET_DIR"
    fi
}

case "${1:-}" in
    build)
        crate blockforge-bedrock-auth crates/bedrock-auth build --locked --release
        crate bedrock-runtime crates/bedrock-runtime build --locked --release --target "$MUSL"
        auth="$TARGET_DIR/release/blockforge-bedrock-auth"
        runtime="$TARGET_DIR/$MUSL/release/bedrock-runtime"
        [ -x "$auth" ] || die "$auth was not built"
        [ -x "$runtime" ] || die "$runtime was not built"
        # The runtime helper has to start inside a container whose C library is not ours: it must be fully static, which
        # shows as a program with no interpreter (INTERP) header and no needed libraries.
        if readelf -lW "$runtime" | grep -q INTERP; then die "$runtime asks for a dynamic loader; it must be static"; fi
        if readelf -d "$runtime" 2> /dev/null | grep -q NEEDED; then die "$runtime needs shared libraries; it must be static"; fi
        echo "Built $auth"
        echo "Built $runtime"
        ;;
    clippy)
        export AFTER="-D warnings"
        crate blockforge-bedrock-auth crates/bedrock-auth clippy --locked --release --all-targets
        crate bedrock-runtime crates/bedrock-runtime clippy --locked --release --all-targets --target "$MUSL"
        ;;
    test)
        crate blockforge-bedrock-auth crates/bedrock-auth test --locked --release
        crate bedrock-runtime crates/bedrock-runtime test --locked --release
        ;;
    *)
        die "usage: helpers.sh build|clippy|test"
        ;;
esac
