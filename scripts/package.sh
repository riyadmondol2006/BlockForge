#!/bin/sh
# Builds a native installer that carries its own Java, so people don't need to install Java first.
#
#   scripts/package.sh <app-image|deb|rpm|dmg> [jar] [output-folder]
#
# Needs a JDK 17 or newer with jpackage (JAVA_HOME is used when set). deb needs dpkg-deb and fakeroot, rpm needs
# rpmbuild, dmg needs macOS. Windows has its own script: scripts/package.ps1
#
# The files are named the way the releases and the in-app updater expect:
#   BlockForge-<version>-linux-amd64.deb, BlockForge-<version>-linux-x86_64.rpm,
#   BlockForge-<version>-macos-arm64.dmg or BlockForge-<version>-macos-x64.dmg, and a BlockForge folder for app-image
set -eu

TYPE="${1:?usage: package.sh <app-image|deb|rpm|dmg> [jar] [output-folder]}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
JAR="${2:-}"
if [ -z "$JAR" ]; then
    for candidate in "$ROOT"/build/libs/BlockForge-*.jar; do
        JAR="$candidate"
        break
    done
fi
OUT="${3:-$ROOT/build/installers}"
JPACKAGE="${JAVA_HOME:+$JAVA_HOME/bin/}jpackage"

[ -f "$JAR" ] || { echo "jar not found: $JAR (build it with ./gradlew shadowJar)" >&2; exit 1; }

# BlockForge-3.4.41.4.jar -> 3.4.41.4. Mac apps only allow three numbers, so the last one is dropped there.
FULL_VERSION="$(basename "$JAR" .jar | sed 's/^BlockForge-//; s/\.Beta$//')"
SHORT_VERSION="$(echo "$FULL_VERSION" | cut -d. -f1-3)"
case "$FULL_VERSION" in
    *[!0-9.]* | "") echo "can't read a version from the jar name: $JAR" >&2; exit 1 ;;
esac

# the parts of Java the launcher needs: all of the standard ones plus the extras that HTTPS, locales and zip files use
MODULES="java.se,jdk.unsupported,jdk.crypto.ec,jdk.crypto.cryptoki,jdk.localedata,jdk.zipfs,jdk.charsets,jdk.management,jdk.management.agent,jdk.net,jdk.httpserver,jdk.jfr,jdk.xml.dom,jdk.naming.dns,jdk.security.auth,jdk.security.jgss"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/input" "$STAGE/out" "$OUT"
cp "$JAR" "$STAGE/input/"

# -Dblockforge.installed makes the launcher keep its files in the user's own folder instead of next to the program.
# The garbage collector options use one collector thread and give unused memory back to the system, which suits a
# small desktop program on slower computers. There's deliberately no -Xmx: big modpack downloads need the room.
# -Xms64m starts the heap small, because this collector never shrinks it below where it started (1/64 of the memory).
# jpackage writes into an empty folder of its own, and the result is moved to the output folder under its final name.
set -- --name BlockForge \
    --vendor BlockForge \
    --description "A Minecraft launcher with a modern interface that plays offline without an account." \
    --copyright "Copyright ATLauncher and BlockForge contributors, GPL-3.0" \
    --input "$STAGE/input" --main-jar "$(basename "$JAR")" --main-class com.blockforge.launcher.App \
    --add-modules "$MODULES" \
    --jlink-options "--strip-debug --no-header-files --no-man-pages" \
    --java-options -Dblockforge.installed=true \
    --java-options -Dawt.useSystemAAFontSettings=on \
    --java-options -Dswing.aatext=true \
    --java-options -XX:+UseSerialGC \
    --java-options -XX:MinHeapFreeRatio=10 \
    --java-options -XX:MaxHeapFreeRatio=30 \
    --java-options -Xms64m \
    --dest "$STAGE/out"

# moves the one file jpackage made to the output folder under the given name
deliver() {
    for made in "$STAGE"/out/*."$1"; do
        mv -f "$made" "$OUT/$2"
        echo "Built $OUT/$2"
        return 0
    done
    echo "jpackage didn't make a .$1 file" >&2
    exit 1
}

case "$TYPE" in
    app-image)
        "$JPACKAGE" --type app-image "$@" --app-version "$FULL_VERSION" \
            --icon "$ROOT/src/main/resources/assets/image/icon.png"
        rm -rf "$OUT/BlockForge"
        mv "$STAGE/out/BlockForge" "$OUT/BlockForge"
        echo "Built $OUT/BlockForge"
        ;;
    deb | rpm)
        # the libraries Minecraft itself needs from the system (graphics and sound), on top of what jpackage adds
        if [ "$TYPE" = deb ]; then
            DEPS="libgl1,libasound2 | libasound2t64,libx11-6,libxext6,libxi6,libxrender1,libxtst6,libfontconfig1,fonts-dejavu-core"
        else
            DEPS="mesa-libGL,alsa-lib,fontconfig"
        fi
        "$JPACKAGE" --type "$TYPE" "$@" --app-version "$FULL_VERSION" \
            --resource-dir "$ROOT/packaging/jpackage/linux" \
            --linux-package-deps "$DEPS" \
            --license-file "$ROOT/LICENSE" \
            --about-url "https://github.com/riyadmondol2006/BlockForge" \
            --icon "$ROOT/src/main/resources/assets/image/icon.png" \
            --install-dir /opt \
            --linux-package-name blockforge \
            --linux-shortcut \
            --linux-menu-group Game \
            --linux-app-category games \
            --linux-deb-maintainer "riyadmondol2006@gmail.com" \
            --linux-rpm-license-type GPLv3
        if [ "$TYPE" = deb ]; then
            deliver deb "BlockForge-$FULL_VERSION-linux-$(dpkg --print-architecture).deb"
        else
            deliver rpm "BlockForge-$FULL_VERSION-linux-$(uname -m).rpm"
        fi
        ;;
    dmg)
        # name the architecture of the Java that is bundled, which is the one jpackage itself runs on
        case "$(uname -m)" in
            arm64 | aarch64) ARCH=arm64 ;;
            x86_64) ARCH=x64 ;;
            *) ARCH="$(uname -m)" ;;
        esac
        # no --license-file here: jpackage adds it with "hdiutil udifrez", which Apple deprecated in macOS 12 with no
        # replacement, and the GPL doesn't have to be accepted before using the program
        "$JPACKAGE" --type dmg "$@" --app-version "$SHORT_VERSION" \
            --icon "$ROOT/src/main/resources/assets/image/icon.icns" \
            --mac-package-name BlockForge \
            --mac-package-identifier com.blockforge.launcher \
            --mac-app-category public.app-category.games
        deliver dmg "BlockForge-$FULL_VERSION-macos-$ARCH.dmg"
        ;;
    *)
        echo "unknown type: $TYPE" >&2
        exit 1
        ;;
esac
