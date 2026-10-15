//! Getting a Minecraft version onto the disk: libraries, the game jar, assets and the Mojang logging config.
//! Everything goes through one parallel downloader that skips what is already there.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use serde::Deserialize;
use sha1::{Digest, Sha1};
use tokio::io::AsyncWriteExt;

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
}

impl Progress {
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
}

impl Job {
    pub fn new(url: impl Into<String>, dest: impl Into<PathBuf>) -> Job {
        Job { url: url.into(), dest: dest.into(), sha1: None, size: None, executable: false }
    }

    pub fn checked(mut self, sha1: Option<&str>, size: Option<i64>) -> Job {
        self.sha1 = sha1.filter(|s| !s.is_empty()).map(str::to_string);
        self.size = size.filter(|s| *s > 0).map(|s| s as u64);
        self
    }
}

/// Whether a file on disk can be trusted without downloading it again. By default size is the test, which is
/// one `stat` per file; `verify` hashes it too.
fn is_good(job: &Job, verify: bool) -> bool {
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
    progress.bytes_total.store(missing.iter().map(|j| j.size.unwrap_or(65_536)).sum(), Ordering::Relaxed);

    let count = missing.len();
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
        // the number from the settings (`http::configure`), but never more than there are files
        .buffer_unordered(crate::net::download_threads(
            http::concurrency(),
            std::thread::available_parallelism().map_or(4, |n| n.get()),
            count,
        ))
        .collect()
        .await;

    let failures: Vec<String> = results.into_iter().filter_map(|r| r.err()).map(|e| format!("{e:#}")).collect();
    if !failures.is_empty() {
        bail!("{} of {} files failed to download. First problem: {}", failures.len(), count, failures[0]);
    }
    Ok(count)
}

async fn download_one(job: &Job, progress: &Progress) -> Result<()> {
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
    for attempt in 0..4u32 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(400 * 2u64.pow(attempt - 1))).await;
        }
        let mut counted = 0u64;
        match fetch_to(&job.url, &part, job.sha1.as_deref(), &mut counted, progress).await {
            Ok(()) => {
                if job.executable {
                    set_executable(&part);
                }
                tokio::fs::rename(&part, &job.dest).await.with_context(|| format!("saving {}", job.dest.display()))?;
                // the planned size was a guess for files with no size; settle the books
                if job.size.is_none() {
                    progress.bytes_total.fetch_add(counted.saturating_sub(65_536), Ordering::Relaxed);
                }
                return Ok(());
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
    Err(last.unwrap()).with_context(|| format!("downloading {}", job.url))
}

async fn fetch_to(url: &str, part: &Path, sha1: Option<&str>, counted: &mut u64, progress: &Progress) -> Result<()> {
    let r = client().get(url).send().await?;
    if !r.status().is_success() {
        bail!("HTTP {}", r.status());
    }
    let mut file = tokio::fs::File::create(part).await?;
    let mut hasher = Sha1::new();
    // pack libraries carry an MD5 in the checksum field (the Java edition tells them apart by length)
    let mut md5 = if sha1.is_some_and(|h| h.len() == 32) { Some(md5::Md5::new()) } else { None };
    let mut stream = r.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
        if let Some(m) = md5.as_mut() {
            md5::Digest::update(m, &chunk);
        }
        file.write_all(&chunk).await?;
        *counted += chunk.len() as u64;
        progress.bytes_done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }
    file.flush().await?;
    if let Some(expected) = sha1 {
        let got = match md5 {
            Some(m) => hex::encode(md5::Digest::finalize(m)),
            None => hex::encode(hasher.finalize()),
        };
        if !got.eq_ignore_ascii_case(expected) {
            bail!("the file arrived damaged (expected {expected}, got {got})");
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
    dirs.libraries
        .join("net")
        .join("minecraft")
        .join("client")
        .join(game_version)
        .join(format!("client-{game_version}.jar"))
}

pub fn asset_index_file(dirs: &Dirs, id: &str) -> PathBuf {
    dirs.assets.join("indexes").join(format!("{id}.json"))
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

pub fn read_asset_index(dirs: &Dirs, id: &str) -> Result<AssetIndex> {
    let bytes = std::fs::read(asset_index_file(dirs, id)).with_context(|| format!("asset index {id} is missing"))?;
    Ok(serde_json::from_slice(&bytes)?)
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
            let path = a.path.clone().unwrap_or_default();
            jobs.push(Job::new(&a.url, dirs.libraries.join(&path)).checked(a.sha1.as_deref(), a.size));
        }
        if let Some(n) = lib.native() {
            let path = n.path.clone().unwrap_or_default();
            jobs.push(Job::new(&n.url, dirs.libraries.join(&path)).checked(n.sha1.as_deref(), n.size));
        }
    }
    if let Some(client) = version.downloads.as_ref().and_then(|d| d.client.as_ref()) {
        jobs.push(Job::new(&client.url, client_jar(dirs, &game_version)).checked(client.sha1.as_deref(), client.size));
    }
    if let Some(log) = version.logging.as_ref().and_then(|l| l.client.as_ref()) {
        if !log.file.url.is_empty() {
            jobs.push(
                Job::new(&log.file.url, dirs.assets.join("log_configs").join(&log.file.id))
                    .checked(Some(&log.file.sha1), Some(log.file.size)),
            );
        }
    }
    if let Some(index) = &version.asset_index {
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
    Ok(())
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
        let dest = target.join(name);
        if dest.exists() {
            continue;
        }
        let src = dirs.assets.join("objects").join(&obj.hash[..2]).join(&obj.hash);
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if std::fs::hard_link(&src, &dest).is_err() {
            std::fs::copy(&src, &dest)?;
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
