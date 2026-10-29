#!/bin/bash
# Mounts the BlockForge disk image and checks what a person gets from it: the app's layout, its Info.plist, its icon,
# its signature, the Applications link, and that the program starts. Meant for CI (macos-26), but works on any
# Apple Silicon Mac. It only reads the image and copies the app into a temporary folder.
#
#   packaging/macos/smoke.sh [dist/BlockForge-<version>-macos-arm64.dmg]
#
# SMOKE_GUI=1 also opens the window for a few seconds and checks that it stays up (needs a graphical session).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
CRATE="$(cd "$HERE/../.." && pwd)"
PB="${PLISTBUDDY:-/usr/libexec/PlistBuddy}"
PROBLEMS=0

fail() {
    echo "smoke.sh: error: $*" >&2
    exit 1
}
ok() {
    echo "  ok    $*"
}
check() { # check <description> <command...>
    local what="$1"
    shift
    if "$@" > /dev/null 2>&1; then
        ok "$what"
    else
        echo "  FAIL  $what"
        PROBLEMS=$((PROBLEMS + 1))
    fi
}
plist() { # plist <key> -> the value, or nothing
    "$PB" -c "Print :$1" "$APP/Contents/Info.plist" 2> /dev/null || true
}
expect_plist() { # expect_plist <key> <value>
    local got
    got="$(plist "$1")"
    if [ "$got" = "$2" ]; then
        ok "Info.plist $1 = $2"
    else
        echo "  FAIL  Info.plist $1 is '$got', expected '$2'"
        PROBLEMS=$((PROBLEMS + 1))
    fi
}

[ "$(uname -s)" = Darwin ] || fail "this has to run on macOS"

DMG="${1:-}"
if [ -z "$DMG" ]; then
    for candidate in "$CRATE"/dist/BlockForge-*-macos-arm64.dmg; do
        [ -f "$candidate" ] && DMG="$candidate"
    done
fi
[ -n "$DMG" ] && [ -f "$DMG" ] || fail "no disk image given and none in dist/ (build it with packaging/macos/build.sh)"
DMG="$(cd "$(dirname "$DMG")" && pwd)/$(basename "$DMG")"
case "$(basename "$DMG")" in
    BlockForge-*-macos-arm64.dmg) ;;
    *) fail "the disk image should be called BlockForge-<version>-macos-arm64.dmg, this one is $(basename "$DMG")" ;;
esac
FILE_VERSION="$(basename "$DMG" | sed -e 's/^BlockForge-//' -e 's/-macos-arm64\.dmg$//')"
BUNDLE_VERSION="$(echo "$FILE_VERSION" | cut -d. -f1-3)"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/blockforge-smoke.XXXXXX")"
MNT="$WORK/mnt"
mkdir -p "$MNT"
MOUNTED=0
cleanup() {
    if [ "$MOUNTED" = 1 ]; then
        hdiutil detach "$MNT" -quiet 2> /dev/null || { sleep 2; hdiutil detach "$MNT" -force -quiet 2> /dev/null || true; }
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT

echo "Smoke test of $DMG"
hdiutil verify "$DMG" > /dev/null || fail "hdiutil verify failed for $DMG"
ok "the disk image verifies"
hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$MNT" "$DMG" > /dev/null || fail "could not mount $DMG"
MOUNTED=1
ok "the disk image mounts"

# ---- what is on the disk ----------------------------------------------------------------------------------------------
APP="$MNT/BlockForge.app"
# shellcheck disable=SC2012
[ -d "$APP" ] || fail "BlockForge.app is not on the disk image (found: $(ls -A "$MNT" | tr '\n' ' '))"
ok "BlockForge.app is on the disk image"
if [ -L "$MNT/Applications" ] && [ "$(readlink "$MNT/Applications")" = /Applications ]; then
    ok "the Applications link points to /Applications"
else
    echo "  FAIL  Applications is missing or does not point to /Applications"
    PROBLEMS=$((PROBLEMS + 1))
fi
if [ -f "$MNT/.DS_Store" ] && [ -n "$(ls -A "$MNT/.background" 2> /dev/null)" ]; then
    ok "the window has its background and icon layout"
else
    echo "  note  the disk image is plain (no background or layout): build.sh fell back because dmgbuild did not work"
fi

# ---- the bundle -------------------------------------------------------------------------------------------------------
EXE="$APP/Contents/MacOS/blockforge"
check "Contents/MacOS/blockforge is an executable file" test -x "$EXE"
check "Info.plist passes plutil -lint" plutil -lint "$APP/Contents/Info.plist"
expect_plist CFBundleIdentifier io.github.riyadmondol2006.BlockForge
expect_plist CFBundleName BlockForge
expect_plist CFBundleDisplayName BlockForge
expect_plist CFBundleExecutable blockforge
expect_plist CFBundlePackageType APPL
expect_plist CFBundleShortVersionString "$BUNDLE_VERSION"
expect_plist CFBundleVersion "$BUNDLE_VERSION"
expect_plist NSHighResolutionCapable true
expect_plist LSApplicationCategoryType public.app-category.games
expect_plist CFBundleDocumentTypes:0:CFBundleTypeExtensions:0 mrpack
MIN_OS="$(plist LSMinimumSystemVersion)"
case "$MIN_OS" in
    [0-9]*.*) ok "Info.plist LSMinimumSystemVersion = $MIN_OS" ;;
    *) echo "  FAIL  Info.plist has no LSMinimumSystemVersion"; PROBLEMS=$((PROBLEMS + 1)) ;;
esac
ICON_NAME="$(plist CFBundleIconFile)"
check "the icon named in Info.plist ($ICON_NAME) is in Resources" test -f "$APP/Contents/Resources/${ICON_NAME%.icns}.icns"
# shellcheck disable=SC2016
check "the icon is a valid icns with all sizes" bash -c '
    iconutil --convert iconset --output "$1/icon.iconset" "$2" && [ "$(ls "$1/icon.iconset" | wc -l)" -ge 10 ]' _ "$WORK" "$APP/Contents/Resources/${ICON_NAME%.icns}.icns"

# ---- program and signature --------------------------------------------------------------------------------------------
ARCHS="$(lipo -archs "$EXE" 2> /dev/null || true)"
if [ "$ARCHS" = arm64 ]; then ok "the program is arm64"; else echo "  FAIL  the program is '$ARCHS', expected arm64"; PROBLEMS=$((PROBLEMS + 1)); fi
OUTSIDE="$(otool -L "$EXE" | tail -n +2 | awk '{ print $1 }' | grep -v -E '^(/usr/lib/|/System/|@)' || true)"
if [ -z "$OUTSIDE" ]; then
    ok "the program only needs libraries that come with macOS"
else
    echo "  FAIL  the program needs libraries that are not part of macOS:"
    echo "$OUTSIDE"
    PROBLEMS=$((PROBLEMS + 1))
fi
check "the signature verifies (codesign --verify --deep --strict)" codesign --verify --deep --strict "$APP"
SIGNATURE="$(codesign -dv "$APP" 2>&1 || true)"
if echo "$SIGNATURE" | grep -q 'Signature=adhoc'; then ok "the app is signed ad hoc"; else echo "  FAIL  the app is not signed ad hoc: $SIGNATURE"; PROBLEMS=$((PROBLEMS + 1)); fi
echo "  info  Gatekeeper says: $(spctl --assess --type execute "$APP" 2>&1 | head -1) (rejected is expected: there is no Apple developer signature)"

# ---- run it from the disk image and from a copy, the way it is after dragging it to Applications ---------------------
INSTALLED="$WORK/Applications/BlockForge.app"
mkdir -p "$WORK/Applications"
ditto "$APP" "$INSTALLED"
# a download in a browser marks the app as quarantined; that must not break the signature
xattr -w com.apple.quarantine "0081;00000000;Safari;" "$INSTALLED"
check "the copy still verifies with the quarantine flag set" codesign --verify --deep --strict "$INSTALLED"

EXPECTED="blockforge $BUNDLE_VERSION"
for program in "$EXE" "$INSTALLED/Contents/MacOS/blockforge"; do
    if OUT="$(BLOCKFORGE_DIR="$WORK/data" "$program" --version 2>&1)"; then
        case "$OUT" in
            "$EXPECTED"*) ok "$(basename "$(dirname "$(dirname "$(dirname "$program")")")") --version printed '$OUT'" ;;
            *) echo "  FAIL  --version printed '$OUT', expected '$EXPECTED'"; PROBLEMS=$((PROBLEMS + 1)) ;;
        esac
    else
        echo "  FAIL  --version failed: $OUT"
        PROBLEMS=$((PROBLEMS + 1))
    fi
done
if OUT="$(BLOCKFORGE_DIR="$WORK/data" "$INSTALLED/Contents/MacOS/blockforge" paths 2>&1)" && echo "$OUT" | grep -q "$WORK/data"; then
    ok "paths prints the data folder (BLOCKFORGE_DIR is honoured)"
else
    echo "  FAIL  paths did not print the data folder: $OUT"
    PROBLEMS=$((PROBLEMS + 1))
fi

if [ "${SMOKE_GUI:-0}" = 1 ]; then
    BLOCKFORGE_DIR="$WORK/data" "$INSTALLED/Contents/MacOS/blockforge" > "$WORK/gui.log" 2>&1 &
    GUI_PID=$!
    sleep 8
    if kill -0 "$GUI_PID" 2> /dev/null; then
        ok "the window stays open for 8 seconds"
        kill "$GUI_PID" 2> /dev/null || true
    else
        echo "  FAIL  the window closed by itself:"
        tail -n 20 "$WORK/gui.log"
        PROBLEMS=$((PROBLEMS + 1))
    fi
fi

if [ "$PROBLEMS" -gt 0 ]; then
    echo "Smoke test FAILED: $PROBLEMS problem(s)"
    exit 1
fi
echo "Smoke test passed"
