//! The backup made before a pack update or reinstall, in the Java edition's format (`snapshots/<instance>/…zip`),
//! so a bad update can be undone from either edition. Only what a pack changes is saved: mods, configs, scripts,
//! `bin` and `instance.json`. Worlds and personal settings are never touched.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use zip::write::SimpleFileOptions;

use crate::instances::Instance;
use crate::paths::Dirs;

/// Top-level entries of an instance that a pack update can change.
pub const MANAGED: [&str; 7] = ["mods", "config", "defaultconfigs", "kubejs", "scripts", "bin", "instance.json"];

/// How many snapshots are kept for one instance; the oldest go first.
pub const KEEP: usize = 8;

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub file: PathBuf,
    pub size: u64,
}

impl Snapshot {
    /// The part of the file name after the date, time and milliseconds.
    pub fn label(&self) -> String {
        let name = self.file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let parts: Vec<&str> = name.splitn(4, '-').collect();
        if parts.len() == 4 { parts[3].to_string() } else { name }
    }
}

/// `<data>/snapshots/<instance folder>`, the same folder the Mods page lists and restores from (see
/// [`crate::snapshots::folder_for`]).
pub fn folder_for(dirs: &Dirs, instance: &Instance) -> PathBuf {
    crate::snapshots::folder_for(dirs, instance)
}

/// Save the instance as it is, ahead of something that changes it. A failure is only returned, never fatal to the
/// caller: not being able to make a snapshot should not stop an update.
pub fn save_before(dirs: &Dirs, instance: &Instance, label: &str) -> Result<Snapshot> {
    create(&instance.root, &folder_for(dirs, instance), label)
}

pub fn create(root: &Path, dir: &Path, label: &str) -> Result<Snapshot> {
    std::fs::create_dir_all(dir)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f").to_string();
    let safe: String =
        label.chars().map(|c| if c.is_ascii_alphanumeric() || "_.-".contains(c) { c } else { '_' }).collect();
    let zip_path = dir.join(format!("{stamp}-{safe}.zip"));
    // written under another name and renamed when whole, so a crash never leaves a broken newest snapshot
    let part = zip_path.with_extension("zip.part");
    let result = (|| -> Result<()> {
        let mut writer = zip::ZipWriter::new(File::create(&part)?);
        // mods are already compressed jars, so a light level is fast and hardly larger
        let opts = SimpleFileOptions::default().compression_level(Some(1));
        for name in MANAGED {
            let entry = root.join(name);
            let Ok(meta) = std::fs::symlink_metadata(&entry) else { continue };
            if meta.file_type().is_symlink() {
                continue;
            }
            add_tree(&mut writer, root, &entry, opts)?;
        }
        writer.finish()?.sync_all()?;
        std::fs::rename(&part, &zip_path)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_file(&zip_path);
        return Err(e).context("saving a snapshot");
    }
    trim(dir);
    let size = std::fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0);
    Ok(Snapshot { file: zip_path, size })
}

fn add_tree(writer: &mut zip::ZipWriter<File>, root: &Path, start: &Path, opts: SimpleFileOptions) -> Result<()> {
    let rel = start.strip_prefix(root)?.to_string_lossy().replace('\\', "/");
    let meta = std::fs::symlink_metadata(start)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_dir() {
        // folders get an entry of their own so an empty mods or config folder comes back on a restore
        writer.add_directory(format!("{rel}/"), opts)?;
        let mut kids: Vec<_> = std::fs::read_dir(start)?.flatten().collect();
        kids.sort_by_key(|e| e.file_name());
        for k in kids {
            add_tree(writer, root, &k.path(), opts)?;
        }
    } else if meta.is_file() {
        writer.start_file(rel, opts)?;
        io::copy(&mut File::open(start)?, writer)?;
    }
    Ok(())
}

/// The snapshots of an instance, newest first (the names start with the time, which is a better order than the
/// modified times copying can change).
pub fn list(dir: &Path) -> Vec<Snapshot> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut all: Vec<Snapshot> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "zip"))
        .map(|e| Snapshot { size: e.metadata().map(|m| m.len()).unwrap_or(0), file: e.path() })
        .collect();
    all.sort_by(|a, b| b.file.file_name().cmp(&a.file.file_name()));
    all
}

fn trim(dir: &Path) {
    for old in list(dir).into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(old.file);
    }
}

/// Put a snapshot back. The current state is saved first so a restore can itself be undone.
pub fn restore(snapshot: &Snapshot, root: &Path, dir: &Path) -> Result<()> {
    let parent = root.parent().context("the instance has no folder to work in")?;
    let staging = parent.join(format!(".restore-{}", uuid::Uuid::new_v4().simple()));
    let outcome = (|| -> Result<()> {
        // unpack first: saving the current state trims the oldest snapshots, which could be this one
        super::fsx::unzip(&snapshot.file, &staging)?;
        let before = create(root, dir, "before-restore")?;
        let swap = (|| -> Result<()> {
            let mut names: Vec<String> = MANAGED.iter().map(|s| s.to_string()).collect();
            for e in std::fs::read_dir(&staging)?.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if !names.contains(&n) {
                    names.push(n);
                }
            }
            for name in names {
                let from = staging.join(&name);
                let current = root.join(&name);
                if std::fs::symlink_metadata(&from).is_err()
                    && std::fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink())
                {
                    continue;
                }
                super::fsx::delete_path(&current);
                if from.exists() {
                    if std::fs::rename(&from, &current).is_err() {
                        super::fsx::copy_dir(&from, &current, false)?;
                    }
                }
            }
            Ok(())
        })();
        if swap.is_err() {
            // the restore failed halfway: bring the instance back as it was
            let _ = restore_quietly(&before, root);
        }
        swap
    })();
    super::fsx::delete_path(&staging);
    outcome
}

fn restore_quietly(snapshot: &Snapshot, root: &Path) -> Result<()> {
    for name in MANAGED {
        super::fsx::delete_path(&root.join(name));
    }
    super::fsx::unzip(&snapshot.file, root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_save_only_what_a_pack_manages_and_restore_undoes_an_update() {
        let base = std::env::temp_dir().join(format!("bf-snap-{}", uuid::Uuid::new_v4().simple()));
        let root = base.join("inst");
        std::fs::create_dir_all(root.join("mods")).unwrap();
        std::fs::create_dir_all(root.join("saves").join("W")).unwrap();
        std::fs::write(root.join("mods").join("a.jar"), "old").unwrap();
        std::fs::write(root.join("saves").join("W").join("level.dat"), "world").unwrap();
        std::fs::write(root.join("instance.json"), "{}").unwrap();
        let dir = base.join("snapshots").join("Inst");
        let snap = create(&root, &dir, "before-update").unwrap();
        assert!(snap.label() == "before-update", "label was {}", snap.label());
        assert!(crate::packs::fsx::zip_contains(&snap.file, "mods/a.jar"));
        assert!(!crate::packs::fsx::zip_contains(&snap.file, "saves/W/level.dat"));
        // an "update" replaces the mod
        std::fs::write(root.join("mods").join("a.jar"), "new").unwrap();
        std::fs::write(root.join("mods").join("b.jar"), "added").unwrap();
        restore(&snap, &root, &dir).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("mods").join("a.jar")).unwrap(), "old");
        assert!(!root.join("mods").join("b.jar").exists());
        assert!(root.join("saves").join("W").join("level.dat").exists(), "worlds are never touched");
        assert!(list(&dir).iter().any(|s| s.label() == "before-restore"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn only_the_newest_snapshots_are_kept() {
        let base = std::env::temp_dir().join(format!("bf-snap2-{}", uuid::Uuid::new_v4().simple()));
        let root = base.join("inst");
        std::fs::create_dir_all(root.join("mods")).unwrap();
        let dir = base.join("s");
        for i in 0..(KEEP + 3) {
            create(&root, &dir, &format!("n{i}")).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        assert_eq!(list(&dir).len(), KEEP);
        let _ = std::fs::remove_dir_all(base);
    }
}
