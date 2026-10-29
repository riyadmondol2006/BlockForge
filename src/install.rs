//! Getting a Minecraft version onto the disk: libraries, the game jar, assets and the Mojang logging config.
//! Everything goes through one parallel downloader that skips what is already there.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tokio::io::AsyncWriteExt;

use crate::config::Settings;
use crate::http::{self, client};
use crate::model::{Features, VersionJson};
use crate::paths::Dirs;

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const RESOURCES_URL: &str = "https://resources.download.minecraft.net";

/// What an install is doing right now; the window polls it, the command line prints it.
#[derive(Default)]
pub struct Progress {
    pub stage: std::sync::Mutex<String>,
    pub files_total: AtomicU64,
    pub files_done: AtomicU64,
    pub bytes_total: AtomicU64,
    pub bytes_done: AtomicU64,
    pub cancelled: AtomicBool,
    /// Things the player should hear about once the job is done (a file kept although its checksum disagreed).
    pub notes: std::sync::Mutex<Vec<String>>,
    /// How fast the bytes come in, worked out now and then from `bytes_done` (see [`Progress::speed`]).
    speed: std::sync::Mutex<Speed>,
}

#[derive(Default)]
struct Speed {
    at: Option<std::time::Instant>,
    bytes: u64,
    per_sec: f64,
}

/// What to tell the player about pack files kept although their checksum disagreed (see `Job::keep_on_mismatch`).
pub fn kept_files_message(files: &[String]) -> String {
    const SHOWN: usize = 5;
    let mut names = files.iter().take(SHOWN).cloned().collect::<Vec<_>>().join(", ");
    if files.len() > SHOWN {
        names.push_str(&format!(" and {} more", files.len() - SHOWN));
    }
    let what = if files.len() == 1 { "1 file didn't".to_string() } else { format!("{} files didn't", files.len()) };
    format!(
        "{what} match the checksum the pack lists and were kept anyway: {names}. They downloaded completely; the \
         pack's own list is likely out of date. If something doesn't work, install the pack again."
    )
}

/// What a file with no size counts for in a download's total until the server says its size or it is in.
pub const SIZE_GUESS: u64 = 65_536;

/// A number of bytes for people: 845 KB, 12.3 MB, 1.42 GB.
pub fn human_bytes(n: u64) -> String {
    let b = n as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", (b / 1e3).max(if n > 0 { 1.0 } else { 0.0 }))
    }
}

impl Progress {
    /// Something to tell the player when the job ends.
    pub fn note(&self, text: impl Into<String>) {
        self.notes.lock().unwrap_or_else(|e| e.into_inner()).push(text.into());
    }

    /// The notes so far, emptied.
    pub fn take_notes(&self) -> Vec<String> {
        std::mem::take(&mut *self.notes.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Bytes per second lately: measured again when half a second has passed since the last look, and smoothed so
    /// the figure doesn't jump with every file.
    pub fn speed(&self) -> f64 {
        let now = std::time::Instant::now();
        let bytes = self.bytes_done.load(Ordering::Relaxed);
        let mut s = self.speed.lock().unwrap_or_else(|e| e.into_inner());
        match s.at {
            Some(at) if now.duration_since(at) >= Duration::from_millis(500) => {
                // a new set of files starts the counter again: nothing to measure across that
                let fresh = bytes.saturating_sub(s.bytes) as f64 / now.duration_since(at).as_secs_f64();
                s.per_sec = if s.per_sec == 0.0 || bytes < s.bytes { fresh } else { 0.6 * s.per_sec + 0.4 * fresh };
                s.at = Some(now);
                s.bytes = bytes;
            }
            Some(_) => {}
            None => {
                s.at = Some(now);
                s.bytes = bytes;
            }
        }
        s.per_sec
    }

    /// How far a download is, for people: `(percent and sizes, files and speed)`, such as ("66% · 210.3 MB of
    /// 318.0 MB", "120 of 187 files · 12.3 MB/s"). `None` while no files are being downloaded.
    pub fn summary(&self) -> Option<(String, String)> {
        let (done, total) = (self.files_done.load(Ordering::Relaxed), self.files_total.load(Ordering::Relaxed));
        if total == 0 {
            return None;
        }
        let (got, of) = (self.bytes_done.load(Ordering::Relaxed), self.bytes_total.load(Ordering::Relaxed));
        let sizes = match self.known_fraction() {
            Some(f) => format!("{}% \u{00B7} {} of {}", (f * 100.0).floor() as u32, human_bytes(got), human_bytes(of)),
            // more came than was planned and nobody knows how much is left: no per cent that says it is done
            None => format!("{} so far", human_bytes(got)),
        };
        let speed = self.speed();
        let mut files = format!("{done} of {total} files");
        if done < total && speed >= 1000.0 {
            files.push_str(&format!(" \u{00B7} {}/s", human_bytes(speed as u64)));
        }
        Some((sizes, files))
    }

    pub fn new() -> Arc<Progress> {
        Arc::new(Progress::default())
    }

    pub fn stage(&self, text: impl Into<String>) {
        *self.stage.lock().unwrap() = text.into();
    }

    pub fn stage_text(&self) -> String {
        self.stage.lock().unwrap().clone()
    }

    pub fn fraction(&self) -> f32 {
        let total = self.bytes_total.load(Ordering::Relaxed).max(1);
        let files = self.files_total.load(Ordering::Relaxed);
        if files == 0 {
            return 0.0;
        }
        (self.bytes_done.load(Ordering::Relaxed) as f32 / total as f32).clamp(0.0, 1.0)
    }

    /// [`Progress::fraction`], or `None` while more has arrived than was planned and files are still coming (a file
    /// whose size the server didn't say): a full bar would say it is done when it isn't.
    pub fn known_fraction(&self) -> Option<f32> {
        let over = self.bytes_done.load(Ordering::Relaxed) > self.bytes_total.load(Ordering::Relaxed);
        let open = self.files_done.load(Ordering::Relaxed) < self.files_total.load(Ordering::Relaxed);
        (!(over && open)).then(|| self.fraction())
    }

    /// Count a file as `actual` bytes in the total instead of what was `planned` for it (the guess for a file with
    /// no size), once the server says its size or once it is in.
    pub fn replan(&self, planned: &mut u64, actual: u64) {
        if actual >= *planned {
            self.bytes_total.fetch_add(actual - *planned, Ordering::Relaxed);
        } else {
            let less = (*planned - actual).min(self.bytes_total.load(Ordering::Relaxed));
            self.bytes_total.fetch_sub(less, Ordering::Relaxed);
        }
        *planned = actual;
    }

    fn reset_counts(&self) {
        self.files_total.store(0, Ordering::Relaxed);
        self.files_done.store(0, Ordering::Relaxed);
        self.bytes_total.store(0, Ordering::Relaxed);
        self.bytes_done.store(0, Ordering::Relaxed);
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub url: String,
    pub dest: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub executable: bool,
    /// A modpack's file: when it arrives complete every time but its checksum still disagrees, it is kept and the
    /// player is told, rather than failing the whole install (a pack's record of a file can be out of date when the
    /// file was replaced on the site). The game's own files, Java and libraries never are.
    pub keep_on_mismatch: bool,
}

impl Job {
    pub fn new(url: impl Into<String>, dest: impl Into<PathBuf>) -> Job {
        Job { url: url.into(), dest: dest.into(), sha1: None, size: None, executable: false, keep_on_mismatch: false }
    }

    pub fn checked(mut self, sha1: Option<&str>, size: Option<i64>) -> Job {
        self.sha1 = sha1.filter(|s| !s.is_empty()).map(str::to_string);
        self.size = size.filter(|s| *s > 0).map(|s| s as u64);
        self
    }
}

/// Whether a file on disk can be trusted without downloading it again. By default size is the test, which is
/// one `stat` per file; `verify` hashes it too.
pub(crate) fn is_good(job: &Job, verify: bool) -> bool {
    let Ok(meta) = std::fs::metadata(&job.dest) else { return false };
    if !meta.is_file() {
        return false;
    }
    if let Some(size) = job.size {
        if meta.len() != size {
            return false;
        }
    } else if meta.len() == 0 {
        return false;
    }
    if verify {
        if let Some(expected) = &job.sha1 {
            return std::fs::read(&job.dest)
                .map(|b| http::sha1_hex(&b).eq_ignore_ascii_case(expected))
                .unwrap_or(false);
        }
    }
    true
}

/// Download every job whose file is missing or wrong. Returns how many files were fetched.
pub async fn download_all(jobs: Vec<Job>, progress: &Arc<Progress>, verify: bool) -> Result<usize> {
    progress.reset_counts();
    // one pass over the disk, off the async threads, before any network is touched
    let missing: Vec<Job> = tokio::task::spawn_blocking(move || {
        let mut seen = std::collections::HashSet::new();
        jobs.into_iter().filter(|j| seen.insert(j.dest.clone())).filter(|j| !is_good(j, verify)).collect::<Vec<_>>()
    })
    .await?;
    if missing.is_empty() {
        return Ok(0);
    }
    progress.files_total.store(missing.len() as u64, Ordering::Relaxed);
    progress.bytes_total.store(missing.iter().map(|j| j.size.unwrap_or(SIZE_GUESS)).sum(), Ordering::Relaxed);

    let count = missing.len();
    // the number from the settings (`http::configure`), but never more than there are files
    let threads = crate::net::download_threads(
        http::concurrency(),
        std::thread::available_parallelism().map_or(4, |n| n.get()),
        count,
    );
    // the biggest files start first, one per thread: a large file begun last is what everything else waits for
    let sizes: Vec<u64> = missing.iter().map(|j| j.size.unwrap_or(0)).collect();
    let mut slots: Vec<Option<Job>> = missing.into_iter().map(Some).collect();
    let missing: Vec<Job> =
        crate::net::start_order(&sizes, threads).into_iter().filter_map(|i| slots[i].take()).collect();
    let results: Vec<Result<()>> = futures::stream::iter(missing)
        .map(|job| {
            let progress = progress.clone();
            async move {
                if progress.cancelled.load(Ordering::Relaxed) {
                    bail!("cancelled");
                }
                let r = download_one(&job, &progress).await;
                progress.files_done.fetch_add(1, Ordering::Relaxed);
                r
            }
        })
        .buffer_unordered(threads)
        .collect()
        .await;

    let failures: Vec<String> = results.into_iter().filter_map(|r| r.err()).map(|e| format!("{e:#}")).collect();
    if !failures.is_empty() {
        bail!("{} of {} files failed to download. First problem: {}", failures.len(), count, failures[0]);
    }
    Ok(count)
}

/// The downloads running now, one lock per destination: two jobs that want the same file at once (two instances of
/// one version, a pack install and a launch) would otherwise write the same `.part` file over each other.
static IN_FLIGHT: std::sync::LazyLock<std::sync::Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
    std::sync::LazyLock::new(Default::default);

async fn download_one(job: &Job, progress: &Progress) -> Result<()> {
    let lock = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner()).entry(job.dest.clone()).or_default().clone();
    let result = match lock.try_lock() {
        Ok(guard) => {
            let r = download_one_now(job, progress).await;
            drop(guard);
            r
        }
        Err(_) => {
            let guard = lock.lock().await;
            // the other job has just fetched and checked this very file
            let r = if is_good(job, false) { Ok(()) } else { download_one_now(job, progress).await };
            drop(guard);
            r
        }
    };
    let mut map = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
    // the map and this function hold the last two handles: nobody else is waiting for this file
    if map.get(&job.dest).is_some_and(|l| Arc::strong_count(l) <= 2) {
        map.remove(&job.dest);
    }
    result
}

/// Why one try at a download failed, and whether trying again could help.
struct Failed {
    error: anyhow::Error,
    /// How many tries in all are worth making for this kind of failure.
    tries: u32,
    /// The whole file arrived (as long as it should be), only its checksum disagrees.
    mismatch: bool,
}

impl Failed {
    fn retry(error: impl Into<anyhow::Error>) -> Failed {
        Failed { error: error.into(), tries: DOWNLOAD_TRIES, mismatch: false }
    }

    fn fatal(error: impl Into<anyhow::Error>) -> Failed {
        Failed { error: error.into(), tries: 1, mismatch: false }
    }
}

/// Tries for a connection that couldn't be made at all: one more, so a single dropped handshake among thousands of
/// files doesn't fail an install, while being offline still fails quickly instead of minutes later.
const CONNECT_TRIES: u32 = 2;

const DOWNLOAD_TRIES: u32 = 4;

/// How much of a download is gathered before it is written to the disk.
pub(crate) const WRITE_BUFFER: usize = 512 * 1024;

/// How long a download may go without a single byte before its connection is dropped and the file tried again. A
/// connection that stops part-way (it happens with the big download networks) would otherwise sit for the whole read
/// timeout, a minute by default, while the speed shown sinks towards nothing; a new one is usually at full speed.
pub(crate) const STALL: Duration = Duration::from_secs(if cfg!(test) { 1 } else { 15 });

/// The next piece of a download, or an error when none has come for [`STALL`].
pub(crate) async fn next_chunk<T, S>(stream: &mut S) -> Option<Result<T>>
where
    S: futures::Stream<Item = reqwest::Result<T>> + Unpin,
{
    match tokio::time::timeout(STALL, stream.next()).await {
        Ok(next) => next.map(|r| r.map_err(anyhow::Error::from)),
        Err(_) => Some(Err(anyhow::anyhow!("the download stopped for {} seconds", STALL.as_secs()))),
    }
}

async fn download_one_now(job: &Job, progress: &Progress) -> Result<()> {
    if job.url.is_empty() {
        bail!("{} is missing and has no address to download it from", job.dest.display());
    }
    if let Some(dir) = job.dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let part = job.dest.with_extension(format!(
        "{}part",
        job.dest.extension().map(|e| format!("{}.", e.to_string_lossy())).unwrap_or_default()
    ));
    let mut last = None;
    let mut planned = job.size.unwrap_or(SIZE_GUESS);
    for attempt in 0..DOWNLOAD_TRIES {
        if attempt > 0 {
            let base = if cfg!(test) { 5 } else { 400 };
            tokio::time::sleep(Duration::from_millis(base * 2u64.pow(attempt - 1))).await;
        }
        if progress.cancelled.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        let mut counted = 0u64;
        // a big file comes in pieces the first time; a try after a failure goes the plain way
        let pieces = attempt == 0;
        match fetch_to(&job.url, &part, job.sha1.as_deref(), job.size, pieces, &mut planned, &mut counted, progress)
            .await
        {
            Ok(()) => {
                if job.executable {
                    set_executable(&part);
                }
                tokio::fs::rename(&part, &job.dest).await.with_context(|| format!("saving {}", job.dest.display()))?;
                // a file with no size counts for what really came
                if job.size.is_none() {
                    progress.replan(&mut planned, counted);
                }
                return Ok(());
            }
            Err(failed) => {
                if failed.mismatch && job.keep_on_mismatch && attempt + 1 >= failed.tries {
                    // the same complete file every time: the pack's record of it is off, not the download
                    tokio::fs::rename(&part, &job.dest)
                        .await
                        .with_context(|| format!("saving {}", job.dest.display()))?;
                    let name = job.dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    crate::logs::warn(format!("Kept {name} although {:#}", failed.error));
                    progress.note(name);
                    return Ok(());
                }
                progress
                    .bytes_done
                    .fetch_sub(counted.min(progress.bytes_done.load(Ordering::Relaxed)), Ordering::Relaxed);
                let _ = tokio::fs::remove_file(&part).await;
                // a missing file or a full disk won't fix itself in a second, nor will being offline
                let tries = failed.tries;
                last = Some(failed.error);
                if attempt + 1 >= tries {
                    break;
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no attempt was made")))
        .with_context(|| format!("downloading {}", job.url))
}

async fn fetch_to(
    url: &str,
    part: &Path,
    sha1: Option<&str>,
    size: Option<u64>,
    pieces: bool,
    planned: &mut u64,
    counted: &mut u64,
    progress: &Progress,
) -> std::result::Result<(), Failed> {
    let r = match client().get(url).send().await {
        Ok(r) => r,
        Err(e) => {
            let tries = if crate::net::is_transient_error(&e) {
                DOWNLOAD_TRIES
            } else if e.is_connect() {
                CONNECT_TRIES
            } else {
                1
            };
            return Err(Failed { error: e.into(), tries, mismatch: false });
        }
    };
    if !r.status().is_success() {
        let tries = if crate::net::is_transient_status(r.status().as_u16()) { DOWNLOAD_TRIES } else { 1 };
        return Err(Failed { error: anyhow::anyhow!("HTTP {}", r.status()), tries, mismatch: false });
    }
    // a file with no size counts for what the server says it is, so the per cent is right while it comes
    if let (None, Some(len)) = (size, r.content_length()) {
        progress.replan(planned, len);
    }
    if let Some(len) = crate::ranged::worth_it(&r).filter(|_| pieces) {
        crate::ranged::download(part, len, r, counted, progress).await.map_err(|e| {
            if progress.cancelled.load(Ordering::Relaxed) { Failed::fatal(e) } else { Failed::retry(e) }
        })?;
        let got = match sha1 {
            Some(expected) => {
                let (p, like) = (part.to_path_buf(), expected.to_string());
                tokio::task::spawn_blocking(move || crate::packs::dl::hash_file(&p, &like))
                    .await
                    .map_err(Failed::fatal)?
            }
            None => None,
        };
        return verify(sha1, size, *counted, got);
    }
    let file = tokio::fs::File::create(part)
        .await
        .map_err(|e| Failed::fatal(anyhow::Error::from(e).context(format!("writing {}", part.display()))))?;
    // the network hands over small pieces: written as they come, each would be a trip to the disk thread of its own
    let mut file = tokio::io::BufWriter::with_capacity(WRITE_BUFFER, file);
    let mut hasher = Sha1::new();
    // pack libraries carry an MD5 in the checksum field (the Java edition tells them apart by length)
    let mut md5 = if sha1.is_some_and(|h| h.len() == 32) { Some(md5::Md5::new()) } else { None };
    let mut stream = r.bytes_stream();
    while let Some(chunk) = next_chunk(&mut stream).await {
        // a big file (the game, a Java runtime) stops when the player presses Cancel, not when it is done
        if progress.cancelled.load(Ordering::Relaxed) {
            return Err(Failed::fatal(anyhow::anyhow!("cancelled")));
        }
        let chunk = chunk.map_err(Failed::retry)?;
        hasher.update(&chunk);
        if let Some(m) = md5.as_mut() {
            md5::Digest::update(m, &chunk);
        }
        // writing fails when the disk is full: trying again would only fill it again
        file.write_all(&chunk)
            .await
            .map_err(|e| Failed::fatal(anyhow::Error::from(e).context(format!("writing {}", part.display()))))?;
        *counted += chunk.len() as u64;
        progress.bytes_done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }
    file.flush().await.map_err(Failed::fatal)?;
    let got = sha1.map(|_| match md5 {
        Some(m) => hex::encode(md5::Digest::finalize(m)),
        None => hex::encode(hasher.finalize()),
    });
    verify(sha1, size, *counted, got)
}

/// Whether the `counted` bytes that came are the file wanted: its checksum, `got` (worked out from what came), is the
/// one expected, or without a checksum its size is.
fn verify(sha1: Option<&str>, size: Option<u64>, counted: u64, got: Option<String>) -> std::result::Result<(), Failed> {
    // the size decides only for a file without a checksum: a size from a mod site can be off where the hash is right
    if let Some(expected) = size.filter(|_| sha1.is_none()) {
        if counted != expected {
            return Err(Failed::retry(anyhow::anyhow!("the file arrived incomplete ({counted} of {expected} bytes)")));
        }
    }
    if let Some(expected) = sha1 {
        let got = got.unwrap_or_default();
        if !got.eq_ignore_ascii_case(expected) {
            // cut short is damaged; complete but different may only be an out-of-date record of the file
            let complete = size.is_none_or(|s| counted == s);
            return Err(Failed {
                error: anyhow::anyhow!("the file arrived damaged (expected {expected}, got {got})"),
                tries: DOWNLOAD_TRIES,
                mismatch: complete,
            });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}

#[cfg(not(unix))]
fn set_executable(_: &Path) {}

// ---------------------------------------------------------------------------------------------------------
// Mojang's list of versions

#[derive(Deserialize, Clone, Debug)]
pub struct ManifestVersion {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub sha1: String,
    #[serde(default, rename = "releaseTime")]
    pub release_time: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Manifest {
    pub latest: HashMap<String, String>,
    pub versions: Vec<ManifestVersion>,
}

pub async fn manifest(dirs: &Dirs) -> Result<Manifest> {
    http::get_json_cached(MANIFEST_URL, &dirs.cache.join("http"), Duration::from_secs(600)).await
}

/// The version JSON for one Minecraft version, from the saved copy when there is one.
pub async fn vanilla_json(dirs: &Dirs, id: &str) -> Result<VersionJson> {
    let saved = dirs.configs.join("json").join("minecraft").join(format!("{id}.json"));
    if let Ok(bytes) = std::fs::read(&saved) {
        if let Ok(v) = serde_json::from_slice::<VersionJson>(&bytes) {
            if !v.libraries.is_empty() || !v.main_class.is_empty() {
                return Ok(v);
            }
        }
    }
    let manifest = manifest(dirs).await?;
    let entry =
        manifest.versions.iter().find(|v| v.id == id).with_context(|| format!("Minecraft {id} doesn't exist"))?;
    let bytes = http::get_bytes(&entry.url).await?;
    let parsed: VersionJson = serde_json::from_slice(&bytes)?;
    if let Some(dir) = saved.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&saved, &bytes);
    Ok(parsed)
}

// ---------------------------------------------------------------------------------------------------------
// Game files

pub fn client_jar(dirs: &Dirs, game_version: &str) -> PathBuf {
    let game_version = crate::paths::safe_component(game_version);
    dirs.libraries
        .join("net")
        .join("minecraft")
        .join("client")
        .join(&game_version)
        .join(format!("client-{game_version}.jar"))
}

pub fn asset_index_file(dirs: &Dirs, id: &str) -> PathBuf {
    dirs.assets.join("indexes").join(format!("{}.json", crate::paths::safe_component(id)))
}

#[derive(Deserialize)]
pub struct AssetIndex {
    pub objects: HashMap<String, AssetObject>,
    #[serde(default, rename = "virtual")]
    pub is_virtual: bool,
    #[serde(default)]
    pub map_to_resources: bool,
}

#[derive(Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

/// An asset index, keeping only objects whose hash is hex digits (a SHA-1): the hash is also the file's path
/// (`objects/<first two>/<hash>`), and a short or odd one crashed the launcher when it was cut.
pub fn read_asset_index(dirs: &Dirs, id: &str) -> Result<AssetIndex> {
    let bytes = std::fs::read(asset_index_file(dirs, id)).with_context(|| format!("asset index {id} is missing"))?;
    let mut index: AssetIndex = serde_json::from_slice(&bytes).with_context(|| format!("reading asset index {id}"))?;
    let before = index.objects.len();
    index.objects.retain(|_, o| is_hex_hash(&o.hash));
    if index.objects.len() != before {
        crate::logs::warn(format!(
            "Asset index {id}: left out {} entries with a broken hash",
            before - index.objects.len()
        ));
    }
    Ok(index)
}

/// At least two hex digits and nothing else, so it can be cut into `objects/<first two>/<hash>` safely.
pub fn is_hex_hash(hash: &str) -> bool {
    hash.len() >= 2 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Make sure everything `version` needs to run is in place: libraries, game jar, assets, logging config.
pub async fn ensure_game_files(
    dirs: &Dirs,
    version: &VersionJson,
    progress: &Arc<Progress>,
    verify: bool,
) -> Result<()> {
    let features = Features::default();
    let game_version = version.game_version().to_string();

    // the asset index decides which assets there are, so it comes first
    if let Some(index) = &version.asset_index {
        progress.stage("Checking assets");
        let job = Job::new(&index.url, asset_index_file(dirs, &index.id)).checked(Some(&index.sha1), Some(index.size));
        download_all(vec![job], progress, verify).await?;
    }

    let mut jobs = Vec::new();
    for lib in version.active_libraries(&features) {
        if let Some(a) = lib.artifact() {
            if let Some(path) = a.path.as_deref() {
                jobs.push(Job::new(&a.url, dirs.libraries.join(path)).checked(a.sha1.as_deref(), a.size));
            }
        }
        if let Some(n) = lib.native() {
            match n.path.as_deref() {
                Some(path) => jobs.push(Job::new(&n.url, dirs.libraries.join(path)).checked(n.sha1.as_deref(), n.size)),
                None => crate::logs::warn(format!("Skipping the natives of {}: their path is not usable", lib.name)),
            }
        }
    }
    if let Some(client) = version.downloads.as_ref().and_then(|d| d.client.as_ref()) {
        jobs.push(Job::new(&client.url, client_jar(dirs, &game_version)).checked(client.sha1.as_deref(), client.size));
    }
    if let Some(log) = version.logging.as_ref().and_then(|l| l.client.as_ref()) {
        if !log.file.url.is_empty() {
            jobs.push(
                Job::new(
                    &log.file.url,
                    dirs.assets.join("log_configs").join(crate::paths::safe_component(&log.file.id)),
                )
                .checked(Some(&log.file.sha1), Some(log.file.size)),
            );
        }
    }
    // on a hard disk, looking at the thousands of asset files takes minutes of head movement once the system has
    // forgotten them, so a pass that found everything in place is trusted for a few days (see `AssetStamp`)
    let spinning =
        !verify && version.asset_index.is_some() && crate::disk::is_spinning(&Settings::load(dirs), &dirs.base);
    let stamp_file =
        version.asset_index.as_ref().map(|i| dirs.cache.join("asset-checks").join(format!("{}.json", i.id)));
    let trust_assets = spinning
        && match (&version.asset_index, &stamp_file) {
            (Some(index), Some(stamp)) => AssetStamp::of(&asset_index_file(dirs, &index.id))
                .zip(std::fs::read_to_string(stamp).ok().and_then(|t| serde_json::from_str::<AssetStamp>(&t).ok()))
                .is_some_and(|(now, saved)| saved.still_good(&now, chrono::Utc::now().timestamp_millis())),
            _ => false,
        };
    if let Some(index) = version.asset_index.as_ref().filter(|_| !trust_assets) {
        let parsed = read_asset_index(dirs, &index.id)?;
        for obj in parsed.objects.values() {
            let prefix = &obj.hash[..2];
            jobs.push(
                Job::new(
                    format!("{RESOURCES_URL}/{prefix}/{}", obj.hash),
                    dirs.assets.join("objects").join(prefix).join(&obj.hash),
                )
                .checked(Some(&obj.hash), Some(obj.size as i64)),
            );
        }
    }

    progress.stage("Downloading game files");
    // files with no address that are already on disk are fine (Forge builds some libraries itself)
    jobs.retain(|j| !j.url.is_empty() || !is_good(j, false));
    download_all(jobs, progress, verify).await?;
    // everything was in place: remember it, so the next starts on a hard disk needn't look at every asset again
    if let (true, false, Some(index), Some(stamp)) = (spinning, trust_assets, &version.asset_index, &stamp_file) {
        if let Some(now) = AssetStamp::of(&asset_index_file(dirs, &index.id)) {
            let saved = AssetStamp { checked_ms: chrono::Utc::now().timestamp_millis(), ..now };
            if let Some(dir) = stamp.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(stamp, serde_json::to_vec(&saved).unwrap_or_default());
        }
    }
    Ok(())
}

/// What is kept about a pass that found every asset of a version in place: the asset index it was for, and when.
/// On a hard disk the next starts skip looking at the thousands of asset files for a few days; the asset index (the
/// list of them) is still compared, so a different version or a new index is always checked. Missing or damaged
/// assets found by the pass before that, and a game that can't find one, are fixed by the next full pass.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AssetStamp {
    index_size: u64,
    index_modified_ms: i64,
    checked_ms: i64,
}

/// How long a pass is trusted on a hard disk.
const ASSET_TRUST_MS: i64 = 3 * 24 * 3600 * 1000;

impl AssetStamp {
    /// The asset index file as it is now (`checked_ms` 0).
    fn of(index_file: &Path) -> Option<AssetStamp> {
        let meta = std::fs::metadata(index_file).ok()?;
        let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
        Some(AssetStamp { index_size: meta.len(), index_modified_ms: modified, checked_ms: 0 })
    }

    /// If the pass saved in `self` still stands for the index `now`, at time `now_ms`.
    fn still_good(&self, now: &AssetStamp, now_ms: i64) -> bool {
        self.checked_ms > 0
            && self.index_size == now.index_size
            && self.index_modified_ms == now.index_modified_ms
            && now_ms - self.checked_ms < ASSET_TRUST_MS
            && now_ms >= self.checked_ms
    }
}

/// Old versions keep their assets under real file names instead of hashes. Copy them out once.
pub fn map_legacy_assets(dirs: &Dirs, version: &VersionJson, game_dir: &Path) -> Result<Option<PathBuf>> {
    let Some(index_ref) = &version.asset_index else { return Ok(None) };
    let index = read_asset_index(dirs, &index_ref.id)?;
    let target = if index.map_to_resources {
        game_dir.join("resources")
    } else if index.is_virtual {
        dirs.assets.join("virtual").join("legacy")
    } else {
        return Ok(None);
    };
    for (name, obj) in &index.objects {
        // names come from the index file: never outside the folder they are copied into
        let Some(dest) = crate::paths::join_inside(&target, name) else { continue };
        // a copy cut short (the launcher closed, a full disk) has the wrong size and is made again
        if std::fs::metadata(&dest).is_ok_and(|m| m.len() == obj.size) {
            continue;
        }
        let src = dirs.assets.join("objects").join(&obj.hash[..2]).join(&obj.hash);
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(&dest);
        if std::fs::hard_link(&src, &dest).is_err() {
            let part = dest.with_file_name(format!(
                ".{}.part",
                dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
            ));
            std::fs::copy(&src, &part).with_context(|| format!("copying {}", src.display()))?;
            std::fs::rename(&part, &dest)?;
        }
    }
    Ok(Some(target))
}

/// Unpack the natives of old versions (before LWJGL shipped them as normal libraries) into `dest`.
pub fn extract_natives(dirs: &Dirs, version: &VersionJson, dest: &Path) -> Result<()> {
    let features = Features::default();
    std::fs::create_dir_all(dest)?;
    for lib in version.active_libraries(&features) {
        let Some(native) = lib.native() else { continue };
        let Some(path) = native.path else { continue };
        let jar = dirs.libraries.join(path);
        let file = std::fs::File::open(&jar).with_context(|| format!("natives jar {} is missing", jar.display()))?;
        let mut archive = zip::ZipArchive::new(file)?;
        let excludes = lib.extract.as_ref().map(|e| e.exclude.clone()).unwrap_or_default();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            if entry.is_dir() {
                continue;
            }
            let Ok(name) = entry.name().map(|n| n.to_string()) else { continue };
            if excludes.iter().any(|ex| name.starts_with(ex.as_str())) {
                continue;
            }
            let Some(rel) = entry.enclosed_name() else { continue };
            let out = dest.join(rel);
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut f = std::fs::File::create(&out)?;
            std::io::copy(&mut entry, &mut f)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod stamp_tests {
    use super::*;

    #[test]
    fn a_pass_is_trusted_only_for_the_same_index_and_a_few_days() {
        let day = 24 * 3600 * 1000;
        let index = AssetStamp { index_size: 400_000, index_modified_ms: 1_000, checked_ms: 0 };
        let saved = AssetStamp { checked_ms: 10 * day, ..index.clone() };
        assert!(saved.still_good(&index, 10 * day + 1));
        assert!(saved.still_good(&index, 12 * day));
        assert!(!saved.still_good(&index, 13 * day + 1), "too old");
        assert!(!saved.still_good(&index, 9 * day), "a clock that went back");
        assert!(!saved.still_good(&AssetStamp { index_size: 1, ..index.clone() }, 10 * day + 1), "other index");
        assert!(!saved.still_good(&AssetStamp { index_modified_ms: 2, ..index.clone() }, 10 * day + 1));
        assert!(!index.still_good(&index, 10 * day), "a stamp that never recorded a pass");
    }
}

/// Downloads as they go wrong in real life: servers that answer 404 or 503, connections that drop half way, files
/// that arrive damaged, a player pressing Cancel, two jobs wanting one file.
#[cfg(test)]
mod progress_tests {
    use super::*;

    #[test]
    fn sizes_read_like_people_write_them() {
        assert_eq!(human_bytes(0), "0 KB");
        assert_eq!(human_bytes(300), "1 KB");
        assert_eq!(human_bytes(845_000), "845 KB");
        assert_eq!(human_bytes(12_345_678), "12.3 MB");
        assert_eq!(human_bytes(1_420_000_000), "1.42 GB");
    }

    #[test]
    fn a_download_says_how_far_it_is_in_per_cent_and_sizes() {
        let p = Progress::new();
        assert_eq!(p.summary(), None, "nothing to download yet");
        p.files_total.store(187, Ordering::Relaxed);
        p.files_done.store(120, Ordering::Relaxed);
        p.bytes_total.store(318_000_000, Ordering::Relaxed);
        p.bytes_done.store(210_300_000, Ordering::Relaxed);
        let (sizes, files) = p.summary().unwrap();
        assert_eq!(sizes, "66% \u{00B7} 210.3 MB of 318.0 MB");
        assert_eq!(files, "120 of 187 files", "no speed before there is anything to measure it by");
        // more arrived than was planned (a file without a size) while files are still coming: never "110%", nor a
        // "100%" for a download that isn't done
        p.bytes_done.store(330_000_000, Ordering::Relaxed);
        assert_eq!(p.summary().unwrap().0, "330.0 MB so far");
        assert_eq!(p.known_fraction(), None);
        p.files_done.store(187, Ordering::Relaxed);
        assert_eq!(p.known_fraction(), Some(1.0));
    }

    #[test]
    fn a_file_without_a_size_counts_for_its_real_size_once_known() {
        let p = Progress::new();
        p.bytes_total.store(1_000 + SIZE_GUESS, Ordering::Relaxed);
        let mut planned = SIZE_GUESS;
        p.replan(&mut planned, 338_972_290);
        assert_eq!(p.bytes_total.load(Ordering::Relaxed), 1_000 + 338_972_290);
        p.replan(&mut planned, 300);
        assert_eq!(p.bytes_total.load(Ordering::Relaxed), 1_300, "a file smaller than the guess lowers the total");
    }

    #[test]
    fn notes_are_handed_over_once() {
        let p = Progress::new();
        p.note("a.jar");
        p.note("b.jar");
        assert_eq!(p.take_notes(), vec!["a.jar", "b.jar"]);
        assert!(p.take_notes().is_empty());
        assert!(kept_files_message(&["a.jar".into()]).starts_with("1 file didn't match"));
        let many: Vec<String> = (0..7).map(|i| format!("m{i}.jar")).collect();
        assert!(kept_files_message(&many).contains("m4.jar and 2 more"));
    }
}

#[cfg(test)]
mod download_robustness_tests {
    use super::*;
    use crate::testutil::{Reply, Server, runtime, scratch, sha1};

    fn job(url: &str, dest: &Path, body: &[u8]) -> Job {
        let mut j = Job::new(url, dest);
        j.sha1 = Some(sha1(body));
        j.size = Some(body.len() as u64);
        j
    }

    /// A modpack's file that arrives whole every time but with another checksum than the pack lists is kept and
    /// said (the pack's list is out of date); one that arrives cut short is still refused, and a game file with a
    /// wrong checksum is never kept.
    #[test]
    fn a_pack_file_with_an_out_of_date_checksum_is_kept_and_said() {
        runtime().block_on(async {
            let server = Server::start().await;
            let dir = scratch("dl-keep");
            let listed = b"the file the pack lists".to_vec();
            let served = b"the file the site serves".to_vec();
            assert_eq!(listed.len() + 1, served.len());
            let mut pack_job = job(&server.route("/mod.jar", Reply::ok(served.clone())), &dir.join("mod.jar"), &listed);
            pack_job.size = Some(served.len() as u64);
            pack_job.keep_on_mismatch = true;
            let progress = Progress::new();
            download_all(vec![pack_job.clone()], &progress, false).await.unwrap();
            assert_eq!(std::fs::read(dir.join("mod.jar")).unwrap(), served);
            assert_eq!(progress.take_notes(), vec!["mod.jar".to_string()]);
            assert_eq!(server.hits("/mod.jar"), DOWNLOAD_TRIES as usize, "asked again before it is believed");

            // cut short: damaged, not out of date
            let mut cut = pack_job.clone();
            cut.url = server.route("/cut.jar", Reply::cut(served.clone(), 5));
            cut.dest = dir.join("cut.jar");
            assert!(download_all(vec![cut], &Progress::new(), false).await.is_err());
            assert!(!dir.join("cut.jar").exists());

            // the game's own files are never kept on a wrong checksum
            let mut game = pack_job.clone();
            game.keep_on_mismatch = false;
            game.dest = dir.join("library.jar");
            assert!(download_all(vec![game], &Progress::new(), false).await.is_err());
            assert!(!dir.join("library.jar").exists());
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// Many small files from a slow server (the game's assets for a far-away player) are asked for many at once,
    /// whatever the processor count, and the biggest file is asked for first.
    #[test]
    fn small_files_from_a_slow_server_are_fetched_many_at_once() {
        runtime().block_on(async {
            let server = Server::start().await;
            let dir = scratch("dl-many");
            let mut jobs = Vec::new();
            for i in 0..120 {
                let body = format!("asset {i}").into_bytes();
                let url = server.route(&format!("/a{i}"), Reply::ok(body.clone()).slow(150));
                jobs.push(job(&url, &dir.join(format!("a{i}")), &body));
            }
            let big = vec![7u8; 300_000];
            let url = server.route("/big.jar", Reply::ok(big.clone()));
            jobs.push(job(&url, &dir.join("big.jar"), &big));
            let started = std::time::Instant::now();
            assert_eq!(download_all(jobs, &Progress::new(), false).await.unwrap(), 121);
            let took = started.elapsed();
            // 120 answers of 150 ms each: one at a time 18 s, 8 at a time 2.3 s, 32 at a time about 0.6 s
            assert!(server.most_at_once() >= 24, "only {} at once", server.most_at_once());
            assert!(took < Duration::from_secs(5), "took {took:?}");
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    #[test]
    fn a_missing_file_fails_at_once_instead_of_being_asked_for_four_times() {
        runtime().block_on(async {
            let server = Server::start().await;
            let dir = scratch("dl-404");
            let url = format!("{}/gone.jar", server.base);
            let started = std::time::Instant::now();
            let err = download_all(vec![Job::new(&url, dir.join("gone.jar"))], &Progress::new(), false).await;
            assert!(err.is_err());
            assert_eq!(server.hits("/gone.jar"), 1, "a 404 is final");
            assert!(started.elapsed() < Duration::from_secs(2));
            assert!(!dir.join("gone.jar").exists() && !dir.join("gone.jar.part").exists());
        });
    }

    #[test]
    fn a_busy_server_is_asked_again_and_a_damaged_file_is_never_kept() {
        runtime().block_on(async {
            let server = Server::start().await;
            let dir = scratch("dl-busy");
            let body = b"the real file".to_vec();
            let url = server.route("/busy.jar", Reply::status(503));
            assert!(
                download_all(vec![job(&url, &dir.join("busy.jar"), &body)], &Progress::new(), false).await.is_err()
            );
            assert_eq!(server.hits("/busy.jar"), DOWNLOAD_TRIES as usize, "a 503 is tried again");

            // a body that doesn't match its checksum is tried again and then refused, and never saved
            // (as long as the real one, so it is the checksum that catches it, not the size)
            let url = server.route("/bad.jar", Reply::ok(b"the fake file".to_vec()));
            let err = download_all(vec![job(&url, &dir.join("bad.jar"), &body)], &Progress::new(), false).await;
            assert!(format!("{:#}", err.unwrap_err()).contains("damaged"));
            assert!(!dir.join("bad.jar").exists());

            // a connection that drops half way leaves nothing behind that a later start would trust
            let url = server.route("/cut.jar", Reply::cut(body.clone(), 4));
            assert!(download_all(vec![job(&url, &dir.join("cut.jar"), &body)], &Progress::new(), false).await.is_err());
            assert!(!dir.join("cut.jar").exists());
            let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
            assert!(leftovers.is_empty(), "{leftovers:?}");
        });
    }

    #[test]
    fn a_short_file_without_a_checksum_is_refused_by_its_size() {
        runtime().block_on(async {
            let server = Server::start().await;
            let dir = scratch("dl-size");
            let url = server.route("/short.bin", Reply::ok(b"only part".to_vec()));
            let mut j = Job::new(&url, dir.join("short.bin"));
            j.size = Some(1000);
            let err = download_all(vec![j], &Progress::new(), false).await.unwrap_err();
            assert!(format!("{err:#}").contains("incomplete"), "{err:#}");
            assert!(!dir.join("short.bin").exists());
        });
    }

    #[test]
    fn a_wrong_size_from_a_mod_site_doesnt_refuse_a_file_whose_checksum_is_right() {
        runtime().block_on(async {
            let server = Server::start().await;
            let dir = scratch("dl-size-off");
            let body = b"exactly the right bytes".to_vec();
            let url = server.route("/m.jar", Reply::ok(body.clone()));
            let mut j = job(&url, &dir.join("m.jar"), &body);
            j.size = Some(body.len() as u64 + 7);
            download_all(vec![j], &Progress::new(), false).await.unwrap();
            assert_eq!(std::fs::read(dir.join("m.jar")).unwrap(), body);
            assert_eq!(server.hits("/m.jar"), 1);
        });
    }

    #[test]
    fn nothing_is_downloaded_after_cancel() {
        runtime().block_on(async {
            let server = Server::start().await;
            let dir = scratch("dl-cancel");
            let body = vec![7u8; 1000];
            let url = server.route("/c.bin", Reply::ok(body.clone()));
            let progress = Progress::new();
            progress.cancelled.store(true, Ordering::Relaxed);
            assert!(download_all(vec![job(&url, &dir.join("c.bin"), &body)], &progress, false).await.is_err());
            assert_eq!(server.total(), 0);
            assert!(!dir.join("c.bin").exists());
        });
    }

    #[test]
    fn two_jobs_wanting_the_same_file_at_once_share_one_download() {
        runtime().block_on(async {
            let server = Server::start().await;
            let dir = scratch("dl-shared");
            let body: Vec<u8> = (0..2_000_000u32).map(|i| (i % 253) as u8).collect();
            let url = server.route("/big.jar", Reply::ok(body.clone()));
            let dest = dir.join("big.jar");
            let (p1, p2) = (Progress::new(), Progress::new());
            let (a, b) = tokio::join!(
                download_all(vec![job(&url, &dest, &body)], &p1, false),
                download_all(vec![job(&url, &dest, &body)], &p2, false),
            );
            a.unwrap();
            b.unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), body);
            assert!(server.hits("/big.jar") <= 2);
            assert!(!dir.join("big.jar.part").exists());
            assert!(IN_FLIGHT.lock().unwrap().get(&dest).is_none(), "the lock is let go afterwards");
        });
    }

    #[test]
    fn a_job_without_an_address_names_the_missing_file() {
        runtime().block_on(async {
            let dir = scratch("dl-nourl");
            let err = download_all(vec![Job::new("", dir.join("x.jar"))], &Progress::new(), false).await.unwrap_err();
            assert!(format!("{err:#}").contains("no address"));
        });
    }
}

#[cfg(test)]
mod asset_tests {
    use super::*;

    #[test]
    fn broken_hashes_in_an_asset_index_are_left_out_instead_of_crashing() {
        let base = crate::testutil::scratch("asset-index");
        let dirs = Dirs::at(&base);
        let good = "a".repeat(40);
        let json = serde_json::json!({"objects": {
            "minecraft/ok.ogg": {"hash": good, "size": 3},
            "short": {"hash": "a", "size": 1},
            "empty": {"hash": "", "size": 1},
            "wide": {"hash": "日本語日本語日本語日本語日本", "size": 1},
            "slash": {"hash": "../../../../../../../../../etc/passwd!!", "size": 1},
        }});
        std::fs::create_dir_all(base.join("assets/indexes")).unwrap();
        std::fs::write(base.join("assets/indexes/17.json"), json.to_string()).unwrap();
        let index = read_asset_index(&dirs, "17").unwrap();
        assert_eq!(index.objects.len(), 1);
        assert!(index.objects.contains_key("minecraft/ok.ogg"));
        // an index id from a file never leads elsewhere
        assert_eq!(asset_index_file(&dirs, "../../x").parent().unwrap(), base.join("assets/indexes"));
        assert!(read_asset_index(&dirs, "missing").is_err());
        std::fs::write(base.join("assets/indexes/bad.json"), b"{not json").unwrap();
        assert!(read_asset_index(&dirs, "bad").is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn legacy_assets_are_copied_whole_and_only_inside_their_folder() {
        let base = crate::testutil::scratch("legacy-assets");
        let dirs = Dirs::at(base.join("data"));
        let hash = sha1_of(b"sound");
        let obj = dirs.assets.join("objects").join(&hash[..2]).join(&hash);
        std::fs::create_dir_all(obj.parent().unwrap()).unwrap();
        std::fs::write(&obj, b"sound").unwrap();
        let json = serde_json::json!({"virtual": true, "objects": {
            "sound/a.ogg": {"hash": hash, "size": 5},
            "../../escaped.ogg": {"hash": hash, "size": 5},
        }});
        std::fs::create_dir_all(dirs.assets.join("indexes")).unwrap();
        std::fs::write(dirs.assets.join("indexes/legacy.json"), json.to_string()).unwrap();
        let mut version = VersionJson::default();
        version.asset_index = Some(
            serde_json::from_value(
                serde_json::json!({"id": "legacy", "url": "", "sha1": "", "size": 0, "totalSize": 0}),
            )
            .unwrap(),
        );
        let target = map_legacy_assets(&dirs, &version, &base.join("game")).unwrap().unwrap();
        // a copy cut short by a crash (its own file, not a link to the object) is noticed by its size and made again
        std::fs::remove_file(target.join("sound/a.ogg")).unwrap();
        std::fs::write(target.join("sound/a.ogg"), b"so").unwrap();
        map_legacy_assets(&dirs, &version, &base.join("game")).unwrap();
        assert_eq!(std::fs::read(target.join("sound/a.ogg")).unwrap(), b"sound");
        assert!(!dirs.assets.join("escaped.ogg").exists() && !base.join("escaped.ogg").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    fn sha1_of(b: &[u8]) -> String {
        crate::http::sha1_hex(b)
    }
}
