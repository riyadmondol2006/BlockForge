# Bedrock native runtime helper

This is the static inner executable for BlockForge's Linux Bedrock backend. The
launcher calls the pinned SteamRT `pressure-vessel-wrap` ELF directly with an
argv array. The container executes this helper, which reads a private 0600 plan,
checks the fonts inside the container, builds an explicit Wine environment and
replaces itself with native `files/bin-wow64/wine` or `wineserver`. It never
executes the engine's `proton`, `umu-run`, Python, protonfixes or BOL.

Build with Rust 1.98 or newer:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --locked --release -p bedrock-runtime --target x86_64-unknown-linux-musl
readelf -l target/x86_64-unknown-linux-musl/release/blockforge-bedrock-runtime
```

There must be no `INTERP` header (BlockForge refuses a dynamically linked helper). The crate is a member of the root
workspace and uses its `Cargo.lock`. Package `blockforge-bedrock-runtime` beside the `blockforge` program; BlockForge
copies it once per build of it into `bedrock/helpers/<sha256>/` in its data folder, because the container has its own
`/usr` and cannot see a copy in `/usr/bin` or inside an AppImage. `BF_BEDROCK_RUNTIME_HELPER` names another one while
developing. Do not ask end users to build it. The library shares `InnerPlan` and `CommandSpec` with the launcher.

How BlockForge uses it (everything below runs on tokio workers, never the window's thread; `base` is
`Dirs.base/bedrock`):

1. `engine::ensure(base, client, progress, cancel, Check::Quick)` and `runtime::ensure(...)`. A warm call reads a
   receipt and a few small files: no archive, no full hash. Cold, they download the pinned archive (exact size and
   SHA-256, through a `.part` file) into `cache/artifacts`, install it read-only under `engine/<revision>/` or
   `runtime/steamrt3/<build>/` with a rollback copy, write the receipt, delete the archive and prune older
   revisions nothing holds. `Check::Full` (Repair) rehashes everything against the reviewed engine manifest and
   reruns `pv-verify`, and rebuilds the runtime's private view. Blocking variants: `engine::open/install`,
   `runtime::open/install`.
2. Hold `Engine::lease()` and `Runtime::lease()` for the whole session, plus the prefix's own idle lease (packages
   C/E). Lock files live outside the replaceable trees and are never unlinked. Installs and pruning get Busy while
   any lease is held, and also while any process still runs from or maps a tree (a crashed supervisor's orphans).
3. `proton::prepare(&engine, prefix, nvapi)` with the prefix idle: copies the engine's default prefix on first use
   (fresh MachineGuid), materialises builtin DLLs as owned copies, installs DXVK/vkd3d-proton, the GDK payload
   (xgameruntime.dll and its `.threading` sidecar) and cryptbase, maps `c:`/`z:`, links ICU and every font from
   both engine font folders, and stamps `.update-timestamp`. User files and deliberately replaced native DLLs are
   kept. Warm calls only check a revision stamp; `proton::invalidate` forces the full pass (Repair).
4. Build an `InnerPlan`: the engine root, absolute owned paths without `:`, the packaged helper's absolute path,
   the profile UUID as `app_id`, allowed env only (`bedrock_runtime::allowed_env`), `Program::Wineboot` with
   `headless: true` for setup. `runtime::write_plan` validates it against these installs and writes it create-new,
   0600. Only the plan's path appears in argv.
5. `runtime::command(&runtime, &engine, &plan)?` gives the outer `CommandSpec` (program, argv, env, env removals,
   cwd; never a shell string); `.command()` builds a `std::process::Command` with a cleared environment. Then a
   second plan with `Program::WineserverWait` waits for the prefix to go idle; only after it returns may the
   registry hives be edited offline. Play is `Program::Wine`, `headless: false`, with the game's NT path as the
   first argument. Delete each plan after its process ends.

Environment inheritance is cleared at both exec boundaries. Supply the actual
user HOME, locale, display/session/audio and GPU values through the allowlist.
The plan supports `WINE_DLL_FILE_MAP`, `WINEGDK_PREAUTH_DEVICE` and sanitized
`WINEGDK_XSTS_RP_*`; package D must validate preauth identity/expiry first. Do not
put refresh tokens in argv. GNU TLS system-priority overrides, WINEARCH and
loader injection variables are removed. The inner helper constructs loader,
GStreamer, sync, graphics and driver values itself. Set ntsync only after
checking /dev/ntsync availability on a worker; esync/fsync are not substitutes.
NVAPI is opt-in after supported NVIDIA selection, never automatic on Mesa.

The stable app identity is the full BF profile UUID (not the Xbox title ID and
not a truncated hash). This deliberately replaces UMU's MD5-derived identity.
No unrelated Steam/VR/client integration, user_settings.py or protonfixes runs.
The required `force_raw_va_cbv` vkd3d setting is explicit.

## Validation

From the repository with the brief's TMPDIR/CARGO_TARGET_DIR settings:

```sh
cargo test --locked bedrock::
BF_BEDROCK_RUNTIME_HELPER=/absolute/path/to/static/blockforge-bedrock-runtime \
BF_STRACE=/absolute/path/to/strace \
cargo test --locked real_native_wineboot -- --ignored --nocapture
```

The ignored test needs no account or game. It installs only under
`~/.cache/bf-bedrock-pkgb/bedrock` (BF's own layout), reusing the research archives after hashing, and starts
from a fresh prefix every run. Small
logs go to `~/bf-ui/bedrock/evidence/pkgb`. Remove the owned scratch root and
build target when finished. A Wineboot pass proves prefix bootstrap and this
native chain, not live-game networking, Xbox login or playability. Live-game
execution remains an integration gate requiring the owning user's sign-in.

## Sources and licensing

Prefix/environment algorithms are ported from BedrockOnLinux contributors,
MIT, commit cc6ac3235bfbbf9d05fe6870b09ef71e7f1a4f52, bol/prefix.py,
bol/winegdk.py and bol/vkd3d.py; complete MIT notice in LICENSE-BOL. The native21
engine's Proton script (Valve Software, GPL-3.0) supplies the documented redist,
font, DLL, WoW64 and init_wine behavior; it is read as source and never executed.
UMU 1.4.3 (Open Wine Components, GPL-3.0, commit
1031d247969a5e8417df378cf175faf6906aec2d) supplies runtime environment semantics.
SteamRT/pressure-vessel and the external engine retain their separate licences
and provenance in their downloaded trees. This crate is GPL-3.0-or-later.

Prefix builtins are materialized as writable owned copies before Wineboot.
An early smoke test showed Wineboot updating some engine files through builtin
links; the materialization and read-only artifact trees prevent that. Internal
template links point into the new prefix, never back into the template. Font
and ICU links remain readable engine data. A crashed supervisor is also covered
by an install-time /proc scan of native executable mappings and tool/loader
paths, so losing a session lease alone does not authorize replacement. Package E
must still hold leases and supervise the complete prefix lifetime.

Set `InnerPlan.mounts` to every staged executable or other payload outside normal
HOME visibility, especially private tmpfs paths. These are explicit container
mounts; the runtime never tries to infer staging paths from credential-bearing
environment values.

Completed prefix setup is cached by the pinned engine archive/revision and
NVAPI policy. Warm setup performs only directory/identity checks and does not
recopy or rehash all builtin DLLs. Idle repair must call `proton::invalidate`
before `prepare`; changing the engine pin automatically invalidates this cache.
Materialized prefix payloads use more disk space than the reference's builtin
links, in exchange for preventing Wineboot writes into shared engine files.

SteamRT's native wrapper opens `files/.ref` read-write even when copying its
runtime. `runtime::install`/`ensure` therefore build one verified private copy beside the read-only runtime,
`runtime/steamrt3/.<build>.view`, with its own writable `.ref`, shared by every profile; each profile's
pressure-vessel variable dir hard-links from it. The downloaded runtime tree stays read-only.

The private view includes pressure-vessel's native tool directories too. Its
copy routine imports their modes into `from-host`; using read-only directories
there prevented it from constructing the host-library tree during a smoke test.
The helper checks `/usr/lib/os-release` for the exact pinned SteamRT build and
refuses execution on the host. It retains only pressure-vessel's explicit
rewritten driver, loader and session paths, prepending engine library paths;
clearing these rewritten values would break container graphics routing.
