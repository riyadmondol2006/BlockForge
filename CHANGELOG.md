# Changelog

The section for a version is used as the text of its release on GitHub. Older releases (the Java versions, up to
3.4.42.x) are on the [releases page](https://github.com/riyadmondol2006/BlockForge/releases).

## 5.1.0

### What is new

- **Play Horizon, Minecraft: Java Edition rewritten in Rust.** New instance now starts with a choice of game:
  Minecraft: Java Edition, or Horizon. Horizon plays the Minecraft version its builds are made for (26.3 now), so there
  is no version or mod loader to pick: the card says what plays (singleplayer, servers, worlds in Minecraft's own
  format, resource packs, Horizon's own mods) and what doesn't (Java mods and mod loaders). BlockForge downloads
  Horizon's signed release, checks it, and prepares it from its own copy of Minecraft with the Java that preparation
  needs, which it gets by itself: about 1.5 GB the first time. The game then plays without Java. Your normal Minecraft
  and its instances are left as they are. It needs a Microsoft account that owns Minecraft: Java Edition.
  Experimental.
- **A Horizon instance is an instance like the others:** Play, Stop, the log, play time, worlds, resource packs,
  screenshots, backups, rename, cover and delete work as usual, and so do the graphics-card choice and the memory
  setting: Horizon draws with the card chosen, and BlockForge sizes its preparation and its first-start render
  distance to the memory budget (automatic by default), with a note when little memory is free. What is Java's (mods,
  the loader, Java and its arguments, commands, fast start and the other helpers, shader packs, modpack export) is
  left out of its page, its menus and Explore's "install to" lists. Its card and banner say "Horizon" where the loader would be, and its page has
  an About Horizon card with the build it plays.
- **The command line knows Horizon too:** `blockforge launch` starts a Horizon instance as Horizon, and `add`,
  `repair` and `export` refuse what is Java's for it, with the reason.
- **Games show their logo in the taskbar on Linux.** GNOME on Wayland only shows a window's icon when a menu entry
  claims it, and none did for Minecraft (newer versions) or Horizon, so their windows had a blank icon. BlockForge now
  adds a hidden entry for each, with Minecraft's icon from your own copy of the game and Horizon's logo, unless another
  program already has one (the Horizon launcher's is left alone). `"desktopIntegration": false` in the settings
  file switches it off.
- **Credit where it is due:** Horizon is made by the Horizon project and its contributors and shared under the MIT
  licence; it is an unofficial fan project, not affiliated with Mojang or Microsoft. New instance, the instance page
  and About credit it and link to [playhorizon.dev](https://playhorizon.dev) and
  [its source](https://github.com/horizon-rust/horizon).

## 5.0.2

A round of polish after an independent review of every page, which went from 6.5 to 8.7 out of 10.

### Better

- **Chinese, Japanese and Korean text** in mod and modpack names and descriptions now shows instead of empty boxes,
  using a font your system already has.
- **Instance page:** Play stays in the same place on every tab, the banner is shorter on the work tabs so lists start
  higher, Open folder and Manage backups sit right under the figures, and the play history says what its bars mean.
- **Crafting calculator:** the answer is in view without scrolling, even in a small window; when your own items change
  the plan, "Your items · N applied · Edit" says so and opens them, and each material shows what your items cover.
- **Explore:** the content kinds come first, then only the sources that have that kind; quieter result rows with the
  version and loaders first; named loading states; pack pages with headings and lists.
- **New instance:** name, memory and performance mods come first and the cover is one row with Change cover; the
  loader version sits right under the chosen loader.
- **World converter:** a conversion with warnings or unmatched blocks says so in amber instead of a plain success;
  steps are ticked only once you have been through them; paths show their last folder.
- **Dialogs** all put Cancel first and the main action last, and open with the safe answer focused. Export shows the
  files it will include and how many; Who's playing puts your account first.
- **Tools:** the network checker runs without the reporting setting, and sharing its report is a separate step that
  asks first.
- **Servers** that need attention are listed first, with Open folder and Reinstall beside the warning.
- **Settings:** search shows the settings named first; Accounts and About open on their own.
- Notes appear in the bottom right with a close button and wait while a dialog is open; switches are named for screen
  readers; lighter shadows in the light look.

## 5.0.1

### What is new

- **A title bar of BlockForge's own on Linux.** GNOME on Wayland leaves the frame of a window to the app, and
  BlockForge used to get a plain grey strip there. Now the window draws its top itself, in the launcher's colours:
  drag the top of the window to move it, double-click it to maximize, and minimize, maximize and close sit in the top
  right corner. The edges and corners resize the window. Prefer your desktop's frame? Settings > Appearance > Use the
  system's title bar switches it back at once. Windows and macOS keep their own title bars.

## 5.0.0

**The Workbench.** BlockForge is rebuilt around playing: the game you play comes first, every instance has a face of
its own, and the technical tools stay a click away without crowding the rest. Every feature of 4.9 is still here.

### A new home for everything

- **Five places** in the sidebar instead of nine: **Home** (play and what you played lately), **Library** (your
  instances), **Explore** (mods, modpacks and every source), **Workshop** (the crafting calculator and the world
  converter) and **Utilities** (tools, diagnostics and servers). Settings, About and your account sit at the bottom.
  Each place opens on the page you left it on.
- **A new look:** a deep blue-black ground with a faint pattern of blocks (Settings > Appearance > Show background
  artwork turns it off), a mint green for Play and the main actions, a warm amber for favourites and waiting updates,
  and versions, paths and logs in a monospace face. The light look has all of it too.
- **Covers.** Every instance gets its own pixel-art landscape, drawn from its version and mod loader: meadows,
  forests, cherry groves, deserts, snow, oceans, the Nether or the End, by day or night, with the loader's colour in
  its banner and ores. Pick a theme for an instance in its settings, or let it choose. A picture of your own still
  shows on top.

### Home and Library

- **Home** opens on a large Continue playing banner on the cover of the instance you played last: its name, version,
  loader and mods, a big Play (Stop while it runs), the other ways to play in its menu, when you last played and a
  strip of the days you played. Under it a row of your other instances as cover tiles, and the news as small cards.
- **Library** shows your instances as cover cards or as a list, with a search, a filter for favourites and for each
  mod loader, and the sort you choose. Star an instance to make it a favourite.

### One instance

- An instance opens on its **cover banner** with Play (and Play as) right there, and five tabs: **Overview**, **Mods**,
  **Resource packs**, **Logs** and **Settings**.
- **Overview:** when you last played, **how long you have played** (counted from now on), the mods and the Java it
  uses, your play over the last eight weeks, its description, its folder, saves and backups.
- **Mods** is a compact table: icon, name, version, loader and source, an on/off switch and an amber "Update available"
  where there is one, with filters for enabled, disabled and updates.
- **Resource packs** (new): the instance's resource packs and shaders, to open, remove or find more.
- **Logs** in a monospace face, warnings and errors marked so they stand out.

### Explore

- **One place for every source:** Modrinth, CurseForge or both together, ATLauncher, Technic and FTB, switched with
  one row of pills at the top. Mods, modpacks, resource packs, shaders and data packs, with the filters of each source.
- Each result shows its icon large, the Minecraft version and loaders it works with, its categories, its downloads and
  an **Install** button that says when it is installed or has an update.

### New instance

- **Three steps:** choose the Minecraft version, choose the mod loader, then the rest (name, cover, picture, memory,
  performance mods). A summary beside the steps shows exactly what will be installed, with the cover the instance will
  get. Importing a modpack or an instance from another launcher is on the page too.

### Workshop and Utilities

- **Crafting calculator:** the items on the left; the item with quick amounts (1, 4, 16, 64 or stacks) and its recipe
  in the crafting grid; then what to gather, the steps, a recipe tree and the storage.
- **World converter, step by step:** Bedrock world, Convert to, What comes along, Save, Convert, each step marked
  ready or saying what it still needs, with every option of 4.9.
- **Tools & diagnostics** and **Servers** in clear groups, with one way of saying what worked, what wants a look and
  what failed.

### Settings

- A list of sections down the left and a **search** across all settings.
- **Theme cards** for Dark, Light and the new **System**, which follows your computer's look as it changes.

### Fixed

- **World converter:** a large `.mcworld` was turned away as "This does not look like a Minecraft world" when the
  system's temporary folder had no room to unpack it (on many Linux systems that folder is a small RAM disk). Worlds
  are now unpacked into BlockForge's own data folder, an unpacked world is read where it is instead of being copied a
  second time, and when a world really cannot be unpacked the page says why (for example, that there is not enough
  free space) and where.
- **World converter:** a world name with colour codes (`§a`, `§l`) is shown plain on the page and in the suggested
  folder name; the converted world keeps its coloured name.
- Worlds left half-unpacked by a launcher that crashed or was closed during a conversion are cleaned up.

## 4.9.0

A new look for the whole launcher, with smooth motion, a layout that fits every window and more options, and a world
converter that brings a Bedrock world over whole: its mobs and pets, item frames, the players and their maps.

### What is new

- **New look everywhere.** Deep layered backgrounds with a soft glow behind the pages, a glass sidebar, cards with
  soft shadows and rounder corners, and an emerald-to-cyan gradient for Play and the main button of each screen.
  Both the dark and the light look are new.
- **Smooth motion.** Pages fade in as they open. Hovers fade in instead of switching, the underline of the open tab
  and the pill of a switch slide to the one you chose, cards lift a little under the pointer, and progress bars
  ease forward. Everything stays still when "Reduce motion" is on, and nothing is drawn again while nothing moves.
- **Home and Library.** A large banner for the instance you play, with a glowing Play button. Instance tiles lift
  under the pointer and light up their Play button.
- **Instance page.** The instance's picture, name and Play sit on a banner above its tabs. The Mods tab has a
  sliding Mods / Resource packs / Shaders switch with counts, filter chips, a clearer state for mods that are
  switched off, and update badges. Its settings sit in cards with a "Saved" or "Unsaved changes" note by Save.
- **Discover, ATLauncher, Technic and FTB.** Results fade into a card under the pointer, the site a result comes
  from is a small coloured pill, and the search and filters sit together in one bar.
- **Settings, Accounts, About and New instance.** Settings come in titled cards. The account that plays is
  highlighted. New instance is a set of numbered steps, with the mod loaders as cards to pick from.
- **Tools, Servers, World converter and Crafting.** Tools are tiles, each server is a card with its status, and the
  converter's set-up is a stepper that ticks off each step.

### Better layout and more options

- **Fits every window.** Below about 1000 points wide, the sidebar folds smoothly into a rail of icons (names in
  tooltips; the account, downloads and running games stay in it). On wide screens, pages of text and forms stay at a
  readable width in the middle, and lists and grids use the room.
- **Settings.** A search field (Ctrl+F) that finds a setting by its name, its explanation or a keyword, and a list of
  the groups that jumps to each one. The groups are reordered (Game, Performance, Appearance, Launcher, Backups,
  Network and downloads, Privacy), rarely needed settings sit under "Advanced", and a changed setting has a "Reset"
  back to its default. Units are shown (GB, MB, seconds). An instance's settings use the same layout.
- **Library.** A grid or list view (the list's columns sort when clicked), more ways to sort (recently added, newest
  Minecraft, loader), and chips to show one loader or the running games, with counts. The search also finds the
  Minecraft version and the loader. The view and the order are remembered.
- **Home.** On a wide window, News sits beside your instances. "Continue playing" lists the others by when they were
  last played. News can be hidden.
- **Instance page.** The overview uses two columns on a wide window. The Logs tab has All / Errors / Warnings chips
  and a "Wrap lines" switch.
- **Discover and the modpack pages.** Filters for category, Minecraft version, loader and client or server, shown
  as chips you can remove, with "Clear filters". Results can be shown as cards on a wide window. The Mods tab sorts
  by name, recently added or updates first, and shows only the mods from Modrinth, CurseForge or added by hand.
- **New instance.** A version search with Releases, Snapshots and Old versions to switch on, the latest release
  clearly marked, and the loader's version shown as soon as a loader is picked. "More options" sets the picture,
  the memory and a description. The Create button stays in view.
- **Accounts, Export and Who's playing** are laid out more clearly; Export's formats are cards and its folder list
  has "Select all" and "Select none".
- **Put a converted world in an instance.** The World converter's last step has a "Put it in" choice: a folder you
  choose, or any of your instances. With an instance picked, the converted Bedrock world is added to its worlds
  under a name that is free, for the instance's own Minecraft version, and shows up in Singleplayer ready to play.
  The choice is remembered, so the next worlds go to the same instance by themselves; "A folder you choose" turns
  that off. When it is done, "Go to" opens the instance.

### World converter: the whole world comes along

- **Mobs, pets and the rest come along.** A converted Bedrock world now has its entities: animals and monsters,
  villagers with their job, level and trades, wandering traders, armor stands with what they wear, paintings on their
  wall, minecarts with their loot or contents, boats, end crystals and the rest. Before, a converted world had none.
- **Pets stay yours.** Tamed wolves, cats, parrots, horses, donkeys and llamas keep being tamed, sitting where they
  sat, with their names, collars, coats, saddles, armor and chests. When the owner's Java account is known, the
  host's pets belong to it; otherwise they stay tamed under an owner made from the Bedrock host.
- **Item frames.** Item frames and glow item frames become Java item frames again, with the item in them turned the
  same way.
- **What mobs carry.** Weapons, armor and held items are kept, with what a mob picked up still sure to drop. Shulker
  boxes keep their contents, and enchanted items their enchantments, wherever they are: held by a mob, in a frame, on
  a villager's trade or in a minecart. This also fixes shulker boxes in chests and other containers, which lost their
  contents.
- **Mobs stay where they were.** Each entity goes to the chunk it stands in, leashed mobs stay tied to their fence,
  bees and turtles remember their home, and a mob that appears twice in the world's data comes over once.
- **For every Java version.** Entities are written the way each target version keeps them, from 1.13 to 26.3
  (separate entity files from 1.17).
- **Options, all on by default.** Convert entities at all; which mobs (all, no hostile ones, only pets and named
  mobs, or none); leave out what lies around only for a moment (dropped items, experience orbs, falling sand and
  projectiles); convert item frames.
- **Your character comes along whole.** A converted Bedrock world keeps the host's inventory and armor (they were
  dropped before), the ender chest, the experience level, health, food, active effects and the bed or respawn anchor
  you respawn at. Each Java version gets the player in its own format.
- **Shulker boxes keep what is inside them.** A shulker box (or a banner, a head ...) inside a chest, in the
  inventory or in the ender chest is no longer emptied.
- **Maps still show their picture.** Every map in a chest, a shulker box, the inventory or the ender chest points at
  its converted map again, in every Java version.
- **The other players are kept too.** Each player who joined the Bedrock world gets their own player file in the
  Java world with their inventory, ender chest and level, instead of being lost.
- **New options, all on by default:** which Java account owns the world (the account you play with, so the host's
  character and pets become yours), whether to keep the ender chest and the stats, whose inventory the owner gets
  (the host's, another player's, or the host's with the other player's items filling the free slots), whether to
  write the other players, and the difficulty, game mode, cheats and hardcore to give the converted world (kept as
  the world had them unless you pick one). Java has no lock on advancements for a world that was in Creative or had
  cheats on, so nothing else needs changing for those.
- **On the converter page.** A new "What comes along" step shows all of this: switches for mobs, item frames,
  what lies around, the owner, the ender chest, experience and the other players, a choice of whose inventory you
  get (the other players are told apart by their level, items and place, as Bedrock keeps no names), and "World
  settings" for the difficulty, game mode, cheats and hardcore. The command-line converter has the same options.
- **Not yet.** Riders are written next to their mount instead of on it, and armor stands in a pose other than the
  default stand straight.

### Fixed

- In a narrow window, the buttons under a mod's details and in the selection bar of the Mods tab now wrap instead of
  running past the edge of the page.
- A new Fabric, Quilt or NeoForge instance gets the newest stable loader, not a test build that happens to be newer.

## 4.8.4

Search every site at once, data packs in Discover, and mods that are only on GitHub.

### What is new

- **All sites together.** Discover's switch is now **Modrinth | CurseForge | All**. **All** searches Modrinth and
  CurseForge at the same time and shows one list. A project that is on both sites is one row, with the downloads of
  both. Its **Install** button uses Modrinth, and its ⋯ menu has **Install from CurseForge instead**. The search
  words, the instance (its Minecraft version and loader) and the order apply to both sites. When one site can't be
  reached, the other's results are still shown, with a note and **Try again**.
- **Data packs.** Discover has a **Data packs** tab. **Install** asks which world to add the pack to (or all of them),
  because a data pack lives in a world. A world that has the pack already is ticked.
- **Smithed.** With **All**, the Data packs tab searches [Smithed](https://smithed.net) too, the main site for data
  packs. A pack from Smithed comes with the packs it needs inside it.
- **Add from GitHub.** For a mod that is on neither Modrinth nor CurseForge, paste the link to its GitHub page. The
  release made for your instance's Minecraft version and the file for its loader are chosen for you. The file is
  then added the way a file dropped on the Mods tab is.

## 4.8.3

A lighter launcher while something is going on: far less processor time while a game loads or a job runs.

### Fixed

- While a game runs, its log lines are shown a few times a second instead of each at once. A modpack that logs
  thousands of lines while it loads kept the launcher drawing its window at the screen's full rate, taking processor
  time from the game when it needed it most. No line is left out.
- Spinners and progress bars that move by themselves (while a pack is unpacked, a search runs or a list loads) draw
  the window 30 times a second instead of at every frame the screen can show.

## 4.8.2

Faster downloads that use more of your connection, and a window that tells you when a new BlockForge is out.

### What is new

- **Update window.** When a new version comes out, a window pops up with what is new in it and **Update now**, which
  downloads it, checks it and installs it the way your copy was installed, then restarts. **Later** and **Skip this
  version** close it. BlockForge now looks for a new version every hour instead of every six (it can still be turned
  off in Settings).

### Faster downloads

- Big files (16 MB and more, such as a modpack's zip) come in 8 MB pieces, several at once, when the server allows
  it. Many servers give one connection only a few MB/s; RLCraft's 339 MB zip went from about 33 to about 25 seconds.
- Each server gets four connections instead of one, so thousands of small files (the game's assets) no longer wait
  on a single one: about 16 MB/s became about 24 MB/s on Mojang's asset server, and steadier.
- 48 files at once instead of 32 when the setting is left at its default (16 on small computers, as before).

### Fixed

- A download that stops part-way is tried again after 15 seconds without data, instead of waiting out the whole
  connection timeout (a minute by default) while the speed shown sank towards nothing.
- The download bar no longer shows 100% while a file is still coming: a file whose size the pack doesn't list counts
  for the size the server says, and while nobody knows it the bar moves and says how much came so far.
- The downloads in the sidebar no longer cover the menu above them, and each reads from the top down (its name, the
  bar, the sizes, Cancel).

## 4.8.1

### Fixed

- A modpack zip that lists an out-of-date checksum for a file inside it no longer stops the install with nothing but
  "Invalid checksum" (RLCraft from Technic did this for 11 of its mods, which are whole). Files are unpacked as the
  Java launchers do, without that check, in Technic and ATLauncher packs, Modrinth and CurseForge packs and imported
  instances; a file that comes out shorter or longer than the zip says is still refused. BlockForge's own updates,
  backups and the game's files are checked as before.
- An error while unpacking a modpack now names the pack and the file it was on.

## 4.8.0

Quicker downloads you can follow, modpacks that don't fail over a stale checksum, and the speed switches next to Play.

### What is new

- **Performance next to Play.** A button beside Play (on the instance page and on Home) switches Fast start and the
  smoothness helper and picks Low-end mode for that instance, without opening its settings.
- **Downloads say how far they are:** the per cent, how much of how much ("66% · 210.3 MB of 318.0 MB"), the files
  and the speed, in the sidebar and on the command line.

### Fixed

- Downloads ask for more files at once (32 by default, 16 on small computers, or the number set in Settings): the
  number no longer drops with fewer processor cores, which made thousands of small files (the game's assets, big
  modpacks) slow on a far-away connection. The biggest files start first, so one large file no longer holds up the
  end, and files are written to the disk in larger pieces.
- A modpack file that arrives whole but with another checksum than the pack lists (the pack's list is out of date,
  common in big packs) no longer fails the install: after the usual retries it is kept and named in a warning. A file
  that arrives cut short is still refused, and the game's own files, Java and libraries are checked as strictly as
  before.
- `blockforge create --loader forge --loader-version 14.23.5.2860` (the short form) finds Forge's full version name
  instead of failing with a 404.
- Home: the strip of played days gets shorter on a narrow line instead of cutting off when the game was last played.

## 4.7.0

A friendlier launcher: it asks before what can't be undone, keeps what you typed, says why a button is greyed out and
what went wrong, and remembers the size you like.

### What is new

- **Stop asks first** when Minecraft is running, since it is closed without saving; a double click on Play no longer
  cancels the start.
- **Quit asks first** while a game, a download, an install or a world conversion is running, and says what would stop.
- **Unsaved instance settings are kept.** Save lights up only when something changed, "Unsaved changes" and a dot on
  the Settings tab show what is waiting, changes survive going to another page, and Play offers to save them first.
- **Interface size** in Settings > Appearance (90% to 150%), and the size set with Ctrl and + or - is remembered.
- **A damaged instance is reported** in the Library instead of vanishing, with a button to open its folder.
- **Sign-in problems stay on the Accounts page** with Try again, instead of a message that is gone before you are back
  from the browser; the code is copied for you, a dropped connection no longer ends the sign-in, and an account that
  must sign in again has a button for it.
- **Browse for a skin file**, and clear messages when a file isn't a skin (or is the wrong size).
- **The instance's description** shows on its Overview.
- **Messages** can be clicked away, stay while pointed at or while you are in the game, and a repeated message counts
  up instead of piling up.
- **Every running job** is a click away in the sidebar ("+N more"), each with its Cancel.

### Fixed

- Discover installed mods into instances without a mod loader (where they never load) and said it worked; it now
  offers to add a loader. The instance lists show each instance's Minecraft version and loader.
- A search result still on screen from the previous tab installed as the wrong kind (a modpack as a shader).
- CurseForge "Remove" deleted at once and for good; it now asks, and uses the Recycle Bin (Trash) like the Mods tab.
- Modpack updates (FTB, ATLauncher, Technic, CurseForge) and changing the mod loader could write over changes made
  meanwhile in the Mods tab or the settings; they now wait their turn and never save an old copy of the instance. A
  failed FTB update keeps its update offer.
- Switching, removing, updating and restoring mods, and restoring a backup, are refused while the game runs (on
  Windows its files can't be changed), before asking rather than after.
- Renaming an instance hid its backups; they are renamed with it. Two instances can no longer have the same name (a
  new one or a copy gets a number).
- The Mods tab no longer checks every mod for updates after each switch; offline, a failed check is shown next to its
  button instead of a red message per click. "Remove 12 items?" lists what goes.
- Typing in a Settings field and going to another page lost what was typed; it is saved now.
- Instance memory can't be set to all of the computer's memory without a warning, and shows GB like the rest.
- Tools: busy tools are greyed out with the reason (not refused after confirming), a tool can't run twice at once,
  Debug mode no longer restarts over running jobs, and clean-ups say how much space they freed.
- "Update now" waits while a game or a job runs, instead of restarting the launcher over them.
- Crafting: coming back after a while no longer wipes the plan and its ticks; saved plans can be opened when no plan
  is shown; a downloaded version no longer replaces the open plan; searching `minecraft:oak_planks` works.
- World converter: errors say what to do (no permission to write, full disk, a damaged world); "Try again" became
  "Back to the settings" (and is gone when the world itself is the problem); `~/...` is your home folder; a folder
  that can't be written to is said before converting; closing the launcher during a conversion cleans up after it.
- Escape closes the Mods tab's windows; Enter creates the instance and starts an import; the mod loader window's
  errors wrap and can be retried; Quick play into a world needs a world.
- Keyboard focus is visible on every control, the outlines of buttons and fields are strong enough to see in both
  looks, and low-key buttons light up when pointed at.
- About shows the memory in GB; GNOME (no tray) no longer shows a tray message at every start; the `_JAVA_OPTIONS`
  warning no longer quits the launcher when you ask how to fix it.

## 4.6.0

A reliability release: what happens when a download breaks off, a file is damaged, a pack is badly made or the
computer goes off at the wrong moment.

### What is new

- **Repair game files.** In an instance's menu (and `blockforge repair <instance>`): every library, asset, the game
  itself and Mojang's Java for it are checked against their checksums, and whatever is damaged or missing is
  downloaded again. Worlds, mods and settings are not touched.
- **Cancel a running job** from the sidebar: downloads, installs and pack updates stop at their next step, big files
  included.
- **Backups to keep.** Settings > Backups can keep only the newest backups of each instance (all of them, as before,
  unless set). Backups whose names could also be another instance's are never removed.
- **Crash hints.** When the game closes with one of the well known crashes (too little memory, the wrong Java), the
  Logs tab and a message say what to do.
- **Leftovers are swept** when the launcher starts: half-written backups and snapshots, temporary copies of settings
  files and unpacking folders from work that was interrupted.

### Fixed

- Settings and accounts survive a damaged file: a value of the wrong kind no longer resets every setting or signs out
  every account, and a file that can't be read is kept as `<name>.broken-<time>`. Saves are atomic, flushed to the
  disk and never share a temporary file; `accounts.json` is private from the moment it exists.
- Modpacks can no longer write or delete files outside the instance (Modrinth paths, ATLauncher `deletes`/`keeps`,
  file names, folders and libraries in pack definitions, version and asset ids), and pack updates never overwrite or
  delete worlds the pack shipped, nor take an `instance.json` from the pack.
- Mod updates download the new file before the old one is removed, so a failed update keeps the mod working; a data
  pack and a resource pack of the same project, or of the same file name, no longer delete each other.
- Updates are only offered for the instance's own Minecraft version, whatever the mod browser is set to show.
- CurseForge pack updates take a snapshot first and put the instance back if the update fails; a cancelled
  ATLauncher reinstall no longer marks the instance as corrupted.
- FTB packs put mods that come from CurseForge into `mods/` (they landed in the instance folder and never loaded).
- Imports keep resource packs' `assets`, world icons and `config.json` files deeper in the instance; commands in an
  imported MultiMC or Prism instance are imported switched off, and Prism's override switches are respected.
- Downloads: a missing file (404), a refused connection or a full disk fail at once instead of being tried again; a
  file without a checksum is checked by its size; two jobs needing one file share one download.
- A Java runtime or a Java for loader installers that was cut off half way is fetched again instead of being trusted
  for ever; a broken Forge or NeoForge installer is downloaded again.
- Playing with a Microsoft account without internet starts the game with the saved sign-in (singleplayer works); a
  busy sign-in server no longer asks for a new sign-in, and a renewed sign-in is never written over.
- The game log keeps going after a line that isn't UTF-8 (Windows), and the sign-in token is hidden from the log,
  copies and uploads.
- Starting the game: a starting heap (`-Xms`) bigger than the game may use, too much memory for a 32-bit Java, and
  quoted paths in a wrapper command no longer stop Java from starting; Stop works while the game is being got ready
  and is no longer reported as a crash; a crash in native code is.
- Crashes on: backup names in other alphabets, short asset hashes, a version folder called `0.0`, huge NBT files and
  bad portal records in worlds being converted.
- Backups, snapshots and class caches of instances named outside ASCII (or with look-alike names) are kept apart; two
  restores at once no longer get in each other's way; a failed snapshot restore never deletes the files it moved aside.
- Instance folders avoid names Windows can't use (`CON`, `NUL` ...) and very long names; a copy or a new instance that
  fails half way is removed; an instance deleted while a job runs stays deleted.
- Exports: a pack name with `/` works, and a jar that isn't the file its record names is stored instead of dropped.
- `blockforge account offline` and `--offline-name` check the player name, and `--offline-name` plays as a real
  offline player; `blockforge list` (and the other commands) work next to a file of the same name.
- Starting a pack server on Windows works again.

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
- The bridge jar (`BlockForge-<version>.jar`) that carried plain 3.x jar installs to the native program is no longer
  built or added to releases (its source, `packaging/bridge/`, is removed too); BlockForge is built and released as
  native programs only.
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
