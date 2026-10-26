//! The downloader for pack files. Packs check mods with MD5 (the Java edition tells the kind of checksum from its
//! length: 32 characters MD5, 40 SHA-1, 64 SHA-256, 128 SHA-512), which the shared game downloader does not, so
//! pack files go through this one. It reports through the same `Progress` the window already shows.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use md5::Md5;
use sha1::{Digest, Sha1};
use sha2::{Sha256, Sha512};
use tokio::io::AsyncWriteExt;

use crate::http::client;
use crate::install::Progress;

const ATTEMPTS: u32 = 3;

#[derive(Clone, Debug)]
pub struct Dl {
    pub url: String,
    pub dest: PathBuf,
    /// A checksum as hex; the kind is told by its length.
    pub hash: Option<String>,
    pub size: Option<u64>,
    /// A file that fails its checksum is kept if it is not empty (the Java edition's `ignoreFailures`).
    pub ignore_failures: bool,
    /// A pack's mod: kept when it arrives whole every time but its checksum still disagrees, and the player is told
    /// (see `install::Job::keep_on_mismatch`). Libraries and installers never are.
    pub keep_on_mismatch: bool,
    /// After the file is in place, unpack it into this folder (Technic Solder mods are zips that unpack into the
    /// instance).
    pub unzip_to: Option<PathBuf>,
}

impl Dl {
    pub fn new(url: impl Into<String>, dest: impl Into<PathBuf>) -> Dl {
        Dl {
            url: url.into(),
            dest: dest.into(),
            hash: None,
            size: None,
            ignore_failures: false,
            keep_on_mismatch: false,
            unzip_to: None,
        }
    }

    pub fn hash(mut self, hash: Option<&str>) -> Dl {
        self.hash = hash.map(str::trim).filter(|h| !h.is_empty() && *h != "-").map(str::to_lowercase);
        self
    }

    pub fn size(mut self, size: i64) -> Dl {
        self.size = Some(size).filter(|s| *s > 0).map(|s| s as u64);
        self
    }

    pub fn ignore_failures(mut self) -> Dl {
        self.ignore_failures = true;
        self
    }

    pub fn keep_on_mismatch(mut self) -> Dl {
        self.keep_on_mismatch = true;
        self
    }

    pub fn unzip_to(mut self, dir: impl Into<PathBuf>) -> Dl {
        self.unzip_to = Some(dir.into());
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HashKind {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl HashKind {
    pub fn of(hash: &str) -> Option<HashKind> {
        match hash.len() {
            32 => Some(HashKind::Md5),
            40 => Some(HashKind::Sha1),
            64 => Some(HashKind::Sha256),
            128 => Some(HashKind::Sha512),
            _ => None,
        }
    }
}

enum Hasher {
    Md5(Md5),
    Sha1(Sha1),
    Sha256(Sha256),
    Sha512(Sha512),
}

impl Hasher {
    fn new(kind: HashKind) -> Hasher {
        match kind {
            HashKind::Md5 => Hasher::Md5(Md5::new()),
            HashKind::Sha1 => Hasher::Sha1(Sha1::new()),
            HashKind::Sha256 => Hasher::Sha256(Sha256::new()),
            HashKind::Sha512 => Hasher::Sha512(Sha512::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Md5(h) => h.update(data),
            Hasher::Sha1(h) => h.update(data),
            Hasher::Sha256(h) => h.update(data),
            Hasher::Sha512(h) => h.update(data),
        }
    }

    fn finish(self) -> String {
        match self {
            Hasher::Md5(h) => hex::encode(h.finalize()),
            Hasher::Sha1(h) => hex::encode(h.finalize()),
            Hasher::Sha256(h) => hex::encode(h.finalize()),
            Hasher::Sha512(h) => hex::encode(h.finalize()),
        }
    }
}

/// The checksum of a file on disk, of the kind that `like` is.
pub fn hash_file(path: &Path, like: &str) -> Option<String> {
    use std::io::Read;
    let kind = HashKind::of(like)?;
    let mut h = Hasher::new(kind);
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finish())
}

/// Whether the file already on disk is the one wanted, so it need not be fetched again. With a checksum the
/// checksum decides; without one a file of the right size (or any non-empty file when the size is unknown) is
/// trusted.
pub fn is_current(job: &Dl) -> bool {
    let Ok(meta) = std::fs::metadata(&job.dest) else { return false };
    if !meta.is_file() {
        return false;
    }
    if let Some(hash) = &job.hash {
        if HashKind::of(hash).is_some() {
            return hash_file(&job.dest, hash).is_some_and(|got| got.eq_ignore_ascii_case(hash));
        }
    }
    match job.size {
        Some(size) => meta.len() == size,
        None => meta.len() > 0,
    }
}

/// Fetch every job whose file is missing or wrong. Returns how many files were downloaded. Files that were
/// already good are still unpacked when the job asks for it.
pub async fn download_all(jobs: Vec<Dl>, progress: &Arc<Progress>) -> Result<usize> {
    let jobs: Vec<(Dl, bool)> = tokio::task::spawn_blocking(move || {
        let mut seen = std::collections::HashSet::new();
        jobs.into_iter()
            .filter(|j| seen.insert(j.dest.clone()))
            .map(|j| {
                let current = is_current(&j);
                (j, current)
            })
            .collect::<Vec<_>>()
    })
    .await?;
    let missing: Vec<&Dl> = jobs.iter().filter(|(_, ok)| !ok).map(|(j, _)| j).collect();
    progress.files_total.store(missing.len() as u64, Ordering::Relaxed);
    progress.files_done.store(0, Ordering::Relaxed);
    progress.bytes_done.store(0, Ordering::Relaxed);
    progress.bytes_total.store(missing.iter().map(|j| j.size.unwrap_or(65_536)).sum(), Ordering::Relaxed);

    let count = missing.len();
    // as many at once as the game's downloader (the setting), the biggest first: a large file begun last is what
    // everything else waits for
    let threads = crate::net::download_threads(
        crate::http::concurrency(),
        std::thread::available_parallelism().map_or(4, |n| n.get()),
        jobs.len(),
    );
    let sizes: Vec<u64> = jobs.iter().map(|(j, current)| if *current { 0 } else { j.size.unwrap_or(0) }).collect();
    let mut slots: Vec<Option<(Dl, bool)>> = jobs.into_iter().map(Some).collect();
    let jobs: Vec<(Dl, bool)> =
        crate::net::start_order(&sizes, threads).into_iter().filter_map(|i| slots[i].take()).collect();
    let results: Vec<Result<()>> = futures::stream::iter(jobs)
        .map(|(job, current)| {
            let progress = progress.clone();
            async move {
                if progress.cancelled.load(Ordering::Relaxed) {
                    bail!("cancelled");
                }
                if !current {
                    let r = download_one(&job, &progress).await;
                    progress.files_done.fetch_add(1, Ordering::Relaxed);
                    r?;
                }
                if let Some(dir) = job.unzip_to.clone() {
                    let zip = job.dest.clone();
                    tokio::task::spawn_blocking(move || super::fsx::unzip(&zip, &dir)).await??;
                }
                Ok(())
            }
        })
        .buffer_unordered(threads)
        .collect()
        .await;

    let failures: Vec<String> = results.into_iter().filter_map(|r| r.err()).map(|e| format!("{e:#}")).collect();
    if !failures.is_empty() {
        bail!("{} file(s) couldn't be downloaded. First problem: {}", failures.len(), failures[0]);
    }
    Ok(count)
}

async fn download_one(job: &Dl, progress: &Progress) -> Result<()> {
    if job.url.is_empty() {
        bail!("{} has no address to download it from", job.dest.display());
    }
    if let Some(dir) = job.dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let part = PathBuf::from(format!("{}.part", job.dest.display()));
    let mut last: Option<anyhow::Error> = None;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            let base = if cfg!(test) { 5 } else { 500 };
            tokio::time::sleep(Duration::from_millis(base * 2u64.pow(attempt - 1))).await;
        }
        if progress.cancelled.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        let mut counted = 0u64;
        match fetch(&job.url, &part, job.hash.as_deref(), &mut counted, progress).await {
            Ok(Verdict::Good) => {
                tokio::fs::rename(&part, &job.dest).await.with_context(|| format!("saving {}", job.dest.display()))?;
                settle(job, counted, progress);
                return Ok(());
            }
            Ok(Verdict::Mismatch { expected, got }) => {
                progress
                    .bytes_done
                    .fetch_sub(counted.min(progress.bytes_done.load(Ordering::Relaxed)), Ordering::Relaxed);
                let whole = counted > 0 && job.size.is_none_or(|s| counted == s);
                if attempt + 1 == ATTEMPTS && counted > 0 && (job.ignore_failures || (job.keep_on_mismatch && whole)) {
                    tokio::fs::rename(&part, &job.dest)
                        .await
                        .with_context(|| format!("saving {}", job.dest.display()))?;
                    if !job.ignore_failures {
                        let name = job.dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        crate::logs::warn(format!("Kept {name} although its checksum is {got}, not {expected}"));
                        progress.note(name);
                    }
                    return Ok(());
                }
                let _ = tokio::fs::remove_file(&part).await;
                last = Some(anyhow::anyhow!("the file arrived damaged (expected {expected}, got {got})"));
            }
            Err(e) => {
                progress
                    .bytes_done
                    .fetch_sub(counted.min(progress.bytes_done.load(Ordering::Relaxed)), Ordering::Relaxed);
                let _ = tokio::fs::remove_file(&part).await;
                last = Some(e);
            }
        }
    }
    if job.ignore_failures {
        // the Java edition goes on without a file it was told it may do without
        return Ok(());
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("download failed"))).with_context(|| format!("downloading {}", job.url))
}

/// Correct the books when the real size differs from the planned one.
fn settle(job: &Dl, counted: u64, progress: &Progress) {
    if job.size.is_none() {
        progress.bytes_total.fetch_add(counted.saturating_sub(65_536), Ordering::Relaxed);
    }
}

enum Verdict {
    Good,
    Mismatch { expected: String, got: String },
}

async fn fetch(url: &str, part: &Path, hash: Option<&str>, counted: &mut u64, progress: &Progress) -> Result<Verdict> {
    let r = client().get(url).send().await?;
    if !r.status().is_success() {
        bail!("HTTP {}", r.status());
    }
    let mut hasher = hash.and_then(HashKind::of).map(Hasher::new);
    let mut file =
        tokio::io::BufWriter::with_capacity(crate::install::WRITE_BUFFER, tokio::fs::File::create(part).await?);
    let mut stream = r.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if let Some(h) = hasher.as_mut() {
            h.update(&chunk);
        }
        file.write_all(&chunk).await?;
        *counted += chunk.len() as u64;
        progress.bytes_done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        if progress.cancelled.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
    }
    file.flush().await?;
    if let (Some(h), Some(expected)) = (hasher, hash) {
        let got = h.finish();
        if !got.eq_ignore_ascii_case(expected) {
            return Ok(Verdict::Mismatch { expected: expected.to_string(), got });
        }
    }
    Ok(Verdict::Good)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An ATLauncher or Technic mod that arrives whole every time but with another checksum than the pack lists is
    /// kept and said; one that doesn't have the listed size is refused.
    #[test]
    fn a_pack_mod_with_an_out_of_date_checksum_is_kept_and_said() {
        use crate::testutil::{Reply, Server, runtime, scratch};
        runtime().block_on(async {
            let server = Server::start().await;
            let dir = scratch("packdl-keep");
            let served = b"the mod the site serves now".to_vec();
            let url = server.route("/mod.jar", Reply::ok(served.clone()));
            let md5_of_another = "5d41402abc4b2a76b9719d911017c592";
            let job = Dl::new(&url, dir.join("mod.jar")).hash(Some(md5_of_another)).size(served.len() as i64);
            let progress = Progress::new();
            download_all(vec![job.clone().keep_on_mismatch()], &progress).await.unwrap();
            assert_eq!(std::fs::read(dir.join("mod.jar")).unwrap(), served);
            assert_eq!(progress.take_notes(), vec!["mod.jar".to_string()]);

            // listed as a different size: not the same file, so not kept
            let other = Dl::new(&url, dir.join("other.jar")).hash(Some(md5_of_another)).size(9999).keep_on_mismatch();
            assert!(download_all(vec![other], &Progress::new()).await.is_err());
            assert!(!dir.join("other.jar").exists());
            // a library or an installer (no keep_on_mismatch) is never kept on a wrong checksum
            let strict = Dl::new(&url, dir.join("lib.jar")).hash(Some(md5_of_another));
            assert!(download_all(vec![strict], &Progress::new()).await.is_err());
            assert!(!dir.join("lib.jar").exists());
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    #[test]
    fn checksum_kind_comes_from_the_length() {
        assert_eq!(HashKind::of("d21c2a06a4e6b175aa01e328f38a1182"), Some(HashKind::Md5));
        assert_eq!(HashKind::of("f571480103066690dd28ba6d8a1f90e946976915"), Some(HashKind::Sha1));
        assert_eq!(HashKind::of(&"a".repeat(64)), Some(HashKind::Sha256));
        assert_eq!(HashKind::of(&"a".repeat(128)), Some(HashKind::Sha512));
        assert_eq!(HashKind::of("abc"), None);
    }

    #[test]
    fn existing_files_are_judged_by_checksum_then_size() {
        let dir = std::env::temp_dir().join(format!("bf-dl-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.jar");
        std::fs::write(&f, b"hello").unwrap();
        // md5("hello")
        let good = Dl::new("u", &f).hash(Some("5d41402abc4b2a76b9719d911017c592"));
        assert!(is_current(&good));
        let bad = Dl::new("u", &f).hash(Some("00000000000000000000000000000000"));
        assert!(!is_current(&bad));
        assert!(is_current(&Dl::new("u", &f).size(5)));
        assert!(!is_current(&Dl::new("u", &f).size(6)));
        assert!(is_current(&Dl::new("u", &f)));
        assert!(!is_current(&Dl::new("u", dir.join("missing.jar"))));
        let _ = std::fs::remove_dir_all(dir);
    }
}
