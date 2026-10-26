//! Bringing instances over from other places: Prism / MultiMC / CurseForge / GDLauncher / ATLauncher folders,
//! exported zips (MultiMC, Prism, Modrinth, CurseForge), and web addresses. Nothing is changed in the source,
//! and a failed import leaves no half-made instance behind.
//!
//! * [`detect`] works out what a file, folder or address is.
//! * [`multimc`] reads and imports the MultiMC / Prism format.
//! * this file has the folder adapters (the Java edition's `imports` package), the copy with its safety rules,
//!   and the entry points that pick the right importer: [`import_input`], [`import_source`].
//!
//! CurseForge `.zip` modpacks and CurseForge page links are recognised here but imported by the CurseForge
//! package, through [`CurseForgeImporter`] (see HANDOFF.md).

pub mod detect;
pub mod multimc;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use futures::future::BoxFuture;
use serde_json::Value;

use crate::cf_seam::CurseForgeSource;
use crate::install::{self, Job, Progress};
use crate::instances::{self, Instance};
use crate::loaders::{self, Loader};
use crate::packutil;
use crate::paths::Dirs;

use detect::{Source, UrlSource};

/// An instance from another launcher, in BlockForge's terms, so every launcher is handled the same way once read.
#[derive(Clone, Debug)]
pub struct ImportedInstance {
    /// Which launcher it came from, for display: "Prism / MultiMC", "CurseForge" ...
    pub source: String,
    pub name: String,
    pub minecraft_version: String,
    /// `fabric`, `forge`, `neoforge` or `quilt`; `None` for vanilla.
    pub loader_type: Option<String>,
    pub loader_version: Option<String>,
    /// Where the game's own files (mods, saves, config ...) are.
    pub game_dir: PathBuf,
    /// The instance's folder in the other launcher.
    pub root: PathBuf,
    pub maximum_memory_mb: Option<u32>,
    pub java_arguments: Option<String>,
    pub warnings: Vec<String>,
    adapter: Adapter,
}

impl ImportedInstance {
    /// "Fabric 0.15.7", "NeoForge", or "Vanilla".
    pub fn loader_label(&self) -> String {
        let Some(kind) = &self.loader_type else { return "Vanilla".into() };
        let pretty = if kind == "neoforge" {
            "NeoForge".to_string()
        } else {
            let mut c = kind.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        };
        match &self.loader_version {
            Some(v) => format!("{pretty} {v}"),
            None => pretty,
        }
    }

    pub fn loader(&self) -> Option<Loader> {
        self.loader_type.as_deref().and_then(Loader::parse).filter(|l| *l != Loader::Vanilla)
    }
}

/// The launchers whose instance folders can be read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Adapter {
    /// Prism Launcher and MultiMC share a format: `instance.cfg` plus `mmc-pack.json`.
    Prism,
    /// The CurseForge app keeps `minecraftinstance.json` in each instance folder.
    CurseForge,
    /// GDLauncher keeps `config.json` with the game's files in an `instance` folder.
    GdLauncher,
    /// ATLauncher (and older BlockForge) keep `instance.json` with the Minecraft version as `id`.
    Atlauncher,
}

const ADAPTERS: [Adapter; 4] = [Adapter::Prism, Adapter::CurseForge, Adapter::GdLauncher, Adapter::Atlauncher];

impl Adapter {
    pub fn source(self) -> &'static str {
        match self {
            Adapter::Prism => "Prism / MultiMC",
            Adapter::CurseForge => "CurseForge",
            Adapter::GdLauncher => "GDLauncher",
            Adapter::Atlauncher => "ATLauncher",
        }
    }

    /// A cheap check on the folder's files, without reading them.
    pub fn matches(self, folder: &Path) -> bool {
        match self {
            Adapter::Prism => folder.join("instance.cfg").is_file() && folder.join("mmc-pack.json").is_file(),
            Adapter::CurseForge => folder.join("minecraftinstance.json").is_file(),
            Adapter::GdLauncher => folder.join("config.json").is_file() && folder.join("instance").is_dir(),
            Adapter::Atlauncher => folder.join("instance.json").is_file(),
        }
    }

    /// Reads the folder, failing if its files are unreadable or incomplete.
    pub fn read(self, folder: &Path) -> Result<ImportedInstance> {
        match self {
            Adapter::Prism => read_prism(folder),
            Adapter::CurseForge => read_curseforge(folder),
            Adapter::GdLauncher => read_gdlauncher(folder),
            Adapter::Atlauncher => read_atlauncher(folder),
        }
    }
}

/// The first adapter that recognises the folder, if any.
pub fn detect_folder(folder: &Path) -> Option<Adapter> {
    if !folder.is_dir() {
        return None;
    }
    ADAPTERS.into_iter().find(|a| a.matches(folder))
}

/// Read an instance folder from any supported launcher. The game folder must exist.
pub fn read(folder: &Path) -> Result<ImportedInstance> {
    let adapter =
        detect_folder(folder).context("This folder doesn't look like an instance from a supported launcher")?;
    let instance = adapter.read(folder)?;
    if !instance.game_dir.is_dir() {
        bail!("The instance has no game folder, so there is nothing to import");
    }
    Ok(instance)
}

fn json_file(path: &Path) -> Result<Value> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let bytes = std::fs::read(path).with_context(|| format!("{name} couldn't be read"))?;
    let v: Value = serde_json::from_slice(&bytes).with_context(|| format!("{name} couldn't be read"))?;
    if !v.is_object() {
        bail!("{name} isn't a valid instance file");
    }
    Ok(v)
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn non_blank(value: Option<&str>, fallback: &str) -> String {
    value.map(str::trim).filter(|v| !v.is_empty()).unwrap_or(fallback).to_string()
}

fn folder_name(folder: &Path) -> String {
    folder.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn make(
    adapter: Adapter,
    name: String,
    mc: String,
    loader: Option<(String, Option<String>)>,
    root: &Path,
    game: PathBuf,
) -> ImportedInstance {
    ImportedInstance {
        source: adapter.source().to_string(),
        name,
        minecraft_version: mc,
        loader_type: loader.as_ref().map(|l| l.0.clone()),
        loader_version: loader.and_then(|l| l.1),
        game_dir: game,
        root: root.to_path_buf(),
        maximum_memory_mb: None,
        java_arguments: None,
        warnings: Vec::new(),
        adapter,
    }
}

fn read_prism(folder: &Path) -> Result<ImportedInstance> {
    let cfg = multimc::Properties::parse(&String::from_utf8_lossy(&std::fs::read(folder.join("instance.cfg"))?));
    let pack = json_file(&folder.join("mmc-pack.json"))?;
    let manifest: multimc::Manifest = serde_json::from_value(pack).context("mmc-pack.json couldn't be read")?;
    let mc = manifest
        .minecraft_version()
        .context("mmc-pack.json doesn't say which Minecraft version the instance uses")?
        .to_string();
    let loader = manifest.loader().map(|(l, v)| (l.label().to_lowercase(), Some(v)));
    let game = if folder.join(".minecraft").is_dir() { folder.join(".minecraft") } else { folder.join("minecraft") };
    let mut result = make(Adapter::Prism, non_blank(cfg.get("name"), &folder_name(folder)), mc, loader, folder, game);
    if cfg.is_true("OverrideMemory") {
        if let Some(raw) = cfg.get("MaxMemAlloc") {
            match raw.trim().parse::<u32>() {
                Ok(mb) => result.maximum_memory_mb = Some(mb),
                Err(_) => {
                    result.warnings.push("The memory setting couldn't be read, so the default will be used.".into())
                }
            }
        }
    }
    if cfg.is_true("OverrideJavaArgs") {
        result.java_arguments = cfg.get_any(&["JvmArgs", "JVMArgs"]).map(|a| a.trim().to_string());
    }
    Ok(result)
}

fn read_curseforge(folder: &Path) -> Result<ImportedInstance> {
    let o = json_file(&folder.join("minecraftinstance.json"))?;
    let loader = o.get("baseModLoader").filter(|l| l.is_object());
    let mc = str_of(&o, "gameVersion")
        .map(str::to_string)
        .or_else(|| loader.and_then(|l| str_of(l, "minecraftVersion")).map(str::to_string))
        .context("minecraftinstance.json doesn't say which Minecraft version the instance uses")?;
    let mut loader_info = None;
    if let Some(l) = loader {
        let name = str_of(l, "name").unwrap_or("").trim().to_lowercase();
        let kind = ["forge", "fabric", "quilt", "neoforge"].into_iter().find(|k| name.starts_with(k));
        // "neoforge" starts with "neo", not "forge"; the order above is the Java edition's, and `forge` is checked
        // against the whole word below
        let kind = if name.starts_with("neoforge") { Some("neoforge") } else { kind };
        if let Some(k) = kind {
            loader_info = Some((k.to_string(), curseforge_loader_version(l, &name, &mc)));
        }
    }
    let mut result = make(
        Adapter::CurseForge,
        non_blank(str_of(&o, "name"), &folder_name(folder)),
        mc,
        loader_info,
        folder,
        folder.to_path_buf(),
    );
    if let Some(mb) = o.get("allocatedMemory").and_then(Value::as_i64).filter(|m| *m > 0) {
        result.maximum_memory_mb = Some(mb as u32);
    }
    Ok(result)
}

/// The loader's own version. CurseForge stores it as `forgeVersion` (for every loader), and names the loader
/// `forge-47.2.0` or, for Fabric and Quilt, `fabric-0.15.7-1.20.1` with the Minecraft version on the end.
fn curseforge_loader_version(loader: &Value, loader_name: &str, minecraft: &str) -> Option<String> {
    if let Some(stored) = str_of(loader, "forgeVersion").map(str::trim).filter(|s| !s.is_empty()) {
        return Some(stored.to_string());
    }
    let (_, version) = loader_name.split_once('-')?;
    if version.is_empty() {
        return None;
    }
    let suffix = format!("-{}", minecraft.to_lowercase());
    Some(match version.strip_suffix(&suffix) {
        Some(stripped) if !stripped.is_empty() => stripped.to_string(),
        _ => version.to_string(),
    })
}

fn read_atlauncher(folder: &Path) -> Result<ImportedInstance> {
    let o = json_file(&folder.join("instance.json"))?;
    let launcher = o.get("launcher").filter(|l| l.is_object());
    let (Some(launcher), Some(mc)) = (launcher, str_of(&o, "id")) else {
        bail!("instance.json isn't an ATLauncher instance")
    };
    let mut loader_info = None;
    if let Some(l) = launcher.get("loaderVersion").filter(|l| l.is_object()) {
        let kind = str_of(l, "type").unwrap_or("").trim().to_lowercase().replace(' ', "");
        if ["fabric", "forge", "neoforge", "quilt"].contains(&kind.as_str()) {
            loader_info = Some((kind, str_of(l, "version").map(str::to_string)));
        }
    }
    let mut result = make(
        Adapter::Atlauncher,
        non_blank(str_of(launcher, "name"), &folder_name(folder)),
        mc.to_string(),
        loader_info,
        folder,
        folder.to_path_buf(),
    );
    result.maximum_memory_mb = launcher.get("maximumMemory").and_then(Value::as_i64).map(|m| m as u32);
    Ok(result)
}

fn read_gdlauncher(folder: &Path) -> Result<ImportedInstance> {
    let o = json_file(&folder.join("config.json"))?;
    let version = o.get("game_configuration").and_then(|g| g.get("version")).filter(|v| v.is_object());
    let mc = version
        .and_then(|v| str_of(v, "release"))
        .context("config.json doesn't say which Minecraft version the instance uses")?;
    let mut loader_info = None;
    if let Some(first) = version
        .and_then(|v| v.get("modloaders"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .filter(|f| f.is_object())
    {
        let kind = str_of(first, "type").unwrap_or("").trim().to_lowercase();
        if ["fabric", "forge", "neoforge", "quilt"].contains(&kind.as_str()) {
            loader_info = Some((kind, str_of(first, "version").map(str::to_string)));
        }
    }
    Ok(make(
        Adapter::GdLauncher,
        non_blank(str_of(&o, "name"), &folder_name(folder)),
        mc.to_string(),
        loader_info,
        folder,
        folder.join("instance"),
    ))
}

// ---------------------------------------------------------------------------------------------------------
// What an import would bring in, and bringing it

/// Things that are rebuilt by the launcher or only hold history, so they aren't worth carrying over.
const SKIPPED: [&str; 22] = [
    "logs",
    "crash-reports",
    "debug",
    "natives",
    "bin",
    "cache",
    ".cache",
    ".fabric",
    ".mixin.out",
    "backups",
    "libraries",
    "versions",
    "assets",
    "temp",
    "instance.json",
    "instance.cfg",
    "mmc-pack.json",
    "minecraftinstance.json",
    "config.json",
    "patches",
    "icon.png",
    "instance.png",
];

fn is_skipped(name: &str) -> bool {
    SKIPPED.contains(&name) || name == ".git" || name.starts_with("hs_err_pid") || name.ends_with(".log")
}

/// What an import would bring in, worked out without changing anything.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    pub mods: usize,
    pub worlds: usize,
    pub resource_packs: usize,
    pub shader_packs: usize,
    pub files: usize,
    pub bytes: u64,
    pub top_level: Vec<String>,
}

pub fn preview(instance: &ImportedInstance) -> Result<Preview> {
    let mut p = Preview::default();
    for entry in std::fs::read_dir(&instance.game_dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let ty = entry.file_type()?;
        if is_skipped(&name) || ty.is_symlink() {
            continue;
        }
        p.top_level.push(name.clone());
        if ty.is_dir() {
            match name.as_str() {
                "mods" => p.mods = count_files(&entry.path(), &[".jar", ".zip", ".disabled"]),
                "saves" => p.worlds = std::fs::read_dir(entry.path())?.flatten().filter(|e| e.path().is_dir()).count(),
                "resourcepacks" => p.resource_packs = std::fs::read_dir(entry.path())?.count(),
                "shaderpacks" => p.shader_packs = std::fs::read_dir(entry.path())?.count(),
                _ => {}
            }
        }
    }
    p.top_level.sort();
    walk_counts(&instance.game_dir, &mut p, true);
    Ok(p)
}

fn count_files(dir: &Path, suffixes: &[&str]) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    e.path().is_file() && suffixes.iter().any(|s| name.ends_with(s))
                })
                .count()
        })
        .unwrap_or(0)
}

fn walk_counts(dir: &Path, p: &mut Preview, top: bool) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(ty) = e.file_type() else { continue };
        // counted the way `copy_tree` copies: the skip list only at the top
        if (top && is_skipped(&name)) || ty.is_symlink() {
            continue;
        }
        if ty.is_dir() {
            walk_counts(&e.path(), p, false);
        } else if ty.is_file() {
            p.files += 1;
            p.bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
}

/// Brings the instance's files into `target`. The files are copied to a staging folder first and only moved in
/// once everything has been copied; if anything goes wrong what was moved in is taken out again.
pub fn copy_into(instance: &ImportedInstance, target: &Path) -> Result<()> {
    copy_game_dir(&instance.game_dir, target)
}

pub fn copy_game_dir(game_dir: &Path, target: &Path) -> Result<()> {
    let root = game_dir.canonicalize().with_context(|| format!("{} can't be read", game_dir.display()))?;
    std::fs::create_dir_all(target)?;
    let parent = target.canonicalize()?.parent().map(Path::to_path_buf).unwrap_or_else(|| target.to_path_buf());
    let staging = parent.join(format!(".import-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&staging)?;
    let result = (|| -> Result<()> {
        for entry in std::fs::read_dir(&root)?.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_skipped(&name) || entry.file_type()?.is_symlink() {
                continue;
            }
            copy_tree(&entry.path(), &staging.join(&name))?;
        }
        let mut placed = Vec::new();
        let moved: Result<()> = std::fs::read_dir(&staging)?
            .flatten()
            .try_for_each(|e| move_into(&e.path(), &target.join(e.file_name()), &mut placed));
        if let Err(e) = moved {
            for p in placed.iter().rev() {
                remove_quietly(p);
            }
            return Err(e);
        }
        Ok(())
    })();
    remove_quietly(&staging);
    result
}

/// Copies a file or folder; links are skipped, never followed, so nothing outside the instance is read. The list
/// of things not worth importing only applies at the top of the instance (the caller checks it): further down the
/// same names are the player's own files (`resourcepacks/MyPack/assets`, a world's `icon.png`, a mod's
/// `config/x/config.json`).
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)?.flatten() {
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        }
    } else if meta.is_file() {
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::copy(from, to).with_context(|| format!("copying {}", from.display()))?;
    }
    Ok(())
}

fn move_into(staged: &Path, destination: &Path, placed: &mut Vec<PathBuf>) -> Result<()> {
    if std::fs::symlink_metadata(destination).is_err() {
        move_path(staged, destination)?;
        placed.push(destination.to_path_buf());
        return Ok(());
    }
    if staged.is_dir() && destination.is_dir() {
        for child in std::fs::read_dir(staged)?.flatten() {
            move_into(&child.path(), &destination.join(child.file_name()), placed)?;
        }
        return Ok(());
    }
    // a file that is already there is replaced by the imported one
    if destination.is_file() {
        std::fs::remove_file(destination)?;
    }
    move_path(staged, destination)?;
    placed.push(destination.to_path_buf());
    Ok(())
}

fn move_path(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        packutil::copy_dir_merge(from, to)?;
    } else {
        std::fs::copy(from, to)?;
    }
    Ok(())
}

fn remove_quietly(p: &Path) {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => {
            let _ = std::fs::remove_dir_all(p);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(p);
        }
        Err(_) => {}
    }
}

/// A name for the new instance that isn't taken yet: the instance's own name, or that name with " (2)", " (3)" ...
/// added. A name without any letters or digits can't be used for an instance, so it is replaced.
pub fn unique_name(wanted: Option<&str>, taken: impl Fn(&str) -> bool) -> String {
    packutil::unique_name(wanted.unwrap_or(""), taken)
}

/// Looks inside the folders where launchers keep their instances for ones that can be imported.
pub fn scan(roots: &[PathBuf]) -> Vec<ImportedInstance> {
    let mut found = Vec::new();
    for root in roots.iter().filter(|r| r.is_dir()) {
        let Ok(rd) = std::fs::read_dir(root) else { continue };
        for child in rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            if let Ok(i) = read(&child) {
                found.push(i);
            }
        }
    }
    found.sort_by_key(|i| i.name.to_lowercase());
    found
}

/// Where other launchers usually keep their instances on this computer.
pub fn default_roots() -> Vec<PathBuf> {
    let home =
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    if cfg!(target_os = "windows") {
        let data = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData/Roaming"));
        vec![
            data.join("PrismLauncher/instances"),
            data.join(".atlauncher/instances"),
            data.join("gdlauncher_carbon/data/instances"),
            home.join("curseforge/minecraft/Instances"),
            home.join("Documents/curseforge/minecraft/Instances"),
        ]
    } else if cfg!(target_os = "macos") {
        let support = home.join("Library/Application Support");
        vec![
            support.join("PrismLauncher/instances"),
            support.join("MultiMC/instances"),
            support.join("ATLauncher/instances"),
            support.join("gdlauncher_carbon/data/instances"),
            home.join("curseforge/minecraft/Instances"),
        ]
    } else {
        vec![
            home.join(".local/share/PrismLauncher/instances"),
            home.join(".var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher/instances"),
            home.join(".local/share/multimc/instances"),
            home.join(".atlauncher/instances"),
            home.join(".local/share/gdlauncher_carbon/data/instances"),
            home.join("curseforge/minecraft/Instances"),
        ]
    }
}

// ---------------------------------------------------------------------------------------------------------
// Making instances

/// Choose the loader to install for an imported instance: the one it had, at the version it had if that exists,
/// otherwise the newest for that Minecraft version. If loader versions can't be found at all the instance is
/// made as plain Minecraft. The strings say what was changed.
pub async fn resolve_loader(
    dirs: &Dirs,
    mc: &str,
    wanted: Option<(Loader, Option<String>)>,
) -> (Loader, Option<String>, Vec<String>) {
    let Some((loader, version)) = wanted.filter(|(l, _)| *l != Loader::Vanilla) else {
        return (Loader::Vanilla, None, Vec::new());
    };
    let list = match loaders::versions(dirs, loader, mc).await {
        Ok(l) if !l.is_empty() => l,
        Ok(_) => {
            return (
                Loader::Vanilla,
                None,
                vec![format!(
                    "{} isn't available for Minecraft {mc}, so the instance was made without it.",
                    loader.label()
                )],
            );
        }
        Err(e) => {
            return (
                Loader::Vanilla,
                None,
                vec![format!(
                    "{} versions couldn't be looked up ({e:#}), so the instance was made without it.",
                    loader.label()
                )],
            );
        }
    };
    match version.as_deref().filter(|v| !v.is_empty()) {
        Some(v) => match packutil::match_loader_version(loader, mc, v, &list) {
            Some(found) => (loader, Some(found), Vec::new()),
            None => (
                loader,
                None,
                vec![format!(
                    "{} {v} isn't available for Minecraft {mc}, so the newest version was used.",
                    loader.label()
                )],
            ),
        },
        None => (loader, None, Vec::new()),
    }
}

/// Make a new instance from an instance folder of any supported launcher (a Prism / MultiMC folder is
/// imported by [`multimc::import_folder`]).
pub async fn import_instance(
    dirs: &Dirs,
    imported: &ImportedInstance,
    name: Option<&str>,
    progress: &Arc<Progress>,
) -> Result<Instance> {
    if imported.adapter == Adapter::Prism {
        let opts = multimc::ImportOptions { name: name.map(str::to_string) };
        return multimc::import_folder(dirs, &imported.root, &opts, progress).await;
    }
    let wanted = name.map(str::to_string).unwrap_or_else(|| imported.name.clone());
    let name = packutil::unique_name(&wanted, |n| packutil::instance_name_taken(dirs, n));
    let (loader, version, warnings) = resolve_loader(
        dirs,
        &imported.minecraft_version,
        imported.loader().map(|l| (l, imported.loader_version.clone())),
    )
    .await;
    let mut instance =
        instances::create(dirs, &name, &imported.minecraft_version, loader, version.as_deref(), progress).await?;
    progress.stage(format!("Copying files from {}", imported.source));
    let from = imported.clone();
    let to = instance.root.clone();
    let copied = tokio::task::spawn_blocking(move || copy_into(&from, &to)).await?;
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&instance.root);
        return Err(e.context("The files couldn't be copied, so nothing was changed in the new instance."));
    }
    if let Some(mb) = imported.maximum_memory_mb {
        instance.file.launcher.maximum_memory = Some(mb);
    }
    if let Some(args) = imported.java_arguments.as_deref().filter(|a| !a.is_empty()) {
        instance.file.launcher.java_arguments = Some(args.to_string());
    }
    instance.file.launcher.description = format!("Imported from {}", imported.source);
    for w in imported.warnings.iter().chain(warnings.iter()) {
        eprintln!("[import] {w}");
    }
    instance.save().context("saving instance.json")?;
    Ok(instance)
}

// ---------------------------------------------------------------------------------------------------------
// The entry points

/// Imports a CurseForge modpack. Implemented by the CurseForge package; see HANDOFF.md.
pub trait CurseForgeImporter: Send + Sync {
    /// Import a CurseForge `.zip` modpack. `project_id` / `file_id` are known when it was downloaded from a page.
    fn import_zip<'a>(
        &'a self,
        dirs: &'a Dirs,
        zip: &'a Path,
        project_id: Option<i64>,
        file_id: Option<i64>,
        progress: &'a Arc<Progress>,
    ) -> BoxFuture<'a, Result<Instance>>;

    /// Import the newest (or the given) file of a CurseForge modpack page.
    fn import_page<'a>(
        &'a self,
        dirs: &'a Dirs,
        slug: &'a str,
        file_id: Option<i64>,
        progress: &'a Arc<Progress>,
    ) -> BoxFuture<'a, Result<Instance>>;
}

/// What the importers can use that lives in other packages.
#[derive(Clone)]
pub struct ImportContext {
    pub curseforge_import: Option<Arc<dyn CurseForgeImporter>>,
    pub curseforge: Arc<dyn CurseForgeSource>,
    /// Look the pack's mods up on the other platforms after an FTB install (see [`crate::ftb::InstallOptions`]).
    pub ftb_options: crate::ftb::InstallOptions,
}

impl Default for ImportContext {
    fn default() -> Self {
        ImportContext {
            curseforge_import: None,
            curseforge: Arc::new(crate::cf_seam::MinimalCurseForge),
            ftb_options: Default::default(),
        }
    }
}

/// Import a file, folder or web address: pick the importer from what it is.
pub async fn import_input(
    dirs: &Dirs,
    ctx: &ImportContext,
    input: &str,
    name: Option<&str>,
    progress: &Arc<Progress>,
) -> Result<Instance> {
    let input = input.trim().trim_matches('"');
    if input.is_empty() {
        bail!("Choose a file or paste a link first");
    }
    if let Some(url) = detect::detect_url(input) {
        return import_url(dirs, ctx, url, name, progress).await;
    }
    let path = Path::new(input.strip_prefix("file://").unwrap_or(input));
    if !path.exists() {
        bail!("{} doesn't exist", path.display());
    }
    import_source(dirs, ctx, detect::detect_path(path), name, progress).await
}

/// Import something already identified.
pub async fn import_source(
    dirs: &Dirs,
    ctx: &ImportContext,
    source: Source,
    name: Option<&str>,
    progress: &Arc<Progress>,
) -> Result<Instance> {
    match source {
        Source::Modrinth(path) => {
            let mut instance = crate::modrinth::import_mrpack(dirs, &path, progress).await?;
            if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
                instance.file.launcher.name = n.to_string();
                instance.save()?;
            }
            Ok(instance)
        }
        Source::CurseForge(path) => match &ctx.curseforge_import {
            Some(importer) => importer.import_zip(dirs, &path, None, None, progress).await,
            None => bail!("This is a CurseForge modpack. CurseForge modpack import isn't connected in this build yet."),
        },
        Source::MultiMcArchive(path) => {
            multimc::import_archive(dirs, &path, &multimc::ImportOptions { name: name.map(str::to_string) }, progress)
                .await
        }
        Source::LauncherFolder(path) => {
            let imported = read(&path)?;
            import_instance(dirs, &imported, name, progress).await
        }
        Source::World(_) => bail!(
            "This is a Minecraft world. Worlds are added to an instance's saves folder, not imported as an instance: drop it on the instance instead."
        ),
        Source::MinecraftFolder(_) => bail!(
            "This looks like a plain Minecraft folder, not an instance from another launcher. Choose the instance folder from Prism, MultiMC, CurseForge, GDLauncher or ATLauncher."
        ),
        Source::Unknown => bail!(
            "This isn't a modpack or an instance from a supported launcher. BlockForge can import Modrinth (.mrpack), CurseForge and MultiMC / Prism zips, and instance folders."
        ),
    }
}

/// Import from a web address.
pub async fn import_url(
    dirs: &Dirs,
    ctx: &ImportContext,
    url: UrlSource,
    name: Option<&str>,
    progress: &Arc<Progress>,
) -> Result<Instance> {
    match url {
        UrlSource::CurseForge { slug, file_id } => match &ctx.curseforge_import {
            Some(importer) => importer.import_page(dirs, &slug, file_id, progress).await,
            None => bail!("CurseForge modpack links aren't connected in this build yet."),
        },
        UrlSource::Modrinth { slug } => crate::modrinth::install_modpack(dirs, &slug, progress).await,
        UrlSource::Ftb { pack_id } => {
            progress.stage("Finding the pack");
            let pack = crate::ftb::pack(dirs, pack_id).await?.with_context(|| format!("FTB has no pack {pack_id}"))?;
            let version = pack
                .newest_release()
                .or_else(|| pack.versions_newest_first().into_iter().next())
                .context("this pack has no versions")?;
            let mut opts = ctx.ftb_options.clone();
            opts.name = name.map(str::to_string).or(opts.name);
            crate::ftb::install(dirs, pack_id, version.id, &opts, ctx.curseforge.as_ref(), progress).await
        }
        UrlSource::File { url, extension } => {
            progress.stage("Downloading the file");
            let temp = dirs.temp.join(format!("import-{}.{extension}", uuid::Uuid::new_v4().simple()));
            install::download_all(vec![Job::new(&url, &temp)], progress, false)
                .await
                .context("Failed to download modpack file")?;
            let result = import_source(dirs, ctx, detect::detect_path(&temp), name, progress).await;
            let _ = std::fs::remove_file(&temp);
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn write(file: &Path, content: &str) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
    }

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bf-imp-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The sample the Java edition's own `InstanceImporterTest` builds.
    fn prism(root: &Path) -> PathBuf {
        let dir = root.join("My Pack");
        write(
            &dir.join("instance.cfg"),
            "name=My Pack\nOverrideMemory=true\nMaxMemAlloc=6144\nOverrideJavaArgs=true\nJvmArgs=-Dfoo=bar\n",
        );
        write(
            &dir.join("mmc-pack.json"),
            r#"{"components":[{"uid":"net.minecraft","version":"1.20.1"},{"uid":"net.fabricmc.fabric-loader","version":"0.15.7"}],"formatVersion":1}"#,
        );
        write(&dir.join(".minecraft/mods/sodium.jar"), "jar");
        write(&dir.join(".minecraft/mods/lithium.jar"), "jar");
        write(&dir.join(".minecraft/saves/World1/level.dat"), "world");
        write(&dir.join(".minecraft/config/sodium.json"), "{}");
        write(&dir.join(".minecraft/options.txt"), "fov:90");
        write(&dir.join(".minecraft/logs/latest.log"), "old log");
        write(&dir.join(".minecraft/crash-reports/crash.txt"), "crash");
        dir
    }

    #[test]
    fn reads_prism_instances() {
        let t = temp();
        let i = read(&prism(&t)).unwrap();
        assert_eq!(i.name, "My Pack");
        assert_eq!(i.minecraft_version, "1.20.1");
        assert_eq!(i.loader_type.as_deref(), Some("fabric"));
        assert_eq!(i.loader_version.as_deref(), Some("0.15.7"));
        assert_eq!(i.maximum_memory_mb, Some(6144));
        assert_eq!(i.java_arguments.as_deref(), Some("-Dfoo=bar"));
        assert_eq!(i.loader_label(), "Fabric 0.15.7");
    }

    #[test]
    fn reads_curseforge_instances() {
        let t = temp();
        let dir = t.join("ATM9");
        write(
            &dir.join("minecraftinstance.json"),
            r#"{"name":"ATM9","gameVersion":"1.20.1","baseModLoader":{"name":"forge-47.2.0","minecraftVersion":"1.20.1"},"allocatedMemory":8192}"#,
        );
        write(&dir.join("mods/a.jar"), "x");
        let i = read(&dir).unwrap();
        assert_eq!(i.source, "CurseForge");
        assert_eq!(i.loader_type.as_deref(), Some("forge"));
        assert_eq!(i.loader_version.as_deref(), Some("47.2.0"));
        assert_eq!(i.maximum_memory_mb, Some(8192));
    }

    #[test]
    fn curseforge_fabric_versions_leave_out_the_minecraft_version() {
        let t = temp();
        let named = t.join("Named");
        write(
            &named.join("minecraftinstance.json"),
            r#"{"name":"Named","gameVersion":"1.20.1","baseModLoader":{"name":"fabric-0.14.21-1.20.1","minecraftVersion":"1.20.1"}}"#,
        );
        let a = read(&named).unwrap();
        assert_eq!((a.loader_type.as_deref(), a.loader_version.as_deref()), (Some("fabric"), Some("0.14.21")));
        let stored = t.join("Stored");
        write(
            &stored.join("minecraftinstance.json"),
            r#"{"name":"Stored","gameVersion":"1.21.1","baseModLoader":{"name":"quilt-0.26.4-1.21.1","forgeVersion":"0.26.4"}}"#,
        );
        let b = read(&stored).unwrap();
        assert_eq!((b.loader_type.as_deref(), b.loader_version.as_deref()), (Some("quilt"), Some("0.26.4")));
        let neo = t.join("Neo");
        write(
            &neo.join("minecraftinstance.json"),
            r#"{"name":"Neo","gameVersion":"1.21.1","baseModLoader":{"name":"neoforge-21.1.57","minecraftVersion":"1.21.1"}}"#,
        );
        assert_eq!(read(&neo).unwrap().loader_type.as_deref(), Some("neoforge"));
    }

    #[test]
    fn picks_a_name_that_is_not_taken_yet() {
        let taken: HashSet<&str> = ["My Pack", "My Pack (2)"].into_iter().collect();
        let t = |n: &str| taken.contains(n);
        assert_eq!(unique_name(Some("Other"), t), "Other");
        assert_eq!(unique_name(Some("My Pack"), t), "My Pack (3)");
        assert_eq!(unique_name(Some("★★★"), t), "Imported instance");
        assert_eq!(unique_name(None, t), "Imported instance");
    }

    #[test]
    fn reads_atlauncher_and_gdlauncher_instances() {
        let t = temp();
        let atl = t.join("atl");
        write(
            &atl.join("instance.json"),
            r#"{"id":"1.19.2","launcher":{"name":"Old Pack","loaderVersion":{"type":"Forge","version":"43.2.0"}}}"#,
        );
        let a = read(&atl).unwrap();
        assert_eq!((a.source.as_str(), a.loader_type.as_deref()), ("ATLauncher", Some("forge")));
        let gd = t.join("gd");
        write(
            &gd.join("config.json"),
            r#"{"name":"GD Pack","game_configuration":{"version":{"release":"1.18.2","modloaders":[{"type":"quilt","version":"0.20"}]}}}"#,
        );
        std::fs::create_dir_all(gd.join("instance")).unwrap();
        let g = read(&gd).unwrap();
        assert_eq!((g.source.as_str(), g.loader_type.as_deref()), ("GDLauncher", Some("quilt")));
        assert_eq!(g.game_dir, gd.join("instance"));
    }

    #[test]
    fn vanilla_instances_have_no_loader() {
        let t = temp();
        let dir = t.join("v");
        write(&dir.join("instance.cfg"), "name=Vanilla\n");
        write(&dir.join("mmc-pack.json"), r#"{"components":[{"uid":"net.minecraft","version":"1.21"}]}"#);
        std::fs::create_dir_all(dir.join(".minecraft")).unwrap();
        let i = read(&dir).unwrap();
        assert_eq!(i.loader_type, None);
        assert_eq!(i.loader_label(), "Vanilla");
        assert_eq!(i.loader(), None);
    }

    #[test]
    fn rejects_broken_or_unknown_folders() {
        let t = temp();
        assert!(read(&t).is_err());
        let broken = t.join("broken");
        write(&broken.join("instance.cfg"), "name=x");
        write(&broken.join("mmc-pack.json"), "{ not json");
        assert!(read(&broken).is_err());
        let no_version = t.join("noversion");
        write(&no_version.join("instance.cfg"), "name=x");
        write(&no_version.join("mmc-pack.json"), r#"{"components":[]}"#);
        assert!(read(&no_version).is_err());
    }

    #[test]
    fn preview_counts_without_changing_anything() {
        let t = temp();
        let i = read(&prism(&t)).unwrap();
        let p = preview(&i).unwrap();
        assert_eq!(p.mods, 2);
        assert_eq!(p.worlds, 1);
        assert!(p.top_level.contains(&"mods".to_string()));
        assert!(!p.top_level.contains(&"logs".to_string()));
        assert!(!p.top_level.contains(&"crash-reports".to_string()));
        assert_eq!(p.files, 5);
        assert!(i.game_dir.join("logs/latest.log").exists());
    }

    #[test]
    fn copies_game_files_and_skips_history() {
        let t = temp();
        let i = read(&prism(&t.join("src"))).unwrap();
        let target = t.join("instance");
        std::fs::create_dir_all(target.join("mods")).unwrap();
        write(&target.join("options.txt"), "fov:70");
        copy_into(&i, &target).unwrap();
        assert!(target.join("mods/sodium.jar").exists());
        assert!(target.join("saves/World1/level.dat").exists());
        assert_eq!(std::fs::read_to_string(target.join("options.txt")).unwrap(), "fov:90");
        assert!(!target.join("logs").exists());
        assert!(!target.join("crash-reports").exists());
        assert!(
            std::fs::read_dir(&t).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".import-"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn skips_symbolic_links_instead_of_following_them() {
        let t = temp();
        let src = prism(&t.join("src"));
        let secret = t.join("secret.txt");
        write(&secret, "do not copy");
        std::os::unix::fs::symlink(&secret, src.join(".minecraft/mods/link.jar")).unwrap();
        let target = t.join("instance");
        std::fs::create_dir_all(&target).unwrap();
        copy_into(&read(&src).unwrap(), &target).unwrap();
        assert!(!target.join("mods/link.jar").exists());
        assert!(target.join("mods/sodium.jar").exists());
    }

    #[test]
    fn scan_finds_instances_in_launcher_folders() {
        let t = temp();
        prism(&t.join("prism"));
        let found = scan(&[t.join("prism"), t.join("missing")]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "My Pack");
        assert!(scan(&[]).is_empty());
    }

    #[test]
    fn a_failed_copy_puts_nothing_in_the_target() {
        let t = temp();
        let src = prism(&t.join("src"));
        let i = read(&src).unwrap();
        let target = t.join("instance");
        // the target can't hold a folder where the imported side has a file with that name
        std::fs::create_dir_all(target.join("options.txt")).unwrap();
        write(&target.join("options.txt/keep.txt"), "x");
        let before: Vec<_> = std::fs::read_dir(&target).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(copy_into(&i, &target).is_err());
        assert!(!target.join("mods").exists(), "what was already moved in is taken out again");
        let after: Vec<_> = std::fs::read_dir(&target).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(before, after);
        assert!(
            std::fs::read_dir(&t).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".import-"))
        );
    }
}

#[cfg(test)]
mod copy_tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn names_on_the_skip_list_are_only_skipped_at_the_top() {
        let base = crate::testutil::scratch("import-depth");
        let game = base.join("game");
        // junk at the top that isn't worth importing
        write(&game.join("logs/latest.log"), "log");
        write(&game.join("assets/indexes/x.json"), "{}");
        write(&game.join("config.json"), "{}");
        write(&game.join("icon.png"), "png");
        // the player's own files deeper down that share those names
        write(&game.join("resourcepacks/MyPack/assets/minecraft/textures/a.png"), "tex");
        write(&game.join("resourcepacks/MyPack/pack.mcmeta"), "{}");
        write(&game.join("kubejs/assets/kubejs/lang/en_us.json"), "{}");
        write(&game.join("config/somemod/config.json"), "{\"a\":1}");
        write(&game.join("saves/World/icon.png"), "thumb");
        write(&game.join("saves/World/level.dat"), "lvl");
        write(&game.join("config/mod/cache/data.bin"), "data");
        let target = base.join("instances/New");
        copy_game_dir(&game, &target).unwrap();
        for kept in [
            "resourcepacks/MyPack/assets/minecraft/textures/a.png",
            "kubejs/assets/kubejs/lang/en_us.json",
            "config/somemod/config.json",
            "saves/World/icon.png",
            "saves/World/level.dat",
            "config/mod/cache/data.bin",
        ] {
            assert!(target.join(kept).is_file(), "{kept} was lost in the import");
        }
        for skipped in ["logs", "assets", "config.json", "icon.png"] {
            assert!(!target.join(skipped).exists(), "{skipped} should not be imported");
        }
        let leftovers: Vec<_> = std::fs::read_dir(base.join("instances"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".import-"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }
}
