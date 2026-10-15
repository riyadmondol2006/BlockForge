#!/bin/sh
# Starts BlockForge with a class data sharing archive, which makes it open noticeably faster.
# The archive is created automatically the first time and reused after that (needs Java 19 or newer,
# on older Java the launcher simply starts the normal way).

DIR="$(cd "$(dirname "$0")" && pwd)"
JAR="${BLOCKFORGE_JAR:-$DIR/BlockForge.jar}"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/blockforge"
mkdir -p "$CACHE"

JAVA="${JAVA_HOME:+$JAVA_HOME/bin/}java"
# one garbage collector thread and a heap that gives unused memory back, which suits a small desktop program on
# slower computers (the same options the installers use). The heap starts small because this collector never shrinks
# it below its starting size.
GC_OPTIONS="-XX:+UseSerialGC -XX:MinHeapFreeRatio=10 -XX:MaxHeapFreeRatio=30 -Xms64m"
MAJOR=$("$JAVA" -version 2>&1 | sed -n 's/.*version "\([0-9]*\).*/\1/p' | head -n 1)

if [ -n "$MAJOR" ] && [ "$MAJOR" -ge 19 ]; then
    # shellcheck disable=SC2086 # GC_OPTIONS is meant to split into separate options
    exec "$JAVA" $GC_OPTIONS -XX:+AutoCreateSharedArchive -XX:SharedArchiveFile="$CACHE/blockforge.jsa" \
        -jar "$JAR" "$@"
fi

# shellcheck disable=SC2086
exec "$JAVA" $GC_OPTIONS -jar "$JAR" "$@"
