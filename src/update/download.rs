//! Fetching the update file: with progress, a size check, a SHA-256 check and a way to cancel.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use super::release::{Asset, Release, is_sha256_hex};

/// The checksum files a release may carry, best name first.
pub const CHECKSUM_FILES: [&str; 2] = ["SHA256SUMS.txt", "SHA256SUMS"];

/// How far a download is. Shared with the window, which draws it.
#[derive(Default, Debug)]
pub struct DownloadProgress {
    pub done: AtomicU64,
    /// 0 when the size isn't known.
    pub total: AtomicU64,
}

impl DownloadProgress {
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 { 0.0 } else { (self.done.load(Ordering::Relaxed) as f32 / total as f32).clamp(0.0, 1.0) }
    }
}

/// The error of a download the player stopped.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("The download was cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// Whether `err` is a download the player cancelled.
pub fn was_cancelled(err: &anyhow::Error) -> bool {
    err.downcast_ref::<Cancelled>().is_some()
}

/// Looks `name` up in the text of a checksum file (`<sha256>  <file name>` per line, the name optionally marked
/// with a `*` for binary mode).
pub fn find_checksum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim().trim_start_matches('*');
        let file = file.rsplit(['/', '\\']).next().unwrap_or(file);
        (file == name && is_sha256_hex(hash)).then(|| hash.to_ascii_lowercase())
    })
}

/// The SHA-256 the release promises for `asset`: its own `digest` when GitHub computed one, otherwise the line in the
/// release's checksum file. `None` when the release says nothing.
pub async fn expected_sha256(release: &Release, asset: &Asset) -> Option<String> {
    if let Some(hex) = asset.sha256() {
        return Some(hex);
    }
    for wanted in CHECKSUM_FILES {
        let Some(file) = release.assets.iter().find(|a| a.name.eq_ignore_ascii_case(wanted)) else { continue };
        let bytes = match crate::http::get_bytes(&file.browser_download_url).await {
            Ok(b) => b,
            Err(e) => {
                crate::logs::warn(format!("Couldn't read {wanted} from the release: {e:#}"));
                continue;
            }
        };
        if let Some(hex) = find_checksum(&String::from_utf8_lossy(&bytes), &asset.name) {
            return Some(hex);
        }
    }
    None
}

/// The text of the SHA-256 of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// The SHA-256 of a file, read in pieces.
pub fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).with_context(|| format!("Couldn't open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            return Ok(hex::encode(hasher.finalize()));
        }
        hasher.update(&buffer[..n]);
    }
}

/// Checks a finished download against what the release promised.
pub fn verify(done: u64, digest: &str, expected_size: u64, expected_sha256: Option<&str>) -> Result<()> {
    if expected_size > 0 && done != expected_size {
        bail!("The download is incomplete ({done} of {expected_size} bytes). Check your connection and try again.");
    }
    if let Some(expected) = expected_sha256 {
        if !expected.eq_ignore_ascii_case(digest) {
            bail!("The download is damaged (its checksum doesn't match the release). Try again.");
        }
    }
    Ok(())
}

/// A file name that is safe to create inside our own folder.
fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String =
        base.chars().map(|c| if c.is_control() || ":*?\"<>|".contains(c) { '_' } else { c }).collect();
    if cleaned.trim_matches('.').is_empty() { "update".to_string() } else { cleaned }
}

/// Downloads `asset` into `folder`, checking its size and checksum. Reports to `progress` and stops with
/// [`Cancelled`] when `cancel` is set. The file only gets its final name once it is complete and checked.
pub async fn download(
    asset: &Asset,
    expected_sha256: Option<&str>,
    folder: &Path,
    progress: &DownloadProgress,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    tokio::fs::create_dir_all(folder).await.with_context(|| format!("Couldn't create {}", folder.display()))?;
    let name = safe_name(&asset.name);
    let target = folder.join(&name);
    let part = folder.join(format!("{name}.part"));
    let result = fetch(asset, expected_sha256, &part, progress, cancel).await;
    match result {
        Ok(()) => {
            tokio::fs::rename(&part, &target).await.with_context(|| format!("Couldn't save {}", target.display()))?;
            Ok(target)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

async fn fetch(
    asset: &Asset,
    expected_sha256: Option<&str>,
    part: &Path,
    progress: &DownloadProgress,
    cancel: &AtomicBool,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut response = crate::http::client()
        .get(&asset.browser_download_url)
        .header("Accept", "application/octet-stream")
        .send()
        .await
        .context("Couldn't reach the download server")?;
    if !response.status().is_success() {
        bail!("The download failed: the server answered {}", response.status());
    }
    let total = response.content_length().filter(|n| *n > 0).unwrap_or(asset.size);
    progress.done.store(0, Ordering::Relaxed);
    progress.total.store(total, Ordering::Relaxed);

    let mut out = tokio::fs::File::create(part).await.with_context(|| format!("Couldn't write {}", part.display()))?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    while let Some(chunk) = response.chunk().await.context("The download was interrupted")? {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        out.write_all(&chunk).await.context("Couldn't write the downloaded file (is the disk full?)")?;
        hasher.update(&chunk);
        done += chunk.len() as u64;
        progress.done.store(done, Ordering::Relaxed);
    }
    out.flush().await?;
    drop(out);
    if cancel.load(Ordering::Relaxed) {
        return Err(Cancelled.into());
    }
    verify(done, &hex::encode(hasher.finalize()), asset.size, expected_sha256)
}
