#!/usr/bin/env bash
# Builds a throwaway BlockForge data folder with the sample data used for the screenshots:
# one offline account ("Alex") and five instances, the first with seven real Fabric mods from Modrinth.
#
#   usage: tools/screenshots/make-data.sh <data-dir>
#
# Needs bash, curl, jq and sha512sum. The mod jars are downloaded once and kept in $BLOCKFORGE_MOD_CACHE
# (default: ${TMPDIR:-/tmp}/blockforge-shot-mods); each is checked against the hash Modrinth publishes.
# Nothing here signs in anywhere: the account is offline and holds no token.
set -euo pipefail

DATA=${1:?usage: make-data.sh <data-dir>}
CACHE=${BLOCKFORGE_MOD_CACHE:-${TMPDIR:-/tmp}/blockforge-shot-mods}
for tool in curl jq sha512sum; do
  command -v "$tool" >/dev/null || { echo "make-data.sh needs $tool" >&2; exit 1; }
done
if [ -e "$DATA/instances" ]; then
  echo "$DATA already holds a data folder; give a new path (or remove it first)" >&2
  exit 1
fi
mkdir -p "$DATA" "$CACHE"
DATA=$(cd "$DATA" && pwd)

for d in instances libraries assets/indexes assets/objects assets/log_configs runtimes/minecraft configs cache logs temp loaders; do
  mkdir -p "$DATA/$d"
done

NOW_MS=$(( $(date +%s) * 1000 ))
HOUR=3600000
DAY=86400000

# --- account: offline, no token -------------------------------------------------------------------------------
# UUID.nameUUIDFromBytes("OfflinePlayer:Alex") as the app computes it (md5, version 3, variant bits).
# build the 32 hex digits first, then insert the dashes
offline_uuid() {
  local h b6 b8 s
  h=$(printf 'OfflinePlayer:%s' "$1" | md5sum | cut -c1-32)
  b6=$(printf '%02x' $(( (0x${h:12:2} & 0x0f) | 0x30 )))
  b8=$(printf '%02x' $(( (0x${h:16:2} & 0x3f) | 0x80 )))
  s="${h:0:12}${b6}${h:14:2}${b8}${h:18:14}"
  printf '%s-%s-%s-%s-%s' "${s:0:8}" "${s:8:4}" "${s:12:4}" "${s:16:4}" "${s:20:12}"
}
ALEX_UUID=$(offline_uuid Alex)
jq -n --arg uuid "$ALEX_UUID" '[{
  accessToken: "0", offline: true, mustLogin: false,
  username: "Alex", minecraftUsername: "Alex", uuid: $uuid
}]' > "$DATA/configs/accounts.json"
chmod 600 "$DATA/configs/accounts.json"

# --- settings: quiet, fixed look; shoot.sh sets the window size in launcherSize ---------------------------------
jq -n --arg uuid "$ALEX_UUID" '{
  lastAccount: $uuid,
  theme: "com.blockforge.launcher.themes.Material3Dark",
  language: "English",
  lightMode: false,
  lowEndMode: "off",
  lowEndHintDismissed: true,
  enableTrayMenu: false,
  checkForLauncherUpdates: false,
  enableAnalytics: false,
  enableConsole: false,
  keepLauncherOpen: true,
  rememberWindowSizePosition: true,
  launcherSize: {width: 1200, height: 700}
}' > "$DATA/configs/BlockForge.json"

# --- instances --------------------------------------------------------------------------------------------------
# make_instance <folder> <name> <mc version> <loader kind or -> <loader version> <last played ms> <plays> <java component> <java major> <main class>
make_instance() {
  local folder=$1 name=$2 mc=$3 kind=$4 lver=$5 played=$6 plays=$7 jcomp=$8 jmajor=$9 main=${10}
  local root="$DATA/instances/$folder" desc loader='null'
  mkdir -p "$root"/{mods,saves,resourcepacks,shaderpacks,config}
  if [ "$kind" = "-" ]; then
    desc="Minecraft $mc"
  else
    desc="Minecraft $mc with $kind"
    loader=$(jq -n --arg k "$kind" --arg v "$lver" '{version: $v, rawVersion: $v, recommended: false, type: $k, downloadables: {}}')
  fi
  # a stable uuid per instance, derived from its folder name
  local uuid
  uuid=$(offline_uuid "instance:$folder")
  jq -n --arg uuid "$uuid" --arg name "$name" --arg desc "$desc" --arg mc "$mc" --argjson loader "$loader" \
        --argjson played "$played" --argjson plays "$plays" --arg jcomp "$jcomp" --argjson jmajor "$jmajor" --arg main "$main" '{
    uuid: $uuid,
    launcher: {
      name: $name, pack: "Minecraft", description: $desc, packId: 0, externalPackId: 0, version: $mc,
      enableCurseForgeIntegration: true, enableEditingMods: true, loaderVersion: $loader,
      requiredMemory: 0, requiredPermGen: 0, vanillaInstance: true,
      lastPlayed: $played, numPlays: $plays,
      isDev: false, isPlayable: true, assetsMapToResources: false, overridePaths: [],
      checkForUpdates: true, ignoredUpdates: [], ignoreAllUpdates: false, quickPlay: {}, mods: []
    },
    id: $mc, type: "release", mainClass: $main,
    libraries: [],
    javaVersion: {component: $jcomp, majorVersion: $jmajor},
    minimumLauncherVersion: 21, complianceLevel: 1
  }' > "$root/instance.json"
}

make_instance FabricSurvival   "Fabric Survival"    1.21.1 Fabric   0.20.0   $(( NOW_MS - 2*HOUR ))  23 java-runtime-delta   21 net.fabricmc.loader.impl.launch.knot.KnotClient
make_instance Minecraft263     "Minecraft 26.3"     26.3   Fabric   0.20.0   $(( NOW_MS - 1*DAY ))   6  java-runtime-epsilon 25 net.fabricmc.loader.impl.launch.knot.KnotClient
make_instance ForgeClassic     "Forge Classic"      1.20.1 Forge    47.4.28  $(( NOW_MS - 3*DAY ))   11 java-runtime-gamma   17 cpw.mods.bootstraplauncher.BootstrapLauncher
make_instance NeoForgeAdventure "NeoForge Adventure" 1.21.1 NeoForge 21.1.262 $(( NOW_MS - 6*DAY ))   4  java-runtime-delta   21 cpw.mods.bootstraplauncher.BootstrapLauncher
make_instance Retro189         "Retro 1.8.9"        1.8.9  -        -        $(( NOW_MS - 20*DAY ))  9  jre-legacy           8  net.minecraft.client.main.Main

# --- play history: Minecraft keeps one dated log per session, which the play strip reads ----------------------------
# sessions <folder> <days ago>:<count> ...
sessions() {
  local folder=$1; shift
  local spec ago n i day
  mkdir -p "$DATA/instances/$folder/logs"
  for spec in "$@"; do
    ago=${spec%%:*}; n=${spec##*:}
    day=$(date -d "-$ago days" +%F)
    for i in $(seq 1 "$n"); do : > "$DATA/instances/$folder/logs/$day-$i.log.gz"; done
  done
}
sessions FabricSurvival 0:1 1:2 2:1 4:1 5:3 8:1 9:1 12:2 15:1 20:1 27:1 33:2 41:1 50:1
sessions Minecraft263 1:1 3:1 7:2 19:1
sessions ForgeClassic 3:1 4:2 10:1 11:1 25:1
sessions NeoForgeAdventure 6:1 13:1
sessions Retro189 20:1 21:2 40:1

# --- mods for Fabric Survival: pinned Modrinth versions (id -> file name and hash come from the API) -----------------
MODS=(
  yllJFBpU   # Entity Culling 1.11.3
  Mys3P7lK   # Fabric API 0.116.17
  ksaGCvSu   # LambDynamicLights 4.8.11
  SMxNOGZ6   # Sodium 0.8.13
  N08Z8wog   # Lithium 0.15.4
  6lgOkclV   # Mod Menu 11.0.5
  sOzRw3CG   # FerriteCore 7.0.3
)
for id in "${MODS[@]}"; do
  meta="$CACHE/$id.json"
  [ -s "$meta" ] || curl -fsS -m 60 "https://api.modrinth.com/v2/version/$id" -o "$meta"
  file=$(jq -r '.files | map(select(.primary))[0] // .files[0] | .filename' "$meta")
  url=$(jq -r '.files | map(select(.primary))[0] // .files[0] | .url' "$meta")
  want=$(jq -r '.files | map(select(.primary))[0] // .files[0] | .hashes.sha512' "$meta")
  jar="$CACHE/$id-$file"
  if [ ! -s "$jar" ]; then
    curl -fsSL -m 300 "$url" -o "$jar.part"
    mv "$jar.part" "$jar"
  fi
  got=$(sha512sum "$jar" | cut -d' ' -f1)
  [ "$got" = "$want" ] || { echo "hash mismatch for $file" >&2; rm -f "$jar"; exit 1; }
  cp "$jar" "$DATA/instances/FabricSurvival/mods/$file"
  echo "mod: $file"
done

echo "sample data ready in $DATA (account Alex, 5 instances, ${#MODS[@]} mods)"
