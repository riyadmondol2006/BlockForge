#!/usr/bin/env bash
# Builds the fast-start agent jar: blockforge-boost.jar
#   ./build.sh                  writes ./blockforge-boost.jar (the file the launcher embeds)
#   OUT=/some/dir ./build.sh    writes it there instead
# Needs a JDK (javac and jar), 8 or newer. The jar runs on Java 8 and up.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="${OUT:-$here}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/classes" "$out"

if ! javac --release 8 -Xlint:-options -encoding UTF-8 -d "$work/classes" "$here/Boost.java" 2>"$work/javac.log"; then
    if grep -q "not supported\|release version" "$work/javac.log"; then
        echo "javac has no --release 8, using --release 11" >&2
        javac --release 11 -encoding UTF-8 -d "$work/classes" "$here/Boost.java"
    else
        cat "$work/javac.log" >&2
        exit 1
    fi
fi

cat > "$work/manifest.txt" <<MANIFEST
Premain-Class: com.blockforge.boost.Boost
Agent-Class: com.blockforge.boost.Boost
Implementation-Title: BlockForge Boost
MANIFEST

jar cfm "$out/blockforge-boost.jar" "$work/manifest.txt" -C "$work/classes" .
echo "Built $out/blockforge-boost.jar"
