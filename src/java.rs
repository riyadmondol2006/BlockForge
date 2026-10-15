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

pub fn runtime_ready(dirs: &Dirs, component: &str) -> bool {
    java_binary(&runtime_home(dirs, component)).is_file()
}

/// Download Mojang's runtime `component` if it isn't here yet. Returns its home folder.
pub async fn ensure_runtime(dirs: &Dirs, component: &str, progress: &Arc<Progress>) -> Result<PathBuf> {
    let home = runtime_home(dirs, component);
    if runtime_ready(dirs, component) {
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
        let dest = root.join(rel);
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
    install::download_all(jobs, progress, false).await?;
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
    tokio::task::spawn_blocking(move || -> Result<()> {
        std::fs::create_dir_all(&root2)?;
        if archive2.extension().is_some_and(|e| e == "zip") {
            zip::ZipArchive::new(std::fs::File::open(&archive2)?)?.extract(&root2)?;
        } else {
            let gz = flate2::read::GzDecoder::new(std::fs::File::open(&archive2)?);
            tar::Archive::new(gz).unpack(&root2)?;
        }
        Ok(())
    })
    .await??;
    let _ = std::fs::remove_file(&archive);
    find_home(&root).context("the downloaded Java 21 has an unexpected layout")
}

fn find_home(root: &Path) -> Option<PathBuf> {
    for e in std::fs::read_dir(root).ok()?.flatten() {
        let mut p = e.path();
        if platform::os() == platform::Os::Mac && p.join("Contents/Home").is_dir() {
            p = p.join("Contents/Home");
        }
        if java_binary(&p).is_file() {
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
