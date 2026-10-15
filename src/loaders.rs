//! Mod loaders: Fabric and Quilt come as a ready-made profile from their meta servers; Forge and NeoForge are
//! installed by running their own installer, which is the only way that stays correct across versions.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;

use crate::http;
use crate::install::{self, Job, Progress};
use crate::instances::LoaderVersion;
use crate::java;
use crate::model::VersionJson;
use crate::paths::Dirs;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Loader {
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl Loader {
    pub const ALL: [Loader; 5] = [Loader::Vanilla, Loader::Fabric, Loader::Quilt, Loader::Forge, Loader::NeoForge];

    pub fn label(self) -> &'static str {
        match self {
            Loader::Vanilla => "Vanilla",
            Loader::Fabric => "Fabric",
            Loader::Quilt => "Quilt",
            Loader::Forge => "Forge",
            Loader::NeoForge => "NeoForge",
        }
    }

    pub fn parse(s: &str) -> Option<Loader> {
        match s.to_ascii_lowercase().as_str() {
            "vanilla" | "none" | "" => Some(Loader::Vanilla),
            "fabric" => Some(Loader::Fabric),
            "quilt" => Some(Loader::Quilt),
            "forge" => Some(Loader::Forge),
            "neoforge" | "neo" => Some(Loader::NeoForge),
            _ => None,
        }
    }

    /// The name Modrinth uses for this loader in its filters.
    pub fn modrinth_name(self) -> Option<&'static str> {
        match self {
            Loader::Vanilla => None,
            other => Some(match other {
                Loader::Fabric => "fabric",
                Loader::Quilt => "quilt",
                Loader::Forge => "forge",
                _ => "neoforge",
            }),
        }
    }
}

#[derive(Deserialize)]
struct MetaEntry {
    loader: MetaLoader,
}

#[derive(Deserialize)]
struct MetaLoader {
    version: String,
}

const TTL: Duration = Duration::from_secs(900);

fn meta_base(loader: Loader) -> &'static str {
    if loader == Loader::Quilt { "https://meta.quiltmc.org/v3" } else { "https://meta.fabricmc.net/v2" }
}

/// Loader versions that fit `mc`, newest first.
pub async fn versions(dirs: &Dirs, loader: Loader, mc: &str) -> Result<Vec<String>> {
    let cache = dirs.cache.join("http");
    match loader {
        Loader::Vanilla => Ok(Vec::new()),
        Loader::Fabric | Loader::Quilt => {
            let list: Vec<MetaEntry> =
                http::get_json_cached(&format!("{}/versions/loader/{mc}", meta_base(loader)), &cache, TTL).await?;
            Ok(list.into_iter().map(|e| e.loader.version).collect())
        }
        Loader::Forge => {
            let all: std::collections::HashMap<String, Vec<String>> = http::get_json_cached(
                "https://files.minecraftforge.net/net/minecraftforge/forge/maven-metadata.json",
                &cache,
                TTL,
            )
            .await?;
            let mut list = all.get(mc).cloned().unwrap_or_default();
            list.reverse();
            Ok(list)
        }
        Loader::NeoForge => {
            #[derive(Deserialize)]
            struct Versions {
                versions: Vec<String>,
            }
            let all: Versions = http::get_json_cached(
                "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge",
                &cache,
                TTL,
            )
            .await?;
            let mut list: Vec<String> =
                all.versions.into_iter().filter(|v| neoforge_game_version(v).as_deref() == Some(mc)).collect();
            list.reverse();
            Ok(list)
        }
    }
}

/// NeoForge numbers its builds after the game: `21.1.57` is for 1.21.1, `20.4.1` for 1.20.4.
pub fn neoforge_game_version(v: &str) -> Option<String> {
    let mut parts = v.split('.');
    let a: u32 = parts.next()?.parse().ok()?;
    let b: u32 = parts.next()?.parse().ok()?;
    if (19..=25).contains(&a) {
        Some(if b == 0 { format!("1.{a}") } else { format!("1.{a}.{b}") })
    } else {
        Some(format!("{a}.{b}"))
    }
}

/// Install `loader` for `mc` and return the finished version description together with what to record in
/// `instance.json`. Vanilla returns the version unchanged.
pub async fn install(
    dirs: &Dirs,
    loader: Loader,
    mc: &str,
    wanted: Option<&str>,
    vanilla: &VersionJson,
    progress: &Arc<Progress>,
) -> Result<(VersionJson, Option<LoaderVersion>)> {
    if loader == Loader::Vanilla {
        return Ok((vanilla.clone(), None));
    }
    progress.stage(format!("Finding {} for Minecraft {mc}", loader.label()));
    let version = match wanted.filter(|v| !v.is_empty()) {
        Some(v) => v.to_string(),
        None => versions(dirs, loader, mc)
            .await?
            .into_iter()
            .next()
            .with_context(|| format!("{} has no version for Minecraft {mc}", loader.label()))?,
    };

    let merged = match loader {
        Loader::Fabric | Loader::Quilt => {
            progress.stage(format!("Reading {} {version}", loader.label()));
            let profile: VersionJson =
                http::get_json(&format!("{}/versions/loader/{mc}/{version}/profile/json", meta_base(loader))).await?;
            profile.merged_over(vanilla.clone())
        }
        Loader::Forge | Loader::NeoForge => run_installer(dirs, loader, mc, &version, vanilla, progress).await?,
        Loader::Vanilla => unreachable!(),
    };
    let raw = if loader == Loader::Forge {
        version.split_once('-').map(|x| x.1.to_string()).unwrap_or(version.clone())
    } else {
        version.clone()
    };
    Ok((
        merged,
        Some(LoaderVersion {
            version: raw.clone(),
            raw_version: version,
            recommended: false,
            kind: loader.label().to_string(),
            downloadables: Default::default(),
        }),
    ))
}

async fn run_installer(
    dirs: &Dirs,
    loader: Loader,
    mc: &str,
    version: &str,
    vanilla: &VersionJson,
    progress: &Arc<Progress>,
) -> Result<VersionJson> {
    let (url, name) = match loader {
        Loader::Forge => (
            format!(
                "https://maven.minecraftforge.net/net/minecraftforge/forge/{version}/forge-{version}-installer.jar"
            ),
            format!("forge-{version}-installer.jar"),
        ),
        _ => (
            format!(
                "https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/neoforge-{version}-installer.jar"
            ),
            format!("neoforge-{version}-installer.jar"),
        ),
    };
    let installer = dirs.loaders.join(&name);
    progress.stage(format!("Downloading the {} installer", loader.label()));
    install::download_all(vec![Job::new(url, &installer)], progress, false).await?;

    // the installer fetches Minecraft itself unless it finds it, so hand it what we already have
    progress.stage("Getting Minecraft for the installer");
    install::ensure_game_files(dirs, vanilla, progress, false).await?;
    let home = java::ensure_installer_java(dirs, progress).await?;

    let work = dirs.temp.join(format!("loader-{}", uuid::Uuid::new_v4().simple()));
    let vdir = work.join("versions").join(mc);
    std::fs::create_dir_all(&vdir)?;
    std::fs::write(work.join("launcher_profiles.json"), json!({"profiles": {}, "version": 3}).to_string())?;
    std::fs::write(vdir.join(format!("{mc}.json")), serde_json::to_vec(vanilla)?)?;
    let jar = install::client_jar(dirs, mc);
    if jar.is_file() {
        std::fs::copy(&jar, vdir.join(format!("{mc}.jar")))?;
    }

    progress.stage(format!("Installing {} {version} (this can take a minute)", loader.label()));
    let output = crate::platform::tokio_command(java::java_binary(&home))
        .arg("-jar")
        .arg(&installer)
        .arg("--installClient")
        .arg(&work)
        .current_dir(&work)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .context("starting the loader installer")?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).to_string() + &String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = text.lines().rev().take(12).collect();
        let _ = std::fs::remove_dir_all(&work);
        bail!("the {} installer failed:\n{}", loader.label(), tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }

    // the installer wrote one new folder under versions/: that's the loader's version
    let mut profile: Option<VersionJson> = None;
    for e in std::fs::read_dir(work.join("versions"))?.flatten() {
        if e.file_name().to_string_lossy() == mc {
            continue;
        }
        let json = e.path().join(format!("{}.json", e.file_name().to_string_lossy()));
        if let Ok(bytes) = std::fs::read(&json) {
            profile = Some(serde_json::from_slice(&bytes)?);
            break;
        }
    }
    let profile = profile.context("the installer finished but left no version behind")?;

    // everything it built or fetched goes into the shared libraries folder
    move_tree(&work.join("libraries"), &dirs.libraries)?;
    let _ = std::fs::remove_dir_all(&work);

    Ok(if profile.inherits_from.is_some() { profile.merged_over(vanilla.clone()) } else { profile })
}

fn move_tree(from: &Path, to: &Path) -> Result<()> {
    let Ok(rd) = std::fs::read_dir(from) else { return Ok(()) };
    for e in rd.flatten() {
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            std::fs::create_dir_all(&dest)?;
            move_tree(&e.path(), &dest)?;
        } else if !dest.exists() {
            if std::fs::rename(e.path(), &dest).is_err() {
                std::fs::copy(e.path(), &dest)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neoforge_versions_map_to_games() {
        assert_eq!(neoforge_game_version("21.1.57").as_deref(), Some("1.21.1"));
        assert_eq!(neoforge_game_version("21.0.5").as_deref(), Some("1.21"));
        assert_eq!(neoforge_game_version("20.4.1").as_deref(), Some("1.20.4"));
        assert_eq!(neoforge_game_version("26.3.1").as_deref(), Some("26.3"));
    }
}
