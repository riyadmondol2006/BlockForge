#!/bin/bash
# Builds BlockForge.app and the disk image for Apple Silicon Macs from an already built blockforge binary.
#
#   packaging/macos/build.sh <version> [path/to/blockforge]
#
# Build the program first, on an Apple Silicon Mac (or a macos-15 runner):
#   MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release
# The result is dist/BlockForge-<version>-macos-arm64.dmg (override the folder with OUT_DIR=...). The app inside is
# signed ad hoc ("codesign --sign -"): that is what lets an unsigned program run on Apple Silicon, but it isn't a
# developer signature, so the first launch needs the steps in packaging/macos/README.md.
#
# The disk image gets its look (background, icon positions, Applications link) from dmgbuild, a Python tool that
# writes the Finder settings itself and so works without a screen. It is installed into a temporary virtual
# environment. If that isn't possible the image is made plain, with just the app and the Applications link.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
CRATE="$(cd "$HERE/../.." && pwd)"
DMGBUILD_SPEC="${DMGBUILD_SPEC:-dmgbuild==1.6.7}"
BUNDLE_ID="io.github.riyadmondol2006.BlockForge"
DEFAULT_MIN_OS="11.0"   # the first macOS that runs on Apple Silicon

fail() {
    echo "build.sh: error: $*" >&2
    exit 1
}
step() {
    echo "==> $*"
}

VERSION="${1:-}"
[ -n "$VERSION" ] || fail "usage: build.sh <version> [path/to/blockforge]   (for example: build.sh 4.0.0)"
case "$VERSION" in
    *[!0-9.]* | .* | *. | *..*) fail "the version must look like 4.0.0, got '$VERSION'" ;;
esac
IFS=. read -r V1 V2 V3 _ <<EOF
$VERSION
EOF
[ -n "${V1:-}" ] && [ -n "${V2:-}" ] && [ -n "${V3:-}" ] || fail "the version needs three numbers like 4.0.0, got '$VERSION'"
# macOS apps allow three numbers; a fourth one stays in the file name of the disk image only
BUNDLE_VERSION="$V1.$V2.$V3"

[ "$(uname -s)" = Darwin ] || fail "this has to run on macOS (it needs hdiutil and codesign)"

BIN="${2:-${BLOCKFORGE_BIN:-}}"
if [ -z "$BIN" ]; then
    for candidate in "$CRATE/target/release/blockforge" "$CRATE/target/aarch64-apple-darwin/release/blockforge"; do
        if [ -f "$candidate" ]; then
            BIN="$candidate"
            break
        fi
    done
fi
[ -n "$BIN" ] && [ -f "$BIN" ] || fail "blockforge binary not found. Build it first: MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release"
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"

ARCHS="$(lipo -archs "$BIN" 2>/dev/null)" || fail "$BIN is not a Mach-O program"
[ "$ARCHS" = arm64 ] || fail "$BIN is built for '$ARCHS', but the disk image is for Apple Silicon (arm64). Build on an Apple Silicon Mac or with --target aarch64-apple-darwin"

# the app must not claim to run on a macOS older than its program needs, or it would crash instead of explaining
ver_num() { echo "$1" | awk -F. '{ printf "%d", $1 * 10000 + $2 * 100 + ($3 + 0) }'; }
BIN_MIN_OS="$(otool -l "$BIN" | awk '/LC_BUILD_VERSION/ { f = 1 } f && /minos/ { print $2; exit }')"
MIN_OS="$DEFAULT_MIN_OS"
if [ -n "$BIN_MIN_OS" ] && [ "$(ver_num "$BIN_MIN_OS")" -gt "$(ver_num "$MIN_OS")" ]; then
    echo "note: the program needs macOS $BIN_MIN_OS (build with MACOSX_DEPLOYMENT_TARGET=$DEFAULT_MIN_OS to run on older ones)"
    MIN_OS="$BIN_MIN_OS"
fi

for f in Info.plist.in BlockForge.icns dmg-background.png dmg-background@2x.png dmg-settings.py; do
    [ -f "$HERE/$f" ] || fail "missing $HERE/$f"
done

OUT_DIR="${OUT_DIR:-$CRATE/dist}"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
DMG_NAME="BlockForge-$VERSION-macos-arm64.dmg"
DMG="$OUT_DIR/$DMG_NAME"

STAGE="$(mktemp -d "${TMPDIR:-/tmp}/blockforge-dmg.XXXXXX")"
cleanup() {
    rm -rf "$STAGE"
}
trap cleanup EXIT

# ---- BlockForge.app ---------------------------------------------------------------------------------------------------
step "Making BlockForge.app $BUNDLE_VERSION (macOS $MIN_OS or newer)"
APP="$STAGE/BlockForge.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
install -m 755 "$BIN" "$APP/Contents/MacOS/blockforge"
cp "$HERE/BlockForge.icns" "$APP/Contents/Resources/BlockForge.icns"
if [ -f "$CRATE/LICENSE" ]; then
    cp "$CRATE/LICENSE" "$APP/Contents/Resources/LICENSE.txt"
fi
sed -e "s/@VERSION@/$BUNDLE_VERSION/g" -e "s/@MIN_OS@/$MIN_OS/g" "$HERE/Info.plist.in" > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"
plutil -lint "$APP/Contents/Info.plist" || fail "Info.plist is not valid"

step "Signing it ad hoc"
xattr -cr "$APP"
codesign --force --deep --sign - --identifier "$BUNDLE_ID" "$APP" || fail "codesign failed"
codesign --verify --deep --strict --verbose=2 "$APP" || fail "the signature of BlockForge.app does not verify"
codesign -dv "$APP" 2>&1 | grep -E 'Identifier|Signature|Format' || true

# ---- disk image -------------------------------------------------------------------------------------------------------
RAW="$STAGE/BlockForge-raw.dmg"

make_styled_dmg() {
    command -v python3 > /dev/null || { echo "python3 not found"; return 1; }
    python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)' || { echo "python3 older than 3.10"; return 1; }
    python3 -m venv "$STAGE/venv" || return 1
    "$STAGE/venv/bin/python" -m pip install --quiet --disable-pip-version-check "$DMGBUILD_SPEC" || return 1
    set -- -s "$HERE/dmg-settings.py" -D "app=$APP" -D "background=$HERE/dmg-background.png"
    # the disk's own icon needs SetFile (Xcode's command line tools); without it the disk simply has the default icon
    if [ -x /usr/bin/SetFile ]; then
        set -- "$@" -D "volicon=$HERE/BlockForge.icns"
    fi
    "$STAGE/venv/bin/dmgbuild" "$@" "BlockForge" "$RAW"
}

make_plain_dmg() {
    local folder="$STAGE/plain"
    mkdir -p "$folder"
    ditto "$APP" "$folder/BlockForge.app"
    ln -s /Applications "$folder/Applications"
    local try
    for try in 1 2 3; do
        rm -f "$RAW"
        if hdiutil create -volname BlockForge -srcfolder "$folder" -fs HFS+ -format UDZO -ov "$RAW"; then
            return 0
        fi
        echo "hdiutil create failed (try $try of 3)"
        sleep 5
    done
    return 1
}

step "Making the disk image"
if make_styled_dmg; then
    echo "disk image made with dmgbuild"
else
    echo "warning: dmgbuild did not work, making a plain disk image (no background or icon layout)" >&2
    rm -f "$RAW"
    make_plain_dmg || fail "hdiutil could not make the disk image"
fi
[ -f "$RAW" ] || fail "no disk image was made"

rm -f "$DMG"
mv "$RAW" "$DMG"
hdiutil verify "$DMG" > /dev/null || fail "$DMG does not verify"

step "Done"
ls -l "$DMG"
shasum -a 256 "$DMG"
