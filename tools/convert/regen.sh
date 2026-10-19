#!/bin/sh
# Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
# Regenerate data/convert/*.z from a Chunker clone. Needs: JDK 17+ (javac, java), python3, curl, git.
# Usage: tools/convert/regen.sh <chunker-clone> [scratch-dir]      (default clone: ../Chunker)
set -e
CHUNKER="${1:-../Chunker}"; SCRATCH="${2:-${TMPDIR:-/tmp}/convert-tables}"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
mkdir -p "$SCRATCH"
"$HERE/fetch_jars.sh" "$SCRATCH/jars"
"$HERE/build_dump.sh" "$CHUNKER" "$SCRATCH/jars" "$SCRATCH/classes"
CP="$SCRATCH/classes:$(ls "$SCRATCH"/jars/*.jar | tr '\n' ':')"
rm -rf "$SCRATCH/dumps"
for e in java bedrock; do
  V=$(python3 "$HERE/versions.py" "$CHUNKER" $e | tr '\n' ' ')
  java -Xmx3g -cp "$CP" Dump "$SCRATCH/dumps" $e $V
done
python3 "$HERE/post.py" "$SCRATCH/dumps" "$CHUNKER" "$ROOT/data/convert" --verify
