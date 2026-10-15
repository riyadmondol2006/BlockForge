# Contributing

Thanks for wanting to help. Bug reports, fixes, translations and features are all welcome.

Please talk about a bigger change in an [issue](https://github.com/riyadmondol2006/BlockForge/issues) first, so the work
doesn't go to waste. Follow the [code of conduct](CODE_OF_CONDUCT.md). Security problems go through
[private reporting](SECURITY.md), not public issues.

## Building

You need the stable [Rust toolchain](https://rustup.rs) (1.88 or newer). Nothing else is needed on Windows and macOS. On
Linux, a C compiler (`gcc` or `clang`) is enough to build; the window needs OpenGL and either Wayland or X11 at run
time, which any desktop has.

```sh
cargo run --release            # the launcher window
cargo run --release -- list    # the same program has command-line commands: try `--help`
cargo test --release           # all unit tests (about 5 seconds)
cargo fmt --check              # formatting (max width 120, see rustfmt.toml)
cargo clippy --release --all-targets -- -D warnings
```

Set `BLOCKFORGE_DIR=/some/folder` to keep all data (instances, accounts, settings) somewhere else while you try
things. Never point tests at your real data folder.

Installers are built by the scripts in `packaging/` and by `.github/workflows/build.yml`; see
[packaging/README.md](packaging/README.md).

## How the code is laid out

| Path | What is in it |
|---|---|
| `src/main.rs` | the command line and the start of the window |
| `src/gui/` | the window (egui): one file per page or dialog; `mod.rs` holds the state and the message loop |
| `src/launch.rs`, `src/launch/` | everything between pressing Play and the game process: Java choice, memory, arguments, natives, class cache |
| `src/install.rs`, `src/java.rs`, `src/loaders.rs` | the parallel downloader, Mojang's files and Java runtimes, Fabric/Quilt/Forge/NeoForge |
| `src/instances.rs`, `src/modrecord.rs`, `src/model.rs` | the instance folder and `instance.json` (the Java edition's format) |
| `src/modrinth.rs`, `src/curseforge/`, `src/packs/`, `src/ftb.rs` | where mods and modpacks come from |
| `src/mods/`, `src/content/` | installed mods (metadata, updates, scan) and installing content of every kind |
| `src/imports/`, `src/export.rs` | bringing in instances from other launchers, and exporting |
| `src/accounts.rs`, `src/msauth.rs`, `src/skins.rs` | accounts, Microsoft sign-in, skins |
| `src/update/`, `src/update.rs` | the launcher's own updates |
| `packaging/` | Linux packages, Windows installer, macOS disk image, the bridge jar |
| `legacy-launch/` | source of the small Java wrapper that starts Minecraft older than 1.6 (`assets/legacy-launch.jar`) |

Files the Java edition wrote (`instance.json`, `accounts.json`, `BlockForge.json`) must keep working in both directions:
unknown fields are carried along untouched, and every new field is optional.

## Pull requests

- One topic per pull request, with a clear title and a short description of what changed and why.
- Add or update tests for the behaviour you change. Pure logic gets a unit test; anything that talks to a real service
  gets an `#[ignore]` test that can be run with `cargo test -- --ignored`.
- `cargo fmt`, `cargo clippy` (no warnings) and `cargo test` must pass; CI checks all three on Linux, Windows and macOS.
- Write comments that say why, in plain words. Error messages are read by players, so make them say what to do.
- Keep your commit history tidy: small commits with a short first line that says what changed.

## Translations

Text goes through `i18n::tr`. Translation files are standard gettext `.po` files named like `de-DE.po`; drop one in
`assets/lang/` and it is built in, or put it in `<data folder>/lang/` to try it without rebuilding.
`cargo run -- pot --output blockforge.pot` writes the template of every text that can be translated.
