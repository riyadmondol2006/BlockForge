# Privacy

BlockForge sends nothing about how you use it unless you turn it on.

## Off unless you turn it on

| What | Where to turn it on | What it sends |
|---|---|---|
| Anonymous usage statistics | Settings > Privacy | Which screens you open and which actions you take (for example "instance launched"), with a random ID that is not linked to your name, account or files. |
| Pack installs and play time | Settings > Privacy | The ID and version of a public modpack when you install, update or play it, and how long you played. Only for packs that have a public page. |
| Crash reports | Start with `--enable-error-reporting` | Error details from the launcher to a third party service. There is no setting for this in the app. |

With these off the launcher doesn't collect the events at all, it doesn't just hold back from sending them. Turning
statistics off again throws away anything that was waiting to be sent. The rule lives in one place,
`privacy/Privacy.java`, and `PrivacyTest` checks that the defaults are off and that a server can't switch error
reporting on.

## Servers BlockForge talks to in order to work

These are needed for the launcher to do its job, and they see your IP address and the launcher's user agent, like
any website would. None of them are sent your player name, account details or files.

- **Mojang and Microsoft**: Minecraft versions, libraries, assets and Java runtimes; Microsoft sign-in only if you sign in.
- **ATLauncher's servers** (`api.atlauncher.com`, `download.nodecdn.net`): the modpack catalogue and news, launcher data files, and
  the loader version lists. The user agent says `ATLauncher` because those servers expect it.
- **CurseForge and Modrinth**: searching for and downloading mods and modpacks.
- **Fabric, Quilt, Forge and NeoForge**: loader versions and downloads.

Offline players never contact Mojang or Microsoft to log in.

## Support bundles

The support bundle (Tools > Support bundle, or an instance's menu) is only ever written to your downloads folder. It is
never uploaded. Before it is written, your player and account names, IDs, access tokens, e-mail addresses, IP addresses
and the name of your user folder are removed, and you can open the ZIP to check. `DiagnosticsTest` covers this.

## Where your data is kept

Accounts, instances and settings are kept in the BlockForge folder on your computer. Microsoft sign-in tokens are stored
there too, so don't share that folder.
