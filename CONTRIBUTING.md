# Contributing

Thanks for wanting to help. Bug reports, fixes, translations and features are all welcome.

Please talk about a bigger change in an [issue](https://github.com/riyadmondol2006/BlockForge/issues) first, so the work
doesn't go to waste. Follow the [code of conduct](CODE_OF_CONDUCT.md). Security problems go through
[private reporting](SECURITY.md), not public issues.

## Building

You need the stable [Rust toolchain](https://rustup.rs) (1.98 or newer). Nothing else is needed on Windows and macOS. On
Linux, a C compiler (`gcc` or `clang`) and `protoc` are needed to build (Debian and Ubuntu: `protobuf-compiler`, Fedora:
`protobuf-compiler`; it compiles the protocol files of the vendored Xodus crates in `third_party/xodus` for Bedrock
Edition, and `PROTOC=/path/to/protoc` points the build at one that is not on the `PATH`). The window needs OpenGL and
either Wayland or X11 at run time, which any desktop has. WebKitGTK is not needed to build or start BlockForge itself.

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

### The Bedrock Edition helpers (Linux)

Two small programs are built next to `blockforge` and shipped beside it in the `.deb`, `.rpm` and AppImage. You only need
them to try Bedrock Edition on Linux (`cargo run` does not build them) and to make the packages:

| Program | Crate | Needs |
|---|---|---|
| `blockforge-bedrock-auth`, the Microsoft sign-in window | `crates/bedrock-auth` | the GTK 3, WebKitGTK 4.1 (2.40 or newer) and libsoup 3 development files: Debian and Ubuntu `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev`, Fedora `webkit2gtk4.1-devel gtk3-devel libsoup3-devel`. Only this program needs WebKitGTK at run time, and BlockForge says so when it is missing. |
| `bedrock-runtime`, which runs inside the Steam Linux Runtime container | `crates/bedrock-runtime` | the Rust target `x86_64-unknown-linux-musl` (`rustup target add x86_64-unknown-linux-musl`); it is built static, so it needs no C compiler for musl |

```sh
packaging/linux/helpers.sh build    # both, in release mode, into the same target folder as blockforge
packaging/linux/helpers.sh clippy   # the lint CI runs on them
packaging/linux/helpers.sh test     # and their tests
```

The vendored Xodus crates are not linted (they are not our code); `cargo clippy` on the program covers the rest.

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
| `packaging/` | Linux packages, Windows installer, macOS disk image |
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

## Releases

Every new release gets a new version: raise `version` in `Cargo.toml` (and the matching entry in `Cargo.lock`), add a
section for it at the top of `CHANGELOG.md`, and add a `<release>` line to the AppStream file in
`packaging/linux/`. Pushing to `main` then builds, checks and publishes `v<version>` on the Releases page. A push
that keeps the same version only refreshes the `nightly` build.
