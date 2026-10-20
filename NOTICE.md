# Notices

BlockForge is a fork of [ATLauncher](https://github.com/ATLauncher/ATLauncher), rewritten in Rust, and is licensed under
the GNU General Public License v3 (see `LICENSE`). The original program's copyright holders and contributors keep their
copyright for the parts of the design and behaviour that BlockForge reproduces: the instance, account and settings file
formats, the modpack installers, and the launch logic follow ATLauncher's. BlockForge is not affiliated with or endorsed
by ATLauncher, Mojang or Microsoft.

## Bundled third-party files

- **Fonts** compiled into the window library (`egui`): Ubuntu-Light (Ubuntu Font Licence 1.0), Hack (MIT licence),
  NotoEmoji (SIL Open Font Licence 1.1) and emoji-icon-font (SIL Open Font Licence 1.1 and MIT licence).
- **Inter** (`assets/fonts/Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf`): the interface typeface, copyright
  (c) 2016 The Inter Project Authors, SIL Open Font License 1.1 (`assets/fonts/OFL.txt`). The files are Latin subsets of
  the static Inter 4.1 fonts with the tabular figures set as the default digits.
- **legacy-launch.jar** (`assets/legacy-launch.jar`): the small wrapper that starts Minecraft versions older than 1.6.
  Its source is in `legacy-launch/` and it comes from ATLauncher (GPL-3.0).
- **Rust libraries**: each one keeps its own licence (MIT, Apache-2.0 or similar). `cargo tree` lists them, and
  `cargo about` or `cargo deny list` can print the licence texts.

The BlockForge logo and icons are original artwork (`docs/logo/`) with no third-party parts.

"Minecraft" is a trademark of Mojang AB.

## Chunker (world conversion)

The world converter (`src/convert/`) is a Rust port of [Chunker](https://github.com/HiveGamesOSS/Chunker) by Hive Games.
Chunker is distributed under the MIT License, reproduced here. BlockForge is not affiliated with or endorsed by Hive
Games. Files derived from Chunker say so in their header.

```
MIT License

Copyright (c) 2024 Hive Games

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
