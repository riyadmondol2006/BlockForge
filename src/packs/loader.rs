//! The loader a pack asks for. A pack names it with a class from the Java edition (`...loaders.forge.ForgeLoader`
//! and so on) and a few facts (`metadata`); this turns that into an installed loader.
//!
//! Fabric, Quilt, and Forge/NeoForge from Minecraft 1.13 on go through [`crate::loaders`]. Forge for Minecraft
//! 1.12.2 and older is read from its installer by hand here, because those installers describe the game in a
//! different file than the newer ones and need no Java to be run. Legacy Fabric uses its own meta server.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use super::DOWNLOAD_SERVER;
use super::dl::{self, Dl};
use crate::http;
use crate::install::Progress;
use crate::instances::LoaderVersion;
use crate::loaders::{self, Loader};
use crate::model::{Artifact, LibDownloads, Library, VersionJson};
use crate::paths::{self, Dirs};

const LEGACY_FABRIC_META: &str = "https://meta.legacyfabric.net/v2";
const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/";
const MC_LIBRARIES: &str = "https://libraries.minecraft.net/";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Forge for Minecraft 1.12.2 and older (`ForgeLoader`).
    Forge,
    /// Forge for Minecraft 1.13 and newer (`Forge113Loader`), which needs its installer run.
    Forge113,
    /// Forge for Minecraft 1.5.2 and older: a zip joined into the game jar.
    LegacyForge,
    Fabric,
    LegacyFabric,
    Quilt,
    NeoForge,
    Paper,
    Purpur,
}

impl Kind {
    /// The loader class of a pack definition decides. The server publishes the old `com.atlauncher...` names and
    /// the Java edition maps them to its own, so only the end of the name counts.
    pub fn from_class(class: &str) -> Option<Kind> {
        let name = class.rsplit('.').next().unwrap_or(class);
        Some(match name {
            "ForgeLoader" => Kind::Forge,
            "Forge113Loader" => Kind::Forge113,
            "LegacyForgeLoader" => Kind::LegacyForge,
            "FabricLoader" => Kind::Fabric,
            "LegacyFabricLoader" => Kind::LegacyFabric,
            "QuiltLoader" => Kind::Quilt,
            "NeoForgeLoader" => Kind::NeoForge,
            "PaperLoader" => Kind::Paper,
            "PurpurLoader" => Kind::Purpur,
            _ => return None,
        })
    }

    /// The `type` written to `launcher.loaderVersion.type` in `instance.json`.
    pub fn type_name(self) -> &'static str {
        match self {
            Kind::Forge | Kind::Forge113 | Kind::LegacyForge => "Forge",
            Kind::Fabric => "Fabric",
            Kind::LegacyFabric => "LegacyFabric",
            Kind::Quilt => "Quilt",
            Kind::NeoForge => "NeoForge",
            Kind::Paper => "Paper",
            Kind::Purpur => "Purpur",
        }
    }

    pub fn is_forge(self) -> bool {
        matches!(self, Kind::Forge | Kind::Forge113 | Kind::LegacyForge)
    }

    /// Whether a client can use this loader (Paper and Purpur are servers only).
    pub fn for_clients(self) -> bool {
        !matches!(self, Kind::Paper | Kind::Purpur)
    }
}

/// The pack's loader class for a Minecraft version that the pack itself does not name, e.g. when the loader
/// came from a Technic modpack's own `version.json`.
pub fn forge_class_for(mc: &str) -> &'static str {
    if match_version(mc, "1.13", false, true) {
        "Forge113Loader"
    } else if match_version(mc, "1.5", true, true) {
        "LegacyForgeLoader"
    } else {
        "ForgeLoader"
    }
}

/// A loader version the player picked, instead of the one the pack was made with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub version: String,
    pub raw_version: String,
}

/// A loader that is installed: the game description with the loader in it, and what to record.
#[derive(Clone, Debug)]
pub struct Installed {
    pub merged: VersionJson,
    pub info: LoaderVersion,
    /// The loader brought a zip to join into the game jar (very old Forge); it was copied to `bin/modpack.jar`.
    pub joins_game_jar: bool,
}

/// The Java edition's version matching: only as many parts of `version` as `matches` has are compared, so
/// `1.12.2` against `1.13` looks at `1.12` only. `less_than` picks the direction, `equal` counts a tie.
pub fn match_version(version: &str, matches: &str, less_than: bool, equal: bool) -> bool {
    if version.contains('_') {
        return false;
    }
    let (Some(v), Some(m)) = (numeric_parts(version), numeric_parts(matches)) else { return false };
    let mut ord = std::cmp::Ordering::Equal;
    for i in 0..m.len() {
        let a = v.get(i).copied().unwrap_or(0);
        let b = m[i];
        ord = a.cmp(&b);
        if ord != std::cmp::Ordering::Equal {
            break;
        }
    }
    if equal && ord == std::cmp::Ordering::Equal {
        return true;
    }
    if less_than { ord == std::cmp::Ordering::Less } else { ord == std::cmp::Ordering::Greater }
}

fn numeric_parts(version: &str) -> Option<Vec<i64>> {
    let mut out = Vec::new();
    for part in version.split('.') {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        out.push(digits.parse().ok()?);
    }
    Some(out)
}

fn meta_str(meta: &Map<String, Value>, key: &str) -> Option<String> {
    meta.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

/// The loaders a player can pick between for a Minecraft version, newest first.
pub async fn choosable(dirs: &Dirs, kind: Kind, mc: &str) -> Result<Vec<LoaderVersion>> {
    let type_name = kind.type_name().to_string();
    let plain =
        |v: String| LoaderVersion { version: v.clone(), raw_version: v, kind: type_name.clone(), ..Default::default() };
    Ok(match kind {
        Kind::Fabric => loaders::versions(dirs, Loader::Fabric, mc).await?.into_iter().map(plain).collect(),
        Kind::Quilt => loaders::versions(dirs, Loader::Quilt, mc).await?.into_iter().map(plain).collect(),
        Kind::NeoForge => loaders::versions(dirs, Loader::NeoForge, mc).await?.into_iter().map(plain).collect(),
        Kind::LegacyFabric => {
            #[derive(serde::Deserialize)]
            struct E {
                loader: L,
            }
            #[derive(serde::Deserialize)]
            struct L {
                version: String,
            }
            let list: Vec<E> = http::get_json_cached(
                &format!("{LEGACY_FABRIC_META}/versions/loader/{mc}"),
                &dirs.cache.join("http"),
                std::time::Duration::from_secs(900),
            )
            .await?;
            list.into_iter().map(|e| plain(e.loader.version)).collect()
        }
        Kind::Forge | Kind::Forge113 | Kind::LegacyForge => loaders::versions(dirs, Loader::Forge, mc)
            .await?
            .into_iter()
            .map(|raw| {
                // `1.12.2-14.23.5.2860` -> `14.23.5.2860`
                let version = raw.split_once('-').map(|x| x.1.to_string()).unwrap_or_else(|| raw.clone());
                LoaderVersion { version, raw_version: raw, kind: type_name.clone(), ..Default::default() }
            })
            .collect(),
        Kind::Paper | Kind::Purpur => Vec::new(),
    })
}

/// Install the loader a pack asks for and put it over `vanilla`.
pub async fn install(
    dirs: &Dirs,
    kind: Kind,
    metadata: &Map<String, Value>,
    mc: &str,
    choice: Option<&Choice>,
    vanilla: &VersionJson,
    root: &Path,
    progress: &Arc<Progress>,
) -> Result<Installed> {
    match kind {
        Kind::Fabric | Kind::Quilt => {
            let wanted = match choice {
                Some(c) => Some(c.version.clone()),
                None => meta_str(metadata, "loader"),
            };
            let loader = if kind == Kind::Fabric { Loader::Fabric } else { Loader::Quilt };
            let (merged, info) = loaders::install(dirs, loader, mc, wanted.as_deref(), vanilla, progress).await?;
            Ok(Installed { merged, info: info.context("the loader didn't report its version")?, joins_game_jar: false })
        }
        Kind::NeoForge | Kind::Forge113 => {
            let (loader, wanted) = if kind == Kind::NeoForge {
                let v = choice
                    .map(|c| c.raw_version.clone())
                    .or_else(|| meta_str(metadata, "rawVersion"))
                    .or_else(|| meta_str(metadata, "version"))
                    .or_else(|| meta_str(metadata, "loader"));
                (Loader::NeoForge, v)
            } else {
                let v = choice
                    .map(|c| c.raw_version.clone())
                    .or_else(|| meta_str(metadata, "rawVersion"))
                    .or_else(|| meta_str(metadata, "version").map(|v| format!("{mc}-{v}")));
                (Loader::Forge, v)
            };
            let (merged, info) = loaders::install(dirs, loader, mc, wanted.as_deref(), vanilla, progress).await?;
            Ok(Installed { merged, info: info.context("the loader didn't report its version")?, joins_game_jar: false })
        }
        Kind::LegacyFabric => {
            let version = match choice.map(|c| c.version.clone()).or_else(|| meta_str(metadata, "loader")) {
                Some(v) => v,
                None => choosable(dirs, kind, mc)
                    .await?
                    .into_iter()
                    .next()
                    .map(|l| l.version)
                    .context("Legacy Fabric has no loader for this Minecraft version")?,
            };
            progress.stage(format!("Reading Legacy Fabric {version}"));
            let profile: VersionJson =
                http::get_json(&format!("{LEGACY_FABRIC_META}/versions/loader/{mc}/{version}/profile/json")).await?;
            let info = LoaderVersion {
                version: version.clone(),
                raw_version: version,
                kind: "LegacyFabric".into(),
                ..Default::default()
            };
            Ok(Installed { merged: profile.merged_over(vanilla.clone()), info, joins_game_jar: false })
        }
        Kind::Forge => install_old_forge(dirs, metadata, mc, choice, vanilla, progress).await,
        Kind::LegacyForge => install_legacy_forge(dirs, metadata, mc, choice, vanilla, root, progress).await,
        Kind::Paper | Kind::Purpur => {
            bail!("{} is a server and can't be installed on a client instance", kind.type_name())
        }
    }
}

fn forge_versions(metadata: &Map<String, Value>, mc: &str, choice: Option<&Choice>) -> Result<(String, String)> {
    if let Some(c) = choice {
        return Ok((c.version.clone(), c.raw_version.clone()));
    }
    let version = meta_str(metadata, "version").context("the pack doesn't say which Forge version it needs")?;
    let raw = meta_str(metadata, "rawVersion").unwrap_or_else(|| format!("{mc}-{version}"));
    Ok((version, raw))
}

// ---------------------------------------------------------------------------------------------------------
// Forge 1.12.2 and older

/// The libraries of an old Forge description, in the Java edition's way of reading them.
fn old_forge_libraries(libs: &[Value], dirs: &Dirs, mirror_has: &std::collections::HashSet<String>) -> Vec<Library> {
    let mut out = Vec::new();
    for l in libs {
        let Some(name) = l.get("name").and_then(Value::as_str) else { continue };
        if let Some(downloads) = l.get("downloads").and_then(|d| serde_json::from_value::<LibDownloads>(d.clone()).ok())
        {
            // 1.13+ style entry: already complete
            let mut lib = Library { name: name.to_string(), downloads: Some(downloads), ..Default::default() };
            if let Some(a) = lib.downloads.as_mut().and_then(|d| d.artifact.as_mut()) {
                if a.path.is_none() {
                    a.path = paths::maven_path(name);
                }
                if a.url.is_empty() && !a.path.as_deref().unwrap_or("").ends_with("-client.jar") {
                    // the installer carries the file itself (14.23.5.2851 and later); when it is not there yet the
                    // official Maven has it
                    a.url = format!("{FORGE_MAVEN}{}", a.path.as_deref().unwrap_or(""));
                }
            }
            out.push(lib);
            continue;
        }
        // older forge versions list natives next to normal libraries; the Java edition leaves them out
        if l.get("natives").is_some() {
            continue;
        }
        let checksums: Vec<String> = l
            .get("checksums")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let Some(mut path) = paths::maven_path(name) else { continue };
        if name.starts_with("net.minecraftforge:forge:")
            && l.get("clientreq").is_none()
            && l.get("serverreq").is_none()
            && l.get("checksums").is_none()
        {
            path = path.trim_end_matches(".jar").to_string() + "-universal.jar";
        }
        let url = match l.get("url").and_then(Value::as_str) {
            Some("") => format!("{FORGE_MAVEN}{path}"),
            Some(u) => {
                let is_forge_maven = [
                    "https://maven.minecraftforge.net/",
                    "http://maven.minecraftforge.net/",
                    "https://files.minecraftforge.net/maven/",
                    "http://files.minecraftforge.net/maven/",
                ]
                .iter()
                .any(|b| u.eq_ignore_ascii_case(b));
                if is_forge_maven {
                    // ATLauncher keeps a copy of Forge's files; use it when it has this one
                    if mirror_has.contains(&path) {
                        format!("{DOWNLOAD_SERVER}/maven/{path}")
                    } else {
                        format!("{FORGE_MAVEN}{path}")
                    }
                } else {
                    format!("{}{path}", if u.ends_with('/') { u.to_string() } else { format!("{u}/") })
                }
            }
            None => format!("{MC_LIBRARIES}{path}"),
        };
        let mut artifact = Artifact {
            path: Some(path.clone()),
            url,
            sha1: if checksums.len() == 1 { Some(checksums[0].clone()) } else { None },
            size: None,
        };
        // the checksum and size of a file we already have are better than none
        let on_disk = dirs.libraries.join(&path);
        if (artifact.size.is_none() || artifact.sha1.is_none()) && on_disk.is_file() {
            artifact.size = std::fs::metadata(&on_disk).ok().map(|m| m.len() as i64);
            artifact.sha1 = super::fsx::sha1_file(&on_disk).ok();
        }
        out.push(Library {
            name: name.to_string(),
            downloads: Some(LibDownloads { artifact: Some(artifact), classifiers: None }),
            ..Default::default()
        });
    }
    out
}

/// Which of an old Forge's libraries ATLauncher's own copy of Forge's Maven has (a quick HEAD request each).
async fn probe_mirror(libs: &[Value]) -> std::collections::HashSet<String> {
    let mut paths_to_check = Vec::new();
    for l in libs {
        let (Some(name), Some(url)) = (l.get("name").and_then(Value::as_str), l.get("url").and_then(Value::as_str))
        else {
            continue;
        };
        if !url.contains("minecraftforge.net") {
            continue;
        }
        if let Some(mut p) = paths::maven_path(name) {
            if name.starts_with("net.minecraftforge:forge:")
                && l.get("clientreq").is_none()
                && l.get("serverreq").is_none()
                && l.get("checksums").is_none()
            {
                p = p.trim_end_matches(".jar").to_string() + "-universal.jar";
            }
            paths_to_check.push(p);
        }
    }
    let mut found = std::collections::HashSet::new();
    for p in paths_to_check {
        let ok = http::client()
            .head(format!("{DOWNLOAD_SERVER}/maven/{p}"))
            .send()
            .await
            .map(|r| r.status().as_u16() == 200)
            .unwrap_or(false);
        if ok {
            found.insert(p);
        }
    }
    found
}

async fn install_old_forge(
    dirs: &Dirs,
    metadata: &Map<String, Value>,
    mc: &str,
    choice: Option<&Choice>,
    vanilla: &VersionJson,
    progress: &Arc<Progress>,
) -> Result<Installed> {
    let (version, raw) = forge_versions(metadata, mc, choice)?;
    let installer = dirs.loaders.join(format!("forge-{mc}-{version}-installer.jar"));
    progress.stage(format!("Downloading the Forge {version} installer"));
    let versioned = format!("{mc}-{version}");
    let mirror =
        format!("{DOWNLOAD_SERVER}/maven/net/minecraftforge/forge/{versioned}/forge-{versioned}-installer.jar");
    let official = format!("{FORGE_MAVEN}net/minecraftforge/forge/{versioned}/forge-{versioned}-installer.jar");
    let mut got = dl::download_all(vec![Dl::new(&mirror, &installer)], progress).await;
    if got.is_err() {
        got = dl::download_all(vec![Dl::new(&official, &installer)], progress).await;
    }
    got.with_context(|| format!("downloading the Forge {version} installer"))?;

    let profile: Value = serde_json::from_slice(
        &super::fsx::read_zip_entry(&installer, "install_profile.json")
            .context("the Forge installer has no install_profile.json")?,
    )
    .context("reading the Forge installer's install_profile.json")?;
    // the game description is inside the profile for the oldest installers, or in a file next to it
    let version_info: Value = match profile.get("versionInfo") {
        Some(v) => v.clone(),
        None => {
            let name = profile
                .get("json")
                .and_then(Value::as_str)
                .unwrap_or("/version.json")
                .trim_start_matches('/')
                .to_string();
            serde_json::from_slice(
                &super::fsx::read_zip_entry(&installer, &name)
                    .with_context(|| format!("the Forge installer has no {name}"))?,
            )?
        }
    };

    let (forge_name, file_in_zip) = if profile.get("spec").is_some_and(|s| !s.is_null()) {
        (profile.get("path").and_then(Value::as_str).map(str::to_string), None)
    } else {
        let install = profile.get("install");
        (
            install.and_then(|i| i.get("path")).and_then(Value::as_str).map(str::to_string),
            install.and_then(|i| i.get("filePath")).and_then(Value::as_str).map(str::to_string),
        )
    };
    let libs_json: Vec<Value> = version_info.get("libraries").and_then(Value::as_array).cloned().unwrap_or_default();
    let libs_json: Vec<Value> = if libs_json.is_empty() {
        profile.get("libraries").and_then(Value::as_array).cloned().unwrap_or_default()
    } else {
        libs_json
    };
    let mirror_has = probe_mirror(&libs_json).await;
    let mut libraries = old_forge_libraries(&libs_json, dirs, &mirror_has);

    // the Forge jar itself is packed inside the installer: put it where the library list expects it
    if let Some(name) = &forge_name {
        let dest = libraries
            .iter()
            .find(|l| l.name == *name)
            .and_then(|l| l.downloads.as_ref()?.artifact.as_ref()?.path.clone())
            .or_else(|| paths::maven_path(name))
            .map(|p| dirs.libraries.join(p));
        if let Some(dest) = dest.filter(|d| !d.is_file()) {
            let rel = paths::maven_path(name).unwrap_or_default();
            let bytes = file_in_zip
                .as_deref()
                .and_then(|f| super::fsx::read_zip_entry(&installer, f))
                .or_else(|| super::fsx::read_zip_entry(&installer, &format!("maven/{rel}")))
                .or_else(|| super::fsx::read_zip_entry(&installer, &rel));
            if let Some(b) = bytes {
                if let Some(dir) = dest.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(&dest, b)?;
            }
        }
    }
    // libraries that the installer carries but the version file leaves without an address
    for lib in &libraries {
        if let Some(path) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()).and_then(|a| a.path.clone()) {
            let dest = dirs.libraries.join(&path);
            if !dest.is_file() {
                if let Some(b) = super::fsx::read_zip_entry(&installer, &format!("maven/{path}")) {
                    if let Some(dir) = dest.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    std::fs::write(&dest, b)?;
                }
            }
        }
    }
    // now that the files are here, their size and checksum can be filled in
    libraries = old_forge_libraries(&libs_json, dirs, &mirror_has);

    let mut profile_version: VersionJson = serde_json::from_value(version_info.clone()).unwrap_or_default();
    profile_version.libraries = libraries;
    profile_version.inherits_from = None;
    if profile_version.main_class.is_empty() {
        bail!("the Forge installer doesn't say what to start the game with");
    }
    let info = LoaderVersion { version: version.clone(), raw_version: raw, kind: "Forge".into(), ..Default::default() };
    Ok(Installed { merged: profile_version.merged_over(vanilla.clone()), info, joins_game_jar: false })
}

// ---------------------------------------------------------------------------------------------------------
// Forge for Minecraft 1.5.2 and older

/// Libraries these Forge versions expect in `lib/` next to the game: name, SHA-1, size.
fn fml_libraries(mc: &str) -> &'static [(&'static str, &'static str, u64)] {
    const L13: [(&str, &str, u64); 3] = [
        ("argo-2.25.jar", "bb672829fde76cb163004752b86b0484bd0a7f4b", 123642),
        ("guava-12.0.1.jar", "b8e78b9af7bf45900e14c6f958486b6ca682195f", 1795932),
        ("asm-all-4.0.jar", "98308890597acb64047f7e896638e0d98753ae82", 212767),
    ];
    const L14: [(&str, &str, u64); 4] = [
        ("argo-2.25.jar", "bb672829fde76cb163004752b86b0484bd0a7f4b", 123642),
        ("guava-12.0.1.jar", "b8e78b9af7bf45900e14c6f958486b6ca682195f", 1795932),
        ("asm-all-4.0.jar", "98308890597acb64047f7e896638e0d98753ae82", 212767),
        ("bcprov-jdk15on-147.jar", "b6f5d9926b0afbde9f4dbe3db88c5247be7794bb", 1997327),
    ];
    const L15: [(&str, &str, u64); 6] = [
        ("argo-small-3.2.jar", "58912ea2858d168c50781f956fa5b59f0f7c6b51", 91333),
        ("guava-14.0-rc3.jar", "931ae21fa8014c3ce686aaa621eae565fefb1a6a", 2189140),
        ("asm-all-4.1.jar", "054986e962b88d8660ae4566475658469595ef58", 214592),
        ("bcprov-jdk15on-148.jar", "960dea7c9181ba0b17e8bab0c06a43f0a5f04e65", 2318161),
        ("deobfuscation_data_1.5.zip", "5f7c142d53776f16304c0bbe10542014abad6af8", 200547),
        ("scala-library.jar", "458d046151ad179c85429ed7420ffb1eaf6ddf85", 7114640),
    ];
    const L151: [(&str, &str, u64); 6] = [
        ("argo-small-3.2.jar", "58912ea2858d168c50781f956fa5b59f0f7c6b51", 91333),
        ("guava-14.0-rc3.jar", "931ae21fa8014c3ce686aaa621eae565fefb1a6a", 2189140),
        ("asm-all-4.1.jar", "054986e962b88d8660ae4566475658469595ef58", 214592),
        ("bcprov-jdk15on-148.jar", "960dea7c9181ba0b17e8bab0c06a43f0a5f04e65", 2318161),
        ("deobfuscation_data_1.5.1.zip", "22e221a0d89516c1f721d6cab056a7e37471d0a6", 200886),
        ("scala-library.jar", "458d046151ad179c85429ed7420ffb1eaf6ddf85", 7114640),
    ];
    const L152: [(&str, &str, u64); 6] = [
        ("argo-small-3.2.jar", "58912ea2858d168c50781f956fa5b59f0f7c6b51", 91333),
        ("guava-14.0-rc3.jar", "931ae21fa8014c3ce686aaa621eae565fefb1a6a", 2189140),
        ("asm-all-4.1.jar", "054986e962b88d8660ae4566475658469595ef58", 214592),
        ("bcprov-jdk15on-148.jar", "960dea7c9181ba0b17e8bab0c06a43f0a5f04e65", 2318161),
        ("deobfuscation_data_1.5.2.zip", "446e55cd986582c70fcf12cb27bc00114c5adfd9", 201404),
        ("scala-library.jar", "458d046151ad179c85429ed7420ffb1eaf6ddf85", 7114640),
    ];
    match mc {
        "1.3.2" => &L13,
        "1.4" | "1.4.1" | "1.4.2" | "1.4.3" | "1.4.4" | "1.4.5" | "1.4.6" | "1.4.7" => &L14,
        "1.5" => &L15,
        "1.5.1" => &L151,
        "1.5.2" => &L152,
        _ => &[],
    }
}

async fn install_legacy_forge(
    dirs: &Dirs,
    metadata: &Map<String, Value>,
    mc: &str,
    choice: Option<&Choice>,
    vanilla: &VersionJson,
    root: &Path,
    progress: &Arc<Progress>,
) -> Result<Installed> {
    let (version, raw) = forge_versions(metadata, mc, choice)?;
    // 1.2.5 and older ship a client zip; later ones a universal zip
    let kind = if match_version(mc, "1.2", true, true) { "client" } else { "universal" };
    let file = format!("forge-{mc}-{version}-{kind}.zip");
    let dest = dirs.loaders.join(&file);
    progress.stage(format!("Downloading Forge {version}"));
    let url = format!("{DOWNLOAD_SERVER}/maven/net/minecraftforge/forge/{mc}-{version}/{file}");
    let mut job = Dl::new(url, &dest).hash(meta_str(metadata, &format!("{kind}Sha1")).as_deref());
    if let Some(size) = metadata.get(&format!("{kind}Size")).and_then(Value::as_f64) {
        job = job.size(size as i64);
    }
    let mut jobs = vec![job];
    for (name, sha1, size) in fml_libraries(mc) {
        jobs.push(
            Dl::new(format!("{DOWNLOAD_SERVER}/fmllibs/{name}"), dirs.libraries.join("fmllib").join(name))
                .hash(Some(sha1))
                .size(*size as i64),
        );
    }
    dl::download_all(jobs, progress).await.context("downloading Forge")?;
    std::fs::create_dir_all(root.join("bin"))?;
    super::fsx::copy_file(&dest, &root.join("bin").join("modpack.jar"), true)?;
    for (name, _, _) in fml_libraries(mc) {
        super::fsx::copy_file(&dirs.libraries.join("fmllib").join(name), &root.join("lib").join(name), true)?;
    }
    // these versions start through the game's own launch arguments, so the game description stays as it is
    let info = LoaderVersion { version, raw_version: raw, kind: "Forge".into(), ..Default::default() };
    Ok(Installed { merged: vanilla.clone(), info, joins_game_jar: true })
}

// ---------------------------------------------------------------------------------------------------------
// working out the loader of a Technic pack

/// The Forge build a library name stands for (`net.minecraftforge:forge:1.12.2-14.23.5.2860` and the older
/// `minecraftforge`/`fmlloader` names), as the full raw version of the Forge list (`1.12.2-14.23.5.2860`).
pub async fn forge_raw_for_library(dirs: &Dirs, mc: &str, library_version: &str) -> Result<String> {
    let list = loaders::versions(dirs, Loader::Forge, mc).await?;
    if let Some(exact) = list.iter().find(|r| r.as_str() == library_version) {
        return Ok(exact.clone());
    }
    let build = forge_build_number(library_version);
    if let Some(b) = &build {
        if let Some(found) = list.iter().find(|r| forge_build_number(r).as_ref() == Some(b)) {
            return Ok(found.clone());
        }
    }
    bail!("Failed to find loader version for {library_version}")
}

/// The `14.23.5.2860` part of a Forge version string, the four-part build number (old ones had `9.11.1.965`).
pub fn forge_build_number(version: &str) -> Option<String> {
    let mut best: Option<String> = None;
    let mut current = String::new();
    let flush = |cur: &mut String, best: &mut Option<String>| {
        let parts = cur.split('.').filter(|p| !p.is_empty()).count();
        if parts >= 4 && best.is_none() {
            *best = Some(cur.trim_matches('.').to_string());
        }
        cur.clear();
    };
    for c in version.chars() {
        if c.is_ascii_digit() || c == '.' {
            current.push(c);
        } else {
            flush(&mut current, &mut best);
        }
    }
    flush(&mut current, &mut best);
    best
}

/// What a Technic modpack's game description (`bin/version.json`, or the one inside `bin/modpack.jar`) says about
/// the loader. Returns the pack-style loader (class name and metadata) to install.
pub async fn loader_from_version_json(
    dirs: &Dirs,
    version_json: &VersionJson,
    mc: &str,
) -> Result<Option<super::json::PackLoader>> {
    let find = |prefixes: &[&str]| {
        version_json.libraries.iter().find(|l| prefixes.iter().any(|p| l.name.starts_with(p))).map(|l| l.name.clone())
    };
    let forge =
        find(&["net.minecraftforge:forge:", "net.minecraftforge:minecraftforge:", "net.minecraftforge:fmlloader:"]);
    let fabric = find(&["net.fabricmc:fabric-loader:"]);
    let is_neoforge = version_json.id.starts_with("neoforge-");
    let tail = |s: &str| s.rsplit(':').next().unwrap_or(s).to_string();
    let mut loader = super::json::PackLoader::default();
    let mut meta = Map::new();
    meta.insert("minecraft".into(), Value::String(mc.to_string()));
    if let Some(name) = forge {
        let lib_version = tail(&name);
        let raw = forge_raw_for_library(dirs, mc, &lib_version).await?;
        let version = raw.split_once('-').map(|x| x.1.to_string()).unwrap_or_else(|| raw.clone());
        meta.insert("version".into(), Value::String(version));
        meta.insert("rawVersion".into(), Value::String(raw));
        loader.class_name =
            Some(format!("com.blockforge.launcher.data.minecraft.loaders.forge.{}", forge_class_for(mc)));
    } else if let Some(name) = fabric {
        meta.insert("loader".into(), Value::String(tail(&name)));
        // not technically supported, no examples, but should be right
        let class = if match_version(mc, "1.13.2", true, true) {
            "legacyfabric.LegacyFabricLoader"
        } else {
            "fabric.FabricLoader"
        };
        loader.class_name = Some(format!("com.blockforge.launcher.data.minecraft.loaders.{class}"));
    } else if is_neoforge {
        let v = version_json.id.rsplit('-').next().unwrap_or("").to_string();
        meta.insert("version".into(), Value::String(v.clone()));
        meta.insert("rawVersion".into(), Value::String(v));
        loader.class_name = Some("com.blockforge.launcher.data.minecraft.loaders.neoforge.NeoForgeLoader".into());
    } else {
        return Ok(None);
    }
    loader.metadata = meta;
    Ok(Some(loader))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_classes_are_told_by_their_last_name() {
        assert_eq!(Kind::from_class("com.atlauncher.data.loaders.forge.ForgeLoader"), Some(Kind::Forge));
        assert_eq!(Kind::from_class("com.atlauncher.data.loaders.forge.Forge113Loader"), Some(Kind::Forge113));
        assert_eq!(
            Kind::from_class("com.blockforge.launcher.data.minecraft.loaders.fabric.FabricLoader"),
            Some(Kind::Fabric)
        );
        assert_eq!(Kind::from_class("com.atlauncher.data.loaders.neoforge.NeoForgeLoader"), Some(Kind::NeoForge));
        assert_eq!(Kind::from_class("x.legacyfabric.LegacyFabricLoader"), Some(Kind::LegacyFabric));
        assert_eq!(Kind::from_class("x.quilt.QuiltLoader"), Some(Kind::Quilt));
        assert_eq!(Kind::from_class("x.Nothing"), None);
        assert_eq!(Kind::Forge113.type_name(), "Forge");
        assert_eq!(Kind::LegacyFabric.type_name(), "LegacyFabric");
        assert!(!Kind::Paper.for_clients());
    }

    #[test]
    fn minecraft_version_matching_follows_the_java_rules() {
        // `1.13` and newer
        assert!(match_version("1.13", "1.13", false, true));
        assert!(match_version("1.16.5", "1.13", false, true));
        assert!(!match_version("1.12.2", "1.13", false, true));
        // `1.5` and older
        assert!(match_version("1.5.2", "1.5", true, true));
        assert!(match_version("1.4.7", "1.5", true, true));
        assert!(!match_version("1.6.4", "1.5", true, true));
        // a version with an underscore cannot be compared
        assert!(!match_version("1.7.10_pre4", "1.13", false, true));
        assert_eq!(forge_class_for("1.12.2"), "ForgeLoader");
        assert_eq!(forge_class_for("1.16.5"), "Forge113Loader");
        assert_eq!(forge_class_for("1.5.2"), "LegacyForgeLoader");
    }

    #[test]
    fn forge_build_numbers_are_found_in_any_spelling() {
        assert_eq!(forge_build_number("1.12.2-14.23.5.2860").as_deref(), Some("14.23.5.2860"));
        assert_eq!(forge_build_number("1.7.10-10.13.4.1614-1.7.10").as_deref(), Some("10.13.4.1614"));
        assert_eq!(forge_build_number("9.11.1.965").as_deref(), Some("9.11.1.965"));
        assert_eq!(forge_build_number("1.12.2"), None);
    }

    #[test]
    fn old_forge_libraries_are_read_like_the_java_edition_does() {
        let dirs = Dirs::at(std::env::temp_dir().join(format!("bf-forgelibs-{}", uuid::Uuid::new_v4().simple())));
        let libs: Vec<Value> = serde_json::from_str(
            r#"[
              {"name":"net.minecraftforge:forge:1.12.2-14.23.5.2860"},
              {"name":"org.ow2.asm:asm-all:5.2","url":"http://files.minecraftforge.net/maven/","checksums":["3354e11e2b34215f06dab629ab88e06aca477c19"],"serverreq":true,"clientreq":true},
              {"name":"org.lwjgl.lwjgl:lwjgl-platform:2.9.1","natives":{"linux":"natives-linux"},"extract":{"exclude":["META-INF/"]}},
              {"name":"lzma:lzma:0.0.1","clientreq":true},
              {"name":"net.minecraft:launchwrapper:1.12","downloads":{"artifact":{"path":"net/minecraft/launchwrapper/1.12/launchwrapper-1.12.jar","url":"https://libraries.minecraft.net/net/minecraft/launchwrapper/1.12/launchwrapper-1.12.jar","sha1":"111e7bea9c968cdb3d06ef4632bf7ff0824d0f36","size":32999}}}
            ]"#,
        )
        .unwrap();
        let out = old_forge_libraries(&libs, &dirs, &std::collections::HashSet::new());
        // the native library is left out, the other four stay
        assert_eq!(out.len(), 4);
        let forge = out[0].artifact().unwrap();
        assert_eq!(
            forge.path.as_deref(),
            Some("net/minecraftforge/forge/1.12.2-14.23.5.2860/forge-1.12.2-14.23.5.2860-universal.jar")
        );
        let asm = out[1].artifact().unwrap();
        assert_eq!(asm.url, "https://maven.minecraftforge.net/org/ow2/asm/asm-all/5.2/asm-all-5.2.jar");
        assert_eq!(asm.sha1.as_deref(), Some("3354e11e2b34215f06dab629ab88e06aca477c19"));
        let lzma = out[2].artifact().unwrap();
        assert_eq!(lzma.url, "https://libraries.minecraft.net/lzma/lzma/0.0.1/lzma-0.0.1.jar");
        assert_eq!(out[3].artifact().unwrap().size, Some(32999));
    }
}
