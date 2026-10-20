#!/usr/bin/env bash
# Pictures BlockForge's pages headless: starts a private Xvfb, runs `blockforge gui` on it with the BLOCKFORGE_SHOT
# switches, writes one PNG per page and stops the Xvfb again. The real desktop (Wayland or X) is never used: its
# variables are removed from the app's environment.
#
#   usage: tools/screenshots/shoot.sh <data-dir> <pages,comma,list> <WxH> <outdir> [zoom]
#   e.g.   tools/screenshots/shoot.sh ~/bf-ui/data home,library,instance-mods 1200x700 ~/bf-ui/before
#
# Environment: BLOCKFORGE_BIN (default $HOME/bf-conv/target/debug/blockforge), BLOCKFORGE_XVFB (default
# $HOME/bf-conv/xvfb/usr/bin/Xvfb, else Xvfb from PATH), BLOCKFORGE_SHOT_TIMEOUT (seconds, default 600).
set -euo pipefail

DATA=${1:?usage: shoot.sh <data-dir> <pages> <WxH> <outdir> [zoom]}
PAGES=${2:?pages, e.g. home,library}
SIZE=${3:?size, e.g. 1200x700}
OUT=${4:?output folder}
ZOOM=${5:-}

BIN=${BLOCKFORGE_BIN:-$HOME/bf-conv/target/debug/blockforge}
XVFB=${BLOCKFORGE_XVFB:-$HOME/bf-conv/xvfb/usr/bin/Xvfb}
[ -x "$XVFB" ] || XVFB=$(command -v Xvfb || true)
[ -x "$BIN" ] || { echo "no binary at $BIN (set BLOCKFORGE_BIN)" >&2; exit 1; }
[ -n "$XVFB" ] || { echo "no Xvfb found (set BLOCKFORGE_XVFB)" >&2; exit 1; }
[ -d "$DATA/instances" ] || { echo "$DATA is not a data folder (run make-data.sh first)" >&2; exit 1; }
case $SIZE in [0-9]*x[0-9]*) ;; *) echo "size must look like 1200x700" >&2; exit 1 ;; esac
W=${SIZE%x*}
H=${SIZE#*x}

DATA=$(cd "$DATA" && pwd)
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
TMP=${TMPDIR:-/tmp}
mkdir -p "$TMP"

# a free display number between :171 and :199
DISP=""
for n in $(seq 171 199); do
  if [ ! -e "/tmp/.X11-unix/X$n" ] && [ ! -e "/tmp/.X$n-lock" ]; then DISP=$n; break; fi
done
[ -n "$DISP" ] || { echo "no free display between :171 and :199" >&2; exit 1; }

XDG=$(mktemp -d "$TMP/bf-shot-xdg.XXXXXX")
chmod 700 "$XDG"
XPID=""
cleanup() {
  [ -n "$XPID" ] && kill "$XPID" 2>/dev/null || true
  [ -n "$XPID" ] && wait "$XPID" 2>/dev/null || true
  rm -rf "$XDG"
}
trap cleanup EXIT

# the window opens at the size asked for; the file is only read at start
if command -v jq >/dev/null && [ -s "$DATA/configs/BlockForge.json" ]; then
  tmp=$(mktemp "$TMP/bf-shot-cfg.XXXXXX")
  jq --argjson w "$W" --argjson h "$H" \
     '.rememberWindowSizePosition = true | .launcherSize = {width: $w, height: $h} | del(.launcherPosition)' \
     "$DATA/configs/BlockForge.json" > "$tmp" && mv "$tmp" "$DATA/configs/BlockForge.json"
fi

"$XVFB" ":$DISP" -screen 0 "$((W + 400))x$((H + 300))x24" -nolisten tcp >"$XDG/xvfb.log" 2>&1 &
XPID=$!
for _ in $(seq 1 50); do [ -e "/tmp/.X11-unix/X$DISP" ] && break; sleep 0.2; done
[ -e "/tmp/.X11-unix/X$DISP" ] || { echo "Xvfb did not start:" >&2; cat "$XDG/xvfb.log" >&2; exit 1; }

ZENV=()
[ -n "$ZOOM" ] && ZENV=(BLOCKFORGE_SHOT_ZOOM="$ZOOM")

run_app() { # <pages> <outdir> <logfile>
  env -u WAYLAND_DISPLAY -u XDG_SESSION_TYPE -u XAUTHORITY -u DBUS_SESSION_BUS_ADDRESS \
    XDG_RUNTIME_DIR="$XDG" DISPLAY=":$DISP" WINIT_UNIX_BACKEND=x11 LIBGL_ALWAYS_SOFTWARE=1 \
    BLOCKFORGE_DIR="$DATA" BLOCKFORGE_SHOT="$2" BLOCKFORGE_SHOT_PAGES="$1" BLOCKFORGE_SHOT_SIZE="$SIZE" \
    "${ZENV[@]}" \
    timeout "${BLOCKFORGE_SHOT_TIMEOUT:-600}" "$BIN" gui >"$3" 2>&1
}

# The first time the Mods tab sees a folder it shows a "Found N new files" toast; look once, unpictured, so it
# doesn't land in the pictures (the scan takes a few seconds, so the visit is repeated; the app remembers it in the data folder).
if [ ! -e "$DATA/.shoot-warm" ]; then
  mkdir -p "$XDG/warm"
  run_app instance-mods,instance-mods,instance-mods "$XDG/warm" "$XDG/warm.log" || true
  touch "$DATA/.shoot-warm"
fi

status=0
run_app "$PAGES" "$OUT" "$OUT/shoot.log" || status=$?

missing=0
IFS=',' read -ra LIST <<< "$PAGES"
for p in "${LIST[@]}"; do
  p=$(echo "$p" | tr -d ' ')
  [ -s "$OUT/$p.png" ] || { echo "missing: $p.png" >&2; missing=1; }
done
echo "app exit status $status; pictures in $OUT (log: $OUT/shoot.log)"
[ "$missing" = 0 ] && [ "$status" = 0 ]
