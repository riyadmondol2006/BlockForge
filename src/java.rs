//! Finding or fetching the Java a Minecraft version needs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::config::Settings;
use crate::http;
use crate::install::{self, Job, Progress};
use crate::paths::Dirs;
use crate::platform;

const RUNTIME_INDEX_URL: &str =
    "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

#[derive(Deserialize)]
struct RuntimeEntry {
    manifest: RuntimeFile,
    version: RuntimeVersion,
}

#[derive(Deserialize)]
struct RuntimeFile {
    sha1: String,
    size: u64,
    url: String,
}

#[derive(Deserialize)]
struct RuntimeVersion {
    name: String,
}

#[derive(Deserialize)]
struct RuntimeManifest {
    files: HashMap<String, ManifestFile>,
}

#[derive(Deserialize)]
struct ManifestFile {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    executable: bool,
    #[serde(default)]
    downloads: Option<ManifestDownloads>,
    #[serde(default)]
    target: Option<String>,
}

#[derive(Deserialize)]
struct ManifestDownloads {
    raw: RuntimeFile,
}

/// Where Mojang's runtime `component` lives for this computer; the layout matches the Java edition's.
pub fn runtime_home(dirs: &Dirs, component: &str) -> PathBuf {
    let root = dirs.runtimes.join(component).join(platform::java_runtime_platform()).join(component);
    if platform::os() == platform::Os::Mac { root.join("jre.bundle/Contents/Home") } else { root }
}

pub fn java_binary(home: &Path) -> PathBuf {
    home.join("bin").join(platform::java_exe())
}

/// Whether Mojang's runtime `component` is here and whole. `bin/java` is one of the first of its hundreds of files
/// to arrive, so its presence alone says nothing: a download that was cancelled, or stopped by a full disk or a lost
/// connection, leaves the [`installing_marker`] behind, and such a runtime is fetched again (only what is missing or
/// wrong) instead of being trusted for ever.
pub fn runtime_ready(dirs: &Dirs, component: &str) -> bool {
    java_binary(&runtime_home(dirs, component)).is_file() && !installing_marker(dirs, component).exists()
}

/// The file that says a runtime's download began and hasn't finished.
fn installing_marker(dirs: &Dirs, component: &str) -> PathBuf {
    dirs.runtimes.join(component).join(platform::java_runtime_platform()).join(".installing")
}

/// Download Mojang's runtime `component` if it isn't here yet. Returns its home folder.
pub async fn ensure_runtime(dirs: &Dirs, component: &str, progress: &Arc<Progress>) -> Result<PathBuf> {
    get_runtime(dirs, component, progress, false).await
}

/// Check every file of Mojang's runtime `component` against its SHA-1 and download again what is damaged or
/// missing ("Repair game files"). Nothing is changed when the list of files can't be fetched (no connection).
pub async fn repair_runtime(dirs: &Dirs, component: &str, progress: &Arc<Progress>) -> Result<PathBuf> {
    get_runtime(dirs, component, progress, true).await
}

async fn get_runtime(dirs: &Dirs, component: &str, progress: &Arc<Progress>, repair: bool) -> Result<PathBuf> {
    let home = runtime_home(dirs, component);
    if !repair && runtime_ready(dirs, component) {
        return Ok(home);
    }
    progress.stage(format!("Getting Java ({component})"));
    let index: HashMap<String, HashMap<String, Vec<RuntimeEntry>>> =
        http::get_json_cached(RUNTIME_INDEX_URL, &dirs.cache.join("http"), Duration::from_secs(3600)).await?;
    let platform_key = platform::java_runtime_platform();
    let entry = index
        .get(platform_key)
        .and_then(|p| p.get(component))
        .and_then(|v| v.first())
        .with_context(|| format!("Mojang has no Java runtime {component} for {platform_key}"))?;

    let manifest_bytes = http::get_bytes(&entry.manifest.url).await?;
    if !http::sha1_hex(&manifest_bytes).eq_ignore_ascii_case(&entry.manifest.sha1) {
        bail!("the Java runtime list arrived damaged");
    }
    let manifest: RuntimeManifest = serde_json::from_slice(&manifest_bytes)?;

    let root = dirs.runtimes.join(component).join(platform_key).join(component);
    std::fs::create_dir_all(&root)?;
    let mut jobs = Vec::new();
    let mut links = Vec::new();
    for (rel, file) in &manifest.files {
        let Some(dest) = crate::paths::join_inside(&root, rel) else { continue };
        match file.kind.as_str() {
            "directory" => std::fs::create_dir_all(&dest)?,
            "file" => {
                if let Some(d) = &file.downloads {
                    let mut job = Job::new(&d.raw.url, dest).checked(Some(&d.raw.sha1), Some(d.raw.size as i64));
                    job.executable = file.executable;
                    jobs.push(job);
                }
            }
            "link" => {
                if let Some(target) = &file.target {
                    links.push((dest, target.clone()));
                }
            }
            _ => {}
        }
    }
    // a repair marks the runtime as being fixed only when something in it is damaged or missing: a whole runtime
    // whose repair was cancelled stays trusted (and usable offline)
    let needs_files = if repair {
        let check = jobs.clone();
        tokio::task::spawn_blocking(move || check.iter().any(|j| !install::is_good(j, true))).await?
    } else {
        true
    };
    if needs_files {
        std::fs::write(installing_marker(dirs, component), b"")
            .with_context(|| format!("writing in {}", root.display()))?;
        // a repair hashes every file; otherwise the size is enough for the files already there
        install::download_all(jobs, progress, repair).await?;
    }
    #[cfg(unix)]
    for (dest, target) in links {
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(&dest);
        std::os::unix::fs::symlink(target, dest)?;
    }
    #[cfg(not(unix))]
    let _ = links;
    let _ = std::fs::write(dirs.runtimes.join(component).join(platform_key).join(".version"), &entry.version.name);
    let _ = std::fs::write(dirs.runtimes.join(component).join("manifest.json"), &manifest_bytes);
    if !java_binary(&home).is_file() {
        bail!("the Java runtime was installed but {} is missing", java_binary(&home).display());
    }
    // another job may have fetched the same runtime at the same time and finished first
    match std::fs::remove_file(installing_marker(dirs, component)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(anyhow::Error::from(e).context(format!("finishing the Java runtime {component}")));
        }
        _ => {}
    }
    Ok(home)
}

/// The Java major version of an installation, read from its `release` file (starting Java would be slower).
pub fn major_version(home: &Path) -> Option<u32> {
    let release = std::fs::read_to_string(home.join("release")).ok()?;
    let line = release.lines().find(|l| l.starts_with("JAVA_VERSION="))?;
    let v = line.trim_start_matches("JAVA_VERSION=").trim_matches('"');
    let mut parts = v.split(['.', '_', '-']);
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 { parts.next()?.parse().ok() } else { Some(first) }
}

#[derive(Clone, Debug)]
pub struct JavaInstall {
    pub home: PathBuf,
    pub major: Option<u32>,
}

/// Look for `bin/java` (or `bin/java.exe`) under `dir`, up to ten levels down, without following links.
fn walk_for_java(dir: &Path, depth: u32, found: &mut Vec<PathBuf>) {
    if depth > 10 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ty) = e.file_type() else { continue };
        let path = e.path();
        if ty.is_dir() {
            walk_for_java(&path, depth + 1, found);
        } else if ty.is_file()
            && path.file_name().is_some_and(|n| n == platform::java_exe())
            && path.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "bin")
        {
            found.push(path);
        }
    }
}

/// Windows keeps the Java installs it knows about in the registry.
fn registry_javas() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for bitness in ["64", "32"] {
        let Ok(o) = crate::platform::command("reg")
            .args([
                "query",
                "HKEY_LOCAL_MACHINE\\Software\\JavaSoft\\",
                "/f",
                "Home",
                "/t",
                "REG_SZ",
                "/s",
                &format!("/reg:{bitness}"),
            ])
            .output()
        else {
            continue;
        };
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            if let Some(i) = line.find("REG_SZ") {
                out.push(PathBuf::from(line[i + 6..].trim()).join("bin").join("java.exe"));
            }
        }
    }
    out
}

/// Javas found on this computer, for the settings page to offer. The folders searched are the Java edition's;
/// `extra` is the player's own `javaInstallLocation`.
pub fn detect_installed_in(extra: Option<&Path>) -> Vec<JavaInstall> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut search: Vec<PathBuf> = Vec::new();
    match platform::os() {
        platform::Os::Linux => search.extend(
            ["/usr/java", "/usr/lib/jvm", "/usr/lib64/jvm", "/usr/lib32/jvm", "/opt/java", "/opt/jdk"]
                .map(PathBuf::from),
        ),
        platform::Os::Mac => {
            search.push(PathBuf::from("/Library/Java/JavaVirtualMachines"));
            search.push(PathBuf::from("/opt/homebrew/opt"));
        }
        platform::Os::Windows => {
            found.extend(registry_javas());
            if let Some(pf) = std::env::var_os("programfiles") {
                for d in [
                    "Java",
                    "Amazon Corretto",
                    "AdoptOpenJDK",
                    "Zulu",
                    "Eclipse Adoptium",
                    "Eclipse Foundation",
                    "Microsoft",
                ] {
                    search.push(PathBuf::from(&pf).join(d));
                }
            }
        }
    }
    if let Some(e) = extra.filter(|p| p.is_dir()) {
        search.push(e.to_path_buf());
    }
    for d in &search {
        walk_for_java(d, 0, &mut found);
    }
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        found.push(PathBuf::from(home).join("bin").join(platform::java_exe()));
    }
    let mut out: Vec<JavaInstall> = Vec::new();
    for bin in found {
        let Some(home) = bin.parent().and_then(|b| b.parent()) else { continue };
        let Ok(home) = home.canonicalize() else { continue };
        if java_binary(&home).is_file() && !out.iter().any(|j| j.home == home) {
            out.push(JavaInstall { major: major_version(&home), home });
        }
    }
    out.sort_by(|a, b| b.major.cmp(&a.major));
    out
}

pub fn detect_installed() -> Vec<JavaInstall> {
    detect_installed_in(None)
}

/// Which Java runs this instance: its own override, Mojang's runtime for its version, the Java in the settings,
/// or whatever `java` is on the path — in that order.
pub struct JavaChoice {
    pub home: Option<PathBuf>,
    pub binary: PathBuf,
}

pub fn choose(
    dirs: &Dirs,
    settings: &Settings,
    instance_java: Option<&str>,
    component: Option<&str>,
    use_mojang: bool,
) -> JavaChoice {
    if let Some(p) = instance_java.filter(|p| !p.is_empty()) {
        let home = PathBuf::from(p);
        return JavaChoice { binary: java_binary(&home), home: Some(home) };
    }
    if use_mojang {
        if let Some(c) = component.filter(|c| !c.is_empty()) {
            if runtime_ready(dirs, c) {
                let home = runtime_home(dirs, c);
                return JavaChoice { binary: java_binary(&home), home: Some(home) };
            }
        }
    }
    if !settings.java_path.is_empty() {
        let home = PathBuf::from(&settings.java_path);
        if java_binary(&home).is_file() {
            return JavaChoice { binary: java_binary(&home), home: Some(home) };
        }
    }
    JavaChoice { home: None, binary: PathBuf::from(platform::java_exe()) }
}

// ---------------------------------------------------------------------------------------------------------
// A full OpenJDK for running the Forge and NeoForge installers

/// Forge's installer rebuilds the game jar with patch tools whose output must match checksums made on a stock
/// OpenJDK. Mojang's Java runtimes (Microsoft builds) produce different bytes and the installer rejects them,
/// so the installers run on Temurin instead. It is downloaded once and kept in `runtimes/`.
pub async fn ensure_installer_java(dirs: &Dirs, progress: &Arc<Progress>) -> Result<PathBuf> {
    let root = dirs.runtimes.parent().unwrap_or(&dirs.base).join("temurin-21");
    if let Some(home) = find_home(&root) {
        return Ok(home);
    }
    let (os, ext) = match platform::os() {
        platform::Os::Linux => ("linux", "tar.gz"),
        platform::Os::Mac => ("mac", "tar.gz"),
        platform::Os::Windows => ("windows", "zip"),
    };
    let arch = if platform::is_arm64() { "aarch64" } else { "x64" };
    let url = format!("https://api.adoptium.net/v3/binary/latest/21/ga/{os}/{arch}/jre/hotspot/normal/eclipse");
    progress.stage("Getting Java 21 for the installer (one time, about 50 MB)");
    let archive = dirs.temp.join(format!("temurin-21.{ext}"));
    let fetched = install::download_all(vec![Job::new(url, &archive)], progress, false).await;
    if let Err(e) = fetched {
        // fall back to a full JDK that is already installed
        if let Some(j) = detect_installed().into_iter().find(|j| j.major.is_some_and(|m| m >= 17)) {
            return Ok(j.home);
        }
        return Err(e).context("couldn't download Java 21 for the loader installer, and no Java 17+ is installed");
    }
    let (archive2, root2) = (archive.clone(), root.clone());
    let unpacked = tokio::task::spawn_blocking(move || unpack_whole(&archive2, &root2)).await?;
    // a damaged download (a portal's web page saved in its place) is not kept to be tried again and again
    let _ = std::fs::remove_file(&archive);
    unpacked?;
    find_home(&root).context("the downloaded Java 21 has an unexpected layout")
}

/// Unpack a Java archive into `root` so that `root` is either missing or complete: it is unpacked next to it first
/// and moved into place only when it holds a whole Java. Closing the launcher half way used to leave a `bin/java`
/// without the rest, trusted from then on.
fn unpack_whole(archive: &Path, root: &Path) -> Result<()> {
    let parent = root.parent().context("no folder for Java")?;
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".unpacking-{}", uuid::Uuid::new_v4().simple()));
    let result = (|| -> Result<()> {
        std::fs::create_dir_all(&staging)?;
        if archive.extension().is_some_and(|e| e == "zip") {
            zip::ZipArchive::new(std::fs::File::open(archive)?)?.extract(&staging)?;
        } else {
            let gz = flate2::read::GzDecoder::new(std::fs::File::open(archive)?);
            tar::Archive::new(gz).unpack(&staging)?;
        }
        find_home(&staging).context("the downloaded Java is incomplete or not a Java")?;
        // whatever is there is not a whole Java (`find_home` said so before the download)
        if root.exists() {
            std::fs::remove_dir_all(root).with_context(|| format!("replacing {}", root.display()))?;
        }
        rename_retrying(&staging, root).with_context(|| format!("moving Java into {}", root.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

/// `rename`, tried again for a few seconds on Windows, where a virus scanner looking at the hundreds of new `.exe`
/// and `.dll` files of a Java just unpacked makes moving their folder fail with "access denied" for a moment.
fn rename_retrying(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0u64;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if cfg!(windows) && tries < 15 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            done => return done,
        }
    }
}

fn find_home(root: &Path) -> Option<PathBuf> {
    for e in std::fs::read_dir(root).ok()?.flatten() {
        let mut p = e.path();
        if platform::os() == platform::Os::Mac && p.join("Contents/Home").is_dir() {
            p = p.join("Contents/Home");
        }
        // `lib/modules` is the biggest file of a Java and among the last to be unpacked: without it the unpacking
        // never finished
        if java_binary(&p).is_file() && p.join("lib").join("modules").is_file() {
            return Some(p);
        }
    }
    None
}

/// Mojang's Java runtimes for this computer as `(component, Java version)`, for the "Java runtime" choice.
pub async fn available_runtimes(dirs: &Dirs) -> Result<Vec<(String, String)>> {
    let index: HashMap<String, HashMap<String, Vec<RuntimeEntry>>> =
        http::get_json_cached(RUNTIME_INDEX_URL, &dirs.cache.join("http"), Duration::from_secs(3600)).await?;
    let mut list: Vec<(String, String)> = index
        .get(platform::java_runtime_platform())
        .map(|p| p.iter().filter_map(|(c, v)| v.first().map(|e| (c.clone(), e.version.name.clone()))).collect())
        .unwrap_or_default();
    list.sort();
    Ok(list)
}

#[cfg(test)]
mod runtime_tests {
    use super::*;

    #[test]
    fn a_runtime_whose_download_stopped_half_way_is_not_ready() {
        let base = crate::testutil::scratch("runtime-ready");
        let dirs = Dirs::at(&base);
        let home = runtime_home(&dirs, "java-runtime-delta");
        assert!(!runtime_ready(&dirs, "java-runtime-delta"), "nothing there");
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::write(java_binary(&home), b"#!").unwrap();
        // a runtime from before the marker existed (or from the Java edition) counts as whole
        assert!(runtime_ready(&dirs, "java-runtime-delta"));
        // a download that began and never finished
        std::fs::write(installing_marker(&dirs, "java-runtime-delta"), b"").unwrap();
        assert!(!runtime_ready(&dirs, "java-runtime-delta"), "bin/java alone must not be trusted");
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod unpack_tests {
    use super::*;

    fn tar_gz(path: &Path, files: &[(&str, &[u8])]) {
        let gz = flate2::write::GzEncoder::new(std::fs::File::create(path).unwrap(), flate2::Compression::fast());
        let mut t = tar::Builder::new(gz);
        for (name, body) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(body.len() as u64);
            h.set_mode(0o755);
            h.set_cksum();
            t.append_data(&mut h, name, *body).unwrap();
        }
        t.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn java_is_moved_into_place_only_when_it_is_whole() {
        let base = crate::testutil::scratch("unpack-java");
        let root = base.join("temurin-21");
        let java = format!("jdk-21/bin/{}", platform::java_exe());
        // an archive that stops before lib/modules (what an interrupted download unpacks to)
        let partial = base.join("partial.tar.gz");
        tar_gz(&partial, &[(&java, b"#!")]);
        assert!(unpack_whole(&partial, &root).is_err());
        assert!(!root.exists(), "nothing half unpacked is left to be trusted");
        assert!(find_home(&root).is_none());
        // a whole one
        let whole = base.join("whole.tar.gz");
        tar_gz(&whole, &[(&java, b"#!"), ("jdk-21/lib/modules", b"modules")]);
        unpack_whole(&whole, &root).unwrap();
        assert!(find_home(&root).is_some());
        // not an archive at all
        std::fs::write(base.join("page.tar.gz"), b"<html>Sign in to the Wi-Fi</html>").unwrap();
        assert!(unpack_whole(&base.join("page.tar.gz"), &base.join("other")).is_err());
        let leftovers: Vec<_> = std::fs::read_dir(&base)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".unpacking-"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }
}
