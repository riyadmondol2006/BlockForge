# Changelog

The section for a version is used as the text of its release on GitHub. Older releases (the Java versions, up to
3.4.42.x) are on the [releases page](https://github.com/riyadmondol2006/BlockForge/releases).

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
