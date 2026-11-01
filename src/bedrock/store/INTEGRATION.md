# Package A integration

No changes to the frozen public Bedrock types/signatures. `store` stays private to the Bedrock facade
on Linux with `bedrock`; the feature is default. Root Rust 1.98 requirement was
already present in the kickoff commit. No 1.89 compatibility is claimed.

Use `store::Context { root: dirs.base.join("bedrock"), client: configured_client,
auth_helper: installed_absolute_helper_path }` on backend workers. It performs no
startup probes. Operations use the frozen `StoreId`, `BuildSelection`, `Cancel`,
`Progress` and `BedrockError` types. Adapter exports:

- `sign_in(&Context, StoreId, Arc<Progress>, Cancel)`
- `sign_out(&Context, StoreId, SignOutMode)`
- `install(&Context, StoreId, BuildSelection, Arc<Progress>, Cancel)`
- `stage(&Context, StoreId, &InstalledBuild, Cancel)`

`InstalledBuild` contains `id`, `content`, `downloaded_files` and an owned build
lease. Keep it for the session. The facade must refuse sign-out/build mutation
while a session references the slot/build, as required by the shared contract.
The Store operation holds its slot lock through provisioning, exchanges and
account publication; merge operations cannot overlap. Explicit User logout keeps
both device license and device tokens. UserAndDevice is the only reset path;
facade must guard this destructive device deletion with its idle-session check.
Corrupt/lost credentials and interrupted device registration request explicit
repair instead of silently spending another device registration. Initial firmware
probing may trigger the optional upstream pkexec prompt; never run BF as root.

Integrated (src/bedrock/linux.rs): `latest(&Context, StoreId, Edition, Cancel)` answers the newest build and its
size for the window's refresh; `cached_state(root, &StoreId)` reads the slot's sign-in hint without its lock or the
network; Play stages with `stage_into(.., dir, ..)` straight into the session supervisor's RAM staging folder
(`<XDG_RUNTIME_DIR or /dev/shm>/blockforge-bedrock*/<session>/`, owned and removed by the supervisor), so there is
one copy of the plaintext and one cleanup policy. `stage` (own folder, removed on drop) remains for the ignored
scratch test. The facade records C's game-folder fixups with `InstalledBuild::record_patches` and checks a build
locally with `damaged_files` on Repair.

`StagedImages.images` has relative image name, tmpfs path and open read/write File
for package C's PE stack patch and package E's Wine file map. Keep the images until
the loader/supervisor confirms release. Drop removes only this session's directory.
The staging namespace is `/dev/shm/blockforge-bedrock-<uid>/<random>/`, verified
as tmpfs, mode 0700/files 0600. There is no disk fallback. Coordinate a shared
staging session cleanup policy with E. A full network license is reacquired at
stage; existing encrypted files alone do not authorize offline launch.

Install receipts are schema 1/adapter 1, include content identity, exact remote
segment hashes and local SHA-256 of post-extraction bytes (ciphertext for retained
images). Current reuse requires matching nonempty segment hashes, offsets,
encryption flag, length and local SHA-256. Hashless entries are conservatively
fetched again and counted. This is not a claim of Microsoft signatures or
end-to-end verification for unencrypted sections. After C applies a known durable patch on an idle generation, call
`InstalledBuild::record_patch(relative, revision)` to atomically record its local
hash, post-patch length and revision while retaining the original remote hashes.
Re-runs preserve those verified patched bytes. The integrator must publish patch
generations under its idle-build policy; do not mutate a live install. Encrypted
executables may only be patched in tmpfs; record_patch rejects them. `.xodus-streaming.msixvc` is REQUIRED
metadata. Never clear it as disposable cache. Completed prior generations are
retained as `.previous-*` after repair; integrate idle-only cleanup/space budgeting.

Package packaging: build `cargo build --locked --release -p blockforge-bedrock-auth`
and install the helper alongside BlockForge. Only that binary links GTK3,
WebKitGTK 4.1, JavaScriptCoreGTK 4.1 and libsoup3. Build inputs include their devel
packages and protoc; root acquisition uses no WebKit. `--self-test` opens a short
blank-page WebKit window, waits for page load, exits without any sign-in/network.

IPC schema is `ipc.rs`, compiled verbatim into both packages: a big-endian u32
length plus JSON `{version:2,message:{type:...}}`, maximum 128 KiB (version 2 added the Xbox step, below). Parent creates
an unpredictable 0700 socket directory in /dev/shm with 0600 Unix socket; verifies
child PID and UID. Only the socket path is in argv. Helper verifies socket owner
and mode. Parent sends Open/Complete; helper sends Credentials/Cancelled. No
payload implements Debug. All helper stdout/stderr is discarded by the parent;
errors exposed to UI contain no raw responses. The helper confines navigation to
exact Microsoft HTTPS login hosts, and DA extraction to the post.srf document.
Second-leg headers and host context match the first leg. Parent does all broker
HTTP exchanges while GTK stays responsive. Close and disconnect never yield
successful login. Only complete issued credentials replace the prior account.

Navigation (main frame and subframes alike, as WebKitGTK asks one policy for both)
is limited to HTTPS on live.com, microsoft.com, microsoftonline.com, msauth.net,
msftauth.net and the two human-verification providers Microsoft frames
(arkoselabs.com, hsprotect.net), plus about:blank/srcdoc. Pages the parent may open,
and pages whose IPC is heard, stay the exact login hosts; DA credentials are accepted
only from login.live.com pages, and the helper forwards just the seven DAProperty
string fields of ServerData (the full object can exceed a frame). If a real sign-in
stalls on a page outside that list, add its domain to `ipc::allowed_navigation`.

Device registration writes a `provisioning` marker before the request. If the
request provably never left (no connection, or it could not be built) the marker is
removed so an offline first sign-in can simply be retried; any later failure keeps it
and asks for explicit repair (sign-out UserAndDevice), since a device may exist.

## One Microsoft sign-in (Store and Xbox in one window)

Bedrock needs two tokens from two Microsoft apps: the Store's DA/Passport tokens (client `000000004424da1f`, the
hosted InlineLogin above) and WineGDK's XUser refresh token (client `0000000048183522`, scope
`service::user.auth.xboxlive.com::MBI_SSL`, the token `xbox/` keeps). BlockForge's Java/Horizon account
(`src/msauth.rs`) belongs to a third app (Azure client `90890812-…`, `XboxLive.signin` on
login.microsoftonline.com); its refresh token can't mint either, so it is not reused. What is shared is the browser
session: after the Store's sign-in the window is still signed in to login.live.com, so it goes on to

```
https://login.live.com/oauth20_authorize.srf?client_id=0000000048183522
  &scope=service%3A%3Auser.auth.xboxlive.com%3A%3AMBI_SSL&response_type=code
  &redirect_uri=https%3A%2F%2Flogin.live.com%2Foauth20_desktop.srf&state=<64 random hex>[&login_hint=<address>]
```

and reads the code off the redirect to `https://login.live.com/oauth20_desktop.srf` (`ipc::xbox_capture`: exact
scheme, host and path, the `state` it was given, the bare OAuth error code on refusal; the redirect is never loaded
and the code is never logged). The parent exchanges it at `oauth20_token.srf` (`grant_type=authorization_code`, same
`redirect_uri` and scope) in `xbox::sign_in_with_code`, which keeps the refresh token exactly like the device code
(same file, account epoch, then the preauth chain). There is no `prompt=none`: when the app was allowed before the
page redirects at once, otherwise Microsoft shows its "allow" page in the same window. Whatever goes wrong there (no
session, a refusal, the window closed, ten minutes without an answer, a refused exchange) is not an error: the
facade starts the device code for that profile, as `AccountTarget::Xbox` does.

IPC version 2 (both binaries are built from the same `ipc.rs`; a version 1 helper is refused): parent `Open` (the
first must be `ipc::login_url(hint)`: `LOGIN` alone or with `&username=<percent-encoded address>`),
`XboxAuthorize { state, login_hint }` (may come first, for an Xbox-only window when the Store is already signed
in), `Complete`; helper `Credentials`, `Cancelled`, `XboxCode { code, state }`, `XboxUnavailable { error }`. The
helper builds the authorisation URL itself from constants, so the parent can't point it anywhere else, and loads it
without the hosted-login headers. Navigation rules are unchanged: login.live.com (and so its `oauth20_*` pages) was
already allowed; `Credentials` seen during the Xbox step are ignored by the parent.

`Backend::sign_in(AccountTarget::Store(id))` (and `sign_in_as`, with the BlockForge account's address as the hint)
runs one window for the Store and one code per game profile on that slot without an Xbox sign-in; a slot already
signed in gets an Xbox-only window. `Backend::sign_out(AccountTarget::Store(id), User)` signs out each of those
profiles' Xbox (registry copy first, then the account) and then the Store user, keeping the device.
`AccountStatus::identity` carries the Store's sign-in name and the Xbox XUID/gamertag/user hash from local records;
the window compares them with the BlockForge account (`msauth` now also reads its XUID and gamertag, best effort,
from an XSTS token for `http://xboxlive.com`) and only says so when they differ.

### Run book: the real check (the user signs in; the agent never does)

```sh
. <the env of "Interactive Release scratch check" below>
cargo build --locked -p blockforge-bedrock-auth
export BF_BEDROCK_AUTH_HELPER=$CARGO_TARGET_DIR/debug/blockforge-bedrock-auth
export BF_BEDROCK_LOGIN_HINT=you@example.com      # optional: checks the prefilled name
cargo test --locked -p blockforge --lib bedrock::linux::tests::real_one_microsoft_sign_in -- --ignored --exact --nocapture
```

It keeps everything under `~/.cache/bf-bedrock-onelogin-real` (BF_BEDROCK_ONE_LOGIN_DIR), never BlockForge's data,
BedrockOnLinux or ~/.minecraft. What to look at, in order:

1. The window's first page has the address filled in (if a hint was set). Sign in to the Store as usual (MFA, the
   "press and hold" challenge). If the address is not filled in, `username=` is not honoured by InlineLogin: harmless,
   drop it from `ipc::login_url`.
2. Without signing in again, the window shows either nothing new (it closes by itself) or Microsoft's "Let this app
   access your info?" page for "Minecraft"/"Xbox": allow it in the window. The log must say `one window: the window
   gave an Xbox code`, `code exchange accepted`, `ONLINE, gamertag present`, `xuid present`.
   - `no Xbox code (state_mismatch)`, `(no_code)` or the window asking for the password again means the hosted
     InlineLogin does not leave a session the authorisation page accepts; BlockForge then falls back to the device
     code (which still works), and the window step needs another look (e.g. a non-hosted login page first).
   - `no Xbox code (timeout)` with a page stuck: note the page's address and add its host to
     `ipc::allowed_navigation` if it is outside it.
   - `code exchange REFUSED`: the `redirect_uri` or scope sent by `sign_in_with_code` is not what Microsoft
     expects for this app; compare with `ipc::xbox_authorize_url`.
3. A second window opens for Xbox only (the Store stays signed in): sign in there and allow; the same lines with
   `xbox-only window:` must appear. (Its webview is fresh, so it asks for the account again.)
4. In BlockForge itself: an instance page's Microsoft account row says "Signed in as <address>", "Xbox Live:
   <gamertag>", and, when the Java account is the same person, "The same account as BlockForge's …"; Play starts
   online. Sign out clears both rows; the Store device stays (no new device registration on the next sign-in).

## Download notes

The package CDN roots are plain `http://` and their hosts have no matching TLS
certificate, so the URL is used as the catalogue gives it (upgrading to https made
every real download fail). Xodus does not check pages against the XVD hash tree;
see third_party/xodus/PATCHES.md "Known limit". Treat that as a release-gate item.

Re-running `install(Latest)` on an unchanged build re-reads the package header,
hashes each installed file once and, if every file of that build is intact, returns
the existing generation: no licence request, no copy, `downloaded_files == 0`.
Otherwise it builds a new generation, copying intact files and fetching the rest;
if publishing the new generation fails, the old one is renamed back.

## Interactive Release scratch check (integrator and user only)

The agent never performs this check or signs in. It deliberately remains ignored.
It uses only `$HOME/.cache/bf-bedrock-pkga/real-store` and a fixed Store slot there;
rerun preserves the device and never reads any BOL/Java/other instance roots.

Build inputs: `protoc` and the WebKitGTK 4.1 / GTK 3 / libsoup 3 development files
(Fedora: `sudo dnf install protobuf-compiler webkit2gtk4.1-devel`). Package A was
built without root by unpacking those RPMs into `$HOME/.cache/bf-tmp-pkga/sysroot` and
setting `PROTOC=$S/usr/bin/protoc PKG_CONFIG_PATH=$S/usr/lib64/pkgconfig:$S/usr/share/pkgconfig
LIBRARY_PATH=$S/usr/lib64`; with the packages installed none of that is needed.

```sh
export PATH=$HOME/.cargo/bin:$PATH
export TMPDIR=$HOME/.cache/bf-tmp-pkga
export CARGO_TARGET_DIR=$HOME/.cache/bf-target-pkga
export CARGO_BUILD_JOBS=3 CARGO_INCREMENTAL=0
mkdir -p "$TMPDIR"
cargo build --locked -p blockforge-bedrock-auth
export BF_BEDROCK_AUTH_HELPER=$CARGO_TARGET_DIR/debug/blockforge-bedrock-auth
"$BF_BEDROCK_AUTH_HELPER" --self-test
cargo test --locked -p blockforge bedrock::store::tests::real_store_release_scratch -- --ignored --exact --nocapture
```

User completes the owning Store account's hosted sign-in/MFA themselves. The test
installs latest owned Release, drops/reopens the build lease, resolves/reinstalls
it and prints actual fetched-file count. Hash-bearing unchanged files must be
reused; hashless entries remain conservative downloads. It reacquires a license,
stages in verified private tmpfs, checks only synthetic-format MZ/PE header fields,
drops the plaintext images, and compares a nonprinted SHA-256 of the device entry
before/after slot restart. The test opens the helper a second time for another
user-completed sign-in and verifies that the device identity is still unchanged.
Preview is optional and was not tested. No game files, keys, tokens or licenses
are committed. Delete the scratch root only when intentionally retiring that test
device; do not turn deletion/reprovisioning into an ordinary retry action.

## Measured cost (2026-11-01, Rust 1.99.1, 3 jobs, release profile: thin LTO)

| Build | Wall time | Result |
| --- | --- | --- |
| `blockforge`, `--no-default-features --features converter` (Bedrock off), from clean | 586 s | 39,078,680 B |
| then `blockforge` with default `bedrock` (shared deps reused) | +654 s | 39,074,704 B |
| then `-p blockforge-bedrock-auth` | +100 s | 1,165,280 B (already stripped) |

Linux crates resolved for `blockforge` (`cargo tree -e normal,build`): 351 without
Bedrock, 521 with it; the helper resolves 145. The lock grows from 565 to 870
entries, most of them other-platform or optional edges: on Linux `blockforge`
pulls in no OpenSSL, native-tls, D-Bus, WebKit or GTK (`cargo tree -i`), and
`ldd` shows only the helper linking libwebkit2gtk-4.1, libgtk-3, libsoup-3.0 and
libjavascriptcoregtk-4.1. Windows and macOS trees contain no Xodus or wry crates.
The binary did not grow because nothing calls `store` until the facade is
wired up and LTO drops it; re-measure after integration. Of the extra 654 s,
the `blockforge` crate itself is ~394 s of unit time (recompiled for the
feature), `xodus` 37 s, `msixvc` 8 s; the rest is feature-unified dependencies
recompiled (`cargo-timing` reports, 341 units).
