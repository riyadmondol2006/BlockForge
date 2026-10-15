<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/logo/banner-dark.svg">
    <img src="docs/logo/banner.svg" alt="BlockForge, a Minecraft launcher" width="640">
  </picture>
</p>

# BlockForge

A Minecraft launcher with a modern interface that opens fast, plays offline without an account, and keeps working when
downloads or mod sites misbehave.

Repository: <https://github.com/riyadmondol2006/BlockForge>

> **BlockForge is a fork of [ATLauncher](https://github.com/ATLauncher/ATLauncher)** (GPL-3.0). All of the launcher's
> core (modpack installs, mod loaders, instance handling, account and launch code) comes from ATLauncher and its
> contributors, and their copyright notices are kept in the source. See [What's different](#whats-different-from-atlauncher)
> for what this fork changes. BlockForge is not affiliated with or endorsed by ATLauncher.

## Features

- **Play without an account**: when BlockForge opens it asks who is playing. Type a name to play offline, pick a saved
  player, or sign in with Microsoft. Microsoft sign-in is optional.
- **Modpacks from many places**: search ATLauncher, CurseForge, Modrinth, FTB and Technic packs in one screen, or build
  an instance from any Minecraft version with Fabric, Forge, NeoForge or Quilt.
- **Modrinth first**: Modrinth is the default source for adding mods and for exporting.
- **Import your old launcher**: finds Prism, MultiMC, CurseForge, GDLauncher and ATLauncher instances and copies them in.
- **Safe updates**: a snapshot of mods and configs is taken before every update or reinstall, and can be restored.
- **Plain-English crash help**: after a crash it says what most likely went wrong, and can make a support bundle with
  all personal details removed.
- **Private by default**: nothing about how you use it is sent unless you turn it on, see [PRIVACY.md](PRIVACY.md).

## What's different from ATLauncher

This fork is a work in progress. Compared to ATLauncher 3.4.41.4 it changes:

**Look and feel**
- A new main window written from scratch: a navigation sidebar and the screens Home, Library, Discover, Create, Tools
  and Settings, with dark and light themes (Roboto font, vector icons drawn in code).
- The original tab layout, the Servers and About screens, the all-in-one settings tabs, the old instance cards and the
  social media buttons (Discord, GitHub, Reddit, Facebook, Twitter, Nodecraft) are removed.
- New name, package (`com.blockforge.launcher`), logo, splash screen and data folder, so it never mixes with an
  existing ATLauncher install. A Material 3 theme is included for the pop-up dialogs that still use the old style.

**Accounts**
- Offline players are built in. The launcher asks who is playing on every start (can be turned off in Settings), and
  there is no longer a need for an account to play.

**Speed**
- The window appears in about 1 second instead of about 2 (about 0.8 with `scripts/blockforge.sh`). The launcher file
  list is cached, the console, tray icon and system information load in the background, and the old forced garbage
  collection is gone.
- Launching Minecraft skips re-hashing files that haven't changed, remembers the Java version, skips the login step for
  offline players and no longer keeps a CPU core busy while waiting on downloads.

**Reliability**
- Downloads retry on busy servers (HTTP 429/5xx) and network hiccups, wait as the server asks, and are written to a
  temporary file so an interrupted download never leaves a half-written file.
- The launch log says which Java was chosen and why, and warns about a Java that is too old, 32-bit or running under
  emulation on ARM.
- New code for importing instances, snapshots and rollback, a mod dependency planner, and matching hand-downloaded
  CurseForge files, all with unit tests.

**Privacy**
- Usage statistics, pack install/play-time reporting and crash reports are off by default, and a remote setting can no
  longer turn crash reporting on. Self-updating is turned off, since it would replace BlockForge with ATLauncher.

## Download and install

Every push to this repository builds installers for each system. Get them from the **Actions** tab (open a run, then
**Artifacts**), or from the **Releases** page: the `nightly` release always holds the latest build from `master`.

| System | File | Notes |
|---|---|---|
| Windows | `BlockForge-<version>-windows-setup.exe` | Installs for your user, no administrator needed. Adds a Start menu entry and a shortcut. |
| macOS | `BlockForge-<version>-macos-arm64.dmg` (Apple Silicon) or `-macos-x64.dmg` (Intel) | Drag BlockForge to Applications. |
| Fedora, RHEL and other RPM systems | `BlockForge-<version>-fedora-x86_64.rpm` | `sudo dnf install ./BlockForge-*.rpm` |
| Debian, Ubuntu and other deb systems | `BlockForge-<version>-debian-ubuntu-amd64.deb` | `sudo apt install ./BlockForge-*.deb` |
| Any system with Java 17 or newer | `BlockForge-<version>.jar` | `java -jar BlockForge-*.jar`, keeps its files next to the jar. |

The installers carry their own Java, so nothing else needs to be installed. An installed BlockForge keeps its
instances, settings and logs in your user folder (`%APPDATA%\BlockForge` on Windows,
`~/Library/Application Support/BlockForge` on macOS, `~/.local/share/blockforge` on Linux), so uninstalling or updating
it never touches them.

The builds aren't signed yet. Windows may show "Windows protected your PC": choose **More info** then **Run anyway**.
On macOS, right-click the app and choose **Open** the first time.

## Run the jar

```sh
java -jar build/libs/BlockForge-<version>.jar
```

On Linux and macOS `scripts/blockforge.sh` starts it with a class cache for a faster start (Java 19 or newer).

## Services it uses

To work, BlockForge talks to Mojang and Microsoft, CurseForge, Modrinth, the loader sites (Fabric, Quilt, Forge,
NeoForge) and, for the modpack catalogue, news and launcher data files, **ATLauncher's servers**
(`api.atlauncher.com`, `download.nodecdn.net`). Upstream asks that others don't use its CDN, files and modpacks in their
own launcher without permission, so if you run or publish this fork widely, please check with ATLauncher or replace
those endpoints in `Constants.java`. More detail is in [PRIVACY.md](PRIVACY.md).

**Before you publish builds:** `Constants.java` still holds ATLauncher's own CurseForge API key and Microsoft login
client ID, which upstream says forks must not reuse. Get your own
([CurseForge key](https://forms.monday.com/forms/dce5ccb7afda9a1c21dab1a1aa1d84eb),
[Microsoft client ID](https://learn.microsoft.com/en-us/azure/active-directory/develop/quickstart-register-app)) and
replace them before sharing the launcher with other people.

## For developers

- `scripts/package.sh` (Linux and macOS) and `scripts/package.ps1` (Windows) build the installers with `jpackage`;
  `.github/workflows/build.yml` runs them on every push.
- `scripts/blockforge.sh` starts the jar with a class cache, which opens the window faster.
- `scripts/startup-budget.sh` fails if the window takes longer than a set time to appear.
- `scripts/loader-canary.sh` checks that the Fabric, Quilt, Forge and NeoForge services still answer as the launcher
  expects.
- [PRIVACY.md](PRIVACY.md) lists what is and isn't sent, and [NOTICE.md](NOTICE.md) lists the third-party files.

## Upstream

BlockForge builds on [ATLauncher](https://github.com/ATLauncher/ATLauncher) by ATLauncher and its contributors. If you
want the original launcher, get it from [atlauncher.com](https://atlauncher.com).

## Contributing

Take a look at [CONTRIBUTING.md](CONTRIBUTING.md)

## Testing

Please see the [TESTING.md](TESTING.md) file for information on how we write tests.

## Prerequisites

In order to build BlockForge you need Java 17 or newer to run Gradle (Gradle 8.12 can't run on Java 25, so use 17 or 21).
The code itself compiles to Java 8 compatibility.

Everything else that's needed for the project is provided by Gradle, and accessed using the Gradle wrapper which can be
invoked by using `./gradlew`.

## Building

To build this project, simply run:

```sh
./gradlew build
```

To build just the runnable jar, run `./gradlew shadowJar`. It is written to `build/libs/`.

## Running in test

If you want to run the launcher while developing with it, you can use your IDE (if you have one) to do that for you.

Alternatively you can run:

```sh
./gradlew run --args="--debug --working-dir=testLauncher"
```

Setting the `--working-dir=testLauncher` argument is necessary as it will ensure that the launchers files are not
spewed in the root directory and are instead contained within a gitignored folder.

## Using an IDE

~~This project is mainly setup and developed to use [VSCode](https://code.visualstudio.com/) for development. You're free
to use any other IDE that you're accustomed to (if any), but by using VSCode, you get the benefit of predefined tasks
and launch commands as well as a list of extensions recommended for the project.~~ VSCode no longer works well after the upgrade to be part Kotlin. Until fixed, it's recommended to use IntelliJ.

We also provide some base project files for [IntelliJ IDEA](https://www.jetbrains.com/idea/) so that if you use that,
you should get access to our base project files which contain correct launch tasks for testing the application as well
as tasks for running the UI/Unit tests.

## Checking for dependency updates

To check for dependency updates with gradle, simply run:

```sh
./gradlew dependencyUpdates
```

This will print a report to the console about any dependencies which have updates.

## Updating new GraphQL queries/mutations

When new GraphQL queries/mutations are added into the `src/main/graphql` directory, you must first download the JSON
schema from <https://studio.apollographql.com/public/ATLauncher/variant/current/schema/sdl> and then run the
`generateApolloSources` task to generate the Java files:

```sh
./gradlew generateApolloSources
```

This will codegen the java files so you can use the query/mutation.

## Updating license headers in all files

If you add new files, or update the `LICENSEHEADER` file, you can add that to all source files by running:

```sh
./gradlew updateLicenses
```

To check that they're all correct, you can run the below command:

```sh
./gradlew checkLicenses
```

This is run during the CI process, and will fail if the license is missing or not up to date, so make sure that you add
this to all new files you create.

## Create Custom Themes

BlockForge supports custom themes. The process is fairly straight forward, but may require a lot of trial and error.

First you must create a `MyThemeName.java` in the `src/main/java/com/blockforge/launcher/themes/` directory. Your theme should
extend one of the base themes depending on what you need:

-   `Dark` is the default theme and is a dark theme. It's a good place to start with some defaults for new dark themes.
-   `Light` is a light theme. It's a good place to start with some defaults for new light themes.
-   `ATLauncherLaf` is a base class which every theme MUST at some point extend. It provides some defaults including our
    brand colours and some defaults. This shouldn't be extended from unless you need absolute power.

Once you've created your class (look at other themes in the directory for an idea on what you can do), you'll need to
create a properties file in the `src/main/resources/com/blockforge/launcher/themes/` directory. This properties file is how you
setup and change UI elements. You should use the existing examples in that directory as examples.

Last step is to register the theme in the file `src/main/java/com/blockforge/launcher/gui/tabs/settings/GeneralSettingsTab.java`.

Now you can open the launcher and then switch to your theme.

We use a library called [FlatLaf](https://github.com/JFormDesigner/FlatLaf) to provide theme support. There are some
good references listed below to see the default values for the themes and see what you can overwrite:

-   <https://github.com/JFormDesigner/FlatLaf/blob/master/flatlaf-core/src/main/resources/com/formdev/flatlaf/FlatLaf.properties>
    -   This file contains all the base properties for all themes
-   <https://github.com/JFormDesigner/FlatLaf/blob/master/flatlaf-core/src/main/resources/com/formdev/flatlaf/FlatLightLaf.properties>
    -   This file contains all the base properties for light themes
-   <https://github.com/JFormDesigner/FlatLaf/blob/master/flatlaf-core/src/main/resources/com/formdev/flatlaf/FlatDarkLaf.properties>
    -   This file contains all the base properties for dark themes

### IntelliJ theme.json Support

You can also take IntelliJ `theme.json` files and convert them to themes for ATLauncher. From within the `theme.json`
file, take the `UI` object and plug that into [this site](https://tools.fromdev.com/json-to-property-converter.html) to
convert it from JSON to properties format.

There are also special rules you need to apply as we currently do not support these `theme.json` files out of the box,
so you need to manually apply the [transformations](https://github.com/JFormDesigner/FlatLaf/blob/master/flatlaf-core/src/main/java/com/formdev/flatlaf/IntelliJTheme.java)
in order for the theme to work exactly right.

For an example, see the `DraculaContrast` theme which uses this method.

### Tools To Help Theme Development

To help with theme development, with the launcher running (not in the release version, only in development), you can
press `Ctrl + Shift + Alt + X` to bring up a tool to highlight UI components to see their properties. You can also press
`Ctrl + Shift + Alt + Y` to bring up a list of all the default properties in the UIManager. These values can be modified
in your `.properties` file.

## Plugging In Your Data

To get started with the code and plug in your own data, you need to edit the
`/src/main/java/com/blockforge/launcher/constants/Constants.java` file.

By using this source code you don't get permissions to use our CDN/files/assets/modpacks. See the License section at the
bottom for more.

Most of of them should be self explanatory, if not please stop by our [Discord](https://atl.pw/discord) and ask in the
`#development` channel if you need help understanding any of the values.

A couple values in the constants file are specific for ATLauncher and shouldn't be used in any forks. These are the
CurseForge Core api key and the Microsoft Login Client ID.

You can apply for a CurseForge Core key through
[this link](https://forms.monday.com/forms/dce5ccb7afda9a1c21dab1a1aa1d84eb) and a Microsoft Login Client ID through
[this link](https://learn.microsoft.com/en-us/azure/active-directory/develop/quickstart-register-app).

## Versioning System

Starting with version 3.2.1.0 a new versioning system was put into place. It works off the following:

Reserved.Major.Minor.Revision.Stream

So for 3.2.1.0 the major number is 2 and minor number is 1 and revision number is 0. Reserved is used as a base, only
incremented on complete rewrites. The stream is optional.

Major should be incremented when large changes/features are made.

Minor should be incremented when small changes/features are made.

Revision should be incremented when there are no new features and only contains bug fixes for the previous minor.

Build is used for beta releases allowing you to have higher version numbers but force users to update when the real
release comes.

Stream is used to define if it's a "Release" or a "Beta". When not provided, it defaults to "Release".

### Updating The Version

The version can be updated in a single place in the `/src/main/resources/version` file.

The stream value doesn't need to be provided, but should only ever be "Beta". When a release is ready to go out, the
stream should be removed from the version so that everything will automatically release.

## Translating

ATLauncher is written for English speakers. All our translations are community run by those who take their time and
submit updates to the text in a different language.

If you wish to help translate ATLauncher, please visit our page on [Crowdin](https://crowdin.com/project/atlauncher) and
start translating.

### Updating the template file

Every push to master will automatically add any new strings that need translating via GitHub actions.

#### Manual Steps

If new strings are added to the launcher, the template file will need to be updated in order to take into account the
new strings.

In order to do this, run `./gradlew generatePots` which will scan the source files and create a
`build/gettext/translations.pot` file.

Note that out of the box, this will not generate in the correct format. You must run the `deduplicateTranslations` script in
the `scripts/deduplicateTranslations` folder which will use Docker to fix the `translations.pot` file.

This file can then be uploaded to Crowdin by ATLauncher staff to give access to the translators.

### Adding new languages from Crowdin

Running [this action](https://github.com/ATLauncher/ATLauncher/actions/workflows/download-translations.yml) will
download all approved translations strings and make a commit to master with then changed language files.

#### Manual Steps

Once a language has been translated enough to add support to the launcher (or update support) there's a few steps we
need to take.

First grab the built project from Crowdin, and then grab out the translation to add/update. For this example, let's take
German.

Pop this file in the `scripts/processTranslations/in` directory and then run the `scripts/processTranslations.bat` or
`scripts/processTranslations.sh` file to fix them up and output them into the `scripts/processTranslations/out`
directory.

Now take the converted files from the `scripts/processTranslations/out` directory and put them in the
`src\main\resources\assets\lang` directory.

Now open `src\main\java\com\atlauncher\data\Language.java` and in the static block at the top, add in the language:

```java
// add in the languages we have support for
static {
    locales.add(Locale.ENGLISH);
    locales.add(new Locale("de", "DE"));
}
```

Now the launcher should have the option to change to the language/s.

## License

BlockForge is a fork of ATLauncher and keeps its license. This work is licensed under the GNU General Public License v3.0. To view a copy of this license, visit
<http://www.gnu.org/licenses/gpl-3.0.txt>.

A simple way to keep in terms of the license is by forking this repository and leaving it open source under the same
license. We love free software, seeing people use our code and then not share the code, breaking the license, is
saddening. So please take a look at the license and respect what we're doing.

Also, while we cannot enforce this under the license, you cannot use our CDN/files/assets/modpacks on your own launcher.
Again we cannot enforce this under the license, but needless to say, we'd be very unhappy if you did that and really
would like to leave cease and desist letters as a last resort.
