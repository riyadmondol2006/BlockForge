//! Instance backups: zip files of an instance's folder, made the way the Java edition makes them, plus listing,
//! restoring and deleting them (the Java edition only creates them).
//!
//! A backup is `<backups folder>/<InstanceName>-<yyyy_MM_dd_HH_mm_ss>.zip`. Which files go in depends on the mode:
//! * `NORMAL`: `options.txt`, `saves/` and `config/`
//! * `NORMAL_PLUS_MODS`: the same plus `mods/`, `jarmods/` and `coremods/`
//! * `FULL`: everything
//!
//! The backups folder is the `backupsPath` setting when it points at a real folder, otherwise `backups/` in the
//! data folder.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::BufWriter;
#[cfg(test)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;

use crate::paths::Dirs;
use crate::svcsettings::ServiceSettings;

/// What a backup contains. The names are what the Java edition writes into `BlockForge.json`.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BackupMode {
    #[default]
    #[serde(rename = "NORMAL")]
    Normal,
    #[serde(rename = "NORMAL_PLUS_MODS")]
    NormalPlusMods,
    #[serde(rename = "FULL")]
    Full,
}

impl BackupMode {
    pub const ALL: [BackupMode; 3] = [BackupMode::Normal, BackupMode::NormalPlusMods, BackupMode::Full];

    /// The name stored in the settings file.
    pub fn key(self) -> &'static str {
        match self {
            BackupMode::Normal => "NORMAL",
            BackupMode::NormalPlusMods => "NORMAL_PLUS_MODS",
            BackupMode::Full => "FULL",
        }
    }

    pub fn from_key(key: &str) -> Option<BackupMode> {
        BackupMode::ALL.into_iter().find(|m| m.key().eq_ignore_ascii_case(key.trim()))
    }

    /// The wording of the instance menu.
    pub fn label(self) -> &'static str {
        match self {
            BackupMode::Normal => "Back up worlds and settings",
            BackupMode::NormalPlusMods => "Back up worlds, settings and mods",
            BackupMode::Full => "Back up everything",
        }
    }

    /// Whether a file at `name` (relative to the instance folder, with `/` separators) goes into a backup of this mode.
    /// Matches the Java edition's `startsWith` rules, so `config-old.txt` is kept by `NORMAL` too.
    pub fn includes(self, name: &str) -> bool {
        match self {
            BackupMode::Full => true,
            BackupMode::Normal => normal_part(name),
            BackupMode::NormalPlusMods => {
                normal_part(name)
                    || name.starts_with("mods")
                    || name.starts_with("jarmods")
                    || name.starts_with("coremods")
            }
        }
    }
}

fn normal_part(name: &str) -> bool {
    name.eq_ignore_ascii_case("options.txt") || name.starts_with("saves") || name.starts_with("config")
}

/// Letters and digits only, like the Java edition's `getSafeName()`.
pub fn safe_name(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// The start of the backup file names of the instance called `instance_name`: the Java edition's safe name, so both
/// editions find each other's backups. A name with no ASCII letters or digits (`Мой мир`, `世界`) keeps its own
/// letters instead, so its backups are found again and aren't mixed up with those of every other such instance.
pub fn backup_prefix(instance_name: &str) -> String {
    let ascii = safe_name(instance_name);
    if !ascii.is_empty() { ascii } else { crate::paths::folder_name(instance_name) }
}

/// Where backups are kept: the `backupsPath` setting if it is an existing folder, otherwise `backups/`.
pub fn backups_dir(dirs: &Dirs, settings: &ServiceSettings) -> PathBuf {
    match settings.backups_path.as_deref().map(Path::new) {
        Some(p) if p.is_dir() => p.to_path_buf(),
        _ => dirs.base.join("backups"),
    }
}

/// `2026_10_16_12_34_56`, the stamp the Java edition puts in backup names.
pub fn timestamp_now() -> String {
    Local::now().format("%Y_%m_%d_%H_%M_%S").to_string()
}

/// A backup file that exists in the backups folder.
#[derive(Clone, Debug)]
pub struct Backup {
    pub path: PathBuf,
    pub file_name: String,
    pub size: u64,
    /// When it was made, taken from the name and failing that from the file's date.
    pub created: DateTime<Local>,
}

/// Read the date out of `<name>-2026_10_16_12_34_56.zip`.
pub fn parse_name(file_name: &str) -> Option<(String, DateTime<Local>)> {
    let stem = file_name.strip_suffix(".zip")?;
    if stem.len() < 20 {
        return None;
    }
    // the stamp is ASCII, so in a backup name the cut is between two characters; in any other name (letters of
    // several bytes, an emoji) it may not be, and cutting there would crash
    let cut = stem.len() - 19;
    if !stem.is_char_boundary(cut) {
        return None;
    }
    let (prefix, stamp) = stem.split_at(cut);
    let name = prefix.strip_suffix('-')?;
    let naive = NaiveDateTime::parse_from_str(stamp, "%Y_%m_%d_%H_%M_%S").ok()?;
    Some((name.to_string(), Local.from_local_datetime(&naive).earliest()?))
}

/// The backups made for the instance called `instance_name`, newest first.
pub fn list(dirs: &Dirs, settings: &ServiceSettings, instance_name: &str) -> Vec<Backup> {
    list_with_prefix(&backups_dir(dirs, settings), &backup_prefix(instance_name))
}

/// Backups made of a server (`Server-<name>-<date>.zip`), newest first.
pub fn list_server(dirs: &Dirs, settings: &ServiceSettings, server_name: &str) -> Vec<Backup> {
    list_with_prefix(&backups_dir(dirs, settings), &format!("Server-{}", safe_name(server_name)))
}

fn list_with_prefix(folder: &Path, prefix: &str) -> Vec<Backup> {
    let Ok(rd) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut out: Vec<Backup> = rd
        .flatten()
        .filter_map(|e| {
            let file_name = e.file_name().to_string_lossy().to_string();
            let (name, created) = parse_name(&file_name)?;
            if name != prefix {
                return None;
            }
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let created = if meta.modified().is_ok() { created } else { Local::now() };
            Some(Backup { path: e.path(), file_name, size: meta.len(), created })
        })
        .collect();
    out.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.file_name.cmp(&a.file_name)));
    out
}

/// A name for a new backup file that isn't taken yet.
fn free_path(folder: &Path, prefix: &str) -> PathBuf {
    let stamp = timestamp_now();
    let first = folder.join(format!("{prefix}-{stamp}.zip"));
    if !first.exists() {
        return first;
    }
    // two backups in the same second: move the second one on by a second so the name still parses
    let base = Local::now();
    (1..3600)
        .map(|s| {
            folder.join(format!("{prefix}-{}.zip", (base + chrono::Duration::seconds(s)).format("%Y_%m_%d_%H_%M_%S")))
        })
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Make a backup of an instance folder. Returns the zip file.
pub fn create(
    dirs: &Dirs,
    settings: &ServiceSettings,
    instance_root: &Path,
    instance_name: &str,
    mode: BackupMode,
) -> Result<PathBuf> {
    // the player chose a folder (an external drive, say): filling the computer's own disk instead without a word
    // isn't what they asked for
    if let Some(p) = &settings.unavailable_backups_path {
        bail!("The backups folder {p} isn't available. Connect its drive, or choose another folder in Settings.");
    }
    let folder = backups_dir(dirs, settings);
    std::fs::create_dir_all(&folder).with_context(|| format!("creating the backups folder {}", folder.display()))?;
    let target = free_path(&folder, &backup_prefix(instance_name));
    pack(instance_root, &target, |name| mode.includes(name))?;
    crate::logs::info(format!("Backup of {instance_name} ({}) saved to {}", mode.key(), target.display()));
    for old in prune(dirs, settings, instance_name, &target) {
        crate::logs::info(format!("Removed the old backup {}", old.display()));
    }
    Ok(target)
}

/// Remove the oldest backups of an instance beyond the `backupsToKeep` setting (nothing when it is 0). Only that
/// instance's own backups (named `<prefix>-<date>.zip` in the backups folder) are ever touched, and never `keep_this`,
/// the one just made. Returns what was removed.
pub fn prune(dirs: &Dirs, settings: &ServiceSettings, instance_name: &str, keep_this: &Path) -> Vec<PathBuf> {
    let keep = settings.backups_to_keep as usize;
    if keep == 0 {
        return Vec::new();
    }
    // the names of backups only tell instances apart as far as their letters and digits do: when another instance
    // gives the same start ("My Pack" and "My-Pack"), or it is the catch-all "Instance" of names in other alphabets,
    // some of these files may be another instance's, and nothing is deleted
    let prefix = backup_prefix(instance_name);
    let owners = crate::instances::load_all(dirs).iter().filter(|i| backup_prefix(i.name()) == prefix).count();
    if prefix == "Instance" || owners > 1 {
        crate::logs::warn(format!(
            "Not removing old backups of {instance_name}: their names can't be told apart from another instance's"
        ));
        return Vec::new();
    }
    let mut removed = Vec::new();
    for old in
        list(dirs, settings, instance_name).into_iter().filter(|b| b.path != keep_this).skip(keep.saturating_sub(1))
    {
        match std::fs::remove_file(&old.path) {
            Ok(()) => removed.push(old.path),
            Err(e) => crate::logs::warn(format!("Couldn't remove the old backup {}: {e}", old.path.display())),
        }
    }
    removed
}

/// After an instance was renamed from `old_name` to `new_name`, give its backups the new name, so its page still lists
/// them (backups are found by name). `others` are the names of the other instances: when one of them shares the old
/// or the new start of a backup name, the files may be that instance's and are left as they are. Returns how many
/// were renamed.
pub fn rename_backups(
    dirs: &Dirs,
    settings: &ServiceSettings,
    old_name: &str,
    new_name: &str,
    others: &[String],
) -> usize {
    let (old_p, new_p) = (backup_prefix(old_name), backup_prefix(new_name));
    let shared = |p: &str| p == "Instance" || others.iter().any(|o| backup_prefix(o) == p);
    if old_p == new_p || shared(&old_p) || shared(&new_p) {
        return 0;
    }
    let folder = backups_dir(dirs, settings);
    let mut renamed = 0;
    for b in list_with_prefix(&folder, &old_p) {
        let Some(rest) = b.file_name.strip_prefix(old_p.as_str()) else { continue };
        let to = folder.join(format!("{new_p}{rest}"));
        if to.exists() {
            continue;
        }
        match std::fs::rename(&b.path, &to) {
            Ok(()) => renamed += 1,
            Err(e) => crate::logs::warn(format!("Couldn't rename the backup {}: {e}", b.path.display())),
        }
    }
    renamed
}

/// Make a backup of a server folder; everything in it goes in, like the Java edition does.
pub fn create_server(
    dirs: &Dirs,
    settings: &ServiceSettings,
    server_root: &Path,
    server_name: &str,
) -> Result<PathBuf> {
    let folder = backups_dir(dirs, settings);
    std::fs::create_dir_all(&folder).with_context(|| format!("creating the backups folder {}", folder.display()))?;
    let target = free_path(&folder, &format!("Server-{}", safe_name(server_name)));
    pack(server_root, &target, |_| true)?;
    crate::logs::info(format!("Backup complete and stored at {}", target.display()));
    Ok(target)
}

/// Zip `root` into `target`, keeping the files `keep` says yes to (called with `/`-separated paths relative to
/// `root`; folders end in `/`). Written to a temporary file first so a failed backup never leaves a broken zip.
pub fn pack(root: &Path, target: &Path, keep: impl Fn(&str) -> bool) -> Result<()> {
    if !root.is_dir() {
        bail!("{} is not a folder, so there is nothing to back up", root.display());
    }
    let part = target.with_extension("zip.part");
    let result = (|| -> Result<()> {
        let file = File::create(&part).with_context(|| format!("creating {}", part.display()))?;
        let mut zip = zip::ZipWriter::new(BufWriter::new(file));
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        add_dir(&mut zip, root, "", &keep, options)?;
        // on the disk before it takes the backup's name (and before older backups may be pruned): a power cut must
        // not leave a broken newest backup in place of good ones
        let file = zip.finish()?.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            std::fs::rename(&part, target).with_context(|| format!("saving {}", target.display()))?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

fn add_dir(
    zip: &mut zip::ZipWriter<BufWriter<File>>,
    dir: &Path,
    rel: &str,
    keep: &dyn Fn(&str) -> bool,
    options: SimpleFileOptions,
) -> Result<()> {
    let mut entries: Vec<_> =
        std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        let path = e.path();
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.is_dir() {
            let entry_name = format!("{rel}{name}/");
            // every rule here is by the start of the path, so nothing inside a folder that isn't kept is kept
            if keep(&entry_name) {
                zip.add_directory(entry_name.clone(), options)?;
                add_dir(zip, &path, &entry_name, keep, options)?;
            }
        } else if meta.is_file() {
            let entry_name = format!("{rel}{name}");
            if !keep(&entry_name) {
                continue;
            }
            let mut f = match File::open(&path) {
                Ok(f) => f,
                Err(err) => {
                    // a file the game has locked: skip it, as the Java edition does
                    crate::logs::warn(format!("Unable to add {} to the backup: {err}", path.display()));
                    continue;
                }
            };
            zip.start_file(entry_name, options)?;
            std::io::copy(&mut f, zip)?;
        }
    }
    Ok(())
}

/// The names of every entry in a backup, with `/` separators (folders end in `/`).
pub fn entry_names(zip_path: &Path) -> BTreeSet<String> {
    let Ok(file) = File::open(zip_path) else { return BTreeSet::new() };
    let Ok(mut archive) = zip::ZipArchive::new(file) else { return BTreeSet::new() };
    (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().and_then(|e| e.name().map(|n| n.to_string()).ok()))
        .collect()
}

/// The first part of every file name in the backup (`saves`, `options.txt`, ...).
pub fn top_level_entries(zip_path: &Path) -> Result<BTreeSet<String>> {
    let mut archive =
        zip::ZipArchive::new(File::open(zip_path).with_context(|| format!("opening {}", zip_path.display()))?)
            .context("that backup is not a readable zip file")?;
    let mut out = BTreeSet::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        if let Some(path) = entry.enclosed_name() {
            if let Some(first) = path.components().next() {
                out.insert(first.as_os_str().to_string_lossy().to_string());
            }
        }
    }
    Ok(out)
}

/// Put a backup back into an instance. Whatever the backup holds at the top level (`saves`, `config`,
/// `options.txt`, ...) replaces what the instance has there; everything else is left alone. With
/// `safety_backup` the parts about to be replaced are saved first as a normal backup of the instance.
pub fn restore(
    dirs: &Dirs,
    settings: &ServiceSettings,
    backup_zip: &Path,
    instance_root: &Path,
    instance_name: &str,
    safety_backup: bool,
) -> Result<()> {
    let tops = top_level_entries(backup_zip)?;
    if tops.is_empty() {
        bail!("that backup is empty");
    }
    if safety_backup && instance_root.is_dir() {
        let tops2 = tops.clone();
        let folder = backups_dir(dirs, settings);
        std::fs::create_dir_all(&folder)?;
        let target = free_path(&folder, &backup_prefix(instance_name));
        pack(instance_root, &target, move |n| tops2.contains(n.split('/').next().unwrap_or("")))?;
        crate::logs::info(format!("Saved what is about to be replaced as {}", target.display()));
    }
    // unpack next to the instance first, so a damaged zip never leaves the instance half-restored
    // a folder of its own, made here: two restores at once (of instances whose names look alike) never share one,
    // and nothing already there is ever removed
    let staging = instance_root.with_file_name(format!(".restore-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&staging).with_context(|| format!("creating {}", staging.display()))?;
    let unpacked = (|| -> Result<()> {
        let mut archive = zip::ZipArchive::new(File::open(backup_zip)?)?;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            // entries with `..` or an absolute path are skipped
            let Some(rel) = entry.enclosed_name() else { continue };
            let out = staging.join(&rel);
            if entry.is_dir() {
                std::fs::create_dir_all(&out)?;
            } else {
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut f = File::create(&out)?;
                std::io::copy(&mut entry, &mut f)?;
            }
        }
        Ok(())
    })();
    if let Err(e) = unpacked {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e.context("the backup could not be unpacked; the instance was not changed"));
    }
    std::fs::create_dir_all(instance_root)?;
    // what is replaced is moved aside first and only deleted once everything new is in place; when a step fails
    // (a file in use, for example) what was moved aside goes back, so the instance is never left with neither
    // named uniquely: a name built from the instance's name and the second would be the same for two restores of
    // look-alike instances at once, and the second one would fail
    let aside = instance_root.with_file_name(format!(".restore-old-{}", uuid::Uuid::new_v4().simple()));
    if let Err(e) = std::fs::create_dir(&aside) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(anyhow::Error::from(e).context("the instance was not changed"));
    }
    let mut done: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let swapped = (|| -> Result<()> {
        for top in &tops {
            let from = staging.join(top);
            if !from.exists() {
                continue;
            }
            let to = instance_root.join(top);
            let old = if std::fs::symlink_metadata(&to).is_ok() {
                let old = aside.join(top);
                std::fs::rename(&to, &old).with_context(|| format!("replacing {}", to.display()))?;
                Some(old)
            } else {
                None
            };
            if let Err(e) = std::fs::rename(&from, &to) {
                if let Some(old) = &old {
                    let _ = std::fs::rename(old, &to);
                }
                return Err(anyhow::Error::from(e).context(format!("restoring {}", to.display())));
            }
            done.push((to, old));
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&staging);
    if let Err(e) = swapped {
        // put back what was already swapped, newest first
        let mut all_back = true;
        for (to, old) in done.iter().rev() {
            if let Some(old) = old {
                all_back &= remove_any(to).and_then(|_| std::fs::rename(old, to)).is_ok();
            } else {
                let _ = remove_any(to);
            }
        }
        if !all_back {
            // never delete what couldn't be put back: say where it is
            return Err(e.context(format!(
                "the backup could not be put back, and some of the old files are still in {}",
                aside.display()
            )));
        }
        let _ = std::fs::remove_dir_all(&aside);
        return Err(e.context("the backup could not be put back; the instance was left as it was"));
    }
    // everything replaced is in the safety backup (when one was asked for), so the old copies can go now
    let _ = std::fs::remove_dir_all(&aside);
    crate::logs::info(format!("Restored {} into {}", backup_zip.display(), instance_root.display()));
    Ok(())
}

/// Removes a file or a whole folder.
fn remove_any(path: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path)?.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }
}

/// Delete one backup file. Only `.zip` files inside the backups folder are ever removed.
pub fn delete(dirs: &Dirs, settings: &ServiceSettings, backup_zip: &Path) -> Result<()> {
    let folder = backups_dir(dirs, settings);
    let inside = backup_zip.parent().and_then(|p| p.canonicalize().ok()) == folder.canonicalize().ok();
    if !inside || backup_zip.extension().and_then(|e| e.to_str()) != Some("zip") {
        bail!("{} is not in the backups folder", backup_zip.display());
    }
    std::fs::remove_file(backup_zip).with_context(|| format!("deleting {}", backup_zip.display()))
}

/// After a game closes: make a backup if "automatic backup after launch" is on, in the chosen mode.
pub fn after_launch(
    dirs: &Dirs,
    settings: &ServiceSettings,
    instance_root: &Path,
    instance_name: &str,
) -> Option<Result<PathBuf>> {
    if !settings.enable_automatic_backup_after_launch {
        return None;
    }
    Some(create(dirs, settings, instance_root, instance_name, settings.backup_mode))
}

/// The date of a file, for backups whose name doesn't carry one.
pub fn file_time(path: &Path) -> Option<DateTime<Local>> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().map(|t: SystemTime| t.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("blockforge-backups-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn sample_instance(root: &Path) {
        for (path, body) in [
            ("options.txt", "fov:70"),
            ("saves/World1/level.dat", "world"),
            ("config/sodium.json", "{}"),
            ("mods/sodium.jar", "jar"),
            ("logs/latest.log", "log"),
            ("instance.json", "{}"),
            ("coremods/x.jar", "core"),
        ] {
            let p = root.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        std::fs::create_dir_all(root.join("saves/Empty")).unwrap();
    }

    fn names(zip_path: &Path) -> BTreeSet<String> {
        let mut a = zip::ZipArchive::new(File::open(zip_path).unwrap()).unwrap();
        (0..a.len()).map(|i| a.by_index(i).unwrap().name().unwrap().to_string()).collect()
    }

    #[test]
    fn modes_pick_the_java_editions_files() {
        assert!(BackupMode::Normal.includes("options.txt"));
        assert!(BackupMode::Normal.includes("OPTIONS.TXT"));
        assert!(BackupMode::Normal.includes("saves/World/level.dat"));
        assert!(BackupMode::Normal.includes("config-old/x"));
        assert!(!BackupMode::Normal.includes("mods/a.jar"));
        assert!(!BackupMode::Normal.includes("instance.json"));
        assert!(BackupMode::NormalPlusMods.includes("mods/a.jar"));
        assert!(BackupMode::NormalPlusMods.includes("jarmods/a.jar"));
        assert!(BackupMode::NormalPlusMods.includes("coremods/a.jar"));
        assert!(!BackupMode::NormalPlusMods.includes("logs/latest.log"));
        assert!(BackupMode::Full.includes("anything/at/all"));
    }

    #[test]
    fn mode_names_match_the_settings_file() {
        assert_eq!(serde_json::to_string(&BackupMode::NormalPlusMods).unwrap(), "\"NORMAL_PLUS_MODS\"");
        assert_eq!(serde_json::from_str::<BackupMode>("\"FULL\"").unwrap(), BackupMode::Full);
        assert_eq!(BackupMode::from_key("normal"), Some(BackupMode::Normal));
        assert_eq!(BackupMode::from_key("nonsense"), None);
    }

    #[test]
    fn names_round_trip() {
        assert_eq!(safe_name("My Pack: 1.21!"), "MyPack121");
        let (name, when) = parse_name("MyPack121-2026_10_16_12_34_56.zip").unwrap();
        assert_eq!(name, "MyPack121");
        assert_eq!(when.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-10-16 12:34:56");
        assert!(parse_name("notabackup.zip").is_none());
        assert!(parse_name("a-2026_10_16_12_34_56.txt").is_none());
    }

    #[test]
    fn create_list_restore_delete() {
        let base = scratch("full");
        let dirs = Dirs::at(base.join("data"));
        let settings = ServiceSettings::default();
        let root = base.join("inst");
        sample_instance(&root);

        let normal = create(&dirs, &settings, &root, "My Pack", BackupMode::Normal).unwrap();
        let n = names(&normal);
        assert!(n.contains("options.txt") && n.contains("saves/World1/level.dat") && n.contains("config/sodium.json"));
        assert!(n.contains("saves/Empty/"), "empty folders are kept: {n:?}");
        assert!(!n.contains("mods/sodium.jar") && !n.contains("instance.json"));

        let full = create(&dirs, &settings, &root, "My Pack", BackupMode::Full).unwrap();
        assert_ne!(normal, full, "two backups in the same second get different names");
        assert!(names(&full).contains("instance.json"));

        let plus = create(&dirs, &settings, &root, "My Pack", BackupMode::NormalPlusMods).unwrap();
        assert!(names(&plus).contains("mods/sodium.jar") && names(&plus).contains("coremods/x.jar"));

        let listed = list(&dirs, &settings, "My Pack");
        assert_eq!(listed.len(), 3);
        assert!(list(&dirs, &settings, "Other").is_empty());

        // ruin the world, then restore the normal backup over it
        std::fs::write(root.join("saves/World1/level.dat"), "ruined").unwrap();
        std::fs::write(root.join("saves/New.txt"), "made later").unwrap();
        std::fs::write(root.join("mods/sodium.jar"), "newer mod").unwrap();
        restore(&dirs, &settings, &normal, &root, "My Pack", true).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("saves/World1/level.dat")).unwrap(), "world");
        assert!(!root.join("saves/New.txt").exists(), "saves/ is replaced as a whole");
        assert_eq!(
            std::fs::read_to_string(root.join("mods/sodium.jar")).unwrap(),
            "newer mod",
            "mods are not in a NORMAL backup, so they stay"
        );
        assert_eq!(std::fs::read_to_string(root.join("options.txt")).unwrap(), "fov:70");
        // the safety backup holds what was replaced
        assert_eq!(list(&dirs, &settings, "My Pack").len(), 4);

        let victim = listed[0].path.clone();
        delete(&dirs, &settings, &victim).unwrap();
        assert!(!victim.exists());
        assert!(delete(&dirs, &settings, &root.join("options.txt")).is_err(), "only backups can be deleted");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The folders a restore works in, next to the instance.
    fn leftovers(root: &Path) -> Vec<String> {
        std::fs::read_dir(root.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(".restore"))
            .collect()
    }

    #[test]
    fn renaming_an_instance_takes_its_backups_along_unless_they_could_be_another_instances() {
        let base = scratch("rename-backups");
        let dirs = Dirs::at(base.join("data"));
        let settings = ServiceSettings::default();
        let root = base.join("inst");
        sample_instance(&root);
        let first = create(&dirs, &settings, &root, "Old Pack", BackupMode::Normal).unwrap();
        assert_eq!(rename_backups(&dirs, &settings, "Old Pack", "New Pack", &["Other".into()]), 1);
        assert!(!first.exists());
        assert_eq!(list(&dirs, &settings, "New Pack").len(), 1, "the renamed instance lists its backup");
        assert!(list(&dirs, &settings, "Old Pack").is_empty());
        // another instance would be called "NewPack" by its backups too: they stay where they are
        assert_eq!(rename_backups(&dirs, &settings, "New Pack", "Third", &["NewPack".into()]), 0);
        assert_eq!(list(&dirs, &settings, "New Pack").len(), 1);
        // a new name that only differs in what backup names leave out changes nothing
        assert_eq!(rename_backups(&dirs, &settings, "New Pack", "New-Pack!", &[]), 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_restore_leaves_nothing_behind() {
        let base = scratch("tidy");
        let dirs = Dirs::at(base.join("data"));
        let settings = ServiceSettings::default();
        let root = base.join("inst");
        sample_instance(&root);
        let normal = create(&dirs, &settings, &root, "Pack", BackupMode::Normal).unwrap();
        restore(&dirs, &settings, &normal, &root, "Pack", true).unwrap();
        assert!(leftovers(&root).is_empty(), "{:?}", leftovers(&root));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A part that can't be replaced (here a folder that may not be moved) puts back what was already replaced.
    #[cfg(unix)]
    #[test]
    fn a_failed_restore_puts_everything_back() {
        use std::os::unix::fs::PermissionsExt;
        let base = scratch("rollback");
        let dirs = Dirs::at(base.join("data"));
        let settings = ServiceSettings::default();
        let root = base.join("inst");
        sample_instance(&root);
        let normal = create(&dirs, &settings, &root, "Pack", BackupMode::Normal).unwrap();
        std::fs::write(root.join("options.txt"), "fov:90").unwrap();
        std::fs::write(root.join("config/sodium.json"), "{\"changed\":1}").unwrap();
        // a folder without write permission can't be moved to another folder (unless this runs as root)
        let saves = root.join("saves");
        std::fs::set_permissions(&saves, std::fs::Permissions::from_mode(0o555)).unwrap();
        let probe = base.join("probe");
        std::fs::create_dir_all(&probe).unwrap();
        let root_user = std::fs::rename(&saves, probe.join("saves")).is_ok();
        if root_user {
            std::fs::rename(probe.join("saves"), &saves).unwrap();
        } else {
            let err = restore(&dirs, &settings, &normal, &root, "Pack", true).unwrap_err();
            assert!(format!("{err:#}").contains("left as it was"), "{err:#}");
            assert_eq!(std::fs::read_to_string(root.join("options.txt")).unwrap(), "fov:90");
            assert_eq!(std::fs::read_to_string(root.join("config/sodium.json")).unwrap(), "{\"changed\":1}");
            assert_eq!(std::fs::read_to_string(saves.join("World1/level.dat")).unwrap(), "world");
            assert!(leftovers(&root).is_empty(), "{:?}", leftovers(&root));
        }
        std::fs::set_permissions(&saves, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_broken_zip_leaves_the_instance_alone() {
        let base = scratch("broken");
        let dirs = Dirs::at(base.join("data"));
        let settings = ServiceSettings::default();
        let root = base.join("inst");
        sample_instance(&root);
        let bad = base.join("bad-2026_10_16_12_00_00.zip");
        std::fs::write(&bad, b"not a zip").unwrap();
        assert!(restore(&dirs, &settings, &bad, &root, "Pack", false).is_err());
        assert_eq!(std::fs::read_to_string(root.join("options.txt")).unwrap(), "fov:70");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn automatic_backup_follows_the_setting() {
        let base = scratch("auto");
        let dirs = Dirs::at(base.join("data"));
        let root = base.join("inst");
        sample_instance(&root);
        let mut s = ServiceSettings::default();
        assert!(after_launch(&dirs, &s, &root, "Pack").is_none());
        s.enable_automatic_backup_after_launch = true;
        s.backup_mode = BackupMode::Full;
        let made = after_launch(&dirs, &s, &root, "Pack").unwrap().unwrap();
        assert!(names(&made).contains("instance.json"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_custom_backups_folder_is_used_when_it_exists() {
        let base = scratch("custom");
        let dirs = Dirs::at(base.join("data"));
        let mut s = ServiceSettings::default();
        s.backups_path = Some(base.join("missing").to_string_lossy().to_string());
        assert_eq!(backups_dir(&dirs, &s), dirs.base.join("backups"));
        std::fs::create_dir_all(base.join("mine")).unwrap();
        s.backups_path = Some(base.join("mine").to_string_lossy().to_string());
        assert_eq!(backups_dir(&dirs, &s), base.join("mine"));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Back up a real instance of the Java edition (only read) and restore it into a scratch folder.
    #[test]
    fn a_real_instance_makes_the_same_backup_the_java_edition_would() {
        let real = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/blockforge/instances"));
        let Some(real) = real.filter(|p| p.is_dir()) else { return };
        let Some(root) = std::fs::read_dir(&real)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.join("instance.json").exists() && p.join("saves").is_dir())
        else {
            return;
        };
        let base = scratch("real");
        let dirs = Dirs::at(base.join("data"));
        let settings = ServiceSettings::default();
        let name = root.file_name().unwrap().to_string_lossy().to_string();

        let zip = create(&dirs, &settings, &root, &name, BackupMode::Normal).unwrap();
        let entries = entry_names(&zip);
        // every file the Java edition's NORMAL rule keeps is in the zip, and nothing else
        let mut expected = BTreeSet::new();
        fn walk(dir: &Path, rel: &str, out: &mut BTreeSet<String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let n = format!("{rel}{}", e.file_name().to_string_lossy());
                let kept = BackupMode::Normal.includes(&n);
                if e.path().is_dir() {
                    if kept {
                        out.insert(format!("{n}/"));
                        walk(&e.path(), &format!("{n}/"), out);
                    }
                } else if kept {
                    out.insert(n);
                }
            }
        }
        walk(&root, "", &mut expected);
        assert_eq!(entries, expected);
        println!("{} entries from {}", entries.len(), root.display());

        let copy = base.join("copy");
        std::fs::create_dir_all(&copy).unwrap();
        restore(&dirs, &settings, &zip, &copy, &name, false).unwrap();
        let first_save = std::fs::read_dir(root.join("saves")).unwrap().flatten().next();
        if let Some(save) = first_save {
            assert!(copy.join("saves").join(save.file_name()).is_dir());
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}

/// What real backups folders hold: files with other people's names, backups of instances named in any language,
/// two restores at once.
#[cfg(test)]
mod robustness_tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("blockforge-backups-rb-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn instance(root: &Path, world: &str) {
        std::fs::create_dir_all(root.join("saves/W")).unwrap();
        std::fs::write(root.join("saves/W/level.dat"), world).unwrap();
        std::fs::write(root.join("options.txt"), "fov:70").unwrap();
    }

    #[test]
    fn odd_file_names_in_the_backups_folder_never_crash_the_list() {
        let base = scratch("names");
        let dirs = Dirs::at(base.join("data"));
        let folder = dirs.base.join("backups");
        std::fs::create_dir_all(&folder).unwrap();
        // names whose 19th byte from the end falls inside a letter used to crash the instance page
        for name in [
            "Мой мир 2024-10-01.zip",
            "世界の保存データ.zip",
            "🎮🎮🎮🎮🎮🎮🎮🎮.zip",
            "é-2026_10_16_12_34_56.zip",
            ".zip",
            "-.zip",
            "Pack-2026_99_99_99_99_99.zip",
            "Pack-not_a_date_at_all_.zip",
        ] {
            std::fs::write(folder.join(name), b"x").unwrap();
            let _ = parse_name(name);
        }
        assert!(list(&dirs, &ServiceSettings::default(), "Pack").is_empty());
        assert_eq!(
            parse_name("Моймир-2026_10_16_12_34_56.zip").map(|(n, _)| n),
            Some("Моймир".to_string()),
            "a name outside ASCII before a real stamp still reads"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn backups_of_instances_named_outside_ascii_are_listed_and_kept_apart() {
        let base = scratch("unicode");
        let dirs = Dirs::at(base.join("data"));
        let settings = ServiceSettings::default();
        let (a, b) = (base.join("a"), base.join("b"));
        instance(&a, "russian");
        instance(&b, "japanese");
        create(&dirs, &settings, &a, "Мой мир", BackupMode::Normal).unwrap();
        create(&dirs, &settings, &b, "世界", BackupMode::Normal).unwrap();
        // each sees its own backup, and only its own
        let la = list(&dirs, &settings, "Мой мир");
        let lb = list(&dirs, &settings, "世界");
        assert_eq!(la.len(), 1, "a backup that was made must be listed");
        assert_eq!(lb.len(), 1);
        assert_ne!(la[0].path, lb[0].path);
        // an instance literally called "Instance" doesn't collect every other one's backups
        assert!(list(&dirs, &settings, "Instance").is_empty());
        // ASCII names keep the Java edition's names, so both editions share backups
        assert_eq!(backup_prefix("My Pack!"), "MyPack");
        assert_eq!(backup_prefix("***"), "Instance");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn two_restores_at_once_never_mix_their_files() {
        let base = scratch("parallel");
        let dirs = Dirs::at(base.join("data"));
        let settings = ServiceSettings::default();
        let roots: Vec<PathBuf> = (0..6).map(|i| base.join("instances").join(format!("i{i}"))).collect();
        let zips: Vec<PathBuf> = roots
            .iter()
            .enumerate()
            .map(|(i, r)| {
                instance(r, &format!("world {i}"));
                // all of these names have no ASCII letters, which used to give one shared staging folder
                create(&dirs, &settings, r, &format!("мир{}", "й".repeat(i + 1)), BackupMode::Normal).unwrap()
            })
            .collect();
        for r in &roots {
            std::fs::write(r.join("saves/W/level.dat"), "ruined").unwrap();
        }
        std::thread::scope(|s| {
            for (i, (r, z)) in roots.iter().zip(&zips).enumerate() {
                let dirs = &dirs;
                let settings = &settings;
                s.spawn(move || restore(dirs, settings, z, r, &format!("мир{}", "й".repeat(i + 1)), false).unwrap());
            }
        });
        for (i, r) in roots.iter().enumerate() {
            assert_eq!(std::fs::read_to_string(r.join("saves/W/level.dat")).unwrap(), format!("world {i}"));
        }
        let left: Vec<_> = std::fs::read_dir(base.join("instances"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| e.file_name())
            .collect();
        assert!(left.is_empty(), "{left:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_zip_with_escaping_entries_restores_only_what_is_inside() {
        let base = scratch("evil");
        let dirs = Dirs::at(base.join("data"));
        let settings = ServiceSettings::default();
        let root = base.join("inst");
        instance(&root, "mine");
        let evil = base.join("evil.zip");
        {
            let mut z = zip::ZipWriter::new(File::create(&evil).unwrap());
            let o = SimpleFileOptions::default();
            z.start_file("../outside.txt", o).unwrap();
            z.write_all(b"escaped").unwrap();
            z.start_file("/abs.txt", o).unwrap();
            z.write_all(b"escaped").unwrap();
            z.start_file("saves/W/level.dat", o).unwrap();
            z.write_all(b"from zip").unwrap();
            z.finish().unwrap();
        }
        let _ = restore(&dirs, &settings, &evil, &root, "Inst", false);
        assert!(!base.join("outside.txt").exists());
        assert!(!Path::new("/abs.txt").exists() || std::fs::read(Path::new("/abs.txt")).unwrap() != b"escaped");
        assert_eq!(std::fs::read_to_string(root.join("saves/W/level.dat")).unwrap(), "from zip");
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod retention_tests {
    use super::*;

    #[test]
    fn only_the_newest_backups_of_that_instance_are_kept() {
        let base = crate::testutil::scratch("backup-retention");
        let dirs = Dirs::at(base.join("data"));
        let mut settings = ServiceSettings::default();
        let folder = backups_dir(&dirs, &settings);
        std::fs::create_dir_all(&folder).unwrap();
        // older backups of this instance and of another one, and a file that isn't a backup
        for day in 10..16 {
            std::fs::write(folder.join(format!("MyPack-2026_01_{day}_12_00_00.zip")), b"old").unwrap();
            std::fs::write(folder.join(format!("Other-2026_01_{day}_12_00_00.zip")), b"other").unwrap();
        }
        std::fs::write(folder.join("notes.zip"), b"mine").unwrap();
        let root = base.join("inst");
        std::fs::create_dir_all(root.join("saves/W")).unwrap();
        std::fs::write(root.join("saves/W/level.dat"), b"w").unwrap();

        // 0 keeps everything, as before
        create(&dirs, &settings, &root, "My Pack", BackupMode::Normal).unwrap();
        assert_eq!(list(&dirs, &settings, "My Pack").len(), 7);

        settings.backups_to_keep = 3;
        let newest = create(&dirs, &settings, &root, "My Pack", BackupMode::Normal).unwrap();
        let kept = list(&dirs, &settings, "My Pack");
        assert_eq!(kept.len(), 3, "{:?}", kept.iter().map(|b| &b.file_name).collect::<Vec<_>>());
        assert!(kept.iter().any(|b| b.path == newest), "the one just made is always kept");
        assert_eq!(list(&dirs, &settings, "Other").len(), 6, "another instance's backups are never touched");
        assert!(folder.join("notes.zip").exists());
        // keeping 1 keeps exactly the new one
        settings.backups_to_keep = 1;
        let last = create(&dirs, &settings, &root, "My Pack", BackupMode::Normal).unwrap();
        let kept = list(&dirs, &settings, "My Pack");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].path, last);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_setting_reads_and_writes_safely() {
        let mut st: crate::config::Settings = serde_json::from_str(r#"{"backupsToKeep": -4}"#).unwrap();
        assert_eq!(ServiceSettings::from_settings(&st).backups_to_keep, 0);
        st = serde_json::from_str(r#"{"backupsToKeep": 99999999}"#).unwrap();
        assert_eq!(ServiceSettings::from_settings(&st).backups_to_keep, crate::svcsettings::MAX_BACKUPS_TO_KEEP);
        let mut s = ServiceSettings::default();
        s.backups_to_keep = 5;
        s.write_to(&mut st);
        assert_eq!(serde_json::to_value(&st).unwrap()["backupsToKeep"], 5);
        s.backups_to_keep = 0;
        s.write_to(&mut st);
        assert!(serde_json::to_value(&st).unwrap().get("backupsToKeep").is_none());
    }
}

#[cfg(test)]
mod prune_safety_tests {
    use super::*;

    fn instance(dirs: &Dirs, folder: &str, name: &str) -> PathBuf {
        let root = dirs.instances.join(folder);
        std::fs::create_dir_all(root.join("saves/W")).unwrap();
        std::fs::write(root.join("saves/W/level.dat"), b"w").unwrap();
        std::fs::write(root.join("instance.json"), serde_json::json!({"launcher": {"name": name}}).to_string())
            .unwrap();
        root
    }

    #[test]
    fn pruning_never_touches_backups_that_could_be_another_instances() {
        let base = crate::testutil::scratch("prune-safety");
        let dirs = Dirs::at(base.join("data"));
        let mut settings = ServiceSettings::default();
        settings.backups_to_keep = 1;
        let folder = backups_dir(&dirs, &settings);
        std::fs::create_dir_all(&folder).unwrap();
        // "My Pack" and "My-Pack" both name their backups MyPack-<date>.zip
        let a = instance(&dirs, "MyPack", "My Pack");
        instance(&dirs, "MyPack2", "My-Pack");
        for day in 10..14 {
            std::fs::write(folder.join(format!("MyPack-2026_01_{day}_12_00_00.zip")), b"whose?").unwrap();
        }
        create(&dirs, &settings, &a, "My Pack", BackupMode::Normal).unwrap();
        assert_eq!(list(&dirs, &settings, "My Pack").len(), 5, "nothing deleted while the names are shared");
        // the catch-all name of instances in other alphabets is never pruned either
        let r = instance(&dirs, "Instance", "Instance");
        std::fs::write(folder.join("Instance-2026_01_10_12_00_00.zip"), b"someone else's").unwrap();
        create(&dirs, &settings, &r, "Instance", BackupMode::Normal).unwrap();
        assert!(folder.join("Instance-2026_01_10_12_00_00.zip").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
