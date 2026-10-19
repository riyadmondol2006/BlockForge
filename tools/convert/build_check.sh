#!/bin/sh
# Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
# Compile the differential oracle (check/Check.java) next to the already compiled Dump classes.
# Usage: build_check.sh <jars-dir> <classes-dir>      (classes-dir as produced by build_dump.sh)
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
javac -nowarn -proc:none -d "$2" -cp "$2:$(ls "$1"/*.jar | tr '\n' ':')" "$HERE/check/Check.java"
