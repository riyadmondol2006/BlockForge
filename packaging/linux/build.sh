#!/bin/sh
# Builds the Linux packages from the release binary:
#
#   packaging/linux/build.sh <version> [output-folder]
#
#   BlockForge-<version>-linux-x86_64.deb       Debian, Ubuntu and relatives
#   BlockForge-<version>-linux-x86_64.rpm       Fedora, openSUSE and relatives
#   BlockForge-<version>-linux-x86_64.AppImage  runs anywhere, adds its own menu entry (src/desktop_integration.rs)
#
# The output folder is dist/ next to Cargo.toml unless given. It needs the binary (cargo build --release) and
# dpkg-deb, rpmbuild, curl, sha256sum and mksquashfs (Debian: dpkg-dev rpm curl squashfs-tools; no root, no FUSE, no
# docker). The AppImage tools are downloaded once, checked against the checksums below and cached.
#
# Environment (all optional):
#   BLOCKFORGE_BIN        the binary to package (default: target/release/blockforge)
#   SOURCE_DATE_EPOCH     the release date (default: the date of the last commit, else now)
#   APPIMAGETOOL          an appimagetool to use instead of downloading one
#   APPIMAGE_RUNTIME      an AppImage runtime file to use instead of downloading one
#   BLOCKFORGE_TOOLS_DIR  where the downloaded tools are kept (default: ~/.cache/blockforge-packaging)
#   ONLY                  build only some of "deb rpm appimage" (default: all three)
set -eu

VERSION="${1:?usage: build.sh <version> [output-folder]}"
VERSION="${VERSION#v}"
case "$VERSION" in
    [0-9]*.[0-9]*.[0-9]* ) ;;
    *) echo "the version must look like 4.0.0 or 4.0.0-beta.1, not '$VERSION'" >&2; exit 1 ;;
esac
case "$VERSION" in
    *[!0-9A-Za-z.+-]*) echo "unexpected characters in the version '$VERSION'" >&2; exit 1 ;;
esac

HERE="$(cd "$(dirname "$0")" && pwd)"
CRATE="$(cd "$HERE/../.." && pwd)"
ROOT="$(cd "$CRATE/.." && pwd)"
BIN="${BLOCKFORGE_BIN:-$CRATE/target/release/blockforge}"
OUT="${2:-$CRATE/dist}"
ONLY="${ONLY:-deb rpm appimage}"
ARCH=x86_64
NAME="BlockForge-$VERSION-linux-$ARCH"

# pinned AppImage tools (https://github.com/AppImage/appimagetool, https://github.com/AppImage/type2-runtime)
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage"
APPIMAGETOOL_SHA256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"
RUNTIME_URL="https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64"
RUNTIME_SHA256="2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"

ICON_PNG="$ROOT/docs/logo/png"
ICON_SVG="$ROOT/docs/logo/blockforge-icon.svg"
ICON_SIZES="16 24 32 48 64 128 256 512"
METAINFO_ID=io.github.riyadmondol2006.BlockForge

die() { echo "build.sh: $*" >&2; exit 1; }
want() { case " $ONLY " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }
need() { command -v "$1" > /dev/null 2>&1 || die "$1 is needed ($2)"; }

[ -f "$BIN" ] || die "$BIN not found (build it with: cargo build --release)"
[ "$(uname -m)" = x86_64 ] || echo "build.sh: not an x86_64 computer, only the files are packed, nothing is run" >&2
for f in "$HERE/blockforge.desktop" "$HERE/blockforge-mime.xml" "$HERE/blockforge.1" \
    "$HERE/$METAINFO_ID.metainfo.xml" "$ROOT/LICENSE" "$ICON_SVG"; do
    [ -f "$f" ] || die "$f is missing"
done
for s in $ICON_SIZES; do [ -f "$ICON_PNG/blockforge-$s.png" ] || die "$ICON_PNG/blockforge-$s.png is missing"; done

# the binary has to be the version being packaged, or the files would be named for one thing and hold another
if [ "$(uname -m)" = x86_64 ] && have="$("$BIN" --version 2> /dev/null)"; then
    [ "$have" = "blockforge $VERSION" ] || die "the binary says '$have', not 'blockforge $VERSION'"
fi

# date of the release, for the changelog and the AppStream file
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
    SOURCE_DATE_EPOCH="$(git -C "$ROOT" log -1 --format=%ct 2> /dev/null || true)"
    case "$SOURCE_DATE_EPOCH" in "" | *[!0-9]*) SOURCE_DATE_EPOCH="$(date +%s)" ;; esac
fi
export SOURCE_DATE_EPOCH
DATE_ISO="$(date -u -d "@$SOURCE_DATE_EPOCH" +%Y-%m-%d)"
DATE_RFC="$(date -u -d "@$SOURCE_DATE_EPOCH" -R)"
DATE_RPM="$(LC_ALL=C date -u -d "@$SOURCE_DATE_EPOCH" '+%a %b %d %Y')"
# 4.0.0-beta.1 sorts before 4.0.0 in both package managers when written with a tilde
PKG_VERSION="$(echo "$VERSION" | sed 's/-/~/')"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
umask 022
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

# ---- the files every package installs, laid out as they are on the disk -------------------------------------------
STAGE="$WORK/stage"
install -d "$STAGE/usr/bin" "$STAGE/usr/share/applications" "$STAGE/usr/share/metainfo" \
    "$STAGE/usr/share/mime/packages" "$STAGE/usr/share/man/man1" "$STAGE/usr/share/icons/hicolor/scalable/apps"
install -m 0755 "$BIN" "$STAGE/usr/bin/blockforge"
install -m 0644 "$HERE/blockforge.desktop" "$STAGE/usr/share/applications/blockforge.desktop"
install -m 0644 "$HERE/blockforge-mime.xml" "$STAGE/usr/share/mime/packages/blockforge.xml"
sed "s|<release version=\"[^\"]*\" date=\"[^\"]*\"/>|<release version=\"$VERSION\" date=\"$DATE_ISO\"/>|" "$HERE/$METAINFO_ID.metainfo.xml" \
    > "$STAGE/usr/share/metainfo/$METAINFO_ID.metainfo.xml"
chmod 0644 "$STAGE/usr/share/metainfo/$METAINFO_ID.metainfo.xml"
gzip -9n -c "$HERE/blockforge.1" > "$STAGE/usr/share/man/man1/blockforge.1.gz"
chmod 0644 "$STAGE/usr/share/man/man1/blockforge.1.gz"
install -m 0644 "$ICON_SVG" "$STAGE/usr/share/icons/hicolor/scalable/apps/blockforge.svg"
for s in $ICON_SIZES; do
    install -d "$STAGE/usr/share/icons/hicolor/${s}x${s}/apps"
    install -m 0644 "$ICON_PNG/blockforge-$s.png" "$STAGE/usr/share/icons/hicolor/${s}x${s}/apps/blockforge.png"
done

# the files must be valid before they go into three packages
if command -v desktop-file-validate > /dev/null 2>&1; then
    desktop-file-validate "$STAGE/usr/share/applications/blockforge.desktop"
fi
if command -v appstreamcli > /dev/null 2>&1; then
    appstreamcli validate --no-net "$STAGE/usr/share/metainfo/$METAINFO_ID.metainfo.xml"
fi

# ---- deb ------------------------------------------------------------------------------------------------------------
build_deb() {
    need dpkg-deb "Debian: dpkg-dev"
    D="$WORK/deb"
    cp -a "$STAGE" "$D"
    install -d "$D/DEBIAN" "$D/usr/share/doc/blockforge"
    cat > "$D/usr/share/doc/blockforge/copyright.in" << 'COPYRIGHT'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: BlockForge
Upstream-Contact: BlockForge contributors <riyadmondol2006@gmail.com>
Source: https://github.com/riyadmondol2006/BlockForge

Files: *
Copyright: 2013-@YEAR@ ATLauncher contributors
           2026 BlockForge contributors
License: GPL-3+

License: GPL-3+
 This program is free software: you can redistribute it and/or modify it under
 the terms of the GNU General Public License as published by the Free Software
 Foundation, either version 3 of the License, or (at your option) any later
 version.
 .
 This program is distributed in the hope that it will be useful, but WITHOUT
 ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
 FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
 .
 On Debian systems, the complete text of the GNU General Public License
 version 3 can be found in "/usr/share/common-licenses/GPL-3".
COPYRIGHT
    sed "s/@YEAR@/$(echo "$DATE_ISO" | cut -c1-4)/" "$D/usr/share/doc/blockforge/copyright.in" > "$D/usr/share/doc/blockforge/copyright"
    rm "$D/usr/share/doc/blockforge/copyright.in"
    {
        echo "blockforge ($PKG_VERSION-1) unstable; urgency=medium"
        echo
        echo "  * The Rust edition of BlockForge. It replaces the Java edition."
        echo
        echo " -- BlockForge contributors <riyadmondol2006@gmail.com>  $DATE_RFC"
    } | gzip -9n > "$D/usr/share/doc/blockforge/changelog.Debian.gz"
    install -D -m 0644 "$HERE/deb/lintian-overrides" "$D/usr/share/lintian/overrides/blockforge"
    chmod 0644 "$D/usr/share/doc/blockforge/copyright" "$D/usr/share/doc/blockforge/changelog.Debian.gz"

    # the C library version the binary needs, so the package cannot be installed where it would not start
    DEPENDS="libc6, libgcc-s1"
    if command -v objdump > /dev/null 2>&1; then
        glibc="$(objdump -T "$BIN" 2>/dev/null | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/GLIBC_//' | sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1)"
        [ -n "$glibc" ] && DEPENDS="libc6 (>= $glibc), libgcc-s1"
    fi
    # opened at run time (dlopen), so dpkg-shlibdeps cannot see them
    DEPENDS="$DEPENDS, libdbus-1-3, libegl1, libgl1, libwayland-client0, libwayland-egl1, libx11-6, libx11-xcb1, libxcursor1, libxi6, libxkbcommon0, libxkbcommon-x11-0, libxrender1"
    SIZE_KB="$(du -sk --exclude=DEBIAN "$D" | cut -f1)"
    sed "s/@DEB_VERSION@/$PKG_VERSION-1/; s/@SIZE_KB@/$SIZE_KB/; s/@DEPENDS@/$DEPENDS/" "$HERE/deb/control.in" \
        > "$D/DEBIAN/control"
    install -m 0755 "$HERE/deb/postinst" "$HERE/deb/postrm" "$D/DEBIAN/"
    (cd "$D" && find usr -type f -print0 | sort -z | xargs -0 md5sum > DEBIAN/md5sums)
    chmod 0644 "$D/DEBIAN/control" "$D/DEBIAN/md5sums"
    find "$D" -type d -exec chmod 0755 {} +
    dpkg-deb --root-owner-group -Zxz --build "$D" "$OUT/$NAME.deb" > /dev/null
    echo "Built $OUT/$NAME.deb"
}

# ---- rpm ------------------------------------------------------------------------------------------------------------
build_rpm() {
    need rpmbuild "Debian: rpm"
    R="$WORK/rpm"
    mkdir -p "$R/SPECS" "$R/BUILD" "$R/RPMS" "$R/SRPMS" "$R/BUILDROOT"
    RS="$WORK/rpm-stage"
    cp -a "$STAGE" "$RS"
    install -d "$RS/usr/share/licenses/blockforge"
    install -m 0644 "$ROOT/LICENSE" "$RS/usr/share/licenses/blockforge/LICENSE"
    cp "$HERE/rpm/blockforge.spec" "$R/SPECS/"
    # --buildroot keeps rpmbuild away from the user's own ~/rpmbuild; the date is the one in the changelog
    rpmbuild -bb "$R/SPECS/blockforge.spec" \
        --define "_topdir $R" \
        --define "_stage $RS" \
        --define "bf_version $PKG_VERSION" \
        --define "bf_date $DATE_RPM" \
        --define "dist %{nil}" \
        --define "_buildhost build" \
        --define "source_date_epoch_from_changelog 0" \
        --define "use_source_date_epoch_as_buildtime 1" \
        --define "clamp_mtime_to_source_date_epoch 1" \
        --target "$ARCH-linux" \
        --buildroot "$R/BUILDROOT/root" > "$WORK/rpmbuild.log" 2>&1 || { cat "$WORK/rpmbuild.log" >&2; die "rpmbuild failed"; }
    cp "$R"/RPMS/*/blockforge-*.rpm "$OUT/$NAME.rpm"
    echo "Built $OUT/$NAME.rpm"
}

# ---- AppImage -------------------------------------------------------------------------------------------------------
# downloads $1 to $2 unless it is there already, and checks it against the checksum $3
fetch() {
    if [ ! -f "$2" ]; then
        need curl "to download $1"
        mkdir -p "$(dirname "$2")"
        curl -fsSL --retry 3 -o "$2.part" "$1" || die "could not download $1"
        mv "$2.part" "$2"
    fi
    echo "$3  $2" | sha256sum -c - > /dev/null 2>&1 || { rm -f "$2"; die "$1 does not match its checksum"; }
}

build_appimage() {
    TOOLS="${BLOCKFORGE_TOOLS_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/blockforge-packaging}"
    TOOL="${APPIMAGETOOL:-$TOOLS/appimagetool-x86_64.AppImage}"
    RUNTIME="${APPIMAGE_RUNTIME:-$TOOLS/runtime-x86_64}"
    [ -n "${APPIMAGETOOL:-}" ] || fetch "$APPIMAGETOOL_URL" "$TOOL" "$APPIMAGETOOL_SHA256"
    [ -n "${APPIMAGE_RUNTIME:-}" ] || fetch "$RUNTIME_URL" "$RUNTIME" "$RUNTIME_SHA256"
    chmod +x "$TOOL"
    need mksquashfs "Debian: squashfs-tools"

    A="$WORK/BlockForge.AppDir"
    cp -a "$STAGE" "$A"
    # the manual and the other stock files don't belong in the image; the menu entry, icons and AppStream file do
    rm -rf "$A/usr/share/man"
    install -m 0755 "$HERE/AppRun" "$A/AppRun"
    # appimagetool wants the desktop file, the icon and .DirIcon at the top
    cp "$HERE/blockforge.desktop" "$A/blockforge.desktop"
    printf 'X-AppImage-Name=BlockForge\nX-AppImage-Version=%s\n' "$VERSION" >> "$A/blockforge.desktop"
    chmod 0644 "$A/blockforge.desktop"
    cp "$ICON_PNG/blockforge-256.png" "$A/blockforge.png"
    ln -s blockforge.png "$A/.DirIcon"

    # --appimage-extract-and-run: the tool is an AppImage itself, and CI machines have no FUSE
    ARCH=$ARCH VERSION="$VERSION" "$TOOL" --appimage-extract-and-run --no-appstream --runtime-file "$RUNTIME" \
        "$A" "$OUT/$NAME.AppImage" > "$WORK/appimagetool.log" 2>&1 || { cat "$WORK/appimagetool.log" >&2; die "appimagetool failed"; }
    chmod 0755 "$OUT/$NAME.AppImage"
    echo "Built $OUT/$NAME.AppImage"
}

want deb && build_deb
want rpm && build_rpm
want appimage && build_appimage
exit 0
