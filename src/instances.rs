//! Instances: one folder each under `instances/`, described by an `instance.json` in the Java edition's format.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::write_json_atomic;
use crate::install::{self, Progress};
use crate::loaders::{self, Loader};
use crate::model::VersionJson;
use crate::paths::{self, Dirs};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct LoaderVersion {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub raw_version: String,
    #[serde(default)]
    pub recommended: bool,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub downloadables: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct LauncherBlock {
    pub name: String,
    pub pack: String,
    pub description: String,
    pub pack_id: i64,
    pub external_pack_id: i64,
    pub version: String,
    pub enable_curse_forge_integration: bool,
    pub enable_editing_mods: bool,
    pub loader_version: Option<LoaderVersion>,
    pub required_memory: i64,
    pub required_perm_gen: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_memory: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automatic_memory: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perm_gen: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java_arguments: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_java_provided_by_minecraft: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrapper_command: Option<String>,
    pub vanilla_instance: bool,
    pub last_played: Option<i64>,
    pub num_plays: Option<i64>,
    #[serde(flatten)]
    pub extras: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct InstanceFile {
    #[serde(default)]
    pub uuid: String,
    #[serde(default)]
    pub launcher: LauncherBlock,
    #[serde(flatten)]
    pub version: VersionJson,
}

#[derive(Clone, Debug)]
pub struct Instance {
    pub folder: String,
    pub mod_count: usize,
    pub root: PathBuf,
    pub file: InstanceFile,
}

impl Instance {
    pub fn name(&self) -> &str {
        if self.file.launcher.name.is_empty() { &self.folder } else { &self.file.launcher.name }
    }

    pub fn mc_version(&self) -> &str {
        self.file.version.game_version()
    }

    /// `("Fabric", "0.16.0")` for a modded instance.
    pub fn loader(&self) -> Option<(&str, &str)> {
        self.file
            .launcher
            .loader_version
            .as_ref()
            .filter(|l| !l.kind.is_empty())
            .map(|l| (l.kind.as_str(), l.version.as_str()))
    }

    pub fn last_played(&self) -> i64 {
        self.file.launcher.last_played.unwrap_or(0)
    }

    pub fn instance_file(&self) -> PathBuf {
        self.root.join("instance.json")
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.root.join("mods")
    }

    pub fn disabled_mods_dir(&self) -> PathBuf {
        self.root.join("disabledmods")
    }

    pub fn save(&self) -> Result<()> {
        write_json_atomic(&self.instance_file(), &self.file)
    }

    pub fn mark_played(&mut self) -> Result<()> {
        self.file.launcher.last_played = Some(chrono::Utc::now().timestamp_millis());
        self.file.launcher.num_plays = Some(self.file.launcher.num_plays.unwrap_or(0) + 1);
        self.save()
    }

    pub fn mod_files(&self) -> Vec<ModFile> {
        let mut out = Vec::new();
        for (dir, enabled) in [(self.mods_dir(), true), (self.disabled_mods_dir(), false)] {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if e.path().is_file()
                    && (name.ends_with(".jar") || name.ends_with(".zip") || name.ends_with(".jar.disabled"))
                {
                    out.push(ModFile {
                        name,
                        path: e.path(),
                        enabled,
                        size: e.metadata().map(|m| m.len()).unwrap_or(0),
                    });
                }
            }
        }
        out.sort_by_key(|m| m.name.to_lowercase());
        out
    }

    pub fn enabled_mod_count(&self) -> usize {
        std::fs::read_dir(self.mods_dir())
            .map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".jar")).count())
            .unwrap_or(0)
    }
}

#[derive(Clone, Debug)]
pub struct ModFile {
    pub name: String,
    pub path: PathBuf,
    pub enabled: bool,
    pub size: u64,
}

/// Move a mod between `mods/` and `disabledmods/`, the way the Java edition does.
pub fn set_mod_enabled(instance: &Instance, file: &ModFile, enabled: bool) -> Result<()> {
    if file.enabled == enabled {
        return Ok(());
    }
    let to_dir = if enabled { instance.mods_dir() } else { instance.disabled_mods_dir() };
    std::fs::create_dir_all(&to_dir)?;
    std::fs::rename(&file.path, to_dir.join(&file.name))?;
    Ok(())
}

pub fn load_one(root: &Path) -> Option<Instance> {
    let bytes = std::fs::read(root.join("instance.json")).ok()?;
    let file: InstanceFile = serde_json::from_slice(&bytes).ok()?;
    let mut inst = Instance {
        folder: root.file_name()?.to_string_lossy().to_string(),
        mod_count: 0,
        root: root.to_path_buf(),
        file,
    };
    inst.mod_count = inst.enabled_mod_count();
    Some(inst)
}

/// Every instance, most recently played first. Folders without a readable `instance.json` are skipped.
pub fn load_all(dirs: &Dirs) -> Vec<Instance> {
    let Ok(rd) = std::fs::read_dir(&dirs.instances) else { return Vec::new() };
    let roots: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    // parsing hundreds of KB of JSON per instance adds up for big libraries; spread it over a few threads
    let threads = roots.len().clamp(1, 8);
    let chunk = roots.len().div_ceil(threads).max(1);
    let mut all: Vec<Instance> = std::thread::scope(|s| {
        let handles: Vec<_> = roots
            .chunks(chunk)
            .map(|c| s.spawn(move || c.iter().filter_map(|r| load_one(r)).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    all.sort_by(|a, b| {
        b.last_played().cmp(&a.last_played()).then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
    });
    all
}

/// The `launcher` fields the Java edition expects on every instance; without `isPlayable` it calls an instance corrupt.
pub fn java_defaults(l: &mut LauncherBlock) {
    use serde_json::json;
    for (k, v) in [
        ("isDev", json!(false)),
        ("isPlayable", json!(true)),
        ("assetsMapToResources", json!(false)),
        ("overridePaths", json!([])),
        ("checkForUpdates", json!(true)),
        ("ignoredUpdates", json!([])),
        ("ignoreAllUpdates", json!(false)),
        ("quickPlay", json!({})),
    ] {
        l.extras.entry(k.to_string()).or_insert(v);
    }
    l.extras.entry("mods".to_string()).or_insert(json!([]));
}

/// Change (or add, or remove) the mod loader of an existing instance, keeping its worlds, mods and settings.
pub async fn set_loader(
    dirs: &Dirs,
    instance: &mut Instance,
    loader: Loader,
    loader_version: Option<&str>,
    progress: &Arc<Progress>,
) -> Result<()> {
    let mc = instance.mc_version().to_string();
    let vanilla = install::vanilla_json(dirs, &mc).await?;
    let (merged, info) = loaders::install(dirs, loader, &mc, loader_version, &vanilla, progress).await?;
    if let Some(jv) = &merged.java_version {
        if !jv.component.is_empty() {
            crate::java::ensure_runtime(dirs, &jv.component, progress).await?;
        }
    }
    install::ensure_game_files(dirs, &merged, progress, false).await?;
    let mut version = merged;
    version.id = mc;
    instance.file.version = version;
    instance.file.launcher.loader_version = info;
    instance.save()
}

/// Create an instance: fetch the version (and loader), download everything it needs, then write `instance.json`.
pub async fn create(
    dirs: &Dirs,
    name: &str,
    mc_version: &str,
    loader: Loader,
    loader_version: Option<&str>,
    progress: &Arc<Progress>,
) -> Result<Instance> {
    progress.stage(format!("Reading Minecraft {mc_version}"));
    let vanilla = install::vanilla_json(dirs, mc_version).await?;
    let (merged, loader_info) = loaders::install(dirs, loader, mc_version, loader_version, &vanilla, progress).await?;

    if let Some(jv) = &merged.java_version {
        if !jv.component.is_empty() {
            crate::java::ensure_runtime(dirs, &jv.component, progress).await?;
        }
    }
    install::ensure_game_files(dirs, &merged, progress, false).await?;

    let folder = paths::unique_folder(&dirs.instances, name);
    let root = dirs.instance_dir(&folder);
    std::fs::create_dir_all(&root)?;
    for sub in ["mods", "saves", "resourcepacks", "shaderpacks", "config"] {
        std::fs::create_dir_all(root.join(sub))?;
    }
    let mut version = merged;
    version.id = mc_version.to_string();
    let file = InstanceFile {
        uuid: uuid_v4(),
        launcher: LauncherBlock {
            name: name.to_string(),
            pack: "Minecraft".into(),
            description: match &loader_info {
                Some(l) => format!("Minecraft {mc_version} with {}", l.kind),
                None => format!("Minecraft {mc_version}"),
            },
            version: mc_version.to_string(),
            enable_curse_forge_integration: true,
            enable_editing_mods: true,
            loader_version: loader_info,
            vanilla_instance: true,
            ..Default::default()
        },
        version,
    };
    let mut instance = Instance { folder, mod_count: 0, root, file };
    java_defaults(&mut instance.file.launcher);
    instance.save().context("saving instance.json")?;
    Ok(instance)
}

pub fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn delete(instance: &Instance) -> Result<()> {
    std::fs::remove_dir_all(&instance.root).with_context(|| format!("deleting {}", instance.root.display()))
}

/// Copy an instance folder under a new name.
pub fn duplicate(dirs: &Dirs, instance: &Instance, new_name: &str) -> Result<Instance> {
    let folder = paths::unique_folder(&dirs.instances, new_name);
    let root = dirs.instance_dir(&folder);
    copy_dir(&instance.root, &root)?;
    let mut copy = Instance { folder, mod_count: instance.mod_count, root, file: instance.file.clone() };
    copy.file.uuid = uuid_v4();
    copy.file.launcher.name = new_name.to_string();
    copy.file.launcher.last_played = None;
    copy.file.launcher.num_plays = None;
    copy.save()?;
    Ok(copy)
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let dest = to.join(e.file_name());
        let ty = e.file_type()?;
        if ty.is_dir() {
            copy_dir(&e.path(), &dest)?;
        } else if ty.is_file() {
            // a real copy: the game edits worlds and options in place, which a hard link would share
            std::fs::copy(e.path(), &dest)?;
        }
    }
    Ok(())
}
