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
use std::io::{BufWriter, Write};
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
    let (prefix, stamp) = stem.split_at(stem.len() - 19);
    let name = prefix.strip_suffix('-')?;
    let naive = NaiveDateTime::parse_from_str(stamp, "%Y_%m_%d_%H_%M_%S").ok()?;
    Some((name.to_string(), Local.from_local_datetime(&naive).earliest()?))
}

/// The backups made for the instance called `instance_name`, newest first.
pub fn list(dirs: &Dirs, settings: &ServiceSettings, instance_name: &str) -> Vec<Backup> {
    list_with_prefix(&backups_dir(dirs, settings), &safe_name(instance_name))
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
    let folder = backups_dir(dirs, settings);
    std::fs::create_dir_all(&folder).with_context(|| format!("creating the backups folder {}", folder.display()))?;
    let prefix = {
        let s = safe_name(instance_name);
        if s.is_empty() { "Instance".to_string() } else { s }
    };
    let target = free_path(&folder, &prefix);
    pack(instance_root, &target, |name| mode.includes(name))?;
    crate::logs::info(format!("Backup of {instance_name} ({}) saved to {}", mode.key(), target.display()));
    Ok(target)
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
        zip.finish()?.flush()?;
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
        let target = free_path(&folder, &{
            let s = safe_name(instance_name);
            if s.is_empty() { "Instance".to_string() } else { s }
        });
        pack(instance_root, &target, move |n| tops2.contains(n.split('/').next().unwrap_or("")))?;
        crate::logs::info(format!("Saved what is about to be replaced as {}", target.display()));
    }
    // unpack next to the instance first, so a damaged zip never leaves the instance half-restored
    let staging = instance_root.with_file_name(format!(".restore-{}-{}", safe_name(instance_name), std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
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
    let aside = instance_root.with_file_name(format!(".restore-old-{}-{}", safe_name(instance_name), timestamp_now()));
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
