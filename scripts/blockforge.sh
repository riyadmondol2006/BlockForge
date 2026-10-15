#!/bin/sh
# Starts BlockForge with a class data sharing archive, which makes it open noticeably faster.
# The archive is created automatically the first time and reused after that (needs Java 19 or newer,
# on older Java the launcher simply starts the normal way).

DIR="$(cd "$(dirname "$0")" && pwd)"
JAR="${BLOCKFORGE_JAR:-$DIR/BlockForge.jar}"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/blockforge"
mkdir -p "$CACHE"

JAVA="${JAVA_HOME:+$JAVA_HOME/bin/}java"
MAJOR=$("$JAVA" -version 2>&1 | sed -n 's/.*version "\([0-9]*\).*/\1/p' | head -n 1)

if [ -n "$MAJOR" ] && [ "$MAJOR" -ge 19 ]; then
    exec "$JAVA" -XX:+AutoCreateSharedArchive -XX:SharedArchiveFile="$CACHE/blockforge.jsa" \
        -jar "$JAR" "$@"
fi

exec "$JAVA" -jar "$JAR" "$@"
