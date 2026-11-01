# Packaging

Everything that turns the release binary (`target/release/blockforge`) into something people install. The GitHub
workflow in `.github/workflows/build.yml` runs these scripts on every push; you can run them yourself.

Build the program first: `cargo build --locked --release`. Every script takes the version (the one in `Cargo.toml`)
and writes into `dist/`. The Linux packages also need the two Bedrock Edition helpers, built with
`packaging/linux/helpers.sh build` (see below).

| Folder | Makes | Run it with | Needs |
|---|---|---|---|
| `linux/` | `BlockForge-<v>-linux-x86_64.deb`, `.rpm` and `.AppImage` | `packaging/linux/helpers.sh build`, then `packaging/linux/build.sh <v>` | `dpkg-dev`, `rpm`, `squashfs-tools`, `curl`, `file` (and `desktop-file-utils`, `appstream` for the checks) |
| `windows/` | `BlockForge-<v>-windows-x64-setup.exe` and `-portable.zip` | `packaging\windows\build.ps1 -Version <v>` | Inno Setup 6 |
| `macos/` | `BlockForge-<v>-macos-arm64.dmg` | `packaging/macos/build.sh <v>` | macOS on Apple Silicon (`hdiutil`, `codesign`) |
| `boost/` | `blockforge-boost.jar`, the fast-start Java agent every game is started with (`-javaagent`); not a release file | `packaging/boost/build.sh` | a JDK |

The boost jar is built from `boost/Boost.java` and the built file is checked in next to it, because the launcher embeds it
(`src/launch/boost.rs`) and `cargo build` must work without a JDK. After changing `Boost.java`, run `boost/build.sh` and
commit the new jar. The jar carries its own check, `java -cp blockforge-boost.jar com.blockforge.boost.SelfTest` (from
`boost/SelfTest.java`; any Java 8 or newer, any system); `build.sh` runs it, and the agents job in CI builds the jars again
and runs it on both jars.

Each system also has a smoke test that CI runs on the result: `windows/smoke.ps1`, `macos/smoke.sh`, and for Linux the
workflow installs the deb, the rpm (in a clean Fedora container) and starts the AppImage.

## The Bedrock Edition helpers (Linux packages)

Every Linux package carries two more programs in `/usr/bin` (in the AppImage: `usr/bin`), next to `blockforge`, which looks
for them beside its own file:

- `blockforge-bedrock-auth`, the Microsoft sign-in window (`crates/bedrock-auth`). It links GTK 3, WebKitGTK 4.1 and
  libsoup 3 from the computer, so WebKit is **not** a hard dependency of anything: BlockForge itself starts without it. The deb
  lists `libwebkit2gtk-4.1-0 (>= 2.40)` under `Recommends` and the rpm `libwebkit2gtk-4.1.so.0()(64bit)` under `Recommends`
  (both installers install these by default, and can be told not to). The AppImage does not bundle WebKit (it is large);
  the helper uses the computer's, and BlockForge shows what to install when it is missing.
- `bedrock-runtime`, which runs inside the Steam Linux Runtime container (`crates/bedrock-runtime`). It is built static
  with `--target x86_64-unknown-linux-musl`, so it depends on nothing, in the container or out of it.

`packaging/linux/helpers.sh build|clippy|test` builds, lints and tests both (the crates are built as members of the
workspace in `Cargo.toml` when they are listed there, from their own `Cargo.toml` otherwise); it needs `protoc` (for
BlockForge itself), the GTK 3, WebKitGTK 4.1 and libsoup 3 development files, and `rustup target add
x86_64-unknown-linux-musl`. `build.sh` refuses to pack a helper that is missing, and a `bedrock-runtime` that is not
static. `packaging/linux/check-helpers.sh [root]` checks an installed system or an unpacked AppImage the same way (CI runs
it on the deb and the AppImage). Windows and macOS build none of this; `.github/scripts/linux-only-deps.sh` fails the
checks if one of these dependencies ever shows up in their dependency tree.

## The release file names matter

The updater of every BlockForge version, including the old Java 3.x ones, picks the release file to download by its
ending (`.exe`, `.dmg`, `.deb`, `.rpm`) and CPU word (`x64`, `x86_64`, `amd64`, `arm64`). So each ending must
belong to **exactly one** file in a release; the publish step of the workflow refuses to release otherwise. The portable
Windows build is a `.zip` and the AppImage is an `.AppImage` for that reason.

## Upgrading older installs

- The Linux packages are named `blockforge` like the old Java ones and have `Replaces`/`Conflicts`/`Obsoletes` entries, so
  installing the new one removes the old one (and its `/opt/blockforge` files).
- The Windows setup finds an old Java install (a jpackage MSI with upgrade code
  `bfd7c18e-448a-4abd-80c9-5df7629af779`) and uninstalls it silently first. It never touches `%APPDATA%\BlockForge`.
- The plain `.jar` of BlockForge 3.x is no longer built; the old Java updater has nothing to download, and people on a
  plain jar install the setup, disk image or package instead (their data folder is the same).

Releases are not code-signed. See `macos/README.md` for what macOS users see on first start.
