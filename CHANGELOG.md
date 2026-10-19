# Changelog

The section for a version is used as the text of its release on GitHub. Older releases (the Java versions, up to
3.4.42.x) are on the [releases page](https://github.com/riyadmondol2006/BlockForge/releases).

## 4.4.0 (unreleased)

World conversion between Java Edition and Bedrock Edition, powered by a Rust port of Chunker by Hive Games.

TODO(lead): one sentence on how far the converter got when this version is released, and delete this line.

### What is new

- **Convert world.** A new Convert world page (a card on the Tools page, and a Convert a world button on an
  instance's overview that lists its worlds) turns a world into the other edition: choose the world (a folder, or a
  `.mcworld` or `.zip` file, or drop it on the page), the edition and version to write, the dimensions to keep and
  whether to keep data the other edition cannot use, and where to save it. It shows the progress, can be stopped, and
  ends with a summary (columns converted, warnings, blocks without an exact match) or the reason it failed with a
  button to copy the details. The original world is never written to, and the page refuses to save into it or into a
  folder that already has files in it.
- TODO(lead): list what the engine converts in this version (block and biome coverage, which versions, which
  directions, items, signs, chests, entities), and what it does not yet.
- **Credits.** The About page and the README credit ATLauncher and Chunker (MIT License, the licence text is in
  NOTICE.md); BlockForge is not affiliated with or endorsed by Hive Games.

### Behind the scenes

- The converter is the cargo feature `converter`, on by default; `cargo build --no-default-features` builds the
  launcher without it and without the Convert entries.
- TODO(lead): release size growth and anything else worth knowing.

## 4.3.0

A crafting calculator, and versions that raise themselves.

### What is new

- **Crafting calculator.** A new Crafting page works out what it takes to make an item: pick the item and how many,
  and it lists what to gather (with a tick-off list), what to craft, smelt or cut and in which order, the fuel the
  furnaces burn (an estimate), and how many stacks, chests or shulker boxes it all takes. Tell it what you already have
  and it only plans what is missing; choose another recipe or another ingredient (planks of another wood, say) and the
  plan follows. Plans can be saved, copied as a shopping list, exported as text or JSON and shared as a plan file. The
  recipes are read from your own Minecraft files, so every version from 1.13 on works (including the newest), and
  nothing of Mojang's is shipped with BlockForge. The page costs nothing while you are not on it. If no version is
  installed it can download just the files it needs.

### Behind the scenes

- **The version raises itself.** When a push to main changes the program and its version was released already, the
  build picks the next version, commits it to main and releases it, so nobody has to edit a version number. Pushes
  that only change documentation or the workflows make no release, `[skip bump]` in a commit message skips it, and
  `[minor]` or `[major]` raises that part instead of the patch number. The rules are in `.github/scripts/version.sh`.

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
- **What the smoothness helper changes.** Each change checks the game's code first, steps aside when another mod has
  changed that code or does the same thing (Sodium, Lithium, FerriteCore, EntityCulling and others), checks its own
  result before it is used, and leaves the game untouched if anything is unexpected:
  - 10 frames a second while the window is minimised, as Minecraft 1.21.2 and newer do by themselves (1.8 to 1.21.1).
  - No forced full memory clean-up at every world open and close (Minecraft 1.7 to 1.12: about 2 s of freezes less).
  - No debug thread guard in every chunk section (25 to 37 MB less memory on 26.3), kept when mods are installed.
  - A shorter loading-screen fade (the game is ready about 1.5 s sooner).
  - No empty full-screen outline pass when nothing glows (1.20 to 1.21.1).
  - Block states share their neighbour tables (17 to 18 MB less memory on 1.20.1 and 1.21.1).
  - Biome colours (grass, leaves, water) remember their last lookups: about a quarter less chunk-building work.
  - Chests, signs, banners and similar blocks outside your view are not drawn (26.2 and newer).
  - Empty sign lines, empty effect lists and unused collision shapes are no longer processed.
  - While Low-end mode is on: the frame limiter on 26.x waits less, a sound starts without waiting for the sound
    thread, and music at volume 0 is not decoded in the background.
- **Fast start was rebuilt.** It loads the game's classes in the order the game needs them, on spare processor threads,
  and checks them ahead of time. Its saved list now carries a fingerprint of Java, the game files and the mods, so a
  loader or version change can never leave it loading the wrong classes; damaged or half-written lists are ignored. It
  never runs on a single processor and never keeps the game from closing. About 5 to 10 % quicker starts on vanilla,
  up to 19 % together with the class cache; Fabric, Forge and NeoForge are unchanged.
- **The launcher window got a long review.** More than 110 problems were fixed across the pages, among them: a crash
  on file paths with odd characters, the window redrawing all the time while a game ran, deleting or clearing big
  folders freezing the window, settings switches shown twice with stale values, text fields that could not be typed
  in, a cleared Java path that stayed, a backup restore that could lose an instance if it failed halfway, two quick
  changes to one instance's mod list overwriting each other, Modrinth installs that skipped the mod records and the
  dependency check, late search answers replacing newer ones, pages wider than the window at the smallest size or when
  zoomed, missing keyboard focus and screen-reader names, and plain-word labels and errors.
- **The game command shown by `--dry-run` can be copied and run as it is** (arguments with spaces are quoted).
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
