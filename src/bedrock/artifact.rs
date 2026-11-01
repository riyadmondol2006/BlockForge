//! Verified, locked artifact transactions: a pinned archive is downloaded with its SHA-256 worked out on the way,
//! unpacked into a stage beside the live tree (keeping the engine's internal links, refusing anything that could
//! escape), checked, made read-only and swapped in by rename, with a rollback copy until the swap is durable. A
//! receipt beside the tree records that it passed the full check, so a warm Play never rehashes 2 GB or keeps an
//! 800 MB archive around. Ported from BedrockOnLinux bol/archive.py, bol/winegdk.py and bol/vkd3d.py (MIT); extraction
//! also refuses writes below a link.
//!
//! Layout under one artifact root (`bedrock/engine`, `bedrock/runtime/steamrt3`):
//! `<revision>/` the live, read-only tree; `.<revision>.lock` the lock every installer and user takes (never
//! deleted); `.<revision>.receipt` the full-check receipt; `.<revision>.stage`/`.rollback` transient; anything else
//! named `.<revision>.*` belongs to that revision too (the runtime's private view, say) and goes with it.
use super::{BedrockError, Cancel, ErrorKind, Result, pins::ArtifactPin};
use crate::install::Progress;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::atomic::Ordering,
};

pub(crate) fn io(e: impl std::fmt::Display) -> BedrockError {
    BedrockError::new(ErrorKind::Integrity, e.to_string())
}
pub(crate) fn invalid(s: impl Into<String>) -> BedrockError {
    BedrockError::new(ErrorKind::Integrity, s)
}
fn cancelled() -> BedrockError {
    BedrockError::new(ErrorKind::Cancelled, "Cancelled")
}

pub fn sha256(path: &Path) -> Result<String> {
    let mut f = File::open(path).map_err(io)?;
    hash_reader(&mut f)
}
fn hash_reader(reader: &mut impl Read) -> Result<String> {
    let mut hash = Sha256::new();
    let mut buf = vec![0; 256 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(io)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}
pub fn verify_hash(path: &Path, expected: &str) -> Result<()> {
    if sha256(path)? != expected {
        return Err(invalid(format!("SHA-256 mismatch: {}", path.display())));
    }
    Ok(())
}

/// Streams `pin` to `dest` through a `.part` file beside it, hashing as it goes; `dest` appears only once the
/// bytes are exactly the pinned ones, so a bad or cut download never replaces a good cached archive. Pins are
/// compile-time constants: tests swap in a local URL, nothing else does.
pub async fn download(
    client: &reqwest::Client,
    pin: ArtifactPin,
    dest: &Path,
    progress: &Progress,
    cancel: &Cancel,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    supported_host()?;
    let network = |e: reqwest::Error| BedrockError::new(ErrorKind::Network, e.to_string());
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(io)?;
    }
    let tmp = dest.with_extension(format!("{}.part", uuid::Uuid::new_v4().simple()));
    let result = async {
        let request = client.get(pin.url).header(reqwest::header::ACCEPT_ENCODING, "identity").send();
        let mut response = tokio::select! {
            _ = cancel.cancelled() => return Err(cancelled()),
            r = request => r.map_err(network)?,
        };
        if response.status() != reqwest::StatusCode::OK {
            return Err(BedrockError::new(ErrorKind::Network, format!("Download answered HTTP {}", response.status())));
        }
        if response.content_length().is_some_and(|n| n != pin.size) {
            return Err(invalid("The download is not the pinned size"));
        }
        progress.bytes_total.fetch_add(pin.size, Ordering::Relaxed);
        let mut f = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&tmp).await.map_err(io)?;
        let mut hash = Sha256::new();
        let mut total = 0u64;
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(cancelled()),
                c = response.chunk() => c.map_err(network)?,
            };
            let Some(chunk) = chunk else { break };
            total += chunk.len() as u64;
            if total > pin.size {
                return Err(invalid("The download is longer than the pinned size"));
            }
            hash.update(&chunk);
            f.write_all(&chunk).await.map_err(io)?;
            progress.bytes_done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
        if total != pin.size || hex::encode(hash.finalize()) != pin.sha256 {
            return Err(invalid("The download's SHA-256 is not the pinned one"));
        }
        f.sync_all().await.map_err(io)?;
        drop(f);
        tokio::fs::rename(&tmp, dest).await.map_err(io)
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

/// A hold on one installed revision: installers and pruning need the same inode exclusively, so while any lease is
/// alive the tree can't be replaced or removed. Keep it (in the supervisor too) until the last prefix process exits.
#[derive(Debug)]
pub struct Lease {
    _file: File,
}
/// How long a lock that is taken waits before saying Busy.
const LOCK_PATIENCE_MS: u64 = 500;
fn lock(root: &Path, name: &str, shared: bool) -> Result<Lease> {
    fs::create_dir_all(root).map_err(io)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(format!(".{name}.lock")))
        .map_err(io)?;
    // A child another thread of this process forks holds a copy of every open descriptor until it execs, a lock
    // just let go of included: a short wait tells that moment apart from a real holder.
    let mut waited = 0;
    loop {
        let result = if shared { file.try_lock_shared() } else { file.try_lock() };
        match result {
            Ok(()) => return Ok(Lease { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) if waited < LOCK_PATIENCE_MS => {
                std::thread::sleep(std::time::Duration::from_millis(10));
                waited += 10;
            }
            Err(e) => return Err(BedrockError::new(ErrorKind::Busy, format!("{name} is in use: {e}"))),
        }
    }
}
pub fn lease(root: &Path, name: &str) -> Result<Lease> {
    lock(root, name, true)
}
pub(crate) fn exclusive(root: &Path, name: &str) -> Result<Lease> {
    lock(root, name, false)
}

/// A crashed supervisor can drop its locks before Wine exits. Fail closed when a native process still runs or maps
/// something from these trees or carries them in its container/loader paths. A blocking, install-only scan of
/// /proc; never a startup or window-thread probe.
pub(crate) fn in_use(roots: &[PathBuf]) -> Result<bool> {
    let roots: Vec<PathBuf> = roots.iter().filter(|r| r.exists()).filter_map(|r| fs::canonicalize(r).ok()).collect();
    if roots.is_empty() {
        return Ok(false);
    }
    let under = |p: &Path| roots.iter().any(|r| p.starts_with(r));
    for entry in fs::read_dir("/proc").map_err(io)?.flatten() {
        if !entry.file_name().as_encoded_bytes().iter().all(u8::is_ascii_digit) {
            continue;
        }
        if let Ok(exe) = fs::read_link(entry.path().join("exe"))
            && under(&exe)
        {
            return Ok(true);
        }
        if let Ok(maps) = fs::read_to_string(entry.path().join("maps"))
            && maps.lines().filter_map(|line| line.find('/').map(|i| &line[i..])).any(|p| under(Path::new(p)))
        {
            return Ok(true);
        }
        if let Ok(env) = fs::read(entry.path().join("environ")) {
            use std::os::unix::ffi::OsStrExt;
            for item in env.split(|b| *b == 0) {
                for key in [b"STEAM_COMPAT_TOOL_PATHS=".as_slice(), b"WINEDLLPATH=", b"LD_LIBRARY_PATH="] {
                    if let Some(value) = item.strip_prefix(key)
                        && value.split(|b| *b == b':').any(|p| under(Path::new(std::ffi::OsStr::from_bytes(p))))
                    {
                        return Ok(true);
                    }
                }
            }
        }
    }
    Ok(false)
}

fn relative(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() || path.is_absolute() || path.to_str().is_none_or(|s| s.contains(['\\', ':', '\0']))
    {
        return Err(invalid("Unsafe archive path"));
    }
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(v) => out.push(v),
            Component::CurDir => (),
            _ => return Err(invalid("Archive path traversal")),
        }
    }
    Ok(out)
}
fn link_target(parent: &Path, target: &Path) -> Result<()> {
    if target.as_os_str().is_empty()
        || target.is_absolute()
        || target.to_str().is_none_or(|s| s.contains(['\\', ':', '\0']))
    {
        return Err(invalid("Unsafe archive link"));
    }
    let mut depth = parent.components().count();
    for c in target.components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::CurDir => (),
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => return Err(invalid("Archive link escape")),
        }
    }
    Ok(())
}
/// Whether `path` or any folder above it is one of `links`.
fn below_link(links: &HashSet<PathBuf>, path: &Path) -> bool {
    path.ancestors().any(|a| links.contains(a))
}

/// Unpacks a tar stream whose members all live in `root/`. Never follows a link while extracting: links are made
/// only after every regular member, no member may sit below a link, and every link must resolve inside `dest`.
/// Absolute and `..` paths, duplicates, devices, FIFOs and anything over `limit` bytes are refused.
pub fn extract(reader: impl Read, dest: &Path, root: &str, limit: u64) -> Result<()> {
    let mut archive = tar::Archive::new(reader);
    let mut seen = HashSet::new();
    let mut links = vec![];
    let mut total = 0u64;
    for entry in archive.entries().map_err(io)? {
        let mut entry = entry.map_err(io)?;
        let path = relative(&entry.path().map_err(io)?)?;
        let path = path.strip_prefix(root).map_err(|_| invalid("Unexpected archive root"))?.to_path_buf();
        if !seen.insert(path.clone()) {
            return Err(invalid(format!("Duplicate archive entry: {}", path.display())));
        }
        if seen.len() > 250_000 {
            return Err(invalid("Archive entry limit exceeded"));
        }
        let kind = entry.header().entry_type();
        if path.as_os_str().is_empty() {
            if !kind.is_dir() {
                return Err(invalid("Archive root is not a directory"));
            }
            continue;
        }
        if kind.is_symlink() || kind.is_hard_link() {
            let target = entry.link_name().map_err(io)?.ok_or_else(|| invalid("Missing link target"))?.into_owned();
            if kind.is_symlink() {
                link_target(path.parent().unwrap_or(Path::new("")), &target)?;
                links.push((path, target, true));
            } else {
                let target =
                    relative(&target)?.strip_prefix(root).map_err(|_| invalid("Hardlink root escape"))?.to_path_buf();
                links.push((path, target, false));
            }
            continue;
        }
        if !kind.is_file() && !kind.is_dir() {
            return Err(invalid("Unsupported archive special file"));
        }
        let dst = dest.join(&path);
        if kind.is_dir() {
            fs::create_dir_all(&dst).map_err(io)?;
        } else {
            total = total.checked_add(entry.size()).ok_or_else(|| invalid("Archive size overflow"))?;
            if total > limit {
                return Err(invalid("Archive unpack limit exceeded"));
            }
            fs::create_dir_all(dst.parent().unwrap_or(dest)).map_err(io)?;
            let mut f = OpenOptions::new().write(true).create_new(true).open(&dst).map_err(io)?;
            std::io::copy(&mut entry, &mut f).map_err(io)?;
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dst, fs::Permissions::from_mode(entry.header().mode().map_err(io)? & 0o777))
                .map_err(io)?;
        }
    }
    let link_paths: HashSet<PathBuf> = links.iter().map(|(p, _, _)| p.clone()).collect();
    for path in &seen {
        if path.parent().is_some_and(|parent| below_link(&link_paths, parent)) {
            return Err(invalid("Archive entry below a link"));
        }
    }
    // Hard links to regular entries only; no chained links or symlink traversal.
    for (path, target, _) in links.iter().filter(|(_, _, symlink)| !*symlink) {
        if below_link(&link_paths, target) || !dest.join(target).is_file() {
            return Err(invalid("Unsafe archive hardlink"));
        }
        fs::create_dir_all(dest.join(path).parent().unwrap_or(dest)).map_err(io)?;
        fs::hard_link(dest.join(target), dest.join(path)).map_err(io)?;
    }
    for (path, target, _) in links.iter().filter(|(_, _, symlink)| *symlink) {
        fs::create_dir_all(dest.join(path).parent().unwrap_or(dest)).map_err(io)?;
        std::os::unix::fs::symlink(target, dest.join(path)).map_err(io)?;
    }
    // Lexically contained links can still form cycles or combine into an escape.
    let owned = fs::canonicalize(dest).map_err(io)?;
    for (path, _, _) in links {
        let resolved = fs::canonicalize(dest.join(path)).map_err(io)?;
        if !resolved.starts_with(&owned) {
            return Err(invalid("Archive link resolves outside staging"));
        }
    }
    Ok(())
}
fn sync_dir(path: &Path) -> Result<()> {
    File::open(path).and_then(|f| f.sync_all()).map_err(io)
}
/// Removes an owned tree, read-only or not, without following any link in it.
pub(crate) fn remove(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(e)),
        Ok(m) if m.is_dir() => {
            thaw(path)?;
            fs::remove_dir_all(path).map_err(io)
        }
        Ok(_) => fs::remove_file(path).map_err(io),
    }
}
fn thaw(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let ty = fs::symlink_metadata(path).map_err(io)?.file_type();
    if ty.is_dir() {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io)?;
        for entry in fs::read_dir(path).map_err(io)? {
            thaw(&entry.map_err(io)?.path())?;
        }
    }
    Ok(())
}
/// Drops every write bit in an installed tree, so nothing (Wine's own setup included) can change a shared engine.
fn freeze(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = fs::symlink_metadata(path).map_err(io)?;
    if meta.is_symlink() {
        return Ok(());
    }
    if meta.is_dir() {
        for entry in fs::read_dir(path).map_err(io)? {
            freeze(&entry.map_err(io)?.path())?;
        }
    }
    fs::set_permissions(path, fs::Permissions::from_mode(meta.permissions().mode() & 0o555)).map_err(io)
}

/// What a receipt holds: the pin a tree was unpacked from and fully checked against.
fn receipt_text(pin: &ArtifactPin) -> String {
    format!("{} {}\n", pin.revision, pin.sha256)
}
fn receipt(root: &Path, pin: &ArtifactPin) -> PathBuf {
    root.join(format!(".{}.receipt", pin.revision))
}

/// The installed tree for `pin`, if it was fully checked at install and `quick` (cheap metadata only) still agrees.
/// No archive, no full hash: this is what Play calls. None means install (or repair) it. Busy while it installs.
pub fn open(root: &Path, pin: ArtifactPin, quick: impl Fn(&Path) -> Result<()>) -> Result<Option<PathBuf>> {
    let _lease = lease(root, pin.revision)?;
    let active = root.join(pin.revision);
    let rollback = root.join(format!(".{}.rollback", pin.revision));
    let receipt_ok = fs::read_to_string(receipt(root, &pin)).is_ok_and(|r| r == receipt_text(&pin));
    if !receipt_ok || rollback.exists() || !active.is_dir() || quick(&active).is_err() {
        return Ok(None);
    }
    Ok(Some(active))
}

/// Installs `pin` from `archive` into `root/<revision>`; call only on a blocking worker. The archive is hashed and
/// unpacked from the same open file, so a cache swapped mid-way can't slip in other bytes. `validate` is the full
/// check. A crash at any step is recovered on the next call: a missing live tree comes back from its rollback copy.
pub fn install(
    root: &Path,
    pin: ArtifactPin,
    archive: &Path,
    validate: impl Fn(&Path) -> Result<()>,
) -> Result<PathBuf> {
    supported_host()?;
    let name = pin.revision;
    let _guard = exclusive(root, name)?;
    let active = root.join(name);
    let backup = root.join(format!(".{name}.rollback"));
    let stage = root.join(format!(".{name}.stage"));
    if in_use(&owned(root, name)?)? {
        return Err(BedrockError::new(
            ErrorKind::Busy,
            "A Wine process still uses this engine or runtime; wait for the game to close",
        ));
    }
    recover(root, name, &validate)?;
    let mut archive_file = File::open(archive).map_err(io)?;
    if hash_reader(&mut archive_file)? != pin.sha256 {
        return Err(invalid("The archive's SHA-256 is not the pinned one"));
    }
    use std::io::Seek;
    archive_file.rewind().map_err(io)?;
    // A repair whose tree turns out fine only needs its receipt back.
    if active.exists() && validate(&active).is_ok() {
        remove(&stage)?;
        freeze(&active)?;
        write_receipt(root, &pin)?;
        return Ok(active);
    }
    remove(&stage)?;
    fs::create_dir(&stage).map_err(io)?;
    let result = (|| {
        if pin.xz {
            extract(xz2::read::XzDecoder::new(archive_file), &stage, pin.archive_root, pin.unpack_limit)?;
        } else {
            extract(flate2::read::GzDecoder::new(archive_file), &stage, pin.archive_root, pin.unpack_limit)?;
        }
        freeze(&stage)?;
        validate(&stage)?;
        // Data reaches the disk before a durable rename or receipt can advertise it.
        sync_tree(&stage)?;
        remove(&receipt(root, &pin))?;
        if active.exists() {
            fs::rename(&active, &backup).map_err(io)?;
            sync_dir(root)?;
        }
        fs::rename(&stage, &active).map_err(io)?;
        sync_dir(root)?;
        remove(&backup)?;
        sync_dir(root)?;
        write_receipt(root, &pin)?;
        Ok(active.clone())
    })();
    if result.is_err() {
        remove(&stage)?;
        recover(root, name, &validate)?;
    }
    result
}
fn write_receipt(root: &Path, pin: &ArtifactPin) -> Result<()> {
    use std::io::Write;
    let path = receipt(root, pin);
    let tmp = root.join(format!(".{}.receipt.tmp", pin.revision));
    let mut f = File::create(&tmp).map_err(io)?;
    f.write_all(receipt_text(pin).as_bytes()).map_err(io)?;
    f.sync_all().map_err(io)?;
    fs::rename(&tmp, path).map_err(io)?;
    sync_dir(root)
}

/// Removes every other revision under `root` that nothing holds or runs from: a changed pin leaves the old engine
/// behind until its last session ends. Returns the names it removed; busy ones stay for a later call.
pub fn prune(root: &Path, keep: &str) -> Result<Vec<String>> {
    let mut removed = vec![];
    let Ok(dir) = fs::read_dir(root) else { return Ok(removed) };
    let names: Vec<String> = dir
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.') && n != keep)
        .collect();
    for name in names {
        let Ok(_guard) = exclusive(root, &name) else { continue };
        let owned = owned(root, &name)?;
        if in_use(&owned)? {
            continue;
        }
        for path in &owned {
            remove(path)?;
        }
        sync_dir(root)?;
        removed.push(name);
    }
    Ok(removed)
}

/// Under the install lock: the live tree passes the full check again (its receipt is renewed) or loses its receipt.
fn revalidate(root: &Path, pin: ArtifactPin, validate: impl Fn(&Path) -> Result<()>) -> Result<Option<PathBuf>> {
    let _guard = exclusive(root, pin.revision)?;
    recover(root, pin.revision, &validate)?;
    let active = root.join(pin.revision);
    if active.is_dir() && validate(&active).is_ok() {
        freeze(&active)?;
        write_receipt(root, &pin)?;
        return Ok(Some(active));
    }
    remove(&receipt(root, &pin))?;
    Ok(None)
}

/// Where one artifact lives and how hard to check it.
pub struct Ensure<'a> {
    pub dir: &'a Path,
    pub cache: &'a Path,
    pub pin: ArtifactPin,
    pub full: bool,
}
/// A name for progress text, the cheap check and the full check.
pub type Checks = (&'static str, fn(&Path) -> Result<()>, fn(&Path) -> Result<()>);

/// Open, or (on `full`) revalidate, or download and install, then prune older revisions nothing uses. A cached
/// archive with the wrong bytes is fetched again once; the archive is deleted after a good install. Each disk step
/// runs on a blocking worker.
pub async fn ensure(
    at: Ensure<'_>,
    client: &reqwest::Client,
    progress: &std::sync::Arc<Progress>,
    cancel: &Cancel,
    (what, quick, validate): Checks,
) -> Result<PathBuf> {
    let blocking = |f: Box<dyn FnOnce() -> Result<Option<PathBuf>> + Send>| async move {
        tokio::task::spawn_blocking(f).await.map_err(|e| BedrockError::new(ErrorKind::Internal, e.to_string()))?
    };
    let (dir, pin) = (at.dir.to_path_buf(), at.pin);
    // A tree without a receipt (a crash just before it was written, say) is checked in full before anything is
    // downloaded again: hashing what is there is far cheaper than fetching it.
    let full = at.full;
    if full {
        progress.stage(format!("Checking every file of {what}"));
    }
    let d = dir.clone();
    let found = blocking(Box::new(move || {
        if !full {
            if let Some(root) = open(&d, pin, quick)? {
                return Ok(Some(root));
            }
            if !d.join(pin.revision).is_dir() {
                return Ok(None);
            }
        }
        revalidate(&d, pin, validate)
    }))
    .await?;
    if let Some(root) = found {
        return Ok(root);
    }
    let archive = at.cache.join(format!("{}.{}", pin.revision, if pin.xz { "tar.xz" } else { "tar.gz" }));
    for attempt in 0..2 {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        if !archive.is_file() {
            progress.stage(format!("Downloading {what}"));
            download(client, pin, &archive, progress, cancel).await?;
        }
        progress.stage(format!("Unpacking and checking {what}"));
        let (d, a) = (dir.clone(), archive.clone());
        match blocking(Box::new(move || install(&d, pin, &a, validate).map(Some))).await {
            Ok(Some(root)) => {
                let _ = tokio::fs::remove_file(&archive).await;
                let d = dir.clone();
                let _ = blocking(Box::new(move || prune(&d, pin.revision).map(|_| None))).await;
                return Ok(root);
            }
            Err(e) if attempt == 0 && e.message.contains("archive's SHA-256") => {
                let _ = tokio::fs::remove_file(&archive).await;
            }
            Err(e) => return Err(e),
            Ok(None) => break,
        }
    }
    Err(invalid(format!("Couldn't install {what}")))
}

/// Everything under `root` that belongs to revision `name`, except its lock inode.
fn owned(root: &Path, name: &str) -> Result<Vec<PathBuf>> {
    let (prefix, lock) = (format!(".{name}."), format!(".{name}.lock"));
    Ok(fs::read_dir(root)
        .map_err(io)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let file = p.file_name().and_then(|f| f.to_str()).unwrap_or_default();
            file == name || (file.starts_with(&prefix) && file != lock)
        })
        .collect())
}

fn supported_host() -> Result<()> {
    if cfg!(target_arch = "x86_64") {
        Ok(())
    } else {
        Err(BedrockError::new(
            ErrorKind::UnsupportedPlatform,
            "The pinned Bedrock engine and SteamRT require x86-64 Linux",
        ))
    }
}
pub(crate) fn sync_tree(path: &Path) -> Result<()> {
    for e in fs::read_dir(path).map_err(io)? {
        let e = e.map_err(io)?;
        let ty = e.file_type().map_err(io)?;
        if ty.is_dir() {
            sync_tree(&e.path())?;
        } else if ty.is_file() {
            File::open(e.path()).and_then(|f| f.sync_all()).map_err(io)?;
        }
    }
    sync_dir(path)
}
fn recover(root: &Path, name: &str, validate: &impl Fn(&Path) -> Result<()>) -> Result<()> {
    let active = root.join(name);
    let backup = root.join(format!(".{name}.rollback"));
    if backup.exists() {
        if active.exists() && validate(&active).is_ok() {
            remove(&backup)?;
        } else {
            remove(&active)?;
            fs::rename(&backup, &active).map_err(io)?;
        }
        sync_dir(root)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    /// A tar of `(path, kind, payload or link target)` entries; kind is the tar type byte.
    pub(crate) fn tar(entries: &[(&str, u8, &str)]) -> Vec<u8> {
        let mut out = vec![];
        {
            let mut b = tar::Builder::new(&mut out);
            for (name, kind, value) in entries {
                let mut h = tar::Header::new_gnu();
                h.set_mode(0o755);
                h.set_entry_type(tar::EntryType::new(*kind));
                h.set_size(if *kind == b'0' { value.len() as u64 } else { 0 });
                if *kind == b'2' || *kind == b'1' {
                    h.set_link_name(value).unwrap();
                }
                h.set_cksum();
                b.append_data(&mut h, name, if *kind == b'0' { value.as_bytes() } else { &[] }).unwrap();
            }
            b.finish().unwrap();
        }
        out
    }
    /// A gzip archive on disk and a pin that names exactly it.
    pub(crate) fn synthetic(
        dir: &Path,
        revision: &'static str,
        entries: &[(&str, u8, &str)],
    ) -> (PathBuf, ArtifactPin) {
        use std::io::Write;
        let mut gz = flate2::write::GzEncoder::new(vec![], flate2::Compression::fast());
        gz.write_all(&tar(entries)).unwrap();
        let bytes = gz.finish().unwrap();
        let path = dir.join(format!("{revision}.tar.gz"));
        fs::write(&path, &bytes).unwrap();
        let sha256 = Box::leak(hex::encode(Sha256::digest(&bytes)).into_boxed_str());
        let pin = ArtifactPin {
            revision,
            url: "http://127.0.0.1:9/unused",
            sha256,
            size: bytes.len() as u64,
            archive_root: "root",
            xz: false,
            unpack_limit: 1 << 20,
        };
        (path, pin)
    }
    fn has_ok(p: &Path) -> Result<()> {
        if fs::read(p.join("ok")).is_ok() { Ok(()) } else { Err(invalid("incomplete")) }
    }

    #[test]
    fn unsafe_links_duplicates_specials_and_size_are_refused() {
        let big = "x".repeat(2048);
        let cases = [
            vec![("root/a", b'2', "../../escape")],
            vec![("root/a", b'2', "/etc/passwd")],
            vec![("root/a", b'2', "b"), ("root/a/file", b'0', "x")],
            vec![("root/a/file", b'0', "x"), ("root/a", b'2', "b")],
            vec![("root/a", b'0', "x"), ("root/a", b'0', "x")],
            vec![("root/a", b'0', "x"), ("root/a", b'2', "b")],
            vec![("root/a", b'1', "root/missing")],
            vec![("root/a", b'1', "../outside")],
            vec![("root/a", b'2', "b"), ("root/b", b'2', "a")],
            vec![("root/d", b'2', "."), ("root/x", b'2', "d/../../escape")],
            vec![("root/fifo", b'6', "")],
            vec![("root/dev", b'3', "")],
            vec![("other/a", b'0', "x")],
            vec![("root/big", b'0', big.as_str())],
        ];
        for entries in cases {
            let dir = crate::testutil::scratch("b-tar");
            assert!(extract(&tar(&entries)[..], &dir, "root", 1024).is_err(), "{entries:?}");
            fs::remove_dir_all(dir).unwrap();
        }
        for path in ["/abs", "../parent", "a/../../x", "C:/drive", "a\\b"] {
            assert!(relative(Path::new(path)).is_err());
        }
    }
    #[test]
    fn safe_internal_links_are_kept_and_readable() {
        let dir = crate::testutil::scratch("b-tar-ok");
        let entries =
            [("root/lib/a", b'0', "payload"), ("root/bin/a", b'2', "../lib/a"), ("root/c", b'1', "root/lib/a")];
        extract(&tar(&entries)[..], &dir, "root", 1024).unwrap();
        assert_eq!(fs::read(dir.join("bin/a")).unwrap(), b"payload");
        assert!(fs::symlink_metadata(dir.join("bin/a")).unwrap().is_symlink());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn wrong_digest_never_unpacks_and_active_leases_block_installs() {
        let dir = crate::testutil::scratch("b-lock");
        let (archive, pin) = synthetic(&dir, "rev-a", &[("root/ok", b'0', "x")]);
        let wrong = ArtifactPin { sha256: super::super::pins::ENGINE.sha256, ..pin };
        let err = install(&dir, wrong, &archive, has_ok).unwrap_err();
        assert!(err.message.contains("SHA-256"));
        assert!(!dir.join("rev-a").exists() && !dir.join(".rev-a.stage").exists());
        let guard = lease(&dir, "rev-a").unwrap();
        assert!(matches!(install(&dir, pin, &archive, has_ok).unwrap_err().kind, ErrorKind::Busy));
        let other = lease(&dir, "rev-a").unwrap();
        drop((other, guard));
        let installer = exclusive(&dir, "rev-a").unwrap();
        assert!(exclusive(&dir, "rev-a").is_err());
        assert!(matches!(open(&dir, pin, has_ok).unwrap_err().kind, ErrorKind::Busy));
        drop(installer);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn installed_tree_is_read_only_and_reopens_without_its_archive() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::testutil::scratch("b-install");
        let (archive, pin) = synthetic(&dir, "rev-a", &[("root/ok", b'0', "x"), ("root/bin/tool", b'0', "y")]);
        assert!(open(&dir, pin, has_ok).unwrap().is_none());
        let active = install(&dir, pin, &archive, has_ok).unwrap();
        assert_eq!(fs::metadata(active.join("ok")).unwrap().permissions().mode() & 0o222, 0);
        assert_eq!(fs::metadata(active.join("bin")).unwrap().permissions().mode() & 0o222, 0);
        assert!(fs::write(active.join("bin/new"), b"x").is_err());
        fs::remove_file(&archive).unwrap();
        assert_eq!(open(&dir, pin, has_ok).unwrap(), Some(active.clone()));
        // A tampered receipt (or a failed quick check) means install again, not trust.
        fs::write(dir.join(".rev-a.receipt"), "rev-a 00\n").unwrap();
        assert!(open(&dir, pin, has_ok).unwrap().is_none());
        remove(&dir).unwrap();
    }
    #[test]
    fn failed_validation_restores_the_previous_tree() {
        let dir = crate::testutil::scratch("b-rollback");
        let (archive, pin) = synthetic(&dir, "rev-a", &[("root/ok", b'0', "first")]);
        let active = install(&dir, pin, &archive, has_ok).unwrap();
        // Same revision, a broken tree in the new archive, and the live one goes bad: the reinstall must fail and
        // leave the old tree in place, never half of the new one.
        let (bad, bad_pin) = synthetic(&dir, "rev-a", &[("root/other", b'0', "x")]);
        let picky = |p: &Path| {
            if fs::read(p.join("ok")).is_ok_and(|b| b == b"second") {
                Ok(())
            } else {
                has_ok(p).and(Err(invalid("old")))
            }
        };
        assert!(install(&dir, bad_pin, &bad, picky).is_err());
        assert_eq!(fs::read(active.join("ok")).unwrap(), b"first");
        assert!(!dir.join(".rev-a.stage").exists() && !dir.join(".rev-a.rollback").exists());
        assert_eq!(open(&dir, pin, has_ok).unwrap(), Some(active.clone()), "the old, checked tree stays trusted");
        assert!(open(&dir, bad_pin, has_ok).unwrap().is_none(), "no receipt for a tree that failed its check");
        // Repair: the full check renews a receipt for a good tree and withdraws it from a bad one.
        assert!(revalidate(&dir, pin, has_ok).unwrap().is_some());
        assert!(revalidate(&dir, pin, |_| Err(invalid("tampered"))).unwrap().is_none());
        assert!(open(&dir, pin, has_ok).unwrap().is_none());
        remove(&dir).unwrap();
    }
    #[test]
    fn recovery_at_every_rename_boundary() {
        let dir = crate::testutil::scratch("b-recovery");
        let active = dir.join("engine");
        let backup = dir.join(".engine.rollback");
        // Before the first rename: nothing to recover.
        fs::create_dir(&active).unwrap();
        fs::write(active.join("ok"), b"old").unwrap();
        recover(&dir, "engine", &has_ok).unwrap();
        assert_eq!(fs::read(active.join("ok")).unwrap(), b"old");
        // Between the renames: the live tree is only in the rollback copy.
        fs::rename(&active, &backup).unwrap();
        recover(&dir, "engine", &has_ok).unwrap();
        assert_eq!(fs::read(active.join("ok")).unwrap(), b"old");
        // After the second rename, before the rollback copy is gone: keep the new tree.
        fs::rename(&active, &backup).unwrap();
        fs::create_dir(&active).unwrap();
        fs::write(active.join("ok"), b"new").unwrap();
        recover(&dir, "engine", &has_ok).unwrap();
        assert!(!backup.exists());
        assert_eq!(fs::read(active.join("ok")).unwrap(), b"new");
        // A half-renamed, incomplete tree beside the rollback copy loses to it.
        fs::rename(&active, &backup).unwrap();
        fs::create_dir(&active).unwrap();
        recover(&dir, "engine", &has_ok).unwrap();
        assert_eq!(fs::read(active.join("ok")).unwrap(), b"new");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn concurrent_installs_never_mix_or_break_a_tree() {
        let dir = crate::testutil::scratch("b-concurrent");
        let (archive, pin) = synthetic(&dir, "rev-a", &[("root/ok", b'0', "x"), ("root/lib/a", b'0', "y")]);
        let barrier = Arc::new(Barrier::new(4));
        let results: Vec<_> = (0..4)
            .map(|_| {
                let (dir, archive, barrier) = (dir.clone(), archive.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    install(&dir, pin, &archive, has_ok)
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect();
        assert!(results.iter().any(|r| r.is_ok()));
        for r in &results {
            if let Err(e) = r {
                assert!(matches!(e.kind, ErrorKind::Busy), "{e}");
            }
        }
        assert_eq!(fs::read(dir.join("rev-a/lib/a")).unwrap(), b"y");
        assert!(open(&dir, pin, has_ok).unwrap().is_some());
        remove(&dir).unwrap();
    }
    #[test]
    fn a_changed_revision_installs_beside_and_prunes_only_idle_old_trees() {
        let dir = crate::testutil::scratch("b-prune");
        let (a, pin_a) = synthetic(&dir, "rev-a", &[("root/ok", b'0', "a")]);
        let (b, pin_b) = synthetic(&dir, "rev-b", &[("root/ok", b'0', "b")]);
        install(&dir, pin_a, &a, has_ok).unwrap();
        install(&dir, pin_b, &b, has_ok).unwrap();
        let held = lease(&dir, "rev-a").unwrap();
        assert!(prune(&dir, "rev-b").unwrap().is_empty());
        assert!(dir.join("rev-a/ok").exists());
        drop(held);
        assert_eq!(prune(&dir, "rev-b").unwrap(), vec!["rev-a".to_string()]);
        assert!(!dir.join("rev-a").exists() && !dir.join(".rev-a.receipt").exists());
        assert!(dir.join(".rev-a.lock").exists(), "lock inodes are never unlinked");
        assert_eq!(fs::read(dir.join("rev-b/ok")).unwrap(), b"b");
        remove(&dir).unwrap();
    }
    #[test]
    fn a_native_child_keeps_assets_busy_after_supervisor_lease_is_lost() {
        let dir = crate::testutil::scratch("b-orphan");
        let (archive, pin) = synthetic(&dir, "rev-a", &[("root/ok", b'0', "x")]);
        let active = install(&dir, pin, &archive, has_ok).unwrap();
        let mut child =
            std::process::Command::new("/bin/sleep").arg("30").env("STEAM_COMPAT_TOOL_PATHS", &active).spawn().unwrap();
        fs::remove_file(dir.join(".rev-a.receipt")).unwrap();
        let result = install(&dir, pin, &archive, has_ok);
        let pruned = prune(&dir, "rev-b");
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(matches!(result.unwrap_err().kind, ErrorKind::Busy));
        assert!(pruned.unwrap().is_empty());
        assert_eq!(fs::read(active.join("ok")).unwrap(), b"x");
        remove(&dir).unwrap();
    }

    #[tokio::test]
    async fn download_rejects_wrong_bytes_without_overwriting_verified_cache() {
        use crate::testutil::{Reply, Server};
        let dir = crate::testutil::scratch("b-download");
        let dest = dir.join("archive");
        fs::write(&dest, b"old verified cache").unwrap();
        let server = Server::start().await;
        let progress = Progress::default();
        let url = Box::leak(server.route("/artifact", Reply::ok(b"wrong payload".to_vec())).into_boxed_str());
        let pin = ArtifactPin { url, size: 13, ..super::super::pins::ENGINE };
        assert!(download(&reqwest::Client::new(), pin, &dest, &progress, &Cancel::default()).await.is_err());
        let longer = ArtifactPin { size: 4, ..pin };
        assert!(download(&reqwest::Client::new(), longer, &dest, &progress, &Cancel::default()).await.is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"old verified cache");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        let cancelled = Cancel::default();
        cancelled.cancel();
        assert!(matches!(
            download(&reqwest::Client::new(), pin, &dest, &progress, &cancelled).await.unwrap_err().kind,
            ErrorKind::Cancelled
        ));
        // The right bytes land, whole.
        let body = b"pinned bytes".to_vec();
        let sha256 = Box::leak(hex::encode(Sha256::digest(&body)).into_boxed_str());
        let url = Box::leak(server.route("/good", Reply::ok(body.clone())).into_boxed_str());
        let good = ArtifactPin { url, sha256, size: body.len() as u64, ..pin };
        download(&reqwest::Client::new(), good, &dest, &progress, &Cancel::default()).await.unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn ensure_downloads_installs_deletes_the_archive_and_then_opens_offline() {
        use crate::testutil::{Reply, Server};
        let dir = crate::testutil::scratch("b-ensure");
        let (archive, pin) = synthetic(&dir, "rev-a", &[("root/ok", b'0', "x")]);
        let server = Server::start().await;
        let url = Box::leak(server.route("/a.tar.gz", Reply::ok(fs::read(&archive).unwrap())).into_boxed_str());
        let pin = ArtifactPin { url, ..pin };
        let (root, cache) = (dir.join("engine"), dir.join("cache"));
        // A stale, wrong cached archive is replaced by one fresh download.
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("rev-a.tar.gz"), b"stale").unwrap();
        let progress = std::sync::Arc::new(Progress::default());
        let client = reqwest::Client::new();
        let at = |full| Ensure { dir: &root, cache: &cache, pin, full };
        let checks: Checks = ("a test artifact", has_ok, has_ok);
        let path = ensure(at(false), &client, &progress, &Cancel::default(), checks).await.unwrap();
        assert_eq!(path, root.join("rev-a"));
        assert_eq!(server.hits("/a.tar.gz"), 1);
        assert!(!cache.join("rev-a.tar.gz").exists(), "the archive goes once installed");
        assert_eq!(progress.bytes_done.load(Ordering::Relaxed), pin.size);
        // Warm, and a full check of a good tree: no network.
        ensure(at(false), &client, &progress, &Cancel::default(), checks).await.unwrap();
        ensure(at(true), &client, &progress, &Cancel::default(), checks).await.unwrap();
        // A lost receipt costs a full check, not a download.
        fs::remove_file(root.join(".rev-a.receipt")).unwrap();
        ensure(at(false), &client, &progress, &Cancel::default(), checks).await.unwrap();
        assert!(root.join(".rev-a.receipt").exists());
        assert_eq!(server.hits("/a.tar.gz"), 1);
        remove(&dir).unwrap();
    }
}
