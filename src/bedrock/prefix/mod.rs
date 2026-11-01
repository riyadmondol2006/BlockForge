//! The Wine prefix of a Bedrock profile: what has to be true of it before the game starts, and the offline
//! repairs that keep it so. Package C of PORT-PLAN.md.
//!
//! Ported from BedrockOnLinux `bol/prefix.py`, `bol/gamesetup.py` and `bol/auth.py` (registry parts), MIT, with
//! credit. Everything here works on a *stopped* prefix: it edits files Wine will read the next time it starts.
//! Nothing here starts Wine, so the engine's wineboot (package B) runs between [`seed_bootstrap_cryptbase`] and
//! [`finish_setup`]:
//!
//! 1. `seed_bootstrap_cryptbase`: the native RNG stub goes into system32 *before* wineboot, because every wineboot
//!    service aborts on its first RtlGenRandom call without a `cryptbase.dll` to forward to.
//! 2. package B: headless wineboot, then `wineserver -w` so the prefix is idle.
//! 3. `finish_setup`: refresh system DLLs if the engine changed, apply the registry values, install GameInput,
//!    and record the engine revision only when all of that worked.
//!
//! Every function is blocking and does file work: call them from `spawn_blocking`, never from the window.

pub mod cab;
pub mod cfb;
pub mod gameinput;
pub mod pe;
pub mod registry;

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{BedrockError, ErrorKind};
pub use registry::{RegChange, update_prefix_registry};

// ---------------------------------------------------------------- errors

/// Why a prefix operation did not happen. Messages are for the player: they say what to do, and never carry a
/// registry value or token.
#[derive(Debug)]
pub enum PrefixError {
    /// Wine processes still use the prefix.
    Busy(String),
    /// A file is not what it should be (not a hive, cut off, not an installer).
    Malformed(String),
    /// A path in the prefix would lead out of it, or is a link where a folder must be.
    Unsafe(String),
    Invalid(String),
    Integrity(String),
    NoSpace(String),
    /// Something the player has to agree to first (a world about to meet an older game version).
    NeedsConfirmation(String),
    Io {
        context: String,
        source: io::Error,
    },
}

impl PrefixError {
    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        if source.raw_os_error() == Some(libc::ENOSPC) {
            return PrefixError::NoSpace(format!("{}: the disk is full", context.into()));
        }
        PrefixError::Io { context: context.into(), source }
    }
}

impl fmt::Display for PrefixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrefixError::Busy(m)
            | PrefixError::Malformed(m)
            | PrefixError::Unsafe(m)
            | PrefixError::Invalid(m)
            | PrefixError::Integrity(m)
            | PrefixError::NoSpace(m)
            | PrefixError::NeedsConfirmation(m) => f.write_str(m),
            PrefixError::Io { context, source } => write!(f, "Could not {context}: {source}"),
        }
    }
}

impl std::error::Error for PrefixError {}

impl From<PrefixError> for BedrockError {
    fn from(e: PrefixError) -> Self {
        let kind = match &e {
            PrefixError::Busy(_) => ErrorKind::Busy,
            PrefixError::Integrity(_) => ErrorKind::Integrity,
            PrefixError::NoSpace(_) => ErrorKind::NoSpace,
            PrefixError::NeedsConfirmation(_) => ErrorKind::InvalidRequest,
            _ => ErrorKind::Prefix,
        };
        BedrockError::new(kind, e.to_string())
    }
}

// ---------------------------------------------------------------- is the prefix there, is it idle

/// Whether Wine completed the prefix: both main hives with Wine's header, and system32.
pub fn prefix_ready(prefix: &Path) -> bool {
    if !prefix.join("drive_c/windows/system32").is_dir() {
        return false;
    }
    ["system.reg", "user.reg"].iter().all(|name| {
        let mut head = [0u8; 22];
        File::open(prefix.join(name)).and_then(|mut f| f.read_exact(&mut head)).is_ok()
            && head.starts_with(b"WINE REGISTRY Version")
    })
}

/// Live processes whose environment names this prefix (an exact `WINEPREFIX=` entry, compared by path so a
/// trailing slash or a link does not hide one). Processes of other users cannot be read and are skipped; the
/// whole scan failing is an error, because "could not look" must not read as "idle".
pub fn prefix_processes(prefix: &Path) -> Result<Vec<u32>, PrefixError> {
    let wanted = fs::canonicalize(prefix).unwrap_or_else(|_| prefix.to_path_buf());
    let me = std::process::id();
    let mut found = Vec::new();
    let dir = fs::read_dir("/proc").map_err(|e| PrefixError::io("look for Wine processes", e))?;
    for entry in dir.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == me {
            continue;
        }
        let Ok(environ) = fs::read(entry.path().join("environ")) else { continue };
        for var in environ.split(|&b| b == 0) {
            let Some(value) = var.strip_prefix(b"WINEPREFIX=") else { continue };
            use std::os::unix::ffi::OsStrExt;
            let path = Path::new(std::ffi::OsStr::from_bytes(value));
            if path == prefix || path == wanted || fs::canonicalize(path).is_ok_and(|p| p == wanted) {
                found.push(pid);
                break;
            }
        }
    }
    found.sort_unstable();
    Ok(found)
}

/// Fail before an offline change while a wineserver may still own the hives (it would later overwrite the
/// change with its in-memory copy).
pub fn require_prefix_idle(prefix: &Path, action: &str) -> Result<(), PrefixError> {
    let live = prefix_processes(prefix)?;
    if live.is_empty() {
        return Ok(());
    }
    Err(PrefixError::Busy(format!(
        "Cannot {action}: {} Wine process(es) still use this prefix. Close Minecraft or stop it first.",
        live.len()
    )))
}

// ---------------------------------------------------------------- files, links and folders

/// `rel` below `root` as an existing, real folder: made if missing, and refused if any step is a link or
/// something that is not a folder. Nothing the prefix holds can lead a write outside it.
pub fn safe_dir(root: &Path, rel: &str) -> Result<PathBuf, PrefixError> {
    let mut current = root.to_path_buf();
    let root_meta = fs::symlink_metadata(root).map_err(|e| PrefixError::io(format!("read {}", root.display()), e))?;
    if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
        return Err(PrefixError::Unsafe(
            "The Wine prefix is a link or not a folder; repair the Bedrock install to rebuild it.".into(),
        ));
    }
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        if part == ".." || part == "." {
            return Err(PrefixError::Unsafe("A path in the prefix is not allowed.".into()));
        }
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(PrefixError::Unsafe(format!(
                    "The Wine prefix has a link where a folder belongs ({}); repair the Bedrock install to rebuild it.",
                    current.display()
                )));
            }
            Ok(m) if !m.is_dir() => {
                return Err(PrefixError::Unsafe(format!("{} should be a folder.", current.display())));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|e| PrefixError::io(format!("create {}", current.display()), e))?;
            }
            Err(e) => return Err(PrefixError::io(format!("read {}", current.display()), e)),
        }
    }
    Ok(current)
}

fn temp_beside(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dir.join(format!(".{name}.bf-{}", uuid::Uuid::new_v4().simple()))
}

/// Write `bytes` to `path` so a reader sees the old file or the new one, never half of it. A link at `path` is
/// replaced, not followed.
pub fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), PrefixError> {
    let staged = temp_beside(path);
    let result = (|| {
        let mut f = OpenOptions::new().write(true).create_new(true).mode(mode).open(&staged)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::set_permissions(&staged, fs::Permissions::from_mode(mode))?;
        fs::rename(&staged, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&staged);
        return Err(PrefixError::io(format!("write {}", path.display()), e));
    }
    Ok(())
}

pub fn sha256_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Whether the regular file at `target` has the same bytes as `source`. Sizes are compared first, so most
/// differing files cost a `stat`, not a hash.
fn same_bytes(source: &Path, target: &Path) -> bool {
    let (Ok(a), Ok(b)) = (fs::metadata(source), fs::symlink_metadata(target)) else { return false };
    if !b.file_type().is_file() || a.len() != b.len() {
        return false;
    }
    matches!((sha256_file(source), sha256_file(target)), (Ok(x), Ok(y)) if x == y)
}

/// Copy `source` over `target` through a staged file whose hash is checked before it takes the target's place.
/// A link at `target` is replaced, never followed.
fn copy_verified(source: &Path, target: &Path) -> Result<(), PrefixError> {
    let staged = temp_beside(target);
    let result = (|| {
        let mode = fs::metadata(source)?.permissions().mode() & 0o777;
        fs::copy(source, &staged)?;
        fs::set_permissions(&staged, fs::Permissions::from_mode(mode | 0o600))?;
        File::open(&staged)?.sync_all()?;
        if sha256_file(&staged)? != sha256_file(source)? {
            return Err(io::Error::other("the copy does not match its source"));
        }
        fs::rename(&staged, target)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&staged);
        return Err(PrefixError::io(
            format!("copy {} into the prefix", target.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()),
            e,
        ));
    }
    Ok(())
}

/// Free bytes on the filesystem holding `path` (or the nearest folder above it).
pub fn free_bytes(path: &Path) -> io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let mut probe = path;
    while !probe.exists() {
        probe = probe.parent().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    }
    let c = std::ffi::CString::new(probe.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` is a properly sized, writable statvfs.
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
}

// ---------------------------------------------------------------- cryptbase (the RNG stub)

/// Put `source` (the native `cryptbase.dll` RNG stub, or the engine's own builtin one as a stopgap) into the
/// prefix's system32, keeping the original once.
///
/// Wine resolves advapi32's `SystemFunction036` (RtlGenRandom) through cryptbase from system32, not from the
/// game folder, and the OpenSSL XCurl aborts at its first TLS random-number call without it. The first
/// `cryptbase` in a prefix is the builtin placeholder; a `n,b` override only falls back to a real file.
pub fn install_cryptbase(prefix: &Path, source: &Path) -> Result<bool, PrefixError> {
    let meta = fs::symlink_metadata(source).map_err(|e| PrefixError::io("read the cryptbase stub", e))?;
    if !meta.file_type().is_file() {
        return Err(PrefixError::Unsafe("The RNG stub is not a plain file.".into()));
    }
    let sys32 = safe_dir(prefix, "drive_c/windows/system32")?;
    let target = sys32.join("cryptbase.dll");
    if same_bytes(source, &target) {
        return Ok(false);
    }
    let backup = sys32.join("cryptbase.dll.bf-orig");
    if fs::symlink_metadata(&backup).is_err() {
        match fs::symlink_metadata(&target) {
            Ok(m) if m.file_type().is_symlink() => {
                let link = fs::read_link(&target).map_err(|e| PrefixError::io("read the old cryptbase link", e))?;
                std::os::unix::fs::symlink(link, &backup).map_err(|e| PrefixError::io("keep the old cryptbase", e))?;
            }
            Ok(m) if m.is_file() => {
                fs::copy(&target, &backup).map_err(|e| PrefixError::io("keep the old cryptbase", e))?;
            }
            Ok(_) => return Err(PrefixError::Unsafe("cryptbase.dll in the prefix is not a plain file.".into())),
            Err(_) => {}
        }
    }
    copy_verified(source, &target)?;
    Ok(true)
}

/// Before the first wineboot: make sure system32 exists and holds a real `cryptbase.dll`. `native` is the
/// verified RNG stub from the OpenSSL XCurl set; `builtin` is the engine's own, used only when the set is not
/// there yet (enough to boot a window; the TLS shim is separate and unaffected).
pub fn seed_bootstrap_cryptbase(prefix: &Path, native: Option<&Path>, builtin: &Path) -> Result<bool, PrefixError> {
    fs::create_dir_all(prefix).map_err(|e| PrefixError::io("create the Wine prefix", e))?;
    safe_dir(prefix, "drive_c/windows/system32")?;
    let source = match native {
        Some(n) if fs::symlink_metadata(n).is_ok_and(|m| m.file_type().is_file()) => n,
        _ => builtin,
    };
    install_cryptbase(prefix, source)
}

/// The line wineboot prints when it has no `cryptbase.dll` to forward RtlGenRandom to. The caller (which runs
/// wineboot) uses this to decide on the one reseed-and-retry the plan allows.
pub fn log_shows_rng_abort(log: &str) -> bool {
    let lower = log.to_ascii_lowercase();
    lower.contains("unimplemented function advapi32.dll.systemfunction036")
        || ((lower.contains("module not found for forward") || lower.contains("function not found for forward"))
            && lower.contains("'cryptbase.systemfunction036'"))
}

// ---------------------------------------------------------------- refreshing system DLLs from the engine

/// Engine DLL folder -> the prefix system folder it populates.
const ENGINE_DLL_DIRS: [(&str, &str); 2] = [
    ("files/lib/wine/x86_64-windows", "drive_c/windows/system32"),
    ("files/lib/wine/i386-windows", "drive_c/windows/syswow64"),
];
const ENGINE_REV_MARKER: &str = ".bf-engine-rev";
const RUNTIME_BACKUP_SUFFIX: &str = ".bf-runtime-backup";
// Wine marks every DLL it builds right after the DOS header; the copies wineboot puts in a prefix keep that mark.
const WINE_DLL_MARKS: [&[u8]; 2] = [b"Wine builtin DLL", b"Wine placeholder DLL"];

pub fn read_engine_rev(prefix: &Path) -> Option<String> {
    fs::read_to_string(prefix.join(ENGINE_REV_MARKER)).ok().map(|s| s.trim().to_string())
}

/// A prefix built by another engine revision (or before the marker existed) has stale system DLLs.
pub fn engine_is_stale(prefix: &Path, rev: &str) -> bool {
    read_engine_rev(prefix).as_deref() != Some(rev)
}

/// Write the revision marker. Only after a refresh worked: a half-refreshed prefix must read as stale.
pub fn record_engine_rev(prefix: &Path, rev: &str) -> Result<(), PrefixError> {
    write_atomic(&prefix.join(ENGINE_REV_MARKER), format!("{rev}\n").as_bytes(), 0o644)
}

fn is_wine_built_dll(path: &Path) -> bool {
    let mut head = Vec::with_capacity(0x40 + 32);
    File::open(path).and_then(|f| f.take(0x40 + 32).read_to_end(&mut head)).is_ok()
        && head.get(0x40..).is_some_and(|rest| WINE_DLL_MARKS.iter().any(|m| rest.starts_with(m)))
}

/// Refresh one DLL of the prefix from the engine, keeping a copy of the old one only when it did not come from
/// an engine (a DXVK copy, a redistributable, a DLL the player put there) and so cannot be had back.
fn replace_dll(source: &Path, target: &Path, backup_suffix: &str) -> Result<bool, PrefixError> {
    if same_bytes(source, target) {
        return Ok(false);
    }
    let mut backup = target.as_os_str().to_owned();
    backup.push(backup_suffix);
    let backup = PathBuf::from(backup);
    if fs::symlink_metadata(&backup).is_err() && !is_wine_built_dll(target) {
        match fs::symlink_metadata(target) {
            Ok(m) if m.file_type().is_symlink() => {
                let link = fs::read_link(target).map_err(|e| PrefixError::io("read a DLL link", e))?;
                std::os::unix::fs::symlink(link, &backup).map_err(|e| PrefixError::io("keep an old DLL", e))?;
            }
            Ok(m) if m.is_file() => {
                fs::copy(target, &backup).map_err(|e| PrefixError::io("keep an old DLL", e))?;
            }
            Ok(_) => {
                return Err(PrefixError::Unsafe(format!(
                    "The Wine prefix has a runtime file that is not a plain file: {}",
                    target.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
                )));
            }
            Err(_) => {}
        }
    }
    copy_verified(source, target)?;
    Ok(true)
}

/// A system folder of the prefix that is a real folder inside it, or None when the prefix does not have it.
fn system_dir(prefix: &Path, rel: &str) -> Result<Option<PathBuf>, PrefixError> {
    let dir = prefix.join(rel);
    match fs::symlink_metadata(&dir) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(PrefixError::Unsafe(
                "The Wine prefix has a link where a system folder belongs; repair the Bedrock install to rebuild it."
                    .into(),
            ));
        }
        Ok(m) if m.is_dir() => {}
        _ => return Ok(None),
    }
    let root = fs::canonicalize(prefix).map_err(|e| PrefixError::io("read the Wine prefix", e))?;
    let real = fs::canonicalize(&dir).map_err(|e| PrefixError::io("read the Wine prefix", e))?;
    if !real.starts_with(&root) {
        return Err(PrefixError::Unsafe(
            "The Wine prefix has a system folder that leads out of it; repair the Bedrock install to rebuild it."
                .into(),
        ));
    }
    Ok(Some(dir))
}

/// Refresh the prefix's system DLLs from the engine after an engine revision change.
///
/// The engine swaps as a whole but the prefix is reused, and its cached system32/syswow64 DLLs were put there by
/// the previous engine: running the new runtime against them faults the account and menu path with an unhandled
/// page fault. Only DLLs the prefix *already has* are replaced, and only when they differ; a DLL the prefix
/// lacks stays absent, because Wine loads the rest straight from the engine folder and injecting engine-only
/// DLLs would disturb that. Nothing under `drive_c/users` (worlds, settings, sign-in) is touched.
/// Returns how many files changed.
pub fn refresh_engine_dlls(prefix: &Path, engine_root: &Path) -> Result<usize, PrefixError> {
    let engine_real = fs::canonicalize(engine_root).map_err(|e| PrefixError::io("read the engine", e))?;
    let mut plan: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (engine_rel, prefix_rel) in ENGINE_DLL_DIRS {
        let src_dir = engine_root.join(engine_rel);
        if !src_dir.is_dir() {
            continue;
        }
        let Some(dst_dir) = system_dir(prefix, prefix_rel)? else { continue };
        let mut sources: Vec<PathBuf> = fs::read_dir(&src_dir)
            .map_err(|e| PrefixError::io("read the engine's DLLs", e))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dll")))
            .collect();
        sources.sort();
        for source in sources {
            let target = dst_dir.join(source.file_name().expect("has a name"));
            if fs::symlink_metadata(&target).is_err() {
                continue;
            }
            // An engine alias is a link inside the engine; anything pointing outside it is not trusted.
            match fs::canonicalize(&source) {
                Ok(real) if real.starts_with(&engine_real) && real.is_file() => {}
                _ => continue,
            }
            if !same_bytes(&source, &target) {
                plan.push((source, target));
            }
        }
    }
    if plan.is_empty() {
        return Ok(0);
    }
    require_prefix_idle(prefix, "refresh the Wine runtime")?;
    let mut changed = 0;
    for (source, target) in plan {
        changed += replace_dll(&source, &target, RUNTIME_BACKUP_SUFFIX)? as usize;
    }
    Ok(changed)
}

/// Put the 64-bit `user32.dll` of the verified engine back into the prefix when it differs. Returns whether it
/// was replaced.
pub fn repair_user32(prefix: &Path, engine_root: &Path) -> Result<bool, PrefixError> {
    let source = engine_root.join("files/lib/wine/x86_64-windows/user32.dll");
    let meta = fs::symlink_metadata(&source).ok();
    if !meta.is_some_and(|m| m.file_type().is_file()) {
        return Err(PrefixError::Integrity(
            "The verified engine has no usable 64-bit user32.dll; reinstall the engine.".into(),
        ));
    }
    let Some(sys32) = system_dir(prefix, "drive_c/windows/system32")? else {
        return Err(PrefixError::Invalid("The prefix has no system32 folder.".into()));
    };
    let target = sys32.join("user32.dll");
    if same_bytes(&source, &target) {
        return Ok(false);
    }
    require_prefix_idle(prefix, "repair the Wine runtime")?;
    replace_dll(&source, &target, ".bf-managed-backup")
}

/// Delete the copies of Wine-built DLLs that refreshes used to leave beside what they replaced (some 1,100 of
/// them, over a gigabyte). Copies of anything that did not come from an engine stay. Returns bytes freed.
pub fn prune_engine_dll_backups(prefix: &Path) -> u64 {
    let mut freed = 0;
    for (_, prefix_rel) in ENGINE_DLL_DIRS {
        let Ok(Some(dir)) = system_dir(prefix, prefix_rel) else { continue };
        let Ok(read) = fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            let path = entry.path();
            if !path.file_name().is_some_and(|n| n.to_string_lossy().ends_with(RUNTIME_BACKUP_SUFFIX)) {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&path) else { continue };
            if meta.file_type().is_file() && is_wine_built_dll(&path) && fs::remove_file(&path).is_ok() {
                freed += meta.len();
            }
        }
    }
    freed
}

// ---------------------------------------------------------------- the registry values the game needs

/// What `finish_setup` writes besides the refresh token. Only the Explorer virtual desktop is optional (it is
/// off unless the player asks, since it changes windowing for the whole prefix).
#[derive(Clone, Debug, Default)]
pub struct GameRegistry {
    pub virtual_desktop: Option<(u32, u32)>,
}

const WINEGDK_KEY: &str = r"Software\Wine\WineGDK";

/// Every registry value the game needs, ported from BedrockOnLinux `wine_apply_winegdk_prereqs`, as
/// (machine hive changes, user hive changes).
pub fn game_registry_changes(opts: &GameRegistry) -> (Vec<RegChange>, Vec<RegChange>) {
    let storage = r"Software\Microsoft\WindowsRuntime\ActivatableClassId";
    let picker = r"C:\windows\system32\windows.storage.dll";
    let machine = vec![
        // 8 is the console enum: the XSAPI path. 1 (Win32 PC) blocks the Servers tab as a "dev build".
        RegChange::dword(r"Software\Microsoft\Windows NT\CurrentVersion\OEM", "ConsoleMode", 8),
        // Wine registers a WinRT class only when its prefix is updated, and the engine's reproducible wine.inf
        // never looks newer, so an existing prefix would not learn about these. Every export (a structure, a
        // world, the album) asks for the save picker. Note the `Microsoft.` prefix of the open picker only.
        RegChange::sz(&format!(r"{storage}\Microsoft.Windows.Storage.Pickers.FileOpenPicker"), "DllPath", picker),
        RegChange::sz(&format!(r"{storage}\Windows.Storage.Pickers.FileSavePicker"), "DllPath", picker),
        // A build that asks for a newer Gaming Services than the engine's is refused at GDKC_InitAPI and the game
        // quits with exit 0 before its window opens (Preview 1.26.70 asks for 7849).
        RegChange::dword(r"Software\Microsoft\GamingServices", "IgnoreVersionMismatch", 1),
        // Azure rejects Wine GnuTLS' TLS 1.3 handshake; forcing TLS 1.2 lets the SISU/XSTS and PlayFab POSTs in.
        RegChange::dword(
            r"Software\Microsoft\Windows\CurrentVersion\Internet Settings\WinHttp",
            "DefaultSecureProtocols",
            2560,
        ),
        RegChange::dword(r"Software\Microsoft\SchannelTLS\Protocols\TLS 1.3\Client", "DisabledByDefault", 1),
    ];
    // pressure-vessel filters MICROSOFT_* out of the host environment, so the Windows App Runtime's UI-mute
    // switches have to live in the hive.
    let mut user: Vec<RegChange> = [
        "MICROSOFT_WINDOWSAPPRUNTIME_BOOTSTRAP_INITIALIZE_SHOWUI",
        "MICROSOFT_WINDOWSAPPRUNTIME_BOOTSTRAP_INITIALIZE_FAILFAST",
        "MICROSOFT_WINDOWSAPPRUNTIME_DEPLOYMENT_INITIALIZE_ONERRORSHOWUI",
    ]
    .into_iter()
    .map(|name| RegChange::sz("Environment", name, "0"))
    .collect();
    if let Some((w, h)) = opts.virtual_desktop {
        user.push(RegChange::sz(r"Software\Wine\Explorer", "Desktop", "Default"));
        user.push(RegChange::sz(r"Software\Wine\Explorer\Desktops", "Default", &format!("{w}x{h}")));
    }
    (machine, user)
}

/// Apply [`game_registry_changes`] to the stopped prefix. Returns whether anything changed.
pub fn apply_game_registry(prefix: &Path, opts: &GameRegistry) -> Result<bool, PrefixError> {
    let (machine, user) = game_registry_changes(opts);
    update_prefix_registry(prefix, &machine, &user)
}

/// Seed the Microsoft refresh token where WineGDK's XUser reads it. Writes the stopped prefix directly instead
/// of starting a Wine session just to run `reg.exe`. The token is never put in a message.
pub fn set_refresh_token(prefix: &Path, token: &str) -> Result<bool, PrefixError> {
    if token.is_empty() || token.contains('\0') {
        return Err(PrefixError::Invalid("The in-game sign-in token is not usable.".into()));
    }
    update_prefix_registry(prefix, &[RegChange::sz(WINEGDK_KEY, "RefreshToken", token)], &[])
}

/// Remove the token (sign-out). A prefix that was never booted cannot hold one and is already clean.
pub fn remove_refresh_token(prefix: &Path) -> Result<bool, PrefixError> {
    match fs::symlink_metadata(prefix.join("system.reg")) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(PrefixError::io("read system.reg", e)),
        Ok(_) => update_prefix_registry(prefix, &[RegChange::delete(WINEGDK_KEY, "RefreshToken")], &[]).map(|_| true),
    }
}

// ---------------------------------------------------------------- the whole setup after wineboot

/// Everything `finish_setup` needs, as plain paths: it knows nothing of the shared DTOs, so package B and the
/// facade can feed it whatever they hold.
#[derive(Clone, Debug)]
pub struct SetupInputs {
    /// `<profile>/compat/pfx`, stopped and booted (system.reg, user.reg, system32 exist).
    pub prefix: PathBuf,
    /// The verified engine root (holds `files/lib/wine/...`).
    pub engine_root: PathBuf,
    /// The engine's revision string, recorded in the prefix when setup completes.
    pub engine_rev: String,
    /// The game folder with `Installers/GameInputRedist.msi`.
    pub game_dir: PathBuf,
    pub registry: GameRegistry,
    /// The verified native RNG stub (`cryptbase.dll` from the OpenSSL XCurl set), for system32. None leaves whatever
    /// the bootstrap seeded.
    pub cryptbase: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct SetupReport {
    pub dlls_refreshed: usize,
    pub registry_changed: bool,
    pub gameinput: gameinput::Outcome,
    /// Things the player may need told (never an error): a missing installer, for one.
    pub notes: Vec<String>,
}

/// Run the post-wineboot setup in BOL's order. The engine revision is recorded only when every step worked, so
/// an interrupted or failed setup shows as stale and is simply run again.
pub fn finish_setup(inputs: &SetupInputs, cancelled: &dyn Fn() -> bool) -> Result<SetupReport, BedrockError> {
    let cancel = || -> Result<(), BedrockError> {
        if cancelled() { Err(BedrockError::new(ErrorKind::Cancelled, "Setup was cancelled.")) } else { Ok(()) }
    };
    let prefix = inputs.prefix.as_path();
    if !prefix_ready(prefix) {
        return Err(PrefixError::Invalid("The Wine prefix is incomplete (system.reg, user.reg or system32 is missing). Repair the Bedrock install to set it up again.".into()).into());
    }
    require_prefix_idle(prefix, "set up the Wine prefix")?;
    let mut notes = Vec::new();
    cancel()?;
    let mut dlls_refreshed = 0;
    if engine_is_stale(prefix, &inputs.engine_rev) {
        dlls_refreshed = refresh_engine_dlls(prefix, &inputs.engine_root)?;
        repair_user32(prefix, &inputs.engine_root)?;
    }
    prune_engine_dll_backups(prefix);
    if let Some(stub) = &inputs.cryptbase {
        install_cryptbase(prefix, stub)?;
    }
    cancel()?;
    let registry_changed = apply_game_registry(prefix, &inputs.registry)?;
    cancel()?;
    let gi = gameinput::install(prefix, &inputs.game_dir)?;
    if gi == gameinput::Outcome::MsiMissing {
        notes.push("The game package has no GameInputRedist.msi, so the native GameInput is not installed: the in-game mouse and controller will not work.".into());
    }
    if engine_is_stale(prefix, &inputs.engine_rev) {
        record_engine_rev(prefix, &inputs.engine_rev)?;
    }
    Ok(SetupReport { dlls_refreshed, registry_changed, gameinput: gi, notes })
}

// ---------------------------------------------------------------- repair: rebuild Wine's part, keep the game's

/// The game's own folders inside the prefix, relative to it: `drive_c/users/*/AppData/Roaming/Minecraft Bedrock*`
/// (worlds, packs, settings, both editions). Only a path with no link anywhere in it counts: Proton links
/// `drive_c/users/<login>` to `steamuser`, and the same folder through the link is not a second one to move.
pub fn game_data_folders(prefix: &Path) -> Vec<PathBuf> {
    let Ok(meta) = fs::symlink_metadata(prefix) else { return Vec::new() };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Vec::new();
    }
    let Ok(root) = fs::canonicalize(prefix) else { return Vec::new() };
    let mut found = Vec::new();
    let Ok(users) = fs::read_dir(prefix.join("drive_c/users")) else { return found };
    for user in users.flatten() {
        let roaming = user.path().join("AppData/Roaming");
        let Ok(entries) = fs::read_dir(&roaming) else { continue };
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().starts_with("Minecraft Bedrock") {
                continue;
            }
            let path = entry.path();
            let Ok(rel) = path.strip_prefix(prefix) else { continue };
            let is_real = fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
                && fs::canonicalize(&path).is_ok_and(|p| p == root.join(rel));
            if is_real {
                found.push(rel.to_path_buf());
            }
        }
    }
    found.sort();
    found
}

const REPAIR_LEFTOVER: &str = ".compatdata-repair-";

/// Delete what an interrupted repair left beside `compat`. A leftover that still holds the game's folders was
/// interrupted before they moved, so it is kept and named instead (returned).
pub fn discard_repair_leftovers(compat: &Path) -> Vec<PathBuf> {
    let mut kept = Vec::new();
    let Some(parent) = compat.parent() else { return kept };
    let Ok(read) = fs::read_dir(parent) else { return kept };
    for entry in read.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(REPAIR_LEFTOVER) {
            continue;
        }
        let path = entry.path();
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
            continue;
        }
        if !game_data_folders(&path.join("pfx")).is_empty() {
            kept.push(path);
            continue;
        }
        let _ = fs::remove_dir_all(&path);
    }
    kept
}

/// Rebuild the Wine part of a profile: delete `compat` (the prefix and Wine's bookkeeping) but keep the game's
/// own folders, which hold the worlds. Wine completes a fresh prefix around them on the next boot.
///
/// Everything that decides whether a world survives is a rename done before any delete: the old tree moves
/// aside, the game's folders move into an otherwise empty new prefix, and only then is the rest deleted.
/// Closing BlockForge during that long delete costs no world. A failure puts every folder back and deletes
/// nothing. Returns the folders kept.
pub fn reset_prefix_keeping_game_data(compat: &Path) -> Result<Vec<PathBuf>, PrefixError> {
    let pfx = compat.join("pfx");
    discard_repair_leftovers(compat);
    let meta = fs::symlink_metadata(compat);
    match meta {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(PrefixError::io("read the Wine prefix", e)),
        // A damaged or dangling link is removed without following it.
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            fs::remove_file(compat).map_err(|e| PrefixError::io("remove a damaged prefix link", e))?;
            return Ok(Vec::new());
        }
        Ok(_) => {}
    }
    require_prefix_idle(&pfx, "repair the Wine prefix")?;
    let keep = game_data_folders(&pfx);
    let parent = compat.parent().ok_or_else(|| PrefixError::Invalid("The prefix has no parent folder.".into()))?;
    let old = parent.join(format!("{REPAIR_LEFTOVER}{}", uuid::Uuid::new_v4().simple()));
    fs::rename(compat, &old).map_err(|e| {
        PrefixError::io(
            "repair the Wine prefix: stopped before deleting anything, the prefix could not be set aside",
            e,
        )
    })?;
    let old_pfx = old.join("pfx");
    let mut moved: Vec<&PathBuf> = Vec::new();
    let result: io::Result<()> = (|| {
        for rel in &keep {
            if let Some(dir) = pfx.join(rel).parent() {
                fs::create_dir_all(dir)?;
            }
            fs::rename(old_pfx.join(rel), pfx.join(rel))?;
            moved.push(rel);
        }
        Ok(())
    })();
    if let Err(e) = result {
        let undo: io::Result<()> = (|| {
            for rel in moved.iter().rev() {
                fs::rename(pfx.join(rel), old_pfx.join(rel))?;
            }
            // Only the empty folders made for them are left in the new tree.
            remove_empty_tree(compat)?;
            fs::rename(&old, compat)
        })();
        return Err(match undo {
            Ok(()) => PrefixError::io(
                "repair the Wine prefix: stopped before deleting anything, the game's folders could not be moved",
                e,
            ),
            Err(_) => PrefixError::Unsafe(format!(
                "Repair stopped half-way ({e}). Nothing was deleted: the old Wine prefix is in {}, and the Minecraft folders already moved out of it are in {}.",
                old.display(),
                pfx.display()
            )),
        });
    }
    fs::remove_dir_all(&old).map_err(|e| PrefixError::io("remove the old prefix", e))?;
    Ok(keep)
}

fn remove_empty_tree(root: &Path) -> io::Result<()> {
    for entry in fs::read_dir(root)?.flatten() {
        if entry.file_type()?.is_dir() {
            remove_empty_tree(&entry.path())?;
        }
    }
    fs::remove_dir(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scratch;

    pub(crate) const HIVE: &str = "WINE REGISTRY Version 2\n;; All keys relative to \\\\Machine\n\n#arch=win64\n\n";

    pub(crate) fn booted_prefix(tag: &str) -> PathBuf {
        let pfx = scratch(tag);
        fs::create_dir_all(pfx.join("drive_c/windows/system32")).unwrap();
        fs::create_dir_all(pfx.join("drive_c/windows/syswow64")).unwrap();
        fs::write(pfx.join("system.reg"), format!("{HIVE}[System\\\\Select] 1\n\"Current\"=dword:00000001\n\n"))
            .unwrap();
        fs::write(pfx.join("user.reg"), HIVE).unwrap();
        pfx
    }

    fn wine_dll(tag: &str, extra: &[u8]) -> Vec<u8> {
        let mut d = vec![0u8; 0x40];
        d[0] = b'M';
        d[1] = b'Z';
        d.extend_from_slice(b"Wine builtin DLL\0");
        d.extend_from_slice(tag.as_bytes());
        d.extend_from_slice(extra);
        d
    }

    fn engine(tag: &str) -> PathBuf {
        let e = scratch(tag);
        fs::create_dir_all(e.join("files/lib/wine/x86_64-windows")).unwrap();
        fs::create_dir_all(e.join("files/lib/wine/i386-windows")).unwrap();
        e
    }

    #[test]
    fn prefix_ready_needs_both_hives_and_system32() {
        let pfx = booted_prefix("ready");
        assert!(prefix_ready(&pfx));
        fs::write(pfx.join("user.reg"), "junk, not a Wine registry").unwrap();
        assert!(!prefix_ready(&pfx));
        fs::write(pfx.join("user.reg"), HIVE).unwrap();
        fs::remove_dir(pfx.join("drive_c/windows/system32")).unwrap();
        assert!(!prefix_ready(&pfx));
        assert!(!prefix_ready(&pfx.join("nothing")));
    }

    #[test]
    fn only_processes_with_this_exact_prefix_count_as_using_it() {
        let pfx = scratch("busy");
        let other = scratch("busy-other");
        let mut mine = std::process::Command::new("sleep").arg("30").env("WINEPREFIX", &pfx).spawn().unwrap();
        let mut theirs = std::process::Command::new("sleep").arg("30").env("WINEPREFIX", &other).spawn().unwrap();
        let mut prefix_of_the_same_name = std::process::Command::new("sleep")
            .arg("30")
            .env("WINEPREFIX", format!("{}-longer", pfx.display()))
            .spawn()
            .unwrap();
        let mut slash = std::process::Command::new("sleep")
            .arg("30")
            .env("WINEPREFIX", format!("{}/", pfx.display()))
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let found = prefix_processes(&pfx).unwrap();
        for c in [&mut mine, &mut theirs, &mut prefix_of_the_same_name, &mut slash] {
            c.kill().ok();
            c.wait().ok();
        }
        assert!(found.contains(&mine.id()) && found.contains(&slash.id()), "{found:?}");
        assert!(!found.contains(&theirs.id()) && !found.contains(&prefix_of_the_same_name.id()), "{found:?}");
        assert!(prefix_processes(&pfx).unwrap().is_empty());
    }

    #[test]
    fn safe_dir_refuses_links_and_files() {
        let pfx = scratch("safedir");
        let outside = scratch("safedir-out");
        assert!(safe_dir(&pfx, "drive_c/windows/system32").unwrap().is_dir());
        std::os::unix::fs::symlink(&outside, pfx.join("evil")).unwrap();
        assert!(safe_dir(&pfx, "evil/x").is_err());
        fs::write(pfx.join("file"), b"x").unwrap();
        assert!(safe_dir(&pfx, "file/x").is_err());
        assert!(safe_dir(&pfx, "../escape").is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        let link_root = scratch("safedir-root");
        let linked = link_root.join("pfx");
        std::os::unix::fs::symlink(&outside, &linked).unwrap();
        assert!(safe_dir(&linked, "a").is_err());
    }

    #[test]
    fn cryptbase_is_installed_atomically_and_the_original_kept_once() {
        let pfx = booted_prefix("crypt");
        let native = scratch("crypt-src").join("cryptbase.dll");
        fs::write(&native, b"native RNG stub").unwrap();
        fs::write(pfx.join("drive_c/windows/system32/cryptbase.dll"), b"placeholder").unwrap();
        assert!(install_cryptbase(&pfx, &native).unwrap());
        let sys = pfx.join("drive_c/windows/system32");
        assert_eq!(fs::read(sys.join("cryptbase.dll")).unwrap(), b"native RNG stub");
        assert_eq!(fs::read(sys.join("cryptbase.dll.bf-orig")).unwrap(), b"placeholder");
        assert!(!install_cryptbase(&pfx, &native).unwrap(), "already there");
        fs::write(&native, b"newer stub").unwrap();
        assert!(install_cryptbase(&pfx, &native).unwrap());
        assert_eq!(fs::read(sys.join("cryptbase.dll.bf-orig")).unwrap(), b"placeholder", "the first original stays");
        assert!(
            fs::read_dir(&sys)
                .unwrap()
                .flatten()
                .all(|e| !e.file_name().to_string_lossy().contains(".bf-") || e.file_name() == "cryptbase.dll.bf-orig")
        );
    }

    #[test]
    fn a_symlinked_cryptbase_is_replaced_without_touching_its_target() {
        let pfx = booted_prefix("crypt-link");
        let target = scratch("crypt-link-target").join("elsewhere.dll");
        fs::write(&target, b"important").unwrap();
        let native = scratch("crypt-link-src").join("cryptbase.dll");
        fs::write(&native, b"stub").unwrap();
        let sys = pfx.join("drive_c/windows/system32");
        std::os::unix::fs::symlink(&target, sys.join("cryptbase.dll")).unwrap();
        assert!(install_cryptbase(&pfx, &native).unwrap());
        assert_eq!(fs::read(&target).unwrap(), b"important");
        assert!(!fs::symlink_metadata(sys.join("cryptbase.dll")).unwrap().file_type().is_symlink());
        assert!(fs::symlink_metadata(sys.join("cryptbase.dll.bf-orig")).unwrap().file_type().is_symlink());
    }

    #[test]
    fn bootstrap_seeding_makes_system32_in_a_brand_new_prefix() {
        let root = scratch("seed");
        let pfx = root.join("compat/pfx");
        let builtin = root.join("builtin-cryptbase.dll");
        fs::write(&builtin, b"engine builtin").unwrap();
        assert!(seed_bootstrap_cryptbase(&pfx, None, &builtin).unwrap());
        assert_eq!(fs::read(pfx.join("drive_c/windows/system32/cryptbase.dll")).unwrap(), b"engine builtin");
        let native = root.join("native.dll");
        fs::write(&native, b"native").unwrap();
        assert!(seed_bootstrap_cryptbase(&pfx, Some(&native), &builtin).unwrap());
        assert_eq!(fs::read(pfx.join("drive_c/windows/system32/cryptbase.dll")).unwrap(), b"native");
        // A prefix that is a link is not seeded.
        let linked = root.join("linked");
        std::os::unix::fs::symlink(&pfx, &linked).unwrap();
        assert!(seed_bootstrap_cryptbase(&linked, None, &builtin).is_err());
    }

    #[test]
    fn the_rng_abort_is_recognised() {
        assert!(log_shows_rng_abort("err:module:... Unimplemented function advapi32.dll.SystemFunction036 called"));
        assert!(log_shows_rng_abort("module not found for forward 'cryptbase.SystemFunction036'"));
        assert!(!log_shows_rng_abort("wine: failed to open the display"));
    }

    #[test]
    fn engine_refresh_replaces_only_dlls_the_prefix_already_has() {
        let pfx = booted_prefix("refresh");
        let eng = engine("refresh-eng");
        let e64 = eng.join("files/lib/wine/x86_64-windows");
        let e32 = eng.join("files/lib/wine/i386-windows");
        fs::write(e64.join("a.dll"), wine_dll("new-a", b"")).unwrap();
        fs::write(e64.join("same.dll"), wine_dll("same", b"")).unwrap();
        fs::write(e64.join("engine_only.dll"), wine_dll("only", b"")).unwrap();
        fs::write(e64.join("notes.txt"), b"x").unwrap();
        fs::write(e32.join("b.dll"), wine_dll("new-b", b"")).unwrap();
        let sys = pfx.join("drive_c/windows/system32");
        let wow = pfx.join("drive_c/windows/syswow64");
        fs::write(sys.join("a.dll"), wine_dll("old-a", b"")).unwrap();
        fs::write(sys.join("same.dll"), wine_dll("same", b"")).unwrap();
        fs::write(sys.join("dxvk.dll"), b"a DXVK build, not in the engine list").unwrap();
        fs::write(wow.join("b.dll"), b"not from an engine at all").unwrap();
        let users = pfx.join("drive_c/users/steamuser/AppData/Roaming/Minecraft Bedrock/Users/Shared/games/com.mojang/minecraftWorlds/w1");
        fs::create_dir_all(&users).unwrap();
        fs::write(users.join("level.dat"), b"WORLD").unwrap();
        assert_eq!(refresh_engine_dlls(&pfx, &eng).unwrap(), 2);
        assert_eq!(fs::read(sys.join("a.dll")).unwrap(), wine_dll("new-a", b""));
        assert!(!sys.join("engine_only.dll").exists(), "a DLL the prefix lacks stays absent");
        assert_eq!(fs::read(sys.join("dxvk.dll")).unwrap(), b"a DXVK build, not in the engine list");
        assert_eq!(fs::read(wow.join("b.dll")).unwrap(), wine_dll("new-b", b""));
        assert!(!sys.join("a.dll.bf-runtime-backup").exists(), "a DLL an engine builds again needs no copy");
        assert_eq!(
            fs::read(wow.join("b.dll.bf-runtime-backup")).unwrap(),
            b"not from an engine at all",
            "a foreign DLL is kept"
        );
        assert_eq!(fs::read(users.join("level.dat")).unwrap(), b"WORLD");
        assert_eq!(refresh_engine_dlls(&pfx, &eng).unwrap(), 0, "and it settles");
    }

    #[test]
    fn engine_refresh_refuses_a_prefix_with_links_and_a_busy_one() {
        let pfx = booted_prefix("refresh-link");
        let eng = engine("refresh-link-eng");
        fs::write(eng.join("files/lib/wine/x86_64-windows/a.dll"), wine_dll("new", b"")).unwrap();
        fs::write(pfx.join("drive_c/windows/system32/a.dll"), wine_dll("old", b"")).unwrap();
        let mut child = std::process::Command::new("sleep").arg("30").env("WINEPREFIX", &pfx).spawn().unwrap();
        let busy = refresh_engine_dlls(&pfx, &eng);
        child.kill().ok();
        child.wait().ok();
        assert!(matches!(busy, Err(PrefixError::Busy(_))));
        assert_eq!(fs::read(pfx.join("drive_c/windows/system32/a.dll")).unwrap(), wine_dll("old", b""));
        let outside = scratch("refresh-link-out");
        fs::write(outside.join("a.dll"), wine_dll("old", b"")).unwrap();
        fs::remove_dir_all(pfx.join("drive_c/windows/system32")).unwrap();
        std::os::unix::fs::symlink(&outside, pfx.join("drive_c/windows/system32")).unwrap();
        assert!(refresh_engine_dlls(&pfx, &eng).is_err());
        assert_eq!(fs::read(outside.join("a.dll")).unwrap(), wine_dll("old", b""));
    }

    #[test]
    fn an_engine_alias_pointing_outside_the_engine_is_not_trusted() {
        let pfx = booted_prefix("refresh-alias");
        let eng = engine("refresh-alias-eng");
        let outside = scratch("refresh-alias-out").join("evil.dll");
        fs::write(&outside, b"evil").unwrap();
        std::os::unix::fs::symlink(&outside, eng.join("files/lib/wine/x86_64-windows/a.dll")).unwrap();
        fs::write(eng.join("files/lib/wine/x86_64-windows/real.dll"), wine_dll("real", b"")).unwrap();
        std::os::unix::fs::symlink("real.dll", eng.join("files/lib/wine/x86_64-windows/alias.dll")).unwrap();
        let sys = pfx.join("drive_c/windows/system32");
        for n in ["a.dll", "alias.dll", "real.dll"] {
            fs::write(sys.join(n), wine_dll("old", n.as_bytes())).unwrap();
        }
        refresh_engine_dlls(&pfx, &eng).unwrap();
        assert_ne!(fs::read(sys.join("a.dll")).unwrap(), b"evil");
        assert_eq!(
            fs::read(sys.join("alias.dll")).unwrap(),
            wine_dll("real", b""),
            "an alias inside the engine is followed"
        );
    }

    #[test]
    fn user32_is_repaired_from_the_engine() {
        let pfx = booted_prefix("user32");
        let eng = engine("user32-eng");
        fs::write(eng.join("files/lib/wine/x86_64-windows/user32.dll"), wine_dll("good", b"")).unwrap();
        let target = pfx.join("drive_c/windows/system32/user32.dll");
        fs::write(&target, b"corrupt").unwrap();
        assert!(repair_user32(&pfx, &eng).unwrap());
        assert_eq!(fs::read(&target).unwrap(), wine_dll("good", b""));
        assert_eq!(fs::read(pfx.join("drive_c/windows/system32/user32.dll.bf-managed-backup")).unwrap(), b"corrupt");
        assert!(!repair_user32(&pfx, &eng).unwrap());
        fs::remove_file(eng.join("files/lib/wine/x86_64-windows/user32.dll")).unwrap();
        assert!(repair_user32(&pfx, &eng).is_err());
    }

    #[test]
    fn old_copies_of_engine_dlls_are_pruned_and_foreign_ones_kept() {
        let pfx = booted_prefix("prune");
        let sys = pfx.join("drive_c/windows/system32");
        fs::write(sys.join("a.dll.bf-runtime-backup"), wine_dll("a", b"")).unwrap();
        fs::write(sys.join("b.dll.bf-runtime-backup"), b"someone's own DLL").unwrap();
        assert!(prune_engine_dll_backups(&pfx) > 0);
        assert!(!sys.join("a.dll.bf-runtime-backup").exists());
        assert!(sys.join("b.dll.bf-runtime-backup").exists());
    }

    #[test]
    fn the_engine_marker_is_stale_until_recorded() {
        let pfx = booted_prefix("marker");
        assert!(engine_is_stale(&pfx, "native21"));
        record_engine_rev(&pfx, "native21").unwrap();
        assert!(!engine_is_stale(&pfx, "native21"));
        assert!(engine_is_stale(&pfx, "native22"));
    }

    #[test]
    fn every_required_registry_value_is_written() {
        let pfx = booted_prefix("gamereg");
        assert!(apply_game_registry(&pfx, &GameRegistry::default()).unwrap());
        let system = fs::read_to_string(pfx.join("system.reg")).unwrap();
        let user = fs::read_to_string(pfx.join("user.reg")).unwrap();
        for needle in [
            "[Software\\\\Microsoft\\\\Windows NT\\\\CurrentVersion\\\\OEM]",
            "\"ConsoleMode\"=dword:00000008",
            "[Software\\\\Microsoft\\\\WindowsRuntime\\\\ActivatableClassId\\\\Microsoft.Windows.Storage.Pickers.FileOpenPicker]",
            "[Software\\\\Microsoft\\\\WindowsRuntime\\\\ActivatableClassId\\\\Windows.Storage.Pickers.FileSavePicker]",
            "\"DllPath\"=\"C:\\\\windows\\\\system32\\\\windows.storage.dll\"",
            "[Software\\\\Microsoft\\\\GamingServices]",
            "\"IgnoreVersionMismatch\"=dword:00000001",
            "[Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Internet Settings\\\\WinHttp]",
            "\"DefaultSecureProtocols\"=dword:00000a00",
            "[Software\\\\Microsoft\\\\SchannelTLS\\\\Protocols\\\\TLS 1.3\\\\Client]",
            "\"DisabledByDefault\"=dword:00000001",
        ] {
            assert!(system.contains(needle), "system.reg lacks {needle}");
        }
        assert!(
            !system.contains("Windows.Storage.Pickers.FileOpenPicker]\n#time")
                || system.contains("Microsoft.Windows.Storage.Pickers.FileOpenPicker")
        );
        assert!(
            !system.contains("ActivatableClassId\\\\Windows.Storage.Pickers.FileOpenPicker"),
            "the open picker keeps its Microsoft. prefix"
        );
        for name in [
            "MICROSOFT_WINDOWSAPPRUNTIME_BOOTSTRAP_INITIALIZE_SHOWUI",
            "MICROSOFT_WINDOWSAPPRUNTIME_BOOTSTRAP_INITIALIZE_FAILFAST",
            "MICROSOFT_WINDOWSAPPRUNTIME_DEPLOYMENT_INITIALIZE_ONERRORSHOWUI",
        ] {
            assert!(user.contains(&format!("\"{name}\"=\"0\"")), "user.reg lacks {name}");
        }
        assert!(user.contains("[Environment]"));
        assert!(!user.contains("Explorer"), "the virtual desktop is opt-in");
        let (before_s, before_u) = (fs::read(pfx.join("system.reg")).unwrap(), fs::read(pfx.join("user.reg")).unwrap());
        assert!(!apply_game_registry(&pfx, &GameRegistry::default()).unwrap(), "idempotent");
        assert_eq!(
            (fs::read(pfx.join("system.reg")).unwrap(), fs::read(pfx.join("user.reg")).unwrap()),
            (before_s, before_u)
        );
        apply_game_registry(&pfx, &GameRegistry { virtual_desktop: Some((1280, 720)) }).unwrap();
        let user = fs::read_to_string(pfx.join("user.reg")).unwrap();
        assert!(user.contains("\"Desktop\"=\"Default\"") && user.contains("\"Default\"=\"1280x720\""));
    }

    #[test]
    fn the_refresh_token_is_written_replaced_and_removed() {
        let pfx = booted_prefix("token");
        set_refresh_token(&pfx, "tok\"en\\1").unwrap();
        let s = fs::read_to_string(pfx.join("system.reg")).unwrap();
        assert!(s.contains("[Software\\\\Wine\\\\WineGDK]") && s.contains("\"RefreshToken\"=\"tok\\\"en\\\\1\""));
        set_refresh_token(&pfx, "second").unwrap();
        let s = fs::read_to_string(pfx.join("system.reg")).unwrap();
        assert_eq!(s.matches("RefreshToken").count(), 1);
        assert!(!s.contains("tok"));
        remove_refresh_token(&pfx).unwrap();
        assert!(!fs::read_to_string(pfx.join("system.reg")).unwrap().contains("RefreshToken"));
        assert!(set_refresh_token(&pfx, "").is_err() && set_refresh_token(&pfx, "a\0b").is_err());
        assert!(remove_refresh_token(&scratch("token-never-booted")).unwrap());
        assert_eq!(fs::metadata(pfx.join("system.reg")).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn finish_setup_runs_in_order_and_records_the_engine_only_at_the_end() {
        let pfx = booted_prefix("setup");
        let eng = engine("setup-eng");
        fs::write(eng.join("files/lib/wine/x86_64-windows/user32.dll"), wine_dll("u", b"")).unwrap();
        fs::write(pfx.join("drive_c/windows/system32/user32.dll"), b"old").unwrap();
        let game = scratch("setup-game");
        let inputs = SetupInputs {
            prefix: pfx.clone(),
            engine_root: eng.clone(),
            engine_rev: "native21".into(),
            game_dir: game.clone(),
            registry: GameRegistry::default(),
            cryptbase: None,
        };
        // No MSI yet: setup works, says so, and still records the engine.
        let report = finish_setup(&inputs, &|| false).unwrap();
        assert_eq!(report.gameinput, gameinput::Outcome::MsiMissing);
        assert_eq!(report.notes.len(), 1);
        assert!(report.registry_changed);
        assert!(!engine_is_stale(&pfx, "native21"));
        // With the installer, a second run completes it.
        fs::create_dir_all(game.join("Installers")).unwrap();
        fs::write(game.join("Installers/GameInputRedist.msi"), gameinput::fixtures::msi((2, 2, 26100, 6106))).unwrap();
        let report = finish_setup(&inputs, &|| false).unwrap();
        assert_eq!(report.gameinput, gameinput::Outcome::Installed);
        assert!(gameinput::redist_ok(&pfx));
        // A failure leaves the engine unrecorded.
        let pfx2 = booted_prefix("setup2");
        fs::write(game.join("Installers/GameInputRedist.msi"), b"junk").unwrap();
        let bad = SetupInputs { prefix: pfx2.clone(), ..inputs.clone() };
        assert!(finish_setup(&bad, &|| false).is_err());
        assert!(engine_is_stale(&pfx2, "native21"));
        // Cancelling stops before anything is written.
        let pfx3 = booted_prefix("setup3");
        let cancelled = SetupInputs { prefix: pfx3.clone(), ..inputs };
        let e = finish_setup(&cancelled, &|| true).unwrap_err();
        assert!(matches!(e.kind, ErrorKind::Cancelled));
        assert_eq!(fs::read_to_string(pfx3.join("user.reg")).unwrap(), HIVE);
    }

    fn compat_with_world(tag: &str) -> (PathBuf, PathBuf) {
        let compat = scratch(tag).join("compat");
        let pfx = compat.join("pfx");
        fs::create_dir_all(pfx.join("drive_c/windows/system32")).unwrap();
        fs::write(pfx.join("system.reg"), HIVE).unwrap();
        fs::write(compat.join("config_info"), b"wine stuff").unwrap();
        let roaming = pfx.join("drive_c/users/steamuser/AppData/Roaming");
        for (edition, world) in [("Minecraft Bedrock", "release-world"), ("Minecraft Bedrock Preview", "preview-world")]
        {
            let w = roaming.join(edition).join("Users/Shared/games/com.mojang/minecraftWorlds").join(world);
            fs::create_dir_all(&w).unwrap();
            fs::write(w.join("level.dat"), world.as_bytes()).unwrap();
        }
        fs::create_dir_all(roaming.join("Other App")).unwrap();
        fs::write(roaming.join("Other App/data"), b"x").unwrap();
        std::os::unix::fs::symlink("steamuser", pfx.join("drive_c/users/login")).unwrap();
        (compat, pfx)
    }

    #[test]
    fn repair_keeps_the_worlds_of_both_editions_and_drops_the_rest() {
        let (compat, pfx) = compat_with_world("reset");
        let kept = reset_prefix_keeping_game_data(&compat).unwrap();
        assert_eq!(kept.len(), 2, "{kept:?}");
        let roaming = pfx.join("drive_c/users/steamuser/AppData/Roaming");
        assert_eq!(
            fs::read(
                roaming.join("Minecraft Bedrock/Users/Shared/games/com.mojang/minecraftWorlds/release-world/level.dat")
            )
            .unwrap(),
            b"release-world"
        );
        assert_eq!(
            fs::read(roaming.join(
                "Minecraft Bedrock Preview/Users/Shared/games/com.mojang/minecraftWorlds/preview-world/level.dat"
            ))
            .unwrap(),
            b"preview-world"
        );
        assert!(
            !pfx.join("system.reg").exists()
                && !compat.join("config_info").exists()
                && !roaming.join("Other App").exists()
        );
        let leftovers: Vec<_> = fs::read_dir(compat.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(REPAIR_LEFTOVER))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn repair_without_worlds_clears_everything_and_a_missing_compat_is_fine() {
        let compat = scratch("reset-empty").join("compat");
        assert!(reset_prefix_keeping_game_data(&compat).unwrap().is_empty());
        fs::create_dir_all(compat.join("pfx/drive_c")).unwrap();
        fs::write(compat.join("pfx/x"), b"y").unwrap();
        assert!(reset_prefix_keeping_game_data(&compat).unwrap().is_empty());
        assert!(!compat.exists());
    }

    #[test]
    fn repair_unlinks_a_dangling_compat_link_without_following_it() {
        let root = scratch("reset-link");
        let target = root.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("keep"), b"k").unwrap();
        std::os::unix::fs::symlink(&target, root.join("compat")).unwrap();
        reset_prefix_keeping_game_data(&root.join("compat")).unwrap();
        assert!(!root.join("compat").exists());
        assert!(target.join("keep").exists());
    }

    #[test]
    fn repair_is_refused_while_the_prefix_is_in_use() {
        let (compat, pfx) = compat_with_world("reset-busy");
        let mut child = std::process::Command::new("sleep").arg("30").env("WINEPREFIX", &pfx).spawn().unwrap();
        let r = reset_prefix_keeping_game_data(&compat);
        child.kill().ok();
        child.wait().ok();
        assert!(matches!(r, Err(PrefixError::Busy(_))));
        assert!(pfx.join("system.reg").exists());
    }

    #[test]
    fn a_folder_that_cannot_move_leaves_the_prefix_intact() {
        let (compat, pfx) = compat_with_world("reset-fail");
        let roaming = pfx.join("drive_c/users/steamuser/AppData/Roaming");
        // Moving a folder to another parent needs write access to the folder itself (its `..` changes): taking
        // it away from the second folder makes its move fail after the first one already moved.
        let stuck = roaming.join("Minecraft Bedrock Preview");
        fs::set_permissions(&stuck, fs::Permissions::from_mode(0o555)).unwrap();
        let result = reset_prefix_keeping_game_data(&compat);
        let still_there = compat.join("pfx/drive_c/users/steamuser/AppData/Roaming/Minecraft Bedrock Preview");
        fs::set_permissions(&still_there, fs::Permissions::from_mode(0o755)).ok();
        assert!(result.is_err(), "the failed move is reported");
        // Everything is back as it was: Wine's own files too, since nothing was deleted.
        assert!(compat.join("config_info").exists() && compat.join("pfx/system.reg").exists());
        assert_eq!(walk_find(&compat, "level.dat"), 2);
        let leftovers: Vec<_> = fs::read_dir(compat.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(REPAIR_LEFTOVER))
            .collect();
        assert!(leftovers.is_empty(), "the aside copy was put back");
    }

    fn walk_find(root: &Path, name: &str) -> usize {
        let mut n = 0;
        if let Ok(read) = fs::read_dir(root) {
            for e in read.flatten() {
                let p = e.path();
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    n += walk_find(&p, name);
                } else if e.file_name() == name {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn an_interrupted_repair_leftover_with_worlds_is_never_deleted() {
        let (compat, _pfx) = compat_with_world("reset-leftover");
        let parent = compat.parent().unwrap().to_path_buf();
        let leftover = parent.join(format!("{REPAIR_LEFTOVER}abc"));
        fs::rename(&compat, &leftover).unwrap();
        let empty_leftover = parent.join(format!("{REPAIR_LEFTOVER}def"));
        fs::create_dir_all(empty_leftover.join("pfx/drive_c")).unwrap();
        let kept = discard_repair_leftovers(&compat);
        assert_eq!(kept, vec![leftover.clone()]);
        assert!(leftover.exists() && !empty_leftover.exists());
    }

    #[test]
    fn game_data_is_found_only_when_no_link_is_in_the_way() {
        let (_compat, pfx) = compat_with_world("gamedata");
        assert_eq!(game_data_folders(&pfx).len(), 2);
        let outside = scratch("gamedata-out");
        fs::create_dir_all(outside.join("Minecraft Bedrock Evil")).unwrap();
        std::os::unix::fs::symlink(&outside, pfx.join("drive_c/users/steamuser/AppData/Roaming/Linked")).unwrap();
        std::os::unix::fs::symlink(
            outside.join("Minecraft Bedrock Evil"),
            pfx.join("drive_c/users/steamuser/AppData/Roaming/Minecraft Bedrock Linked"),
        )
        .unwrap();
        assert_eq!(game_data_folders(&pfx).len(), 2);
    }

    #[test]
    fn free_bytes_reads_a_real_filesystem() {
        let dir = scratch("free");
        assert!(free_bytes(&dir.join("not/yet/there")).unwrap() > 0);
    }

    /// Real check, needs the user's Microsoft sign-in (package A's download) and package B's engine and wineboot.
    ///
    /// 1. Package A installs the current Release build into a scratch folder (user signs in) and package B
    ///    installs the engine and runs headless wineboot plus `wineserver -w` on a *scratch* prefix, after
    ///    `seed_bootstrap_cryptbase`.
    /// 2. Run, with the scratch paths (never the BedrockOnLinux prefix or data):
    ///
    /// ```text
    /// BF_C_PREFIX=~/.cache/bf-bedrock-pkgc/pfx BF_C_ENGINE=<engine root> BF_C_ENGINE_REV=native21 \
    /// BF_C_GAME=<build>/content BF_C_CRYPTBASE=<openssl set>/cryptbase.dll \
    /// cargo test --locked --lib real_prefix_setup -- --ignored --nocapture
    /// ```
    ///
    /// It extracts GameInputRedist.msi without msiexec, writes every registry value, checks the hives and that the
    /// redistributable tree exists, then runs a second time to prove nothing changes. Nothing is printed from a
    /// registry value.
    #[test]
    #[ignore = "needs package B's wineboot and package A's downloaded game; see the doc comment"]
    fn real_prefix_setup_with_a_downloaded_game() {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from).unwrap_or_else(|| panic!("set {k}"));
        let inputs = SetupInputs {
            prefix: env("BF_C_PREFIX"),
            engine_root: env("BF_C_ENGINE"),
            engine_rev: std::env::var("BF_C_ENGINE_REV").unwrap_or_else(|_| "native21".into()),
            game_dir: env("BF_C_GAME"),
            registry: GameRegistry::default(),
            cryptbase: std::env::var_os("BF_C_CRYPTBASE").map(PathBuf::from),
        };
        assert!(prefix_ready(&inputs.prefix), "boot the scratch prefix with the engine's wineboot first");
        let first = finish_setup(&inputs, &|| false).expect("setup");
        println!("{first:?}");
        assert!(
            matches!(
                first.gameinput,
                gameinput::Outcome::Installed | gameinput::Outcome::Updated | gameinput::Outcome::AlreadyCurrent
            ),
            "{:?}",
            first.gameinput
        );
        assert!(gameinput::redist_ok(&inputs.prefix));
        let second = finish_setup(&inputs, &|| false).expect("second setup");
        assert!(!second.registry_changed && second.dlls_refreshed == 0);
        assert_eq!(second.gameinput, gameinput::Outcome::AlreadyCurrent);
        let hbui = crate::bedrock::fixups::hbui_inspect(&inputs.game_dir);
        println!("hbui: {:?}", hbui.status().diagnostic());
    }

    #[test]
    fn errors_map_onto_the_backends_kinds() {
        assert!(matches!(BedrockError::from(PrefixError::Busy("x".into())).kind, ErrorKind::Busy));
        assert!(matches!(BedrockError::from(PrefixError::Integrity("x".into())).kind, ErrorKind::Integrity));
        assert!(matches!(BedrockError::from(PrefixError::Malformed("x".into())).kind, ErrorKind::Prefix));
        assert!(matches!(
            BedrockError::from(PrefixError::io("w", io::Error::from_raw_os_error(libc::ENOSPC))).kind,
            ErrorKind::NoSpace
        ));
    }
}
