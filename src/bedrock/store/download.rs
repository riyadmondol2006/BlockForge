//! Ported from Xodus CLI commands/streaming.rs; failures cannot publish an install receipt.
use super::*;
use futures::{StreamExt, TryStreamExt};
use msixvc::{
    streaming::{HttpRead, PrefixCacheFile},
    xvd::{SegmentFile, XvdFile},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::File,
    sync::atomic::Ordering,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncWriteExt};

pub fn disk_length(length: u64, encrypted: bool) -> u64 {
    if encrypted { length.div_ceil(4096).saturating_mul(4096) } else { length }
}
pub const METADATA: &str = ".xodus-streaming.msixvc";
#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub offset: u64,
    pub length: u64,
    pub hashes: Vec<[u8; 20]>,
    pub encrypted: bool,
    pub disk_sha256: String,
    pub local_length: u64,
    pub patch_revision: Option<String>,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct Receipt {
    pub schema: u32,
    pub adapter: u32,
    pub build: BuildId,
    pub content_id: String,
    pub files: BTreeMap<String, Entry>,
}
/// Keep the build lease through prefix preparation and staging. Fields cannot be constructed by callers.
pub struct InstalledBuild {
    pub downloaded_files: u64,
    pub id: BuildId,
    pub content: PathBuf,
    pub(crate) receipt: Receipt,
    _lease: Arc<File>,
}
impl InstalledBuild {
    /// Called by fixups on an idle, exclusively leased generation after applying a known patch.
    /// Remote hashes remain unchanged; local bytes/length and patch identity are recorded atomically.
    pub async fn record_patch(&mut self, relative: &str, revision: &str) -> Result<()> {
        let name = safe_name(relative)?;
        if revision.is_empty() || revision.len() > 128 {
            return Err(err(ErrorKind::InvalidRequest, "Invalid patch revision"));
        }
        let mut receipt = self.receipt.clone();
        let entry = receipt.files.get_mut(&name).ok_or_else(|| integrity("Unknown patched file"))?;
        if entry.encrypted {
            return Err(err(ErrorKind::InvalidRequest, "Patch encrypted executables only after tmpfs staging"));
        }
        entry.disk_sha256 = disk_hash(&self.content.join(&name)).await?;
        entry.local_length = tokio::fs::metadata(self.content.join(&name)).await.map_err(io_error)?.len();
        entry.patch_revision = Some(revision.into());
        let root = self.content.parent().ok_or_else(|| integrity("Missing build root"))?;
        tokens::atomic(&root.join("install.json"), &serde_json::to_vec(&receipt).map_err(integrity)?)
            .map_err(io_error)?;
        self.receipt = receipt;
        Ok(())
    }
}
impl InstalledBuild {
    /// After the game-folder fixups ran on this idle, leased generation: record every file `wanted` names whose bytes
    /// no longer match the receipt (a patched or replaced original) as patch `revision`. Files the fixups added
    /// beside the game are not in the receipt and are left alone; encrypted images are never patched on disk.
    /// Returns how many were recorded.
    pub async fn record_patches(&mut self, revision: &str, wanted: impl Fn(&str) -> bool) -> Result<usize> {
        let names: Vec<(String, String)> = self
            .receipt
            .files
            .iter()
            .filter(|(name, entry)| !entry.encrypted && wanted(name))
            .map(|(name, entry)| (name.clone(), entry.disk_sha256.clone()))
            .collect();
        let mut recorded = 0;
        for (name, recorded_hash) in names {
            if disk_hash(&self.content.join(&name)).await? != recorded_hash {
                self.record_patch(&name, revision).await?;
                recorded += 1;
            }
        }
        Ok(recorded)
    }
    /// The files whose bytes on disk differ from the receipt (missing, cut off, changed): a local check of every
    /// file, no network. Repair uses it to decide whether the build must be fetched again.
    pub async fn damaged_files(&self, cancel: &Cancel) -> Result<Vec<String>> {
        let mut damaged = Vec::new();
        for (name, entry) in &self.receipt.files {
            cancelled(cancel)?;
            let path = self.content.join(name);
            let intact = tokio::fs::symlink_metadata(&path).await.is_ok_and(|m| m.len() == entry.local_length)
                && disk_hash(&path).await.is_ok_and(|h| h == entry.disk_sha256);
            if !intact {
                damaged.push(name.clone());
            }
        }
        Ok(damaged)
    }
    /// Bytes of the encrypted images, the room their decrypted copies need when staged.
    pub fn encrypted_bytes(&self) -> u64 {
        self.receipt.files.values().filter(|e| e.encrypted).map(|e| e.length).sum()
    }
}
impl std::fmt::Debug for InstalledBuild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstalledBuild").field("id", &self.id).finish_non_exhaustive()
    }
}
fn build_root(ctx: &Context, id: &BuildId) -> Result<PathBuf> {
    uuid::Uuid::parse_str(&id.package_id).map_err(integrity)?;
    if id.version.split('.').count() != 4
        || !id.version.split('.').all(|part| !part.is_empty() && part.parse::<u32>().is_ok())
    {
        return Err(integrity("Invalid build"));
    }
    Ok(ctx
        .root
        .join("builds")
        .join(match id.edition {
            Edition::Release => "release",
            Edition::Preview => "preview",
        })
        .join(&id.version)
        .join(&id.package_id))
}
fn lease(ctx: &Context, id: &BuildId) -> Result<Arc<File>> {
    let root = ctx.root.join("locks");
    tokens::private_dir(&root)?;
    let file = tokens::private_open(&root.join(format!("build-{}", id.package_id)), true).map_err(io_error)?;
    file.try_lock().map_err(|_| err(ErrorKind::Busy, "Build is in use"))?;
    Ok(Arc::new(file))
}
pub async fn load(ctx: &Context, id: &BuildId) -> Result<InstalledBuild> {
    let lease = lease(ctx, id)?;
    load_with_lease(ctx, id, lease).await
}
async fn load_with_lease(ctx: &Context, id: &BuildId, lease: Arc<File>) -> Result<InstalledBuild> {
    let root = build_root(ctx, id)?;
    let receipt: Receipt = serde_json::from_slice(&tokio::fs::read(root.join("install.json")).await.map_err(io_error)?)
        .map_err(integrity)?;
    if receipt.schema != 1
        || receipt.adapter != 1
        || &receipt.build != id
        || receipt.content_id != catalog::content_id(id.edition)
    {
        return Err(integrity("Invalid receipt"));
    }
    let content = root.join("content");
    tokio::fs::File::open(content.join(METADATA))
        .await
        .map_err(|_| err(ErrorKind::Integrity, "Required encrypted MSIXVC metadata is missing; repair the build"))?;
    Ok(InstalledBuild { downloaded_files: 0, id: id.clone(), content, receipt, _lease: lease })
}

/// Windows paths are normalized once; reject traversal, aliases, streams and special devices.
pub fn safe_name(raw: &str) -> Result<String> {
    let name = raw.replace('\\', "/");
    if name.len() > 4096 || name.chars().any(|c| c.is_control() || ":|<>\"*?".contains(c)) {
        return Err(integrity("Unsafe package path"));
    }
    for component in name.split('/') {
        let stem = component.split('.').next().unwrap_or("").to_ascii_uppercase();
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with([' ', '.'])
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && stem.as_bytes()[3].is_ascii_digit())
            || component.to_ascii_lowercase().starts_with(".xodus")
        {
            return Err(integrity("Unsafe package path"));
        }
    }
    Ok(name)
}
pub async fn metadata<R: AsyncRead + AsyncSeek + Unpin>(
    xvd: &XvdFile,
    reader: &mut R,
) -> Result<HashMap<String, SegmentFile>> {
    let mut files = HashMap::new();
    for (name, segment) in xvd.parse_user_package_files(&mut *reader).await.map_err(integrity)? {
        if name == "SegmentMetadata.bin" {
            files = xvd.parse_segment_metadata(&mut *reader, &segment).await.map_err(integrity)?;
        }
    }
    files.extend(xvd.parse_ntfs_segment_metadata(reader, !files.is_empty()).await.map_err(integrity)?);
    let mut names = HashSet::new();
    let mut normalized = HashMap::new();
    for (name, segment) in files {
        let name = safe_name(&name)?;
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(integrity("Duplicate package path"));
        }
        normalized.insert(name, segment);
    }
    if normalized.is_empty() {
        return Err(integrity("Empty package"));
    }
    Ok(normalized)
}
pub async fn disk_hash(path: &Path) -> Result<String> {
    let meta = tokio::fs::symlink_metadata(path).await.map_err(io_error)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(integrity("Unsafe local file"));
    }
    let mut file = tokio::fs::File::open(path).await.map_err(io_error)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 128 * 1024];
    loop {
        let n = file.read(&mut buf).await.map_err(io_error)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}
pub async fn reusable(path: &Path, old: &Entry, segment: &SegmentFile) -> bool {
    old.length == segment.length
        && old.offset == segment.offset
        && old.encrypted == segment.keep_encrypted
        && !segment.data_hashs.is_empty()
        && old.hashes == segment.data_hashs
        && tokio::fs::symlink_metadata(path)
            .await
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && m.len() == old.local_length)
        && disk_hash(path).await.is_ok_and(|h| h == old.disk_sha256)
}
/// Names whose installed bytes still match both the new package and the old receipt.
pub(super) async fn reusable_files(
    old: Option<&InstalledBuild>,
    files: &HashMap<String, SegmentFile>,
    cancel: &Cancel,
) -> Result<HashSet<String>> {
    let Some(old) = old else { return Ok(HashSet::new()) };
    // Owned inputs keep the stream's future Send for any caller lifetime.
    let candidates: Vec<_> = files
        .iter()
        .filter_map(|(name, segment)| {
            let entry = old.receipt.files.get(name)?.clone();
            let segment = SegmentFile {
                offset: segment.offset,
                length: segment.length,
                data_hashs: segment.data_hashs.clone(),
                keep_encrypted: segment.keep_encrypted,
            };
            Some((name.clone(), old.content.join(name), entry, segment))
        })
        .collect();
    let checks = futures::stream::iter(candidates)
        .map(|(name, path, entry, segment)| async move { reusable(&path, &entry, &segment).await.then_some(name) })
        .buffer_unordered(4)
        .filter_map(std::future::ready)
        .collect::<HashSet<_>>();
    interrupt(cancel, async { Ok(checks.await) }).await
}
struct Pending(PathBuf);
impl Drop for Pending {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub async fn install(
    ctx: &Context,
    slot: &tokens::Slot,
    package: catalog::Package,
    progress: Arc<Progress>,
    cancel: &Cancel,
) -> Result<InstalledBuild> {
    let lease = lease(ctx, &package.build)?;
    let root = build_root(ctx, &package.build)?;
    let parent = root.parent().ok_or_else(|| integrity("Missing build parent"))?;
    tokens::private_dir(parent)?;
    let pending = parent.join(format!(".install-{}", uuid::Uuid::new_v4()));
    tokens::private_dir(&pending)?;
    let _cleanup = Pending(pending.clone());
    let out = pending.join("content");
    tokens::private_dir(&out)?;
    let mut old = load_with_lease(ctx, &package.build, lease.clone()).await.ok();
    *progress.stage.lock().unwrap_or_else(|e| e.into_inner()) = "Reading Store package metadata".into();
    interrupt(cancel, async {
        let http = HttpRead::open(ctx.client.clone(), &package.url, Some(|_, _| {})).await.map_err(network)?;
        if http.len() != package.bytes {
            return Err(integrity("Package size changed"));
        }
        let mut reader = PrefixCacheFile::new(http, package.bytes, out.join(METADATA)).await.map_err(io_error)?;
        let xvd = XvdFile::parse(&mut reader).await.map_err(integrity)?;
        if xvd.content_id().to_string() != catalog::content_id(package.build.edition) {
            return Err(integrity("Unexpected XVD content ID"));
        }
        let files = metadata(&xvd, &mut reader).await?;
        let needed = files
            .values()
            .try_fold(0u64, |total, s| total.checked_add(disk_length(s.length, s.keep_encrypted)))
            .ok_or_else(|| integrity("Package size overflow"))?;
        let space = rustix::fs::statvfs(&out).map_err(integrity)?;
        if space.f_bavail.saturating_mul(space.f_frsize) < needed {
            return Err(err(ErrorKind::NoSpace, "Not enough space for a staged Store install"));
        }
        // Hash each kept file once. When every file of the same build is intact, the
        // installed generation is the answer: no licence, no copy, no new generation.
        let reuse = reusable_files(old.as_ref(), &files, cancel).await?;
        if let Some(old) = old.take_if(|old| reuse.len() == files.len() && old.receipt.files.len() == files.len()) {
            progress.files_total.store(files.len() as u64, Ordering::Relaxed);
            progress.files_done.store(files.len() as u64, Ordering::Relaxed);
            return Ok(old);
        }
        // Header identity, never an untrusted catalogue parameter, drives licensing.
        let key = license::acquire(&ctx.client, slot, xvd.content_id().to_string(), cancel).await?;
        progress.files_total.store(files.len() as u64, Ordering::Relaxed);
        progress.bytes_total.store(files.values().map(|s| s.length).sum(), Ordering::Relaxed);
        *progress.stage.lock().unwrap_or_else(|e| e.into_inner()) = "Installing owned Store content".into();
        let downloaded = std::sync::atomic::AtomicU64::new(0);
        let entries = futures::stream::iter(files.into_iter().map(|(name, segment)| {
            let out = &out;
            let old = &old;
            let progress = &progress;
            let xvd = &xvd;
            let url = &package.url;
            let downloaded = &downloaded;
            let reuse = &reuse;
            async move {
                cancelled(cancel)?;
                if progress.cancelled.load(Ordering::Relaxed) {
                    return Err(err(ErrorKind::Cancelled, "Install cancelled"));
                }
                let target = out.join(&name);
                if let Some(parent) = target.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(io_error)?;
                }
                let mut reused = false;
                if let Some(old) = old
                    && reuse.contains(&name)
                {
                    tokio::fs::copy(old.content.join(&name), &target).await.map_err(io_error)?;
                    tokio::fs::File::open(&target).await.map_err(io_error)?.sync_all().await.map_err(io_error)?;
                    reused = true;
                }
                if !reused {
                    downloaded.fetch_add(1, Ordering::Relaxed);
                    let mut source = HttpRead::open_at(ctx.client.clone(), url, package.bytes, segment.offset)
                        .await
                        .map_err(network)?;
                    let mut file = tokio::fs::File::from_std(tokens::private_open(&target, true).map_err(io_error)?);
                    // HttpRead validates every range; Xodus's CLI HTTP downloader does not.
                    xvd.extract_file(&mut source, &mut file, &segment, key, |_, _| {}).await.map_err(integrity)?;
                    file.flush().await.map_err(io_error)?;
                    file.sync_all().await.map_err(io_error)?;
                }
                if reused {
                    let entry = old
                        .as_ref()
                        .and_then(|b| b.receipt.files.get(&name))
                        .ok_or_else(|| integrity("Missing reused entry"))?
                        .clone();
                    progress.files_done.fetch_add(1, Ordering::Relaxed);
                    progress.bytes_done.fetch_add(segment.length, Ordering::Relaxed);
                    return Ok((name, entry));
                }
                if tokio::fs::metadata(&target).await.map_err(io_error)?.len()
                    != disk_length(segment.length, segment.keep_encrypted)
                {
                    return Err(integrity("Truncated installed file"));
                }
                let hash = disk_hash(&target).await?;
                progress.files_done.fetch_add(1, Ordering::Relaxed);
                progress.bytes_done.fetch_add(segment.length, Ordering::Relaxed);
                Ok((
                    name,
                    Entry {
                        offset: segment.offset,
                        length: segment.length,
                        hashes: segment.data_hashs,
                        encrypted: segment.keep_encrypted,
                        disk_sha256: hash,
                        local_length: disk_length(segment.length, segment.keep_encrypted),
                        patch_revision: None,
                    },
                ))
            }
        }))
        .buffer_unordered(4)
        .try_collect::<BTreeMap<_, _>>()
        .await?;
        drop(reader); // all queued prefix writes are flushed by carried patch 0001
        cancelled(cancel)?;
        if progress.cancelled.load(Ordering::Relaxed) {
            return Err(err(ErrorKind::Cancelled, "Install cancelled"));
        }
        let receipt = Receipt {
            schema: 1,
            adapter: 1,
            build: package.build.clone(),
            content_id: xvd.content_id().to_string(),
            files: entries,
        };
        tokens::atomic(&pending.join("install.json"), &serde_json::to_vec(&receipt).map_err(integrity)?)
            .map_err(io_error)?;
        // Keep the prior generation after an interrupted promotion. Never erase it to retry.
        let previous = parent.join(format!(".previous-{}", uuid::Uuid::new_v4()));
        let hid_old = root.exists();
        if hid_old {
            std::fs::rename(&root, &previous).map_err(io_error)?;
        }
        // Commit is a short synchronous rename/fsync boundary on this backend worker.
        // There is no cancellation yield between hiding the old generation and publication.
        if let Err(e) = std::fs::rename(&pending, &root) {
            // Put the old generation back rather than leave no build at all.
            if hid_old {
                let _ = std::fs::rename(&previous, &root);
            }
            return Err(io_error(e));
        }
        std::fs::File::open(parent).map_err(io_error)?.sync_all().map_err(io_error)?;
        Ok(InstalledBuild {
            downloaded_files: downloaded.load(Ordering::Relaxed),
            id: package.build.clone(),
            content: root.join("content"),
            receipt,
            _lease: lease,
        })
    })
    .await
}
