# BlockForge for macOS (Apple Silicon)

`BlockForge-<version>-macos-arm64.dmg` holds `BlockForge.app` for Macs with an M-series chip (macOS 11 or newer).
Open the disk image and drag BlockForge onto the Applications folder next to it.

## The first time you open it

BlockForge is not signed with an Apple Developer ID and is not notarized (that needs a paid Apple developer account).
The app carries an *ad hoc* signature, which is what lets any program run on Apple Silicon, but macOS still shows a
warning for apps that were downloaded from the internet and are not notarized. It is shown once; after that
BlockForge opens like any other app.

**macOS 14 Sonoma and older**

1. Open the Applications folder, hold Control and click BlockForge (or right-click it), choose **Open**.
2. Click **Open** in the dialog.

**macOS 15 Sequoia and newer** removed that shortcut:

1. Double-click BlockForge. macOS says it could not verify it. Click **Done**.
2. Open **System Settings > Privacy & Security** and scroll to **Security**.
3. Next to "BlockForge was blocked to protect your Mac" click **Open Anyway**, then enter your password and click
   **Open Anyway** again.

**From a terminal** (any version): `xattr -dr com.apple.quarantine /Applications/BlockForge.app`

If macOS says "BlockForge is damaged and can't be opened", the app was changed after it was downloaded (a broken
download, or a tool that unpacked it). Download the disk image again and compare it with `SHA256SUMS.txt` on the
release page.

## Upgrading from the Java edition

The Java edition was also installed as `/Applications/BlockForge.app`. Drag the new BlockForge onto Applications and
choose **Replace**. Your instances, accounts and settings stay where they were: both editions keep them in
`~/Library/Application Support/BlockForge`. Removing the app (drag it to the Bin) never touches that folder.

## What is in the app

| | |
| --- | --- |
| `Contents/MacOS/blockforge` | the program, a single native arm64 executable (no Java needed) |
| `Contents/Info.plist` | id `io.github.riyadmondol2006.BlockForge`, version, minimum macOS, Retina support, category Games, `.mrpack` files |
| `Contents/Resources/BlockForge.icns` | the icon (16 to 1024 pixels, with Retina sizes) |

The command line works from the app too: `/Applications/BlockForge.app/Contents/MacOS/blockforge --help`.

## Building

On an Apple Silicon Mac (the GitHub `macos-15` runner is one):

```sh
MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release
packaging/macos/build.sh 4.0.0          # makes dist/BlockForge-4.0.0-macos-arm64.dmg
packaging/macos/smoke.sh                # mounts it and checks it
```

`build.sh` lays out the bundle, fills in `Info.plist` from `Info.plist.in`, signs it ad hoc
(`codesign --force --deep --sign -`) and makes the image. The look of the image (green background with an arrow,
icon positions) is written by [dmgbuild](https://dmgbuild.readthedocs.io), which it installs into a throw-away
virtual environment; it does not need a screen or Finder scripting. If dmgbuild can't be installed, the image is made
with `hdiutil` alone: same contents, no background. `smoke.sh` says which one it got.

`dmg-background.png` and `dmg-background@2x.png` are the image's background (the Retina one is picked up
automatically); `BlockForge.icns` is a copy of `src/main/resources/assets/image/icon.icns` of the Java edition.
