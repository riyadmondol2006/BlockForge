#!/usr/bin/env bash
# Builds the bridge jar: BlockForge-<version>.jar
#   ./build.sh [version]        version defaults to the one in ../../Cargo.toml
#   OUT=/some/dir ./build.sh    where the jar goes (default: ./dist)
# Needs a JDK (javac and jar), 8 or newer. The jar runs on Java 8 and up.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

version="${1:-}"
if [ -z "$version" ]; then
    version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$here/../../Cargo.toml" | head -n1)"
fi
[ -n "$version" ] || { echo "Could not work out the version; pass it as the first argument." >&2; exit 1; }

out="${OUT:-$here/dist}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/classes" "$out"

# --release 8 makes the classes run on Java 8; newest JDKs may only offer 11 and up
if ! javac --release 8 -Xlint:-options -encoding UTF-8 -d "$work/classes" "$here/Bridge.java" 2>"$work/javac.log"; then
    if grep -q "not supported\|release version" "$work/javac.log"; then
        echo "javac has no --release 8, using --release 11" >&2
        javac --release 11 -encoding UTF-8 -d "$work/classes" "$here/Bridge.java"
    else
        cat "$work/javac.log" >&2
        exit 1
    fi
fi

cat > "$work/manifest.txt" <<MANIFEST
Main-Class: com.blockforge.launcher.Bridge
Implementation-Title: BlockForge
Implementation-Version: $version
MANIFEST

jar cfm "$out/BlockForge-$version.jar" "$work/manifest.txt" -C "$work/classes" .
echo "Built $out/BlockForge-$version.jar"
