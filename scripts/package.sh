#!/bin/sh
# Builds a native installer that carries its own Java, so people don't need to install Java first.
#
#   scripts/package.sh <app-image|deb|rpm|dmg> [jar] [output-folder]
#
# Needs a JDK 17 or newer with jpackage. deb needs dpkg-deb and fakeroot, rpm needs rpmbuild, dmg needs macOS.
# Windows has its own script: scripts/package.ps1
set -eu

TYPE="${1:?usage: package.sh <app-image|deb|rpm|dmg> [jar] [output-folder]}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
JAR="${2:-$(ls "$ROOT"/build/libs/BlockForge-*.jar | head -n 1)}"
OUT="${3:-$ROOT/build/installers}"
JPACKAGE="${JAVA_HOME:+$JAVA_HOME/bin/}jpackage"

[ -f "$JAR" ] || { echo "jar not found: $JAR (build it with ./gradlew shadowJar)"; exit 1; }

# 3.4.41.4.Beta -> 3.4.41.4. Mac installers only allow three numbers, so the last one is dropped there.
FULL_VERSION="$(basename "$JAR" .jar | sed 's/^BlockForge-//; s/\.Beta$//')"
SHORT_VERSION="$(echo "$FULL_VERSION" | cut -d. -f1-3)"

# the parts of Java the launcher needs: all of the standard ones plus the extras that HTTPS, locales and zip files use
MODULES="java.se,jdk.unsupported,jdk.crypto.ec,jdk.crypto.cryptoki,jdk.localedata,jdk.zipfs,jdk.charsets,jdk.management,jdk.management.agent,jdk.net,jdk.httpserver,jdk.jfr,jdk.xml.dom,jdk.naming.dns,jdk.security.auth,jdk.security.jgss"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp "$JAR" "$STAGE/"
mkdir -p "$OUT"

# -Dblockforge.installed makes the launcher keep its files in the user's own folder instead of next to the program
set -- --name BlockForge \
    --app-version "$FULL_VERSION" \
    --vendor BlockForge \
    --description "A Minecraft launcher with a modern interface that plays offline without an account." \
    --copyright "Copyright ATLauncher and BlockForge contributors, GPL-3.0" \
    --input "$STAGE" --main-jar "$(basename "$JAR")" --main-class com.blockforge.launcher.App \
    --add-modules "$MODULES" \
    --jlink-options "--strip-debug --no-header-files --no-man-pages" \
    --java-options -Dblockforge.installed=true \
    --java-options -Dawt.useSystemAAFontSettings=on \
    --java-options -Dswing.aatext=true \
    --dest "$OUT"

case "$TYPE" in
    app-image)
        exec "$JPACKAGE" --type app-image "$@" --icon "$ROOT/src/main/resources/assets/image/icon.png"
        ;;
    deb|rpm)
        # the libraries Minecraft itself needs from the system (graphics and sound), on top of what jpackage adds
        if [ "$TYPE" = deb ]; then
            DEPS="libgl1,libasound2 | libasound2t64,libx11-6,libxext6,libxi6,libxrender1,libxtst6"
        else
            DEPS="mesa-libGL,alsa-lib"
        fi
        exec "$JPACKAGE" --type "$TYPE" "$@" \
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
        ;;
    dmg)
        # the version must be three numbers, so replace the full one that was set above
        set -- "$@" --app-version "$SHORT_VERSION"
        exec "$JPACKAGE" --type dmg "$@" \
            --license-file "$ROOT/LICENSE" \
            --icon "$ROOT/src/main/resources/assets/image/icon.icns" \
            --mac-package-name BlockForge \
            --mac-package-identifier com.blockforge.launcher \
            --mac-app-category public.app-category.games
        ;;
    *)
        echo "unknown type: $TYPE"
        exit 1
        ;;
esac
