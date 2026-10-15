# Testing

## Automatic tests

```sh
cargo test --release                       # unit tests, no network, about 5 seconds
cargo test --release -- --ignored          # also the checks that talk to real services (Mojang, Modrinth, CurseForge, ...)
cargo clippy --release --all-targets -- -D warnings
cargo fmt --check
```

CI runs the first, third and fourth on every push, then builds and smoke-tests every package: the deb on Ubuntu, the rpm
on a clean Fedora, the AppImage, the Windows installer (silent install, shortcut and uninstall entry present,
`blockforge.exe --version` runs, uninstall) and the macOS disk image (mount, bundle layout, `--version`).

## Trying the program by hand

Use a separate data folder so your own instances and accounts are never touched:

```sh
export BLOCKFORGE_DIR=$HOME/blockforge-test
cargo run --release -- account offline Tester
cargo run --release -- create "Test" --mc 1.21.1 --loader fabric
cargo run --release -- launch Test --dry-run     # prints the command line without starting the game
cargo run --release -- launch Test               # starts it
```

## Developer switches (environment variables)

| Variable | What it does |
|---|---|
| `BLOCKFORGE_DIR` | the data folder to use |
| `BLOCKFORGE_BENCH=1` | `blockforge gui` prints how long the first frame took and quits |
| `BLOCKFORGE_SHOT=<folder>` | `blockforge gui` visits every page, saves a PNG of each into the folder and quits (used for the screenshots in `docs/`) |
| `BLOCKFORGE_AUTOPLAY=<instance folder>` | with `BLOCKFORGE_SHOT`: presses Play through the window's own code, saves a picture of the log and stops the game |
| `BLOCKFORGE_CF_KEY` | CurseForge API key to use instead of the built-in one |
| `BLOCKFORGE_MS_CLIENT_ID` | Microsoft sign-in application id to use instead of the built-in one |
| `BLOCKFORGE_UPDATE_API` | address that stands in for GitHub's "latest release" (for testing the updater) |

## Before a release

1. Raise `version` in `Cargo.toml` and add a section for it to `CHANGELOG.md`; the section text becomes the release notes.
2. Push to `main`. CI builds everything and publishes `v<version>` the first time a version is built.
3. Start the installer on a clean Windows, a Mac and a Linux machine, once, and check that the first start, play, update check
   and uninstall all work.
