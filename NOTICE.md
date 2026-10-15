# Notices

BlockForge is a fork of [ATLauncher](https://github.com/ATLauncher/ATLauncher), rewritten in Rust, and is licensed under
the GNU General Public License v3 (see `LICENSE`). The original program's copyright holders and contributors keep their
copyright for the parts of the design and behaviour that BlockForge reproduces: the instance, account and settings file
formats, the modpack installers, and the launch logic follow ATLauncher's. BlockForge is not affiliated with or endorsed
by ATLauncher, Mojang or Microsoft.

## Bundled third-party files

- **Fonts** compiled into the window library (`egui`): Ubuntu-Light (Ubuntu Font Licence 1.0), Hack (MIT licence),
  NotoEmoji (SIL Open Font Licence 1.1) and emoji-icon-font (SIL Open Font Licence 1.1 and MIT licence).
- **legacy-launch.jar** (`assets/legacy-launch.jar`): the small wrapper that starts Minecraft versions older than 1.6.
  Its source is in `legacy-launch/` and it comes from ATLauncher (GPL-3.0).
- **Rust libraries**: each one keeps its own licence (MIT, Apache-2.0 or similar). `cargo tree` lists them, and
  `cargo about` or `cargo deny list` can print the licence texts.

The BlockForge logo and icons are original artwork (`docs/logo/`) with no third-party parts.

"Minecraft" is a trademark of Mojang AB.
