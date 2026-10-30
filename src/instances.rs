//! Instances: one folder each under `instances/`, described by an `instance.json` in the Java edition's format.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

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

/// How many days of sessions an instance's file remembers.
pub const PLAY_DAYS_KEPT: i64 = 120;

/// The day of a Minecraft log file called `2026-10-17-2.log.gz` (one per session), if that is what the name is.
fn log_day(name: &str) -> Option<chrono::NaiveDate> {
    let stem = name.strip_suffix(".log.gz").or_else(|| name.strip_suffix(".log"))?;
    let (date, n) = stem.rsplit_once('-')?;
    n.parse::<u32>().ok()?;
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
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

    /// Write `instance.json`. An instance whose folder is gone (deleted while a job for it was still running) is not
    /// brought back as an empty folder with only this file in it.
    pub fn save(&self) -> Result<()> {
        if !self.root.is_dir() {
            anyhow::bail!("{} was deleted, so its settings weren't saved", self.name());
        }
        write_json_atomic(&self.instance_file(), &self.file)
    }

    pub fn mark_played(&mut self) -> Result<()> {
        self.file.launcher.last_played = Some(chrono::Utc::now().timestamp_millis());
        self.file.launcher.num_plays = Some(self.file.launcher.num_plays.unwrap_or(0) + 1);
        self.record_play_day(chrono::Local::now().date_naive());
        self.save()
    }

    /// How long this instance has been played, in seconds: the games this launcher started and saw end (kept as
    /// `playSeconds` in its file, so it is 0 for an instance only played before that was counted).
    pub fn play_seconds(&self) -> u64 {
        self.file.launcher.extras.get("playSeconds").and_then(Value::as_u64).unwrap_or(0)
    }

    /// Add the length of a session that ended to [`Self::play_seconds`], and save.
    pub fn add_play_time(&mut self, secs: u64) -> Result<()> {
        let total = self.play_seconds().saturating_add(secs);
        self.file.launcher.extras.insert("playSeconds".into(), json!(total));
        self.save()
    }

    /// Count a session on `day` in the instance's own file (kept for the last [`PLAY_DAYS_KEPT`] days), so the play
    /// history stays right when Minecraft's logs are deleted.
    fn record_play_day(&mut self, day: chrono::NaiveDate) {
        let entry = self.file.launcher.extras.entry("playDays").or_insert_with(|| json!({}));
        let Some(map) = entry.as_object_mut() else { return };
        let key = day.format("%Y-%m-%d").to_string();
        let n = map.get(&key).and_then(Value::as_u64).unwrap_or(0) + 1;
        map.insert(key, json!(n));
        let oldest = (day - chrono::Duration::days(PLAY_DAYS_KEPT)).format("%Y-%m-%d").to_string();
        map.retain(|k, _| k.as_str() >= oldest.as_str());
    }

    /// How many sessions were played on each of the last `days` days, oldest first and ending with `today`. It joins
    /// the days this launcher recorded with the dated log files Minecraft keeps in the instance (one per session), so
    /// an instance played before the record existed still has a history.
    pub fn play_days(&self, today: chrono::NaiveDate, days: usize) -> Vec<u8> {
        let mut counts: std::collections::HashMap<chrono::NaiveDate, u32> = std::collections::HashMap::new();
        if let Some(map) = self.file.launcher.extras.get("playDays").and_then(Value::as_object) {
            for (k, v) in map {
                if let (Ok(d), Some(n)) = (chrono::NaiveDate::parse_from_str(k, "%Y-%m-%d"), v.as_u64()) {
                    counts.insert(d, n as u32);
                }
            }
        }
        let logs = self.root.join("logs");
        let mut from_logs: std::collections::HashMap<chrono::NaiveDate, u32> = std::collections::HashMap::new();
        if let Ok(rd) = std::fs::read_dir(&logs) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if let Some(d) = log_day(&name) {
                    *from_logs.entry(d).or_insert(0) += 1;
                } else if name == "latest.log" {
                    if let Some(modified) = e.metadata().ok().and_then(|m| m.modified().ok()) {
                        let d = chrono::DateTime::<chrono::Local>::from(modified).date_naive();
                        let n = from_logs.entry(d).or_insert(0);
                        *n = (*n).max(1);
                    }
                }
            }
        }
        for (d, n) in from_logs {
            let c = counts.entry(d).or_insert(0);
            *c = (*c).max(n);
        }
        // the day of the last session is known from the instance itself, even without a record or a log
        let last =
            self.file.launcher.last_played.filter(|ms| *ms > 0).and_then(chrono::DateTime::from_timestamp_millis);
        if let Some(last) = last {
            let d = last.with_timezone(&chrono::Local).date_naive();
            let c = counts.entry(d).or_insert(0);
            *c = (*c).max(1);
        }
        (0..days)
            .map(|i| {
                let d = today - chrono::Duration::days((days - 1 - i) as i64);
                counts.get(&d).copied().unwrap_or(0).min(255) as u8
            })
            .collect()
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
    let dest = to_dir.join(&file.name);
    // on Linux and macOS a rename quietly replaces a file of the same name: never lose the other copy
    if dest.exists() {
        anyhow::bail!("{} is already in {}", file.name, to_dir.display());
    }
    std::fs::rename(&file.path, dest)?;
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
    load_all_and_unreadable(dirs).0
}

/// [`load_all`], and the folders whose `instance.json` is there but can't be read (see [`unreadable`]), found in the
/// same pass.
pub fn load_all_and_unreadable(dirs: &Dirs) -> (Vec<Instance>, Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(&dirs.instances) else { return (Vec::new(), Vec::new()) };
    // folders starting with a dot are the launcher's own work folders (a restore in progress, a copy being made),
    // which can hold an instance.json for a moment; instance folders never start with one
    let roots: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    // parsing hundreds of KB of JSON per instance adds up for big libraries; spread it over a few threads
    let threads = roots.len().clamp(1, 8);
    let chunk = roots.len().div_ceil(threads).max(1);
    let read: Vec<Result<Instance, PathBuf>> = std::thread::scope(|s| {
        let handles: Vec<_> = roots
            .chunks(chunk)
            .map(|c| {
                s.spawn(move || {
                    c.iter()
                        .filter_map(|r| match load_one(r) {
                            Some(i) => Some(Ok(i)),
                            // a folder without an instance.json isn't counted: it may be one being made
                            None => r.join("instance.json").is_file().then(|| Err(r.clone())),
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    let (mut all, mut broken) = (Vec::new(), Vec::new());
    for r in read {
        match r {
            Ok(i) => all.push(i),
            Err(p) => broken.push(p),
        }
    }
    all.sort_by(|a, b| {
        b.last_played().cmp(&a.last_played()).then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
    });
    broken.sort();
    (all, broken)
}

/// Instance folders whose `instance.json` is there but can't be read (damaged, cut short, edited by hand): they are
/// left out of [`load_all`], and the player is told rather than left to think the instance and its worlds are gone.
/// A folder without an `instance.json` at all isn't counted (it may be one being made, or not an instance).
pub fn unreadable(dirs: &Dirs) -> Vec<PathBuf> {
    load_all_and_unreadable(dirs).1
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
    // the file as it is now, not as it was when this began minutes ago: a rename, a setting or a mod changed
    // meanwhile is kept
    let mut fresh = load_one(&instance.root).unwrap_or_else(|| instance.clone());
    fresh.file.version = version;
    fresh.file.launcher.loader_version = info;
    fresh.save()?;
    *instance = fresh;
    Ok(())
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

    let (folder, root) = paths::claim_unique_folder(&dirs.instances, name)?;
    // a folder left without its instance.json would hold the name and never show up anywhere
    let made = (|| -> Result<Instance> {
        for sub in ["mods", "saves", "resourcepacks", "shaderpacks", "config"] {
            std::fs::create_dir_all(root.join(sub))?;
        }
        new_instance_file(name, mc_version, merged, loader_info, folder, root.clone())
    })();
    if made.is_err() {
        let _ = std::fs::remove_dir_all(&root);
    }
    made
}

fn new_instance_file(
    name: &str,
    mc_version: &str,
    merged: VersionJson,
    loader_info: Option<LoaderVersion>,
    folder: String,
    root: PathBuf,
) -> Result<Instance> {
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

/// Check every file the instance's game needs (libraries, assets, the game itself, Mojang's Java for it) against its
/// checksum and download again whatever is damaged or missing: the cure for "it crashes at start since the
/// computer went off during a download". The instance's own files (worlds, mods, settings) are not touched.
pub async fn repair(dirs: &Dirs, instance: &Instance, progress: &Arc<Progress>) -> Result<()> {
    let version = &instance.file.version;
    if let Some(jv) = version.java_version.as_ref().filter(|j| !j.component.is_empty()) {
        progress.stage(format!("Checking Java ({})", jv.component));
        crate::java::repair_runtime(dirs, &jv.component, progress).await.context("checking Java")?;
    }
    progress.stage("Checking the game's files");
    install::ensure_game_files(dirs, version, progress, true).await.context("checking the game's files")
}

pub fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn delete(instance: &Instance) -> Result<()> {
    std::fs::remove_dir_all(&instance.root).with_context(|| format!("deleting {}", instance.root.display()))
}

/// Copy an instance folder under a new name.
pub fn duplicate(dirs: &Dirs, instance: &Instance, new_name: &str) -> Result<Instance> {
    let (folder, root) = paths::claim_unique_folder(&dirs.instances, new_name)?;
    let made = (|| -> Result<Instance> {
        copy_dir(&instance.root, &root)?;
        let mut copy =
            Instance { folder, mod_count: instance.mod_count, root: root.clone(), file: instance.file.clone() };
        copy.file.uuid = uuid_v4();
        copy.file.launcher.name = new_name.to_string();
        copy.file.launcher.last_played = None;
        copy.file.launcher.num_plays = None;
        copy.file.launcher.extras.remove("playDays");
        copy.file.launcher.extras.remove("playSeconds");
        copy.save()?;
        Ok(copy)
    })();
    // a copy that stopped half way (a full disk, a file the game holds open) is removed rather than left half there
    if made.is_err() {
        let _ = std::fs::remove_dir_all(&root);
    }
    made
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from).with_context(|| format!("reading {}", from.display()))?.flatten() {
        let dest = to.join(e.file_name());
        let ty = e.file_type()?;
        if ty.is_dir() {
            copy_dir(&e.path(), &dest)?;
        } else if ty.is_file() {
            // a real copy: the game edits worlds and options in place, which a hard link would share
            std::fs::copy(e.path(), &dest).with_context(|| format!("copying {}", e.path().display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod unreadable_tests {
    use super::*;

    #[test]
    fn a_damaged_instance_file_is_reported_not_hidden() {
        let base = crate::testutil::scratch("unreadable");
        let dirs = Dirs::at(&base);
        std::fs::create_dir_all(dirs.instances.join("Good")).unwrap();
        std::fs::write(dirs.instances.join("Good/instance.json"), "{}").unwrap();
        std::fs::create_dir_all(dirs.instances.join("Broken")).unwrap();
        std::fs::write(dirs.instances.join("Broken/instance.json"), "{\"launcher\": {\"name\": \"Bro").unwrap();
        // no instance.json yet (one being made) and the launcher's own work folders don't count
        std::fs::create_dir_all(dirs.instances.join("Empty")).unwrap();
        std::fs::create_dir_all(dirs.instances.join(".restore-1")).unwrap();
        std::fs::write(dirs.instances.join(".restore-1/instance.json"), "x").unwrap();
        assert_eq!(unreadable(&dirs), vec![dirs.instances.join("Broken")]);
        assert_eq!(load_all(&dirs).len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod play_day_tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn blank(root: PathBuf) -> Instance {
        Instance { folder: "x".into(), mod_count: 0, root, file: InstanceFile::default() }
    }

    #[test]
    fn log_names_give_their_day() {
        assert_eq!(log_day("2026-10-17-2.log.gz"), Some(day(2026, 10, 17)));
        assert_eq!(log_day("2026-10-17-1.log"), Some(day(2026, 10, 17)));
        assert_eq!(log_day("latest.log"), None);
        assert_eq!(log_day("debug.log.gz"), None);
        assert_eq!(log_day("2026-13-40-1.log.gz"), None);
    }

    #[test]
    fn recorded_days_count_and_old_ones_fall_away() {
        let mut i = blank(std::env::temp_dir().join("bf-no-such-instance-dir"));
        let today = day(2026, 10, 20);
        i.record_play_day(day(2026, 10, 18));
        i.record_play_day(today);
        i.record_play_day(today);
        // a day too old for the record is dropped when a newer one is written
        i.file.launcher.extras.insert("playDays".into(), json!({"2026-01-01": 4, "2026-10-18": 1}));
        i.record_play_day(today);
        let map = i.file.launcher.extras["playDays"].as_object().unwrap();
        assert!(!map.contains_key("2026-01-01"));
        let days = i.play_days(today, 5);
        // the 16th, 17th, 18th, 19th and today (the 20th): one session on the 18th, one today
        assert_eq!(days, vec![0, 0, 1, 0, 1]);
    }

    #[test]
    fn dated_logs_fill_in_a_history_the_record_never_had() {
        let root = std::env::temp_dir().join(format!("bf-play-days-{}", std::process::id()));
        let logs = root.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        for n in ["2026-10-17-1.log.gz", "2026-10-17-2.log.gz", "2026-10-19-1.log.gz", "debug.log"] {
            std::fs::write(logs.join(n), b"x").unwrap();
        }
        let mut i = blank(root.clone());
        // the record knows one session on the 19th too: the two sources do not add up, the larger wins
        i.file.launcher.extras.insert("playDays".into(), json!({"2026-10-19": 1}));
        let days = i.play_days(day(2026, 10, 20), 4);
        // oldest first: the 17th, 18th, 19th and today (the 20th)
        assert_eq!(days, vec![2, 0, 1, 0]);
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod robustness_tests {
    use super::*;

    fn temp_dirs(tag: &str) -> Dirs {
        let base = std::env::temp_dir().join(format!("bf-instances-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dirs = Dirs::at(&base);
        dirs.ensure().unwrap();
        dirs
    }

    fn make(dirs: &Dirs, name: &str) -> Instance {
        let (folder, root) = paths::claim_unique_folder(&dirs.instances, name).unwrap();
        let mut file = InstanceFile::default();
        file.launcher.name = name.to_string();
        let i = Instance { folder, mod_count: 0, root, file };
        i.save().unwrap();
        i
    }

    #[test]
    fn switching_a_mod_off_never_overwrites_a_file_of_the_same_name() {
        let dirs = temp_dirs("toggle");
        let inst = make(&dirs, "Pack");
        std::fs::create_dir_all(inst.mods_dir()).unwrap();
        std::fs::create_dir_all(inst.disabled_mods_dir()).unwrap();
        std::fs::write(inst.mods_dir().join("sodium.jar"), b"new").unwrap();
        std::fs::write(inst.disabled_mods_dir().join("sodium.jar"), b"old").unwrap();
        let file = inst.mod_files().into_iter().find(|m| m.enabled).unwrap();
        assert!(set_mod_enabled(&inst, &file, false).is_err());
        assert_eq!(std::fs::read(inst.mods_dir().join("sodium.jar")).unwrap(), b"new");
        assert_eq!(std::fs::read(inst.disabled_mods_dir().join("sodium.jar")).unwrap(), b"old");
        // without the clash it moves
        std::fs::remove_file(inst.disabled_mods_dir().join("sodium.jar")).unwrap();
        set_mod_enabled(&inst, &file, false).unwrap();
        assert!(inst.disabled_mods_dir().join("sodium.jar").exists());
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn duplicates_get_their_own_folder_identity_and_history() {
        let dirs = temp_dirs("dup");
        let mut inst = make(&dirs, "My Pack");
        std::fs::create_dir_all(inst.root.join("saves/World")).unwrap();
        std::fs::write(inst.root.join("saves/World/level.dat"), b"lvl").unwrap();
        inst.mark_played().unwrap();
        inst.add_play_time(90).unwrap();
        inst.add_play_time(30).unwrap();
        assert_eq!(load_one(&inst.root).unwrap().play_seconds(), 120);
        let copy = duplicate(&dirs, &inst, "My Pack").unwrap();
        assert_eq!(copy.play_seconds(), 0);
        assert_ne!(copy.root, inst.root);
        assert_ne!(copy.file.uuid, inst.file.uuid);
        assert_eq!(copy.file.launcher.num_plays, None);
        assert_eq!(std::fs::read(copy.root.join("saves/World/level.dat")).unwrap(), b"lvl");
        // the copy is real, not a link: changing it leaves the original alone
        std::fs::write(copy.root.join("saves/World/level.dat"), b"changed").unwrap();
        assert_eq!(std::fs::read(inst.root.join("saves/World/level.dat")).unwrap(), b"lvl");
        assert_eq!(load_all(&dirs).len(), 2);
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[cfg(unix)]
    #[test]
    fn a_duplicate_that_fails_half_way_leaves_nothing_behind() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe_is_root() {
            return; // root can read everything, so the copy can't be made to fail this way
        }
        let dirs = temp_dirs("dup-fail");
        let inst = make(&dirs, "Pack");
        let locked = inst.root.join("saves");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("x"), b"x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = duplicate(&dirs, &inst, "Copy");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_err());
        let folders: Vec<_> = std::fs::read_dir(&dirs.instances).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(folders.len(), 1, "the half-made copy was removed: {folders:?}");
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[cfg(unix)]
    fn unsafe_is_root() -> bool {
        std::process::Command::new("id").arg("-u").output().is_ok_and(|o| o.stdout.starts_with(b"0"))
    }

    #[test]
    fn broken_instance_files_are_skipped_and_never_crash_the_list() {
        let dirs = temp_dirs("broken");
        make(&dirs, "Good");
        for (folder, text) in [
            ("Empty", ""),
            ("Truncated", r#"{"launcher":{"name":"Half"#),
            ("Array", "[]"),
            ("WrongTypes", r#"{"launcher":{"name":5,"lastPlayed":"yesterday"}}"#),
        ] {
            let root = dirs.instances.join(folder);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("instance.json"), text).unwrap();
        }
        // a stray file and a folder without instance.json
        std::fs::write(dirs.instances.join("notes.txt"), b"hi").unwrap();
        std::fs::create_dir_all(dirs.instances.join("NoJson")).unwrap();
        let all = load_all(&dirs);
        assert!(all.iter().any(|i| i.name() == "Good"));
        assert!(all.len() <= 2, "{:?}", all.iter().map(|i| i.name().to_string()).collect::<Vec<_>>());
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn play_history_survives_absurd_values() {
        let dirs = temp_dirs("history");
        let mut inst = make(&dirs, "H");
        inst.file.launcher.last_played = Some(i64::MAX);
        inst.file.launcher.extras.insert("playDays".into(), json!({"not a date": 3, "2026-10-20": u64::MAX}));
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
        let days = inst.play_days(today, 3);
        assert_eq!(days.len(), 3);
        assert_eq!(days[2], 255, "a huge count is shown as the most the strip can hold");
        assert!(inst.play_days(today, 0).is_empty());
        inst.file.launcher.extras.insert("playDays".into(), json!("garbage"));
        inst.record_play_day(today);
        let _ = std::fs::remove_dir_all(&dirs.base);
    }
}

#[cfg(test)]
mod deleted_tests {
    use super::*;

    #[test]
    fn a_job_finishing_after_the_instance_was_deleted_doesnt_bring_it_back() {
        let base = crate::testutil::scratch("deleted-save");
        let dirs = Dirs::at(&base);
        let (folder, root) = paths::claim_unique_folder(&dirs.instances, "Gone").unwrap();
        let mut inst = Instance { folder, mod_count: 0, root: root.clone(), file: InstanceFile::default() };
        inst.save().unwrap();
        delete(&inst).unwrap();
        // a background job (a mod update, a loader change) still holds its copy and saves it at the end
        inst.file.launcher.name = "changed".into();
        assert!(inst.save().is_err());
        assert!(inst.mark_played().is_err());
        assert!(!root.exists(), "the deleted instance must stay deleted");
        assert!(load_all(&dirs).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod repair_tests {
    use super::*;
    use crate::testutil::{Reply, Server, runtime, scratch, sha1};

    #[test]
    fn repair_replaces_a_damaged_library_that_a_normal_start_would_trust() {
        runtime().block_on(async {
            let server = Server::start().await;
            let base = scratch("repair");
            let dirs = Dirs::at(&base);
            dirs.ensure().unwrap();
            let good = b"the real library bytes".to_vec();
            let url = server.route("/lib.jar", Reply::ok(good.clone()));
            let version: crate::model::VersionJson = serde_json::from_value(json!({
                "id": "1.20.1",
                "libraries": [{"name": "org.example:lib:1.0", "downloads": {"artifact": {
                    "path": "org/example/lib/1.0/lib-1.0.jar", "url": url, "sha1": sha1(&good), "size": good.len()
                }}}]
            }))
            .unwrap();
            let (folder, root) = paths::claim_unique_folder(&dirs.instances, "Pack").unwrap();
            let inst = Instance { folder, mod_count: 0, root, file: InstanceFile { version, ..Default::default() } };
            // the same size, other bytes: what a bad sector or a copy cut short leaves
            let lib = dirs.libraries.join("org/example/lib/1.0/lib-1.0.jar");
            std::fs::create_dir_all(lib.parent().unwrap()).unwrap();
            let mut bad = good.clone();
            bad[0] ^= 0xff;
            std::fs::write(&lib, &bad).unwrap();

            // a normal start goes by the size and doesn't notice
            install::ensure_game_files(&dirs, &inst.file.version, &Progress::new(), false).await.unwrap();
            assert_eq!(std::fs::read(&lib).unwrap(), bad);
            assert_eq!(server.hits("/lib.jar"), 0);
            // a repair checks every file's checksum
            repair(&dirs, &inst, &Progress::new()).await.unwrap();
            assert_eq!(std::fs::read(&lib).unwrap(), good);
            assert_eq!(server.hits("/lib.jar"), 1);
            // nothing to do the second time
            repair(&dirs, &inst, &Progress::new()).await.unwrap();
            assert_eq!(server.hits("/lib.jar"), 1);
            let _ = std::fs::remove_dir_all(&base);
        });
    }
}
