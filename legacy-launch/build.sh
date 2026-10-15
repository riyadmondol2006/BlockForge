#!/usr/bin/env bash
# Rebuilds ../assets/legacy-launch.jar from the sources next to this script. The jar is the small wrapper that starts
# Minecraft versions older than 1.6; it is compiled into BlockForge. Needs a JDK (javac and jar), 8 or newer.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
javac --release 8 -Xlint:-options -encoding UTF-8 -d "$work" $(find "$here/src" -name '*.java')
jar cf "$here/../assets/legacy-launch.jar" -C "$work" .
echo "Built $here/../assets/legacy-launch.jar"
