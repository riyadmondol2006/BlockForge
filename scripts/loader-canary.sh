#!/bin/sh
# Checks that the services BlockForge installs mod loaders from still answer the way the launcher expects.
# Run it on a schedule (for example daily in CI); a non-zero exit code means an install could break.
#
#   scripts/loader-canary.sh            checks the newest Minecraft release
#   scripts/loader-canary.sh 1.20.1     checks a specific version

set -u
FAILED=0
UA="BlockForge-canary"

fetch() { curl -fsSL --max-time 30 -A "$UA" "$1"; }

check() { # name, command that must succeed
    if eval "$2" >/dev/null 2>&1; then
        echo "ok    $1"
    else
        echo "FAIL  $1"
        FAILED=1
    fi
}

MC="${1:-}"
if [ -z "$MC" ]; then
    MC=$(fetch https://piston-meta.mojang.com/mc/game/version_manifest_v2.json \
        | sed -n 's/.*"release"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
fi
echo "Minecraft version: ${MC:-unknown}"
[ -n "$MC" ] || { echo "FAIL  could not read the Minecraft version manifest"; exit 1; }

check "Mojang version manifest lists $MC" \
    "fetch https://piston-meta.mojang.com/mc/game/version_manifest_v2.json | grep -q '\"id\": *\"$MC\"'"

check "Fabric has loader versions for $MC" \
    "fetch https://meta.fabricmc.net/v2/versions/loader/$MC | grep -q '\"version\"'"

check "Fabric profile for $MC has libraries and a main class" \
    "V=\$(fetch https://meta.fabricmc.net/v2/versions/loader/$MC | grep -o 'fabric-loader:[0-9][^\"]*' | head -n 1 | cut -d: -f2); \
     [ -n \"\$V\" ] && fetch https://meta.fabricmc.net/v2/versions/loader/$MC/\$V/profile/json | grep -q mainClass"

check "Quilt has loader versions for $MC" \
    "fetch https://meta.quiltmc.org/v3/versions/loader/$MC | grep -q '\"version\"'"

check "Forge promotions are published" \
    "fetch https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json | grep -q '\"promos\"'"

check "NeoForge maven metadata is published" \
    "fetch https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml | grep -q '<version>'"

check "Fabric maven serves its loader jar" \
    "curl -fsI --max-time 30 -A '$UA' https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.15.7/fabric-loader-0.15.7.jar"

check "Minecraft libraries host answers" \
    "curl -fsI --max-time 30 -A '$UA' https://libraries.minecraft.net/org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar"

if [ "$FAILED" -ne 0 ]; then
    echo "At least one loader service doesn't look right, installs may fail."
    exit 1
fi
echo "All loader services look healthy."
