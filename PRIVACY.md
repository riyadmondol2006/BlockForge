# Privacy

BlockForge collects nothing about you or how you use it. There are no usage statistics, no play-time reports and no
crash reports in this version. The old settings called "Share anonymous usage statistics" and "Report pack installs
and play time" are still read from existing settings files so nothing breaks, but nothing is ever sent.

## Servers BlockForge talks to in order to work

These are needed for the launcher to do its job. They see your IP address and the launcher's user agent
(`BlockForge/<version>`), like any website would. None of them is sent your player name, your account details or your
files.

| Server | Why |
|---|---|
| Mojang (`piston-meta.mojang.com`, `piston-data.mojang.com`, `libraries.minecraft.net`, `resources.download.minecraft.net`, `launchermeta.mojang.com`) | Minecraft versions, libraries, assets and Java runtimes |
| Microsoft and Xbox (`login.microsoftonline.com`, `user.auth.xboxlive.com`, `xsts.auth.xboxlive.com`, `api.minecraftservices.com`, `sessionserver.mojang.com`) | Signing in, your profile and skin. **Only if you sign in.** Offline players never contact them. |
| Modrinth and CurseForge | Searching for and downloading mods, modpacks, resource packs and shaders |
| ATLauncher (`api.atlauncher.com`, `download.nodecdn.net`) | ATLauncher's modpack catalogue and news. The user agent says `ATLauncher` there because those servers expect it. |
| FTB (`api.feed-the-beast.com`) and Technic (`api.technicpack.net`) | Their modpack lists and downloads |
| Fabric, Quilt, Forge and NeoForge | Loader versions and downloads |
| Adoptium (`api.adoptium.net`) | One download of Java 21, only when a Forge or NeoForge instance is created and no suitable Java is installed |
| GitHub (`api.github.com`, `github.com`) | Looking for a newer BlockForge and downloading it. Turn this off in Settings > Check for BlockForge updates. |

## Only when you press a button

- **Upload Log** in the console window sends the console text to ATLauncher's paste service
  (`paste.atlauncher.com`) and puts the link on your clipboard.
- **Network checker** in Tools sends its report to the same service when you tell it to.

## Support bundles

The support bundle (Tools > Support bundle) is only ever written to your downloads folder. It is never uploaded.
Before it is written, your player and account names, IDs, access tokens, e-mail addresses, IP addresses and the name of
your user folder are removed, and you can open the ZIP to check.

## Where your data is kept

Accounts, instances and settings are kept in the BlockForge folder on your computer (see the README). Microsoft
sign-in tokens are stored in `configs/accounts.json` inside it, readable only by your user on Linux and macOS. Don't
share that folder.
