# Reproducible screenshots

Two scripts make the same pictures every time, without opening a window on your desktop.

## make-data.sh

```sh
tools/screenshots/make-data.sh <data-dir>
```

Builds a throwaway BlockForge data folder (the path must not hold one already):

- one offline account, Alex (no token anywhere), also set as the last used account;
- five instances, most recently played first: Fabric Survival (Minecraft 1.21.1, Fabric 0.20.0), Minecraft 26.3
  (Fabric 0.20.0), Forge Classic (1.20.1, Forge 47.4.28), NeoForge Adventure (1.21.1, NeoForge 21.1.262) and
  Retro 1.8.9 (vanilla), each with a plausible last-played time and play count;
- seven real Fabric 1.21.1 mods from Modrinth in Fabric Survival (Entity Culling, Fabric API, LambDynamicLights,
  Sodium, Lithium, Mod Menu, FerriteCore). Exact versions are pinned by Modrinth version id and every jar is checked
  against the sha512 Modrinth publishes. Jars are cached in `$BLOCKFORGE_MOD_CACHE` (default
  `${TMPDIR:-/tmp}/blockforge-shot-mods`), so a second run needs no network;
- `configs/BlockForge.json` with the dark theme, no tray icon, no update check, Low-end mode off and a remembered
  window size (`shoot.sh` sets the size).

The instances have no game files (they are for looking at, not for playing). Needs bash, curl, jq and sha512sum.

## shoot.sh

```sh
tools/screenshots/shoot.sh <data-dir> <pages,comma,list> <WxH> <outdir> [zoom]
tools/screenshots/shoot.sh ~/bf-ui/data home,library,instance-mods,home-light 1200x700 ~/bf-ui/before
```

Starts a private Xvfb on a free display between :171 and :199, runs `blockforge gui` on it with
`BLOCKFORGE_SHOT`, `BLOCKFORGE_SHOT_PAGES`, `BLOCKFORGE_SHOT_SIZE` (and `BLOCKFORGE_SHOT_ZOOM` when a zoom is given),
writes `<outdir>/<page>.png` for each page, and stops the Xvfb again. The variables that point at the real desktop
(`WAYLAND_DISPLAY`, `XDG_SESSION_TYPE`, `XAUTHORITY`, `DBUS_SESSION_BUS_ADDRESS`) are removed from the app's
environment and it gets its own empty `XDG_RUNTIME_DIR`, so nothing can appear on your screen. The app's output is
saved as `<outdir>/shoot.log`. The script exits non-zero when a picture is missing.

It also writes the window size into the data folder's `configs/BlockForge.json` and, on the first run, does one unpictured visit to the Mods tab (so its "Found new files" toast is not in your pictures), so use a throwaway folder.

Environment:

| Variable | Default |
|---|---|
| `BLOCKFORGE_BIN` | `$HOME/bf-conv/target/debug/blockforge` |
| `BLOCKFORGE_XVFB` | `$HOME/bf-conv/xvfb/usr/bin/Xvfb`, else `Xvfb` from PATH |
| `BLOCKFORGE_SHOT_TIMEOUT` | `600` (seconds) |
| `TMPDIR` | `/tmp` (private runtime folder and settings scratch file go here) |

Page names are the `SHOT_PAGES` list in `src/gui/mod.rs` (see also the developer switches in `TESTING.md`). Add
`-light` to a name for the light theme, for example `home-light`. Notes:

- the `instance*` pages open the most recently played instance (Fabric Survival);
- `discover`, `packs`, `ftb`, `about` and the mod icons use the network (Modrinth, the pack lists, the news), so they
  come out empty or partly empty offline;
- `instance-mods-light` shows the Overview tab, not Mods, because only the exact names pick the tab; use
  `mods-light` for the light Mods tab;
- the Mods tab needs a taller window than 1200x700 to show all seven mods.
