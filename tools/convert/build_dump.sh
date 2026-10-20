#!/bin/sh
# Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
# Compile the Dump harness together with the parts of Chunker it needs.
# Usage: build_dump.sh <chunker-clone> <jars-dir> <out-classes-dir>
# Needs JDK 17+. Jars: guava, failureaccess, gson, fastutil, caffeine, jetbrains annotations, jspecify, leveldb-api,
# leveldb, picocli, lz4-java (see README.md). Chunker uses a patched leveldb; the few symbols that differ are
# replaced by the stubs in stubs/ (they are never executed).
set -e
CH="$1/cli/src/main/java"; JARS="$2"; OUT="$3"
HERE="$(cd "$(dirname "$0")" && pwd)"
CP=$(ls "$JARS"/*.jar | tr '\n' ':')
RES="$CH/com/hivemc/chunker/conversion/encoding"
# Explicit list of every resolver class, so that tables not reachable from Dump.java are compiled too.
FILES=$(find "$RES/java/base/resolver" "$RES/bedrock/base/resolver" -name '*.java' \
  \( -path '*/identifier/*' -o -path '*/biome/*' -o -path '*/entity/*Resolver.java' -o -path '*/entity/*IDResolver.java' -o -path '*/itemstack/*Resolver.java' \) \
  ! -name '*ItemStackResolver.java')
mkdir -p "$OUT"
javac -nowarn -proc:none -Xprefer:source -d "$OUT" -cp "$CP" -sourcepath "$HERE/dump/stubs:$CH" "$HERE/dump/Dump.java" $FILES
