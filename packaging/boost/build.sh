#!/usr/bin/env bash
# Builds the fast-start agent jar: blockforge-boost.jar
#   ./build.sh                  writes ./blockforge-boost.jar (the file the launcher embeds) and runs its self-test
#   OUT=/some/dir ./build.sh    writes it there instead
#   NO_TEST=1 ./build.sh        skips the self-test
# Needs a JDK 19 or newer (javac and jar --date); the jar runs on Java 8 and up. The same JDK gives a byte-identical jar
# (fixed file dates and order). Check it anywhere with:
#   java -cp blockforge-boost.jar com.blockforge.boost.SelfTest
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="${OUT:-$here}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/classes" "$out"

# --release 8: class files Java 8 can read, and only Java 8 API (no lambdas either: they would cost start-up time)
javac --release 8 -Xlint:-options -encoding UTF-8 -g:none -d "$work/classes" "$here/Boost.java" "$here/SelfTest.java"

cat > "$work/manifest.txt" <<MANIFEST
Premain-Class: com.blockforge.boost.Boost
Agent-Class: com.blockforge.boost.Boost
Implementation-Title: BlockForge Boost
MANIFEST

# fixed dates and a sorted file order, so the same sources give the same jar
(cd "$work/classes" && find . -name '*.class' | LC_ALL=C sort | sed 's|^\./||' > "$work/files.txt")
(cd "$work/classes" && jar --create --file "$work/out.jar" --manifest "$work/manifest.txt" \
    --date 2026-01-01T00:00:00Z @"$work/files.txt")
mv "$work/out.jar" "$out/blockforge-boost.jar"
echo "Built $out/blockforge-boost.jar ($(wc -c < "$out/blockforge-boost.jar") bytes)"

if [ -z "${NO_TEST:-}" ]; then
    java -cp "$out/blockforge-boost.jar" com.blockforge.boost.SelfTest
fi
