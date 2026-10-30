//! Horizon's releases: which one is the newest, checking that a download is the one the Horizon project signed, and
//! unpacking it into `horizon/builds/<tag>/`.
//!
//! Every release carries `SHA256SUMS` (`<sha256>  <file>` per line) and `SHA256SUMS.sig`, base64 Ed25519
//! signatures, one per line, over `horizon release <tag>\n` followed by the list. The tag is part of what is signed,
//! so an old release's list can't be passed off as a newer one. An archive is unpacked only when the list is signed
//! by the release key built in here and the archive's SHA-256 is the one the list gives.
//!
//! The archive is downloaded into memory and checked and unpacked from that one buffer, so what was checked is
//! what is installed. The unpacking writes only plain files and folders. What it installed is then written down in
//! the build folder (see [`check_build`]), and a build that doesn't match that record is never run.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::install::Progress;
use crate::paths::Dirs;

/// The newest stable release, from GitHub's API.
const LATEST_API: &str = "https://api.github.com/repos/horizon-rust/horizon/releases/latest";
/// Where a release's files download from (GitHub's fixed address for them).
pub(crate) const DOWNLOADS: &str = "https://github.com/horizon-rust/horizon/releases/download";
/// The Horizon project's release key (crates/horizon-release-sign/release-keys.txt in their repository).
const RELEASE_KEY: &str = "MU4oiIQcCxkZvbwSjjXs7SySCuM6ywRQ3nGaJYZG0gw=";
pub(crate) const SUMS: &str = "SHA256SUMS";
pub(crate) const SIGNATURE: &str = "SHA256SUMS.sig";

/// How long a look at GitHub is trusted: Horizon releases often, but asking on every Play would be slow and use up
/// GitHub's 60 questions an hour.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// How long Play waits for GitHub before going on with the build already here.
const API_TIMEOUT: Duration = Duration::from_secs(8);

/// An Ed25519 public key.
pub(crate) type PublicKey = [u8; 32];

/// The keys a release may be signed with.
pub(crate) fn trusted_keys() -> Vec<PublicKey> {
    base64::engine::general_purpose::STANDARD
        .decode(RELEASE_KEY)
        .ok()
        .and_then(|k| PublicKey::try_from(k.as_slice()).ok())
        .into_iter()
        .collect()
}

/// Everything of Horizon in BlockForge's data folder.
pub fn horizon_dir(dirs: &Dirs) -> PathBuf {
    dirs.base.join("horizon")
}

/// The unpacked releases, one folder per tag.
pub fn builds_dir(dirs: &Dirs) -> PathBuf {
    horizon_dir(dirs).join("builds")
}

pub fn build_dir(dirs: &Dirs, tag: &str) -> PathBuf {
    builds_dir(dirs).join(crate::paths::safe_component(tag))
}

/// The Minecraft server jar of version `mc`, which preparing reads (Mojang's file, downloaded once).
pub fn server_jar(dirs: &Dirs, mc: &str) -> PathBuf {
    horizon_dir(dirs).join("server-jars").join(format!("{}.jar", crate::paths::safe_component(mc)))
}

fn state_file(dirs: &Dirs) -> PathBuf {
    horizon_dir(dirs).join("state.json")
}

/// The release file for this computer, if Horizon makes one.
pub fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("horizon-linux-x86_64.tar.gz"),
        ("macos", "aarch64") => Some("horizon-macos-arm64.zip"),
        ("windows", "x86_64") => Some("horizon-windows-x64.zip"),
        _ => None,
    }
}

/// The programs of a build, with `.exe` on Windows.
pub fn program(build: &Path, name: &str) -> PathBuf {
    build.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// The numbers of a stable release's tag (`v0.2.97` is `[0, 2, 97]`). Nightlies (`nightly…`) and the Horizon
/// launcher's own releases (`launcher@…`) aren't game releases and give `None`.
pub fn tag_version(tag: &str) -> Option<Vec<u64>> {
    let numbers = tag.strip_prefix('v')?;
    numbers.split('.').map(|n| n.parse::<u64>().ok().filter(|_| !n.is_empty())).collect::<Option<Vec<_>>>()
}

/// What signatures cover: the tag, then the checksum list.
pub(crate) fn signed_message(tag: &str, sums: &[u8]) -> Vec<u8> {
    let mut message = format!("horizon release {tag}\n").into_bytes();
    message.extend_from_slice(sums);
    message
}

/// The `(file name, SHA-256 in lower-case hex)` pairs of a checksum list (`<hex>  <name>` or `<hex> *<name>`).
pub fn parse_sums(text: &str) -> Result<Vec<(String, String)>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let line = line.trim_end_matches('\r');
            let bad = || anyhow::anyhow!("{SUMS}: {line:?} is not `<sha256>  <file>`");
            let (hex, name) = line.split_once(' ').ok_or_else(bad)?;
            let name = name.strip_prefix([' ', '*']).unwrap_or(name);
            if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) || name.is_empty() {
                return Err(bad());
            }
            Ok((name.to_string(), hex.to_ascii_lowercase()))
        })
        .collect()
}

/// The SHA-256 the signed checksum list of release `tag` gives for `asset`, or why the download must be refused.
pub fn signed_sha256(tag: &str, asset: &str, sums: &[u8], signature: &str, keys: &[PublicKey]) -> Result<String> {
    use ring::signature::{ED25519, UnparsedPublicKey};
    let message = signed_message(tag, sums);
    let signed = signature
        .lines()
        .filter_map(|line| base64::engine::general_purpose::STANDARD.decode(line.trim()).ok())
        .any(|sig| keys.iter().any(|key| UnparsedPublicKey::new(&ED25519, key).verify(&message, &sig).is_ok()));
    if !signed {
        bail!(
            "Horizon {tag} isn't signed by the Horizon project's release key, so it was not installed. The download \
             may have been tampered with."
        );
    }
    let text = std::str::from_utf8(sums).map_err(|_| anyhow::anyhow!("{SUMS} of Horizon {tag} is not text"))?;
    parse_sums(text)?
        .into_iter()
        .find(|(name, _)| name == asset)
        .map(|(_, hex)| hex)
        .with_context(|| format!("{asset} is not in the signed checksums of Horizon {tag}, so it was not installed"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> Result<String> {
    use sha2::Digest;
    let mut file = std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// What installing writes into a build folder: the tag, the signed checksum of the archive it came from and the
/// SHA-256 of every program and of research-lock.json, worked out from the bytes just unpacked.
const MANIFEST: &str = ".blockforge-verified.json";
/// The programs a build may hold; the first two are needed (to prepare and to play).
const PROGRAMS: [&str; 4] = ["horizon-client", "horizon-prepare", "horizon-ctl", "horizon-server"];
const LOCK: &str = "research-lock.json";

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Manifest {
    tag: String,
    archive_sha256: String,
    /// File name in the build folder -> SHA-256.
    files: BTreeMap<String, String>,
}

/// The files a manifest may list, as named on this computer.
fn sealed_names() -> Vec<String> {
    PROGRAMS.iter().map(|p| format!("{p}{}", std::env::consts::EXE_SUFFIX)).chain([LOCK.to_string()]).collect()
}

/// The files every build must have.
fn required_names() -> Vec<String> {
    sealed_names().into_iter().filter(|n| !n.starts_with("horizon-ctl") && !n.starts_with("horizon-server")).collect()
}

/// The metadata of a plain file, looked at without following a link: a symlink (to a file anywhere) is an error.
fn regular(path: &Path) -> Result<std::fs::Metadata> {
    let meta = std::fs::symlink_metadata(path).with_context(|| format!("{} is missing", path.display()))?;
    if !meta.file_type().is_file() {
        bail!("{} is not a plain file", path.display());
    }
    Ok(meta)
}

/// What was looked at when a build was last hashed, so that unchanged files aren't hashed on every look.
type Stamp = Vec<(u64, Option<SystemTime>)>;

/// Builds this run of BlockForge has hashed and found good: the manifest's bytes and the size and time of every file
/// it lists. A file that changes in size or time is hashed again.
static CHECKED: std::sync::LazyLock<Mutex<HashMap<PathBuf, (Vec<u8>, Stamp)>>> =
    std::sync::LazyLock::new(Default::default);
/// Folders already named in the log as not usable.
static REPORTED: std::sync::LazyLock<Mutex<HashSet<PathBuf>>> = std::sync::LazyLock::new(Default::default);

/// Whether `dir` is a build this installer unpacked and that is still as it was.
///
/// The threat this meets is a damaged, half-written, planted or swapped folder in `horizon/builds/`: a folder that
/// is a link, one without the record written at install, one named like a newer release that was never installed,
/// or a program that was changed afterwards is not run (running it would hand the player's account to it). Someone
/// who can already write BlockForge's data folder as the player can do anything the player can, for instance rewrite
/// this record or BlockForge itself; that is out of scope.
pub(crate) fn check_build(dir: &Path) -> Result<()> {
    if !std::fs::symlink_metadata(dir).with_context(|| format!("{} is missing", dir.display()))?.is_dir() {
        bail!("{} is not a real folder", dir.display());
    }
    let manifest_file = dir.join(MANIFEST);
    let manifest_meta = regular(&manifest_file).context("the build has no record of its installation")?;
    let bytes = std::fs::read(&manifest_file)?;
    let manifest: Manifest = serde_json::from_slice(&bytes).context("the record of the installation is unreadable")?;
    if dir.file_name().map(|n| n.to_string_lossy()) != Some(manifest.tag.as_str().into()) {
        bail!("the build's record says it is {}", manifest.tag);
    }
    let allowed = sealed_names();
    if let Some(odd) = manifest.files.keys().find(|f| !allowed.contains(f)) {
        bail!("the record of the installation lists {odd:?}");
    }
    if let Some(missing) = required_names().into_iter().find(|f| !manifest.files.contains_key(f)) {
        bail!("the record of the installation doesn't list {missing}");
    }
    let mut stamp: Stamp = vec![(manifest_meta.len(), manifest_meta.modified().ok())];
    for file in manifest.files.keys() {
        let meta = regular(&dir.join(file))?;
        stamp.push((meta.len(), meta.modified().ok()));
    }
    let same = |checked: &(Vec<u8>, Stamp)| checked.0 == bytes && checked.1 == stamp;
    if CHECKED.lock().unwrap_or_else(|e| e.into_inner()).get(dir).is_some_and(same) {
        return Ok(());
    }
    for (file, want) in &manifest.files {
        if sha256_file(&dir.join(file))? != *want {
            CHECKED.lock().unwrap_or_else(|e| e.into_inner()).remove(dir);
            bail!("{file} is not the file that was installed");
        }
    }
    CHECKED.lock().unwrap_or_else(|e| e.into_inner()).insert(dir.to_path_buf(), (bytes, stamp));
    Ok(())
}

/// The installed builds, newest first: folders of `builds/` named after a release that [`check_build`] accepts.
/// Half-unpacked ones (`<tag>.partial`) and anything else aren't builds; a folder that looks like one but fails the
/// check is ignored and named once in the log.
pub fn installed(dirs: &Dirs) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(builds_dir(dirs)) else { return Vec::new() };
    let mut found: Vec<(Vec<u64>, String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let tag = e.file_name().to_string_lossy().to_string();
            let version = tag_version(&tag)?;
            let dir = e.path();
            match check_build(&dir) {
                Ok(()) => Some((version, tag, dir)),
                Err(why) => {
                    if REPORTED.lock().unwrap_or_else(|e| e.into_inner()).insert(dir.clone()) {
                        crate::logs::warn(format!(
                            "Horizon: ignoring the folder {} ({why:#}); it was not installed by BlockForge or has \
                             been changed",
                            dir.display()
                        ));
                    }
                    None
                }
            }
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, tag, dir)| (tag, dir)).collect()
}

/// When GitHub was last asked, and what it said.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
struct State {
    /// Seconds since 1970.
    #[serde(default)]
    checked: i64,
    #[serde(default)]
    latest: Option<String>,
}

fn read_state(dirs: &Dirs) -> State {
    std::fs::read(state_file(dirs)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write_state(dirs: &Dirs, state: &State) {
    let file = state_file(dirs);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, serde_json::to_vec_pretty(state).unwrap_or_default());
}

#[derive(Deserialize)]
struct GithubRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// The tag of the newest stable release, asking GitHub at `api`.
async fn latest_tag(api: &str) -> Result<String> {
    let r = crate::http::client()
        .get(api)
        .timeout(API_TIMEOUT)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .context("couldn't reach GitHub")?;
    if !r.status().is_success() {
        bail!("GitHub answered {}", r.status());
    }
    let release: GithubRelease = r.json().await.context("couldn't read GitHub's answer")?;
    if release.draft || release.prerelease || tag_version(&release.tag_name).is_none() {
        bail!("the newest release on GitHub ({}) is not a stable game release", release.tag_name);
    }
    Ok(release.tag_name)
}

/// The newest release as far as is known: from the last look at GitHub while it is recent, else from asking again
/// (and remembering the answer). An error when GitHub can't be asked.
async fn newest_release(dirs: &Dirs, api: &str, now: i64) -> Result<String> {
    let state = read_state(dirs);
    if let Some(tag) = state.latest.as_ref().filter(|_| fresh(state.checked, now)) {
        return Ok(tag.clone());
    }
    let tag = latest_tag(api).await?;
    write_state(dirs, &State { checked: now, latest: Some(tag.clone()) });
    Ok(tag)
}

/// Whether a look at GitHub at `checked` still holds at `now` (seconds since 1970). A clock that went back counts
/// as stale.
fn fresh(checked: i64, now: i64) -> bool {
    (0..CHECK_EVERY.as_secs() as i64).contains(&(now - checked))
}

/// The build to play: the newest release, downloaded when it isn't here yet. When GitHub can't be reached or the
/// download fails, the newest build already installed is used (and `notes` says why); without one, the error.
pub async fn ensure_newest(dirs: &Dirs, progress: &Arc<Progress>, notes: &mut Vec<String>) -> Result<PathBuf> {
    ensure_newest_from(dirs, LATEST_API, DOWNLOADS, &trusted_keys(), progress, notes).await
}

pub(crate) async fn ensure_newest_from(
    dirs: &Dirs,
    api: &str,
    downloads: &str,
    keys: &[PublicKey],
    progress: &Arc<Progress>,
    notes: &mut Vec<String>,
) -> Result<PathBuf> {
    if asset_name().is_none() {
        bail!("{} makes no build for this computer", super::NAME);
    }
    let here = installed(dirs);
    progress.stage(format!("Looking for the newest {}", super::NAME));
    let newest = newest_release(dirs, api, chrono::Utc::now().timestamp()).await;
    let wanted = match newest {
        Ok(tag) => tag,
        Err(e) => {
            return match here.into_iter().next() {
                Some((tag, dir)) => {
                    notes.push(format!("Couldn't look for a newer {} ({e:#}); playing {tag}", super::NAME));
                    Ok(dir)
                }
                None => Err(e.context(format!(
                    "{} isn't installed yet and couldn't be downloaded. Check your internet connection",
                    super::NAME
                ))),
            };
        }
    };
    // the build here is already as new (a release taken back on GitHub leaves an older "latest")
    if let Some((tag, dir)) = here.first() {
        if tag_version(tag) >= tag_version(&wanted) {
            return Ok(dir.clone());
        }
    }
    match install_from(dirs, &wanted, downloads, keys, progress).await {
        Ok(dir) => Ok(dir),
        Err(e) if progress.cancelled.load(Ordering::Relaxed) => Err(e),
        Err(e) => match here.into_iter().next() {
            Some((tag, dir)) => {
                notes.push(format!("Couldn't install {} {wanted} ({e:#}); playing {tag}", super::NAME));
                Ok(dir)
            }
            None => Err(e),
        },
    }
}

/// Download release `tag` from `downloads`, check it, and unpack it into its build folder.
async fn install_from(
    dirs: &Dirs,
    tag: &str,
    downloads: &str,
    keys: &[PublicKey],
    progress: &Arc<Progress>,
) -> Result<PathBuf> {
    let asset = asset_name().context("Horizon makes no build for this computer")?;
    if tag_version(tag).is_none() {
        bail!("{tag} is not a Horizon release");
    }
    progress.stage(format!("Checking {} {tag}", super::NAME));
    let sums = crate::http::get_bytes(&format!("{downloads}/{tag}/{SUMS}"))
        .await
        .with_context(|| format!("Horizon {tag} has no checksum list ({SUMS}), so it can't be installed"))?;
    let signature = crate::http::get_bytes(&format!("{downloads}/{tag}/{SIGNATURE}"))
        .await
        .with_context(|| format!("Horizon {tag} isn't signed ({SIGNATURE}), so it can't be installed"))?;
    let expected = signed_sha256(tag, asset, &sums, &String::from_utf8_lossy(&signature), keys)?;

    progress.stage(format!("Downloading {} {tag}", super::NAME));
    let bytes = download_archive(&format!("{downloads}/{tag}/{asset}"), progress).await?;

    progress.stage(format!("Unpacking {} {tag}", super::NAME));
    let (d, t) = (dirs.clone(), tag.to_string());
    tokio::task::spawn_blocking(move || unpack(&bytes, asset, &builds_dir(&d), &t, &expected)).await?
}

/// The largest archive taken in: a release is about 30 MB, and this much memory is better spent than a server's
/// endless answer.
const MAX_ARCHIVE: u64 = 1 << 30;

/// Why one try at downloading the archive failed, and whether another could help.
struct Attempt {
    error: anyhow::Error,
    retry: bool,
}

/// The archive at `url`, in memory (it is checked and unpacked from there, never read again from a file that
/// something else could change in between). Shows its progress and stops when the player cancels.
async fn download_archive(url: &str, progress: &Progress) -> Result<Vec<u8>> {
    progress.files_total.store(1, Ordering::Relaxed);
    progress.files_done.store(0, Ordering::Relaxed);
    progress.bytes_total.store(crate::install::SIZE_GUESS, Ordering::Relaxed);
    let mut last = None;
    for attempt in 0..4u32 {
        if attempt > 0 {
            let base = if cfg!(test) { 5 } else { 400 };
            tokio::time::sleep(Duration::from_millis(base * 2u64.pow(attempt - 1))).await;
        }
        if progress.cancelled.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        progress.bytes_done.store(0, Ordering::Relaxed);
        match fetch_archive(url, progress).await {
            Ok(bytes) => {
                progress.files_done.store(1, Ordering::Relaxed);
                return Ok(bytes);
            }
            Err(Attempt { error, retry }) => {
                last = Some(error);
                if !retry {
                    break;
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no attempt was made"))).with_context(|| format!("downloading {url}"))
}

async fn fetch_archive(url: &str, progress: &Progress) -> std::result::Result<Vec<u8>, Attempt> {
    let retry = |e: anyhow::Error| Attempt { error: e, retry: true };
    let r = crate::http::client()
        .get(url)
        .send()
        .await
        .map_err(|e| Attempt { retry: crate::net::is_transient_error(&e) || e.is_connect(), error: e.into() })?;
    if !r.status().is_success() {
        let retry = crate::net::is_transient_status(r.status().as_u16());
        return Err(Attempt { error: anyhow::anyhow!("HTTP {}", r.status()), retry });
    }
    let length = r.content_length();
    if length.is_some_and(|n| n > MAX_ARCHIVE) {
        return Err(Attempt { error: anyhow::anyhow!("the archive is larger than {MAX_ARCHIVE} bytes"), retry: false });
    }
    if let Some(n) = length {
        progress.bytes_total.store(n, Ordering::Relaxed);
    }
    let mut bytes = Vec::with_capacity(length.unwrap_or(0) as usize);
    let mut stream = r.bytes_stream();
    while let Some(chunk) = crate::install::next_chunk(&mut stream).await {
        if progress.cancelled.load(Ordering::Relaxed) {
            return Err(Attempt { error: anyhow::anyhow!("cancelled"), retry: false });
        }
        let chunk = chunk.map_err(retry)?;
        if bytes.len() as u64 + chunk.len() as u64 > MAX_ARCHIVE {
            return Err(Attempt {
                error: anyhow::anyhow!("the archive is larger than {MAX_ARCHIVE} bytes"),
                retry: false,
            });
        }
        bytes.extend_from_slice(&chunk);
        progress.bytes_done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }
    if length.is_some_and(|n| n != bytes.len() as u64) {
        return Err(retry(anyhow::anyhow!("the archive arrived incomplete")));
    }
    Ok(bytes)
}

/// Whether an archive entry is a file or a folder: a release holds nothing else.
enum Kind {
    Dir,
    File,
}

/// The entry's path inside the build, or `None` for the folder itself (`./`). Any path that is absolute, goes up
/// (`..`) or names a drive or a stream is refused.
fn clean_path(raw: &Path) -> Result<Option<PathBuf>> {
    let mut out = PathBuf::new();
    for part in raw.components() {
        match part {
            Component::Normal(p) if !p.to_string_lossy().contains([':', '\\']) => out.push(p),
            Component::CurDir => {}
            _ => bail!("the archive holds a path that leaves the build folder ({})", raw.display()),
        }
    }
    Ok((!out.as_os_str().is_empty()).then_some(out))
}

/// Call `visit` with every entry of the archive that is a plain file or folder, with its data. Anything else, a
/// symlink, hard link, device or pipe, or a path that leaves the folder, is an error: the release is flat files and
/// folders, and a link in it could point a program of the build anywhere.
fn each_entry(
    archive: &[u8],
    zip: bool,
    visit: &mut dyn FnMut(PathBuf, Kind, &mut dyn Read) -> Result<()>,
) -> Result<()> {
    if zip {
        let mut z = zip::ZipArchive::new(Cursor::new(archive)).context("reading the Horizon archive")?;
        for i in 0..z.len() {
            let mut f = z.by_index(i).context("reading the Horizon archive")?;
            let name = String::from_utf8_lossy(f.name_raw()).to_string();
            // 0o100000 is a file, 0o040000 a folder; no type means a file made without Unix modes
            let plain = f.unix_mode().is_none_or(|m| matches!(m & 0o170000, 0 | 0o100000 | 0o040000));
            if f.is_symlink() || !plain {
                bail!("the Horizon archive holds {name:?}, which is not a plain file or folder");
            }
            let Some(path) = f.enclosed_name() else {
                bail!("the Horizon archive holds a path that leaves the build folder ({name})");
            };
            let kind = if f.is_dir() { Kind::Dir } else { Kind::File };
            if let Some(path) = clean_path(&path)? {
                visit(path, kind, &mut f)?;
            }
        }
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(Cursor::new(archive)));
        for entry in tar.entries().context("reading the Horizon archive")? {
            let mut e = entry.context("reading the Horizon archive")?;
            let kind = match e.header().entry_type() {
                tar::EntryType::Regular | tar::EntryType::Continuous => Kind::File,
                tar::EntryType::Directory => Kind::Dir,
                tar::EntryType::XGlobalHeader => continue,
                other => {
                    let name = e.path().map(|p| p.display().to_string()).unwrap_or_default();
                    bail!("the Horizon archive holds {name:?} ({other:?}), which is not a plain file or folder");
                }
            };
            let path = e.path().context("reading the Horizon archive")?.into_owned();
            if let Some(path) = clean_path(&path)? {
                visit(path, kind, &mut e)?;
            }
        }
    }
    Ok(())
}

/// Check `archive` (a release's `.tar.gz` or `.zip`, its files in one folder) against `expected`, the SHA-256 the
/// signed list gives, and unpack that same buffer into `<builds>/<tag>`: first into `<tag>.partial`, moved into place
/// only when whole, so a build folder is never half there. Nothing is written when the checksum is wrong or when
/// any entry is not a plain file or folder inside the build.
pub(crate) fn unpack(archive: &[u8], asset: &str, builds: &Path, tag: &str, expected: &str) -> Result<PathBuf> {
    let archive_sha256 = sha256_hex(archive);
    if !archive_sha256.eq_ignore_ascii_case(expected) {
        bail!(
            "the downloaded {asset} doesn't match its signed checksum (damaged or tampered with), so it was not \
             installed. Try again"
        );
    }
    let zip = asset.ends_with(".zip");
    each_entry(archive, zip, &mut |_, _, _| Ok(()))?;
    let name = crate::paths::safe_component(tag);
    let target = builds.join(&name);
    let partial = builds.join(format!("{name}.partial"));
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial).with_context(|| format!("writing in {}", builds.display()))?;
    let result = (|| -> Result<PathBuf> {
        // every file's hash as it is written, so the record is of the bytes that came out of the archive
        let mut hashes: BTreeMap<PathBuf, String> = BTreeMap::new();
        each_entry(archive, zip, &mut |path, kind, data| {
            let dest = partial.join(&path);
            match kind {
                Kind::Dir => std::fs::create_dir_all(&dest)?,
                Kind::File => {
                    use sha2::Digest;
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    // a name that is already there, even a link, is an error rather than written through
                    let mut out = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&dest)
                        .with_context(|| format!("writing {}", dest.display()))?;
                    let mut hasher = sha2::Sha256::new();
                    let mut buf = vec![0u8; 1 << 16];
                    loop {
                        let n = data.read(&mut buf)?;
                        if n == 0 {
                            break;
                        }
                        hasher.update(&buf[..n]);
                        out.write_all(&buf[..n])?;
                    }
                    hashes.insert(path, hex::encode(hasher.finalize()));
                }
            }
            Ok(())
        })
        .context("unpacking the Horizon archive")?;
        // the archive holds one folder (`horizon-linux-x86_64/`) with the build in it
        let inner = std::fs::read_dir(&partial)?.flatten().map(|e| e.path()).collect::<Vec<_>>();
        let root = match inner.as_slice() {
            [one] if std::fs::symlink_metadata(one)?.is_dir() => one.clone(),
            _ => partial.clone(),
        };
        // looked at without following links: the build must be plain files inside this folder
        for file in required_names() {
            regular(&root.join(&file)).with_context(|| format!("the Horizon archive doesn't hold a build ({file})"))?;
        }
        let inside = root.strip_prefix(&partial)?.to_path_buf();
        let mut files = BTreeMap::new();
        for file in sealed_names() {
            if let Some(hash) = hashes.get(&inside.join(&file)) {
                files.insert(file, hash.clone());
            }
        }
        // a zip made elsewhere doesn't keep the programs' permission to run
        #[cfg(unix)]
        for p in PROGRAMS {
            use std::os::unix::fs::PermissionsExt;
            let path = program(&root, p);
            if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file()) {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
            }
        }
        let manifest = Manifest { tag: name.clone(), archive_sha256, files };
        write_manifest(&root, &manifest)?;
        if std::fs::symlink_metadata(&target).is_ok() {
            // another start finished the same build first
            if check_build(&target).is_ok() {
                return Ok(target.clone());
            }
            if std::fs::symlink_metadata(&target)?.is_dir() {
                std::fs::remove_dir_all(&target)?;
            } else {
                std::fs::remove_file(&target)?;
            }
        }
        std::fs::rename(&root, &target).with_context(|| format!("moving Horizon into {}", target.display()))?;
        // hashes it once now, here, rather than when Play looks for the build
        if let Err(e) = check_build(&target) {
            let _ = std::fs::remove_dir_all(&target);
            return Err(e.context("the installed Horizon doesn't match what was unpacked"));
        }
        Ok(target.clone())
    })();
    let _ = std::fs::remove_dir_all(&partial);
    result
}

fn write_manifest(root: &Path, manifest: &Manifest) -> Result<()> {
    let file = root.join(MANIFEST);
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&file)?;
    out.write_all(&serde_json::to_vec_pretty(manifest)?).with_context(|| format!("writing {}", file.display()))
}

/// Write the record of an installation for a build folder made by hand, for tests of what reads builds.
#[cfg(test)]
pub(crate) fn seal_for_tests(dir: &Path) {
    let mut files = BTreeMap::new();
    for file in sealed_names() {
        if let Ok(hash) = sha256_file(&dir.join(&file)) {
            files.insert(file, hash);
        }
    }
    let tag = dir.file_name().unwrap().to_string_lossy().to_string();
    let _ = std::fs::remove_file(dir.join(MANIFEST));
    write_manifest(dir, &Manifest { tag, archive_sha256: "0".repeat(64), files }).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{Reply, Server, runtime, scratch};
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn key() -> Ed25519KeyPair {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn public(k: &Ed25519KeyPair) -> PublicKey {
        PublicKey::try_from(k.public_key().as_ref()).unwrap()
    }

    fn sign(k: &Ed25519KeyPair, tag: &str, sums: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(k.sign(&signed_message(tag, sums)).as_ref())
    }

    #[test]
    fn the_built_in_release_key_reads() {
        assert_eq!(trusted_keys().len(), 1);
    }

    #[test]
    fn a_checksum_list_reads_like_sha256sum_writes_it() {
        let hex = "eb5d611d2b6e9295a35ab914e4a095053e23978b0ecc8195a643a3c3eebff68d";
        let text = format!("{hex}  horizon-linux-x86_64.tar.gz\n{} *horizon-windows-x64.zip\r\n\n", hex.to_uppercase());
        let sums = parse_sums(&text).unwrap();
        assert_eq!(sums[0], ("horizon-linux-x86_64.tar.gz".to_string(), hex.to_string()));
        // a binary-mode star and upper-case hex are read the same
        assert_eq!(sums[1], ("horizon-windows-x64.zip".to_string(), hex.to_string()));
        assert!(parse_sums("abc  short.zip").is_err());
        assert!(parse_sums(hex).is_err());
    }

    #[test]
    fn only_a_list_signed_for_its_own_tag_by_the_key_passes() {
        let k = key();
        let keys = [public(&k)];
        let hex = "a".repeat(64);
        let sums = format!("{hex}  horizon.zip\n").into_bytes();
        let sig = sign(&k, "v0.2.97", &sums);
        assert_eq!(signed_sha256("v0.2.97", "horizon.zip", &sums, &sig, &keys).unwrap(), hex);
        // any line of the signature file may be the one that verifies
        let two = format!("bm90IGEgc2lnbmF0dXJl\n{sig}\n");
        assert!(signed_sha256("v0.2.97", "horizon.zip", &sums, &two, &keys).is_ok());
        // another tag (an old release's list passed off as new), another list, another key, another file: refused
        assert!(signed_sha256("v0.2.98", "horizon.zip", &sums, &sig, &keys).is_err());
        let changed = format!("{}  horizon.zip\n", "b".repeat(64)).into_bytes();
        assert!(signed_sha256("v0.2.97", "horizon.zip", &changed, &sig, &keys).is_err());
        assert!(signed_sha256("v0.2.97", "horizon.zip", &sums, &sig, &[public(&key())]).is_err());
        assert!(signed_sha256("v0.2.97", "other.zip", &sums, &sig, &keys).is_err());
        assert!(signed_sha256("v0.2.97", "horizon.zip", &sums, "", &keys).is_err());
        // a nightly's list is signed for the tag `nightly` and is never a release's
        let nightly = sign(&k, "nightly", &sums);
        assert!(signed_sha256("v0.2.97", "horizon.zip", &sums, &nightly, &keys).is_err());
    }

    #[test]
    fn the_real_v0_2_97_list_verifies_with_the_built_in_key_and_a_nightlys_doesnt_pass_for_a_release() {
        // SHA256SUMS and SHA256SUMS.sig of horizon-rust/horizon v0.2.97, as GitHub serves them
        let sums = "16e62f7a9ac95d6be788e92cd5fb50a569926d2378531a4e0ea3a07337b79c11  horizon-linux-x86_64.tar.gz\n\
                    29d3cef224aecdafea314cf6a3acb5caf8228edef9a3c5d73a0b4fe3b2696fca  horizon-windows-x64.zip\n\
                    587fb8a653324e6f1b3bf59f1f2227ad99201ccaaf3f31024db25ccd4f37c63e  horizon-macos-arm64.zip\n\
                    57422db46533e077277ef1505dcc03805d52d15f3a213200569118c36c5f7cc9  horizon-server-linux-x86_64.tar.gz\n";
        let sig = "XDSckSAve+YAwmmuQ1wU8Wpr8rpN3U17OknKPGg3IK+mOUbFquPtS5pYd9vPA9GuxdxXmRkcD4/vAI2xrFr8Bg==\n";
        let keys = trusted_keys();
        let got = signed_sha256("v0.2.97", "horizon-linux-x86_64.tar.gz", sums.as_bytes(), sig, &keys);
        assert_eq!(got.unwrap(), "16e62f7a9ac95d6be788e92cd5fb50a569926d2378531a4e0ea3a07337b79c11");
        assert!(signed_sha256("v0.2.98", "horizon-linux-x86_64.tar.gz", sums.as_bytes(), sig, &keys).is_err());

        // the Nightly of 2026-10-30, signed for the nightly channel: genuine, but not a release
        let nightly = "eb5d611d2b6e9295a35ab914e4a095053e23978b0ecc8195a643a3c3eebff68d  horizon-linux-x86_64.tar.gz\n\
                       169b744d4f7c764d8197e6d164ef8b4b3e390f4d44f5f45cb7410f234f9500b9  horizon-windows-x64.zip\n\
                       3c77932ad4d286592f74a15006da73c78e3eaf12f0aea336aa6ec8672b36510d  horizon-macos-arm64.zip\n\
                       86108fa6477d1f645199b2cb1cff36239a58354fcca98a1ca3c6bd1875c76e9b  \
                       horizon-server-linux-x86_64.tar.gz\n\
                       a8af7058fd7c34671087f6724c44f53c9ebbb14c86f0aaf177fe2c205507b98d  horizon-source.tar.gz\n";
        let nightly_sig = "5ucgwR4JJt9mXINbxWbvbySy9okgLdLwfh7M9CHhJez3pWU/kxbYlL/FSlHRQ/Kp4amLEeO9iJWlwFvf4gkvCA==\n";
        let asset = "horizon-linux-x86_64.tar.gz";
        assert!(signed_sha256("nightly", asset, nightly.as_bytes(), nightly_sig, &keys).is_ok());
        assert!(signed_sha256("v0.2.97", asset, nightly.as_bytes(), nightly_sig, &keys).is_err());
    }

    #[test]
    fn only_stable_release_tags_count_and_they_sort_by_number() {
        assert_eq!(tag_version("v0.2.97"), Some(vec![0, 2, 97]));
        assert!(tag_version("v0.2.100") > tag_version("v0.2.97"));
        for tag in ["nightly", "nightly-2026-10-28", "launcher@0.3.1", "0.2.97", "v", "v0..1", "v1.x"] {
            assert_eq!(tag_version(tag), None, "{tag}");
        }
    }

    #[test]
    fn a_look_at_github_holds_for_six_hours() {
        assert!(fresh(1000, 1000));
        assert!(fresh(1000, 1000 + 6 * 3600 - 1));
        assert!(!fresh(1000, 1000 + 6 * 3600));
        // the clock went back: look again
        assert!(!fresh(1000, 999));
    }

    /// A build folder as a release archive holds it.
    fn fake_build(dir: &Path, mc: &str) {
        std::fs::create_dir_all(dir.join("crates/horizon-prepare/src")).unwrap();
        std::fs::write(
            dir.join("research-lock.json"),
            format!(r#"{{"minecraft":{{"version":"{mc}","client_sha1":"c","server_sha1":"s","java_major":25}}}}"#),
        )
        .unwrap();
        std::fs::write(dir.join("crates/horizon-prepare/src/lib.rs"), "pub const OUTPUT_VERSION: u32 = 2;\n").unwrap();
        for p in ["horizon-client", "horizon-prepare"] {
            std::fs::write(program(dir, p), b"#!/bin/sh\n").unwrap();
        }
    }

    fn tar_gz_of(dir: &Path, folder: &str) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut out, flate2::Compression::fast());
            let mut tar = tar::Builder::new(gz);
            tar.append_dir_all(folder, dir).unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        out
    }

    #[test]
    fn builds_are_listed_newest_first_and_half_unpacked_ones_left_out() {
        let base = scratch("hz-builds");
        let dirs = Dirs::at(&base);
        for tag in ["v0.2.9", "v0.2.97", "v0.2.10"] {
            fake_build(&build_dir(&dirs, tag), "26.3");
            seal_for_tests(&build_dir(&dirs, tag));
        }
        fake_build(&builds_dir(&dirs).join("v0.3.0.partial"), "26.3");
        seal_for_tests(&builds_dir(&dirs).join("v0.3.0.partial"));
        fake_build(&builds_dir(&dirs).join("nightly"), "26.3");
        seal_for_tests(&builds_dir(&dirs).join("nightly"));
        std::fs::create_dir_all(build_dir(&dirs, "v0.4.0")).unwrap();
        let tags: Vec<String> = installed(&dirs).into_iter().map(|(t, _)| t).collect();
        assert_eq!(tags, ["v0.2.97", "v0.2.10", "v0.2.9"]);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_signed_release_is_downloaded_checked_and_unpacked_and_a_tampered_one_refused() {
        let base = scratch("hz-install");
        let dirs = Dirs::at(base.join("data"));
        let src = base.join("src");
        fake_build(&src, "26.3");
        let asset = asset_name().unwrap_or("horizon-linux-x86_64.tar.gz");
        // the zip formats would need their own archive; the check and the unpacking are the same
        if !asset.ends_with(".tar.gz") {
            return;
        }
        let archive = tar_gz_of(&src, "horizon-linux-x86_64");
        let k = key();
        let rt = runtime();
        rt.block_on(async {
            let server = Server::start().await;
            let sha = {
                use sha2::Digest;
                hex::encode(sha2::Sha256::digest(&archive))
            };
            let sums = format!("{sha}  {asset}\n");
            server.route(&format!("/dl/v0.2.97/{SUMS}"), Reply::ok(sums.clone()));
            server.route(&format!("/dl/v0.2.97/{SIGNATURE}"), Reply::ok(sign(&k, "v0.2.97", sums.as_bytes())));
            server.route(&format!("/dl/v0.2.97/{asset}"), Reply::ok(archive.clone()));
            server.route("/latest", Reply::ok(r#"{"tag_name":"v0.2.97","draft":false,"prerelease":false}"#));
            let downloads = format!("{}/dl", server.base);
            let api = format!("{}/latest", server.base);
            let progress = Progress::new();
            let mut notes = Vec::new();

            // nothing installed, and a key that didn't sign it: refused, nothing unpacked
            let wrong = [public(&key())];
            let refused = ensure_newest_from(&dirs, &api, &downloads, &wrong, &progress, &mut notes).await;
            assert!(format!("{:#}", refused.unwrap_err()).contains("isn't signed"));
            assert!(installed(&dirs).is_empty());

            let keys = [public(&k)];
            let dir = ensure_newest_from(&dirs, &api, &downloads, &keys, &progress, &mut notes).await.unwrap();
            assert_eq!(dir, build_dir(&dirs, "v0.2.97"));
            assert!(dir.join("research-lock.json").is_file());
            assert!(check_build(&dir).is_ok());
            assert!(!builds_dir(&dirs).join("v0.2.97.partial").exists());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(program(&dir, "horizon-client")).unwrap().permissions().mode();
                assert_eq!(mode & 0o111, 0o111);
            }
            // the answer is remembered: the next start doesn't ask GitHub
            let asked = server.hits("/latest");
            ensure_newest_from(&dirs, &api, &downloads, &keys, &progress, &mut notes).await.unwrap();
            assert_eq!(server.hits("/latest"), asked);

            // a newer release whose archive was changed after signing: refused, the build here is played
            write_state(&dirs, &State::default());
            server.route("/latest", Reply::ok(r#"{"tag_name":"v0.2.98","draft":false,"prerelease":false}"#));
            let sums = format!("{}  {asset}\n", "0".repeat(64));
            server.route(&format!("/dl/v0.2.98/{SUMS}"), Reply::ok(sums.clone()));
            server.route(&format!("/dl/v0.2.98/{SIGNATURE}"), Reply::ok(sign(&k, "v0.2.98", sums.as_bytes())));
            server.route(&format!("/dl/v0.2.98/{asset}"), Reply::ok(archive.clone()));
            notes.clear();
            let played = ensure_newest_from(&dirs, &api, &downloads, &keys, &progress, &mut notes).await.unwrap();
            assert_eq!(played, build_dir(&dirs, "v0.2.97"));
            assert!(notes[0].contains("doesn't match its signed checksum"), "{notes:?}");
            assert!(!build_dir(&dirs, "v0.2.98").exists());

            // GitHub can't be reached: the build here is played
            write_state(&dirs, &State::default());
            notes.clear();
            let offline = format!("{}/nothing-here", server.base);
            let played = ensure_newest_from(&dirs, &offline, &downloads, &keys, &progress, &mut notes).await.unwrap();
            assert_eq!(played, build_dir(&dirs, "v0.2.97"));
            assert_eq!(notes.len(), 1);
        });
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn nothing_installed_and_no_github_is_an_error() {
        let base = scratch("hz-offline");
        let dirs = Dirs::at(&base);
        let rt = runtime();
        rt.block_on(async {
            let server = Server::start().await;
            let mut notes = Vec::new();
            let api = format!("{}/latest", server.base);
            let r = ensure_newest_from(&dirs, &api, &server.base, &[], &Progress::new(), &mut notes).await;
            assert!(format!("{:#}", r.unwrap_err()).contains("isn't installed yet"));
        });
        let _ = std::fs::remove_dir_all(base);
    }

    /// A release archive as the bytes of a `.tar.gz` with entries written as given (a path, a type, a link target
    /// and the content), including the ones `tar::Builder` refuses to write.
    fn tar_gz_with(entries: &[(&str, tar::EntryType, &str, &[u8])]) -> Vec<u8> {
        let mut raw = Vec::new();
        for (path, kind, link, data) in entries {
            let mut header = tar::Header::new_gnu();
            {
                let gnu = header.as_gnu_mut().unwrap();
                gnu.name[..path.len()].copy_from_slice(path.as_bytes());
                gnu.linkname[..link.len()].copy_from_slice(link.as_bytes());
            }
            header.set_entry_type(*kind);
            header.set_mode(0o644);
            header.set_size(data.len() as u64);
            header.set_cksum();
            raw.extend_from_slice(header.as_bytes());
            raw.extend_from_slice(data);
            raw.resize(raw.len().next_multiple_of(512), 0);
        }
        raw.extend_from_slice(&[0u8; 1024]);
        let mut out = Vec::new();
        let mut gz = flate2::write::GzEncoder::new(&mut out, flate2::Compression::fast());
        gz.write_all(&raw).unwrap();
        gz.finish().unwrap();
        out
    }

    fn zip_with(entries: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(&mut out);
        let options = zip::write::SimpleFileOptions::default();
        for (path, link, data) in entries {
            match link {
                Some(target) => z.add_symlink(*path, *target, options).unwrap(),
                None => {
                    z.start_file(*path, options).unwrap();
                    z.write_all(data).unwrap();
                }
            }
        }
        z.finish().unwrap();
        out.into_inner()
    }

    /// The files of a folder as (relative path, bytes), to see what an unpacking left.
    fn listing(dir: &Path) -> Vec<String> {
        fn walk(dir: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                out.push(e.path().display().to_string());
                if e.file_type().unwrap().is_dir() {
                    walk(&e.path(), out);
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, &mut out);
        out
    }

    const TAG: &str = "v0.2.97";
    const TAR: &str = "horizon-linux-x86_64.tar.gz";
    const ZIP: &str = "horizon-windows-x64.zip";

    /// A good release archive as tar.gz.
    fn good_tar() -> Vec<u8> {
        let base = scratch("hz-good-src");
        fake_build(&base, "26.3");
        let tar = tar_gz_of(&base, "horizon-linux-x86_64");
        let _ = std::fs::remove_dir_all(base);
        tar
    }

    /// Install the good archive as `tag` into `builds`; the build folder.
    fn install_good(builds: &Path, tag: &str) -> PathBuf {
        let bytes = good_tar();
        unpack(&bytes, TAR, builds, tag, &sha256_hex(&bytes)).unwrap()
    }

    #[test]
    fn a_good_archive_installs_with_a_record_that_passes() {
        let base = scratch("hz-good");
        let builds = base.join("builds");
        let bytes = good_tar();
        let dir = unpack(&bytes, TAR, &builds, TAG, &sha256_hex(&bytes)).unwrap();
        assert_eq!(dir, builds.join(TAG));
        assert!(check_build(&dir).is_ok());
        assert!(dir.join("crates/horizon-prepare/src/lib.rs").is_file());
        assert!(!builds.join(format!("{TAG}.partial")).exists());
        let manifest: Manifest = serde_json::from_slice(&std::fs::read(dir.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(manifest.tag, TAG);
        assert_eq!(manifest.archive_sha256, sha256_hex(&bytes));
        assert_eq!(manifest.files.len(), 3);
        assert_eq!(manifest.files[LOCK], sha256_file(&dir.join(LOCK)).unwrap());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn what_is_checked_is_what_is_unpacked_because_both_are_the_one_buffer() {
        let base = scratch("hz-swap");
        let builds = base.join("builds");
        let signed = good_tar();
        let expected = sha256_hex(&signed);
        // a replacement for the archive (as a file could be swapped after its hash was taken) is not the signed one:
        // the checksum is taken from the very bytes that would be unpacked
        let other = tar_gz_with(&[
            ("horizon-linux-x86_64/research-lock.json", tar::EntryType::Regular, "", b"{}"),
            ("horizon-linux-x86_64/horizon-client", tar::EntryType::Regular, "", b"evil"),
            ("horizon-linux-x86_64/horizon-prepare", tar::EntryType::Regular, "", b"evil"),
        ]);
        let err = unpack(&other, TAR, &builds, TAG, &expected).unwrap_err();
        assert!(format!("{err:#}").contains("doesn't match its signed checksum"), "{err:#}");
        let mut damaged = signed.clone();
        let middle = damaged.len() / 2;
        damaged[middle] ^= 1;
        assert!(unpack(&damaged, TAR, &builds, TAG, &expected).is_err());
        assert!(!builds.exists() || listing(&builds).is_empty(), "{:?}", listing(&builds));
        assert!(unpack(&signed, TAR, &builds, TAG, &expected).is_ok());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_tar_with_anything_but_plain_files_and_folders_is_refused_before_anything_is_written() {
        use tar::EntryType as T;
        let base = scratch("hz-tar-bad");
        let builds = base.join("builds");
        let ok = ("horizon-linux-x86_64/research-lock.json", T::Regular, "", &b"{}"[..]);
        let bad: [(&str, (&str, T, &str, &[u8])); 7] = [
            ("symlink", ("horizon-linux-x86_64/horizon-client", T::Symlink, "/etc/passwd", b"")),
            ("hard link", ("horizon-linux-x86_64/horizon-client", T::Link, "horizon-linux-x86_64/x", b"")),
            ("absolute", ("/tmp/hz-escape", T::Regular, "", b"x")),
            ("parent", ("../hz-escape", T::Regular, "", b"x")),
            ("nested parent", ("horizon-linux-x86_64/../../hz-escape", T::Regular, "", b"x")),
            ("device", ("horizon-linux-x86_64/dev", T::Char, "", b"")),
            ("fifo", ("horizon-linux-x86_64/pipe", T::Fifo, "", b"")),
        ];
        for (what, entry) in bad {
            let bytes = tar_gz_with(&[ok, entry]);
            let err = unpack(&bytes, TAR, &builds, TAG, &sha256_hex(&bytes)).unwrap_err();
            assert!(format!("{err:#}").contains("archive"), "{what}: {err:#}");
            // not even the good file before it, and nothing outside
            assert!(!builds.exists() || listing(&builds).is_empty(), "{what}: {:?}", listing(&builds));
            assert!(!base.join("hz-escape").exists() && !Path::new("/tmp/hz-escape").exists(), "{what}");
        }
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_zip_with_anything_but_plain_files_and_folders_is_refused_before_anything_is_written() {
        let base = scratch("hz-zip-bad");
        let builds = base.join("builds");
        let ok = ("horizon-windows-x64/research-lock.json", None, &b"{}"[..]);
        let bad: [(&str, (&str, Option<&str>, &[u8])); 4] = [
            ("symlink", ("horizon-windows-x64/horizon-client.exe", Some("/etc/passwd"), b"")),
            ("relative symlink", ("horizon-windows-x64/horizon-client.exe", Some("../../x"), b"")),
            ("absolute", ("/tmp/hz-escape", None, b"x")),
            ("parent", ("../hz-escape", None, b"x")),
        ];
        for (what, entry) in bad {
            let bytes = zip_with(&[ok, entry]);
            let err = unpack(&bytes, ZIP, &builds, TAG, &sha256_hex(&bytes)).unwrap_err();
            assert!(format!("{err:#}").contains("archive"), "{what}: {err:#}");
            assert!(!builds.exists() || listing(&builds).is_empty(), "{what}: {:?}", listing(&builds));
            assert!(!base.join("hz-escape").exists() && !Path::new("/tmp/hz-escape").exists(), "{what}");
        }
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_good_zip_installs_too() {
        let base = scratch("hz-zip-good");
        let builds = base.join("builds");
        let files = [
            ("horizon-windows-x64/", None, &b""[..]),
            ("horizon-windows-x64/research-lock.json", None, b"{}"),
            ("horizon-windows-x64/horizon-client", None, b"client"),
            ("horizon-windows-x64/horizon-prepare", None, b"prepare"),
            ("horizon-windows-x64/exporters/a.txt", None, b"a"),
        ];
        let bytes = zip_with(&files);
        // the programs are named without `.exe` here, as on the computers that test this
        let dir = unpack(&bytes, ZIP, &builds, TAG, &sha256_hex(&bytes));
        if cfg!(windows) {
            assert!(dir.is_err());
        } else {
            let dir = dir.unwrap();
            assert!(check_build(&dir).is_ok());
            assert!(dir.join("exporters/a.txt").is_file());
        }
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_program_changed_after_installing_is_not_a_build_any_more() {
        let base = scratch("hz-changed");
        let dirs = Dirs::at(&base);
        let dir = install_good(&builds_dir(&dirs), TAG);
        assert_eq!(installed(&dirs).len(), 1);
        // a longer file
        let client = program(&dir, "horizon-client");
        let mut text = std::fs::read(&client).unwrap();
        text.extend_from_slice(b"# evil\n");
        std::fs::write(&client, &text).unwrap();
        assert!(installed(&dirs).is_empty());
        assert!(format!("{:#}", check_build(&dir).unwrap_err()).contains("not the file that was installed"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_change_that_keeps_the_size_is_seen_when_the_time_moves_and_the_lock_is_covered_too() {
        let base = scratch("hz-same-size");
        let dirs = Dirs::at(&base);
        let dir = install_good(&builds_dir(&dirs), TAG);
        for file in [program(&dir, "horizon-prepare"), dir.join(LOCK)] {
            let before = installed(&dirs).len();
            assert_eq!(before, 1);
            let mut bytes = std::fs::read(&file).unwrap();
            let last = bytes.len() - 1;
            bytes[last] ^= 0x20;
            std::fs::write(&file, &bytes).unwrap();
            let later = SystemTime::now() + Duration::from_secs(60);
            std::fs::OpenOptions::new().write(true).open(&file).unwrap().set_modified(later).unwrap();
            assert!(installed(&dirs).is_empty(), "{}", file.display());
            bytes[last] ^= 0x20;
            std::fs::write(&file, &bytes).unwrap();
            std::fs::OpenOptions::new().write(true).open(&file).unwrap().set_modified(later).unwrap();
            assert_eq!(installed(&dirs).len(), 1, "back as it was: {}", file.display());
        }
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_fabricated_newer_folder_without_a_record_doesnt_win_over_the_installed_build() {
        let base = scratch("hz-fabricated");
        let dirs = Dirs::at(&base);
        let real = install_good(&builds_dir(&dirs), TAG);
        // what the review planted: a higher version with a plausible lock and client, and no record of an install
        let fake = build_dir(&dirs, "v999.0.0");
        fake_build(&fake, "26.3");
        // or with a record copied from the real build
        let copied = build_dir(&dirs, "v998.0.0");
        fake_build(&copied, "26.3");
        std::fs::copy(real.join(MANIFEST), copied.join(MANIFEST)).unwrap();
        // or one whose own record lists its own files
        let own = build_dir(&dirs, "v997.0.0");
        fake_build(&own, "26.3");
        seal_for_tests(&own);
        std::fs::write(program(&own, "horizon-client"), b"changed afterwards").unwrap();
        let tags: Vec<String> = installed(&dirs).into_iter().map(|(t, _)| t).collect();
        assert_eq!(tags, [TAG]);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_record_that_lists_other_paths_is_refused() {
        let base = scratch("hz-record");
        let dirs = Dirs::at(&base);
        let dir = install_good(&builds_dir(&dirs), TAG);
        let file = dir.join(MANIFEST);
        let mut manifest: Manifest = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        manifest.files.insert("../outside".into(), "0".repeat(64));
        std::fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(installed(&dirs).is_empty());
        manifest.files.remove("../outside");
        manifest.files.remove("horizon-client");
        std::fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(installed(&dirs).is_empty());
        std::fs::write(&file, b"not json").unwrap();
        assert!(installed(&dirs).is_empty());
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_build_folder_or_program_is_not_a_build() {
        let base = scratch("hz-links");
        let dirs = Dirs::at(&base);
        let builds = builds_dir(&dirs);
        // a complete, valid build for the tag, kept elsewhere and linked into place
        std::fs::create_dir_all(&builds).unwrap();
        let elsewhere = install_good(&base.join("elsewhere"), "v0.3.0");
        std::os::unix::fs::symlink(&elsewhere, builds.join("v0.3.0")).unwrap();
        assert!(check_build(&elsewhere).is_ok());
        assert!(installed(&dirs).is_empty());
        // a program that is a link to a file with the very same content
        let real = install_good(&builds, TAG);
        assert_eq!(installed(&dirs).len(), 1);
        let client = program(&real, "horizon-client");
        let copy = base.join("copy");
        std::fs::copy(&client, &copy).unwrap();
        std::fs::remove_file(&client).unwrap();
        std::os::unix::fs::symlink(&copy, &client).unwrap();
        assert!(installed(&dirs).is_empty());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn installing_over_a_planted_folder_replaces_it() {
        let base = scratch("hz-replace");
        let builds = base.join("builds");
        let planted = builds.join(TAG);
        fake_build(&planted, "26.3");
        let dir = install_good(&builds, TAG);
        assert!(check_build(&dir).is_ok());
        assert!(!dir.join("crates/horizon-prepare/src/lib.rs").metadata().is_err());
        // a second start that finishes the same build later keeps the first
        let again = install_good(&builds, TAG);
        assert_eq!(again, dir);
        let _ = std::fs::remove_dir_all(base);
    }
}
