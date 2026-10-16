# Changelog

The section for a version is used as the text of its release on GitHub. Older releases (the Java versions, up to
3.4.42.x) are on the [releases page](https://github.com/riyadmondol2006/BlockForge/releases).

## 4.2.0

Smoother games, on every computer and every mod loader.

### What is new

- **Smoothness helper.** A second small helper (a Java agent, like Fast start) makes small fixes inside the running
  game that make it smoother. It is not a mod, so it works the same with vanilla, Fabric, Quilt, Forge and NeoForge, and
  it leaves alone what an installed performance mod already does (BlockForge tells it which mods are installed). On
  Linux computers with 4 processor threads or fewer it also gives Minecraft's world-generation threads a lower
  priority, so the game's own server thread waits less. It is on by default, with a switch in
  Settings > Game > Performance and on each instance (on its page and in its menu); Settings shows what it did in the
  last game ("Details").
- **Low-end mode.** Lowers the graphics settings of a game on a slow computer: view and simulation distance, particles,
  clouds, shadows, smooth lighting, mipmaps, biome blend, entity distance, menu blur and a 60 FPS cap (on Minecraft
  1.21.11 and newer the graphics preset becomes Custom with the same lowered values). It only ever lowers a setting,
  never raises one, and switching it off puts your own settings back, except the ones you changed yourself in the
  meantime. Automatic (the default) turns it on when the computer looks slow (4 processor threads or fewer, 6 GB of
  memory or less, built-in graphics with 8 GB or less, or a hard disk with 8 GB or less); On and Off force it, for all
  games or for one instance.
- **Which helpers are on** is written at the top of every game's log and printed by `blockforge launch --dry-run`.

### Changes you may notice

- If a game closes early with a helper on, BlockForge now offers to turn off the helpers for that game. After two
  early closes in a row, the smoothness helper switches itself off for that instance and says so in the log.
- On a slow computer with Low-end mode switched off, the home page suggests it once.
- Computers with a single processor now also get the G1 garbage collector (Java's own choice there froze the game for
  up to a second while a world loaded), and every game asks Java to run Minecraft's own full collections alongside the
  game: Minecraft 1.8.9 and 1.12.2 ask for one at every world change, which stopped the game for 0.3-0.4 s and now
  for well under 0.1 s.
- On Java 25 the class cache is made to match games started with the helpers.

## 4.1.0

Faster starts, several games at once, and a few things that were missing.

### What is new

- **Several games at once.** The "Play as" button next to Play starts the same instance again with another Microsoft
  or offline account, each game with its own log. BlockForge no longer asks who is playing every time it opens.
- **Fast start.** A tiny helper (a Java agent, so it works with every Minecraft version and mod loader) loads the
  game's classes ahead of time on spare processor cores. It is on by default with a switch on each instance and in
  Settings, and about 7% quicker to the title screen in our tests on a fast computer. It starts working from the
  second start of an instance, does nothing on computers with fewer than four cores or a slow disk, and if the game
  closes early with it on, BlockForge offers to turn it off.
- **Quicker starts on slow computers.** Computers with few processor cores start the game with Java's quick compiler
  (Settings > Game > Java compiler: Automatic, Light or Full performance), hard disks are read in one sweep
  and not asked for thousands of file checks, and a Microsoft sign-in is renewed in the background so Play never
  waits for it.
- **Logos.** Instances without a picture get a cube in the colour of their mod loader; GIF, JPEG and WebP logos show,
  failed logo downloads are retried, and logos are kept on disk for the next start.
- **Choose the graphics card.** On computers with two graphics cards, Settings > Game > Graphics card picks which one
  Minecraft uses: power saving, high performance, or (on Linux) a card by name.

### Changes you may notice

- BlockForge no longer asks who is playing every time it opens. Pick an account once; "Play as" next to the Play
  button plays as another one.

## 4.0.0

BlockForge is now a native program written in Rust. It looks and works like before, and it keeps working with the
instances, accounts and settings you already have.

### What is new

- **A native program.** The launcher itself no longer runs on Java, so it opens in a fraction of a second, uses much
  less memory and is a single small download. Minecraft still needs Java, and BlockForge downloads the right one
  from Mojang for each game, as before.
- **The same features.** Offline and Microsoft accounts, ATLauncher, CurseForge, Modrinth, FTB and Technic modpacks, any
  Minecraft version with Fabric, Quilt, Forge or NeoForge, mod updates, backups, import from Prism, MultiMC and other
  launchers, export, skins, news, servers, console, tray icon, light and dark themes.
- **Installers for every system.** Windows installer and portable zip, macOS disk image (Apple Silicon), Debian and
  Ubuntu `.deb`, Fedora `.rpm`, Linux `AppImage`. Menu entries, icons and shortcuts are set up for you.
- **One-click updates.** BlockForge checks GitHub releases and installs the right file for how it was installed.


### Upgrading from 3.x

- **Installed copies** (Windows installer, macOS disk image, deb and rpm): use the update button in the launcher, or
  install the new package over the old one. Your instances, accounts and settings are kept; they are in the same folder.
- **Plain `.jar` copies**: the update button downloads a small bridge jar. Starting it once downloads the native
  program next to your files and starts it; your instances, accounts and settings stay where they are.

### Changes you may notice

- Settings that only made sense for the Java launcher (such as the Java used to run the launcher) are gone.
- The launcher's usage statistics and error reports no longer exist at all, rather than being off by default.
