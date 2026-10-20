#!/bin/sh
# Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
# Download the jars the Dump harness compiles against into <dir> (all Apache-2.0 / MIT licensed; build tools only,
# never committed or shipped). Usage: fetch_jars.sh <dir>
set -e
DIR="$1"; mkdir -p "$DIR"; cd "$DIR"
B=https://repo1.maven.org/maven2
for u in \
  com/google/guava/guava/33.4.0-jre/guava-33.4.0-jre.jar \
  com/google/guava/failureaccess/1.0.2/failureaccess-1.0.2.jar \
  com/google/code/gson/gson/2.11.0/gson-2.11.0.jar \
  it/unimi/dsi/fastutil/8.5.15/fastutil-8.5.15.jar \
  com/github/ben-manes/caffeine/caffeine/3.1.8/caffeine-3.1.8.jar \
  org/jetbrains/annotations/26.0.1/annotations-26.0.1.jar \
  org/jspecify/jspecify/1.0.0/jspecify-1.0.0.jar \
  org/iq80/leveldb/leveldb-api/0.12/leveldb-api-0.12.jar \
  org/iq80/leveldb/leveldb/0.12/leveldb-0.12.jar \
  info/picocli/picocli/4.7.6/picocli-4.7.6.jar \
  org/lz4/lz4-java/1.8.0/lz4-java-1.8.0.jar; do
  [ -f "$(basename $u)" ] || curl -sSfLO "$B/$u"
done
