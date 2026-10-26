//! What interrupted work leaves behind (the launcher closed, the computer lost power): half-written backups and
//! snapshots, temporary copies of settings files, folders a restore, an import or an unpacked Java was being put
//! together in. They are swept when the launcher starts, once they are old enough that no other running launcher
//! can still be using them.
//!
//! Two kinds of folder are never removed, because they can hold the only copy of a player's files: what a restore
//! moved aside (`.restore-old-*`) and what a snapshot restore parked (`.parked-*`). They are named in the log instead.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::paths::Dirs;

/// How old something must be before it is taken for a leftover.
pub const MIN_AGE: Duration = Duration::from_secs(3600);

/// What a sweep did.
#[derive(Debug, Default)]
pub struct Swept {
    pub removed: Vec<PathBuf>,
    /// Folders that may hold a player's files and were left alone.
    pub kept: Vec<PathBuf>,
}

/// Sweep leftovers older than `min_age` from the data folder and from `backups` (the backups folder in use).
pub fn sweep(dirs: &Dirs, backups: &Path, min_age: Duration) -> Swept {
    let mut swept = Swept::default();
    let old = |p: &Path| {
        std::fs::symlink_metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= min_age)
    };

    // work folders next to the instances, and temporary files inside each instance
    for entry in read_dir(&dirs.instances) {
        let name = file_name(&entry);
        if name.starts_with(".restore-old-") || name.starts_with(".parked-") {
            swept.kept.push(entry);
            continue;
        }
        if (name.starts_with(".restore-") || name.starts_with(".import-")) && old(&entry) {
            remove(&entry, &mut swept);
            continue;
        }
        if !name.starts_with('.') && entry.is_dir() {
            for inner in read_dir(&entry) {
                if is_temporary_settings_file(&file_name(&inner)) && old(&inner) {
                    remove(&inner, &mut swept);
                }
            }
        }
    }
    // temporary copies of the launcher's own settings files
    for entry in read_dir(&dirs.configs) {
        if is_temporary_settings_file(&file_name(&entry)) && old(&entry) {
            remove(&entry, &mut swept);
        }
    }
    // backups that were being written: only ones named the way BlockForge names them, since the backups folder can
    // be any folder (the downloads folder, where a browser keeps its own `.zip.part` files)
    for entry in read_dir(backups) {
        let name = file_name(&entry);
        let ours = name.strip_suffix(".part").is_some_and(|zip| crate::backups::parse_name(zip).is_some());
        if ours && old(&entry) {
            remove(&entry, &mut swept);
        }
    }
    // snapshots that were being written (`snapshots/<key>/` and `snapshots/by-folder/<folder>/`)
    let snapshots = dirs.base.join("snapshots");
    let snapshot_folders = read_dir(&snapshots).into_iter().chain(read_dir(&snapshots.join("by-folder")));
    for folder in snapshot_folders {
        for entry in read_dir(&folder) {
            if file_name(&entry).ends_with(".zip.part") && old(&entry) {
                remove(&entry, &mut swept);
            }
        }
    }
    // a Java that was being unpacked
    if let Some(runtimes_parent) = dirs.runtimes.parent() {
        for entry in read_dir(runtimes_parent) {
            if file_name(&entry).starts_with(".unpacking-") && old(&entry) {
                remove(&entry, &mut swept);
            }
        }
    }
    for kept in &swept.kept {
        crate::logs::warn(format!(
            "{} holds files from a restore that didn't finish; look in it before deleting it",
            kept.display()
        ));
    }
    swept
}

/// `.instance.json.1234-0.tmp` (what `config::write_atomic` writes first) or `instance.json.tmp` (what older versions
/// wrote).
fn is_temporary_settings_file(name: &str) -> bool {
    (name.starts_with('.') && name.ends_with(".tmp") && name.contains(".json.")) || name.ends_with(".json.tmp")
}

fn read_dir(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// Removes a file or a folder; links are removed themselves, never followed.
fn remove(p: &Path, swept: &mut Swept) {
    let is_dir = std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
    let done = if is_dir { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
    if done.is_ok() {
        swept.removed.push(p.to_path_buf());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    #[test]
    fn leftovers_go_and_the_players_files_stay() {
        let base = crate::testutil::scratch("leftovers");
        let dirs = Dirs::at(&base);
        dirs.ensure().unwrap();
        let backups = base.join("backups");
        let leftovers = [
            dirs.instances.join(".restore-0123/saves/W/level.dat"),
            dirs.instances.join(".import-4567/mods/a.jar"),
            dirs.instances.join("Pack/.instance.json.999-0.tmp"),
            dirs.instances.join("Pack/instance.json.tmp"),
            dirs.configs.join(".BlockForge.json.999-3.tmp"),
            dirs.configs.join("accounts.json.tmp"),
            backups.join("Pack-2026_01_01_00_00_00.zip.part"),
            base.join("snapshots/Pack/20260101-000000-000-x.zip.part"),
            base.join("snapshots/by-folder/Сборка/20260101-000000-000-x.zip.part"),
            base.join("runtimes/.unpacking-89ab/bin/java"),
        ];
        let precious = [
            dirs.instances.join(".restore-old-cafe/saves/W/level.dat"),
            dirs.instances.join(".parked-beef/mods/a.jar"),
            dirs.instances.join("Pack/instance.json"),
            dirs.instances.join("Pack/saves/W/level.dat"),
            dirs.configs.join("BlockForge.json"),
            dirs.configs.join("accounts.json"),
            backups.join("Pack-2026_01_01_00_00_00.zip"),
            base.join("snapshots/Pack/20260101-000000-000-x.zip"),
            dirs.instances.join("Pack/options.txt"),
            // a browser's unfinished download in a backups folder that is also the downloads folder
            backups.join("holiday photos.zip.part"),
            backups.join("Pack-not-a-date.zip.part"),
        ];
        for p in leftovers.iter().chain(&precious) {
            touch(p);
        }
        // nothing is young enough to be in use... unless the age limit says so
        let none = sweep(&dirs, &backups, Duration::from_secs(3600));
        assert!(none.removed.is_empty(), "files from a minute ago may belong to a launcher that is running");
        let swept = sweep(&dirs, &backups, Duration::ZERO);
        for p in &leftovers {
            let gone = !p.exists() && swept.removed.iter().any(|r| p.starts_with(r));
            assert!(gone, "{} should have been swept", p.display());
        }
        for p in &precious {
            assert!(p.exists(), "{} must never be removed", p.display());
        }
        assert_eq!(swept.kept.len(), 2, "{:?}", swept.kept);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_missing_or_empty_data_folder_is_fine() {
        let base = crate::testutil::scratch("leftovers-empty");
        let dirs = Dirs::at(base.join("nothing-here"));
        let swept = sweep(&dirs, &base.join("no-backups"), Duration::ZERO);
        assert!(swept.removed.is_empty() && swept.kept.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }
}
