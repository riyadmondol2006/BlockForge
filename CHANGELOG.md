# Changelog

The section for a version is used as the text of its release on GitHub. Older releases (the Java versions, up to
3.4.42.x) are on the [releases page](https://github.com/riyadmondol2006/BlockForge/releases).

## 4.5.0

A new look for the whole launcher: calmer, flatter and easier to scan, with one rule at its heart: green means Play.

### What is new

- **A redesigned window.** Every page was reworked with one set of colours, shapes and type: a neutral dark look and
  a clean light look, flat surfaces with a single fine line instead of boxes inside boxes, 32 px controls, and Inter as
  the typeface (with real medium and semibold weights and aligned digits). Green is now only for Play, a running game
  and the progress of getting one started; every other main action is white on dark (near-black on light), switches and
  selections are neutral, and each mod loader keeps its own colour wherever it is named.
- **A sidebar that groups things.** Play (Home, Library), Mods and modpacks (Discover, ATLauncher and Technic, FTB)
  and Utilities (Crafting, World converter, Servers, Tools), with Settings, About and the account at the bottom. The
  **World converter** now has its own entry. In a short window the group names fold away so everything still fits. A
  **Now playing** row shows a running game (and how long) from every page; a click opens it.
- **Instance cards and Home.** Home opens with a Continue block for the instance you played last, with a **play
  strip**: one small square for each of the last 28 days, filled in the instance's loader colour on the days it was
  played. The instance's Overview has the same strip for 56 days (from the sessions BlockForge records and the dated
  logs Minecraft keeps). Cards in the Library and on Home fill the row evenly, show the version and loader, how long
  ago it was played, and a quiet Play button that turns green when the card is pointed at.
- **The instance page.** A quiet back link, one meta line, Play as and Play on the right, and a four-figure facts
  strip. Fast start and the smoothness helper moved from the header to a Speed and smoothness section of the
  Overview. The Mods tab has calm 56 px rows with Details and Remove showing on hover, and a designed empty, loading
  and error state.
- **Discover and Create.** Modrinth and CurseForge are a segmented switch; search runs as you type (and on Enter);
  result rows show the downloads in a column and the loaders in their colours. The New instance page is a single form
  with the Minecraft versions as a grid, the loader as a segmented choice and Create pinned under it.
- **Settings, Tools, Accounts and About.** Settings are grouped cards with fine row lines, a 760 px reading width and
  a quiet Saved stamp. The Java compiler choice is a segmented control. Tools are grouped into Diagnose and Clean up.
  Accounts are rows with a Playing as mark. About has one status line and a Check for updates button, then the
  details of this copy and the credits (ATLauncher, Chunker, Inter).
- **Crafting and the world converter.** Both use the same facts strip, rows and tick boxes; the converter is a single
  column with a drop zone, the chosen world as a row, a progress bar and a result summary.
- **Small windows and zoom.** The smallest window (860 by 560) shows every sidebar entry without scrolling, and the
  Home block moves its buttons under the title when there is not room beside it.

### Behind the scenes

- One place defines the look: `src/gui/theme.rs` (colours, radii 8/6/4 and 12 for dialogs, the type scale 12/14/16/20/22)
  and `src/gui/widgets.rs` (buttons, tabs, segmented control, facts strip, notices, skeleton rows, progress bar).
- Inter Regular, Medium and SemiBold are built into the program as small Latin subsets with aligned digits (about 135 KB,
  SIL Open Font License, see NOTICE.md).
- Each instance now remembers the days it was played in its `instance.json` (`playDays`, the last 120 days).
- `tools/screenshots/` makes a throwaway sample data folder and takes the screenshots of every page on a private virtual
  display, so the pictures in `docs/` can be remade without touching a real desktop.

## 4.4.0

World conversion, powered by a Rust port of Chunker by Hive Games. This version converts Bedrock Edition worlds to
Java Edition (BlockForge does not convert in the other direction).

### What is new

- **Convert a Bedrock world.** A new page (a card on the Tools page) turns a Bedrock Edition world into a Java Edition
  world: choose the world (a folder, or a `.mcworld` or `.zip` file, or drop it on the page), the Java version to
  write, the dimensions to keep, and where to save it. A Java world is refused with a message, because BlockForge
  converts in this direction only. It shows the progress, can be stopped, and ends with a summary (columns converted,
  warnings, blocks without an exact match) or the reason it failed with a button to copy the details. The original
  world is never written to, and the page refuses to save into it or into a folder that already has files in it.
- **Add a Bedrock world.** An instance's overview has an Add a Bedrock world button. It opens the same page, set to
  save the world into the instance's saves folder (under a free name) and to write the instance's Minecraft version
  when it is 1.13 or newer. When it is done, Open the saves folder shows it, and the world is in Singleplayer.
- **What converts.** Bedrock worlds (a folder or a `.mcworld`, Bedrock 1.13 up to 1.26) to Java Edition 1.13 up to the
  newest: blocks and their states, biomes, block entities (signs and hanging signs, chests and other containers with
  their items and enchantments, command blocks, beds, banners, skulls, spawners, ...), the level settings and the
  player's spawn. It was tried on a real Bedrock map and the result opens in Minecraft 26.3. Not converted yet:
  entities (mobs, armor stands, paintings), pending block updates, map items' pictures, and old
  Bedrock worlds from before 1.13. Blocks without an equivalent become air and are listed in the summary.
- **Credits.** The About page and the README credit ATLauncher and Chunker (MIT License, the license text is in
  NOTICE.md); BlockForge is not affiliated with or endorsed by Hive Games.

### Behind the scenes

- The converter is the cargo feature `converter`, on by default; `cargo build --no-default-features` builds the
  launcher without it and without the Convert entries.
- The block, item and biome tables are not written by hand: they were produced by running Chunker's own resolvers
  (`tools/convert/`), and they are 131 KB in the program. The release binary grew by about 2.5 MB.
- Heavy work runs on worker threads with bounded memory, and nothing of the converter loads until a world is converted.

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
