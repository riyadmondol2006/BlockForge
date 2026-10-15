//! Saves the parts of an instance that a modpack update can change (mods, configs, scripts), and puts them back.
//! An update never touches worlds or personal settings, so they aren't included.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::instances::Instance;
use crate::paths::Dirs;

/// Top level entries that belong to the pack and are changed by updates.
pub const MANAGED: [&str; 7] = ["mods", "config", "defaultconfigs", "kubejs", "scripts", "bin", "instance.json"];

/// How many snapshots are kept for an instance, the oldest go first.
pub const KEEP: usize = 8;

/// One saved state.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub file: PathBuf,
    /// Milliseconds since the epoch the file was written.
    pub created_at: i64,
    pub size: u64,
}

impl Snapshot {
    /// The name given when it was saved ("before-update"): the file name is date-time-millis-label.
    pub fn label(&self) -> String {
        let name = self.file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let name = name.strip_suffix(".zip").unwrap_or(&name).to_string();
        let parts: Vec<&str> = name.splitn(4, '-').collect();
        if parts.len() == 4 { parts[3].to_string() } else { name }
    }
}

fn snapshot_of(file: PathBuf) -> Snapshot {
    let meta = std::fs::metadata(&file).ok();
    let created_at = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Snapshot { size: meta.map(|m| m.len()).unwrap_or(0), created_at, file }
}

/// Where the snapshots of an instance are kept (`snapshots/<name without symbols>` in the data folder).
pub fn folder_for(dirs: &Dirs, instance: &Instance) -> PathBuf {
    let safe: String = instance.name().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    dirs.base.join("snapshots").join(safe)
}

/// Saves the instance as it is, ahead of something that changes it. A failure is only reported through the return
/// value: not being able to make a snapshot shouldn't stop the player from updating.
pub fn save_before(dirs: &Dirs, instance: &Instance, label: &str) -> Result<Snapshot> {
    create(&instance.root, &folder_for(dirs, instance), label)
}

/// Saves the instance's managed files into a new snapshot and trims old ones.
pub fn create(instance_root: &Path, snapshots_dir: &Path, label: &str) -> Result<Snapshot> {
    std::fs::create_dir_all(snapshots_dir)?;
    let safe_label: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') { c } else { '_' })
        .collect();
    // the names start with a timestamp that decides the order, so two snapshots never share one
    let zip_path = loop {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f").to_string();
        let taken =
            std::fs::read_dir(snapshots_dir)?.flatten().any(|e| e.file_name().to_string_lossy().starts_with(&stamp));
        if !taken {
            break snapshots_dir.join(format!("{stamp}-{safe_label}.zip"));
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    };

    let result = (|| -> Result<()> {
        let file = std::fs::File::create(&zip_path)?;
        let mut z = zip::ZipWriter::new(file);
        // mods are already compressed jars, so a quick setting is much faster and hardly larger
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(1));
        let mut top: Vec<PathBuf> = std::fs::read_dir(instance_root)?.flatten().map(|e| e.path()).collect();
        top.sort();
        for entry in top {
            let name = entry.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let is_link = std::fs::symlink_metadata(&entry).map(|m| m.file_type().is_symlink()).unwrap_or(true);
            if !MANAGED.contains(&name.as_str()) || is_link {
                continue;
            }
            add_tree(&mut z, instance_root, &entry, options)?;
        }
        z.finish()?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&zip_path);
        return Err(e);
    }
    trim(snapshots_dir);
    Ok(snapshot_of(zip_path))
}

fn relative_name(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn add_tree(
    z: &mut zip::ZipWriter<std::fs::File>,
    root: &Path,
    start: &Path,
    options: zip::write::SimpleFileOptions,
) -> Result<()> {
    let meta = std::fs::symlink_metadata(start)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_dir() {
        // folders get an entry of their own, so an empty mods or config folder comes back on a restore
        z.add_directory(format!("{}/", relative_name(root, start)), options)?;
        let mut children: Vec<PathBuf> = std::fs::read_dir(start)?.flatten().map(|e| e.path()).collect();
        children.sort();
        for c in children {
            add_tree(z, root, &c, options)?;
        }
    } else if meta.is_file() {
        z.start_file(relative_name(root, start), options)?;
        let mut f = std::fs::File::open(start)?;
        std::io::copy(&mut f, z)?;
    }
    Ok(())
}

/// Snapshots of an instance, newest first.
pub fn list(snapshots_dir: &Path) -> Vec<Snapshot> {
    let Ok(rd) = std::fs::read_dir(snapshots_dir) else { return Vec::new() };
    let mut found: Vec<Snapshot> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "zip"))
        .map(snapshot_of)
        .collect();
    // the names start with a timestamp, which is a better order than modified times that copying can change
    found.sort_by(|a, b| b.file.file_name().cmp(&a.file.file_name()));
    found
}

/// The newest `count` snapshots, convenient for menus.
pub fn latest(snapshots_dir: &Path, count: usize) -> Vec<Snapshot> {
    list(snapshots_dir).into_iter().take(count).collect()
}

fn trim(snapshots_dir: &Path) {
    for s in list(snapshots_dir).into_iter().skip(KEEP) {
        // an old snapshot that can't be removed now will be next time
        let _ = std::fs::remove_file(s.file);
    }
}

fn unique_dir(parent: &Path, prefix: &str) -> Result<PathBuf> {
    let dir = parent.join(format!("{prefix}{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

fn exists_nofollow(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

/// Puts a snapshot back. The current state is saved as a snapshot first, so a restore can itself be undone, and if
/// the restore fails halfway the instance is returned to how it was.
pub fn restore(snapshot: &Snapshot, instance_root: &Path, snapshots_dir: &Path) -> Result<()> {
    let parent = std::path::absolute(instance_root)?
        .parent()
        .map(Path::to_path_buf)
        .context("the instance has no parent folder")?;
    let staging = unique_dir(&parent, ".restore-")?;
    let mut before: Option<Snapshot> = None;

    let result = (|| -> Result<()> {
        // unpack first: saving the current state trims the oldest snapshots, which could be this one
        extract(&snapshot.file, &staging)?;
        before = Some(create(instance_root, snapshots_dir, "before-restore")?);

        // every part a snapshot manages goes back to how it was, so something an update added since (a new kubejs
        // folder, say) is taken away again. Worlds and anything else in the instance are left alone.
        let mut names: BTreeSet<String> = MANAGED.iter().map(|s| s.to_string()).collect();
        for e in std::fs::read_dir(&staging)?.flatten() {
            names.insert(e.file_name().to_string_lossy().to_string());
        }

        let mut replaced: Vec<PathBuf> = Vec::new();
        let parked = unique_dir(&parent, ".parked-")?;
        let moved = (|| -> Result<()> {
            for name in &names {
                let entry = staging.join(name);
                let current = instance_root.join(name);
                let in_snapshot = exists_nofollow(&entry);
                // links are never saved in a snapshot, so one the snapshot knows nothing about is left as it is
                let is_link = std::fs::symlink_metadata(&current).map(|m| m.file_type().is_symlink()).unwrap_or(false);
                if !in_snapshot && is_link {
                    continue;
                }
                if exists_nofollow(&current) {
                    std::fs::rename(&current, parked.join(name))
                        .with_context(|| format!("moving {} out of the way", current.display()))?;
                }
                if in_snapshot {
                    std::fs::rename(&entry, &current).with_context(|| format!("putting {name} back"))?;
                    replaced.push(current);
                }
            }
            Ok(())
        })();
        if let Err(e) = moved {
            // put back what was moved out of the way
            for done in &replaced {
                delete_tree(done);
            }
            if let Ok(held) = std::fs::read_dir(&parked) {
                for entry in held.flatten() {
                    let back = instance_root.join(entry.file_name());
                    delete_tree(&back);
                    let _ = std::fs::rename(entry.path(), back);
                }
            }
            delete_tree(&parked);
            return Err(e);
        }
        delete_tree(&parked);
        Ok(())
    })();

    if result.is_err() {
        if let Some(b) = &before {
            let _ = std::fs::remove_file(&b.file);
        }
    }
    delete_tree(&staging);
    result
}

/// Unpacks a snapshot, refusing any entry that would land outside the destination.
fn extract(zip_path: &Path, destination: &Path) -> Result<()> {
    let root = std::path::absolute(destination)?;
    let mut z = zip::ZipArchive::new(
        std::fs::File::open(zip_path).with_context(|| format!("opening {}", zip_path.display()))?,
    )?;
    for i in 0..z.len() {
        let mut entry = z.by_index(i)?;
        let name = entry.name()?.to_string();
        let relative = Path::new(&name);
        let unsafe_path = relative.is_absolute()
            || relative
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_) | Component::RootDir));
        if unsafe_path {
            bail!("The snapshot contains an unsafe path: {name}");
        }
        let target = root.join(relative);
        if entry.is_dir() || name.ends_with('/') {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut out = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;
        out.flush()?;
    }
    Ok(())
}

/// Best effort clean up that never follows links.
fn delete_tree(path: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return };
    if meta.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(file: &Path, content: &str) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
    }

    fn read(file: &Path) -> String {
        std::fs::read_to_string(file).unwrap()
    }

    /// A fresh folder and an instance inside it.
    fn instance(name: &str) -> (PathBuf, PathBuf) {
        let temp = std::env::temp_dir().join(format!("blockforge-snap-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let root = temp.join("instance");
        write(&root.join("mods/a.jar"), "A1");
        write(&root.join("config/a.toml"), "setting=1");
        write(&root.join("instance.json"), "{\"v\":1}");
        write(&root.join("saves/World/level.dat"), "my world");
        write(&root.join("options.txt"), "fov:90");
        (temp, root)
    }

    #[test]
    fn restoring_puts_back_managed_files_and_leaves_the_rest() {
        let (temp, root) = instance("restore");
        let dir = temp.join("snapshots");
        let snapshot = create(&root, &dir, "before-update").unwrap();

        // an "update" changes, adds and removes things, and the player keeps playing
        write(&root.join("mods/a.jar"), "A2");
        write(&root.join("mods/new.jar"), "NEW");
        write(&root.join("config/a.toml"), "setting=2");
        write(&root.join("saves/World/level.dat"), "my world, played more");
        std::fs::remove_file(root.join("instance.json")).unwrap();

        restore(&snapshot, &root, &dir).unwrap();

        assert_eq!(read(&root.join("mods/a.jar")), "A1");
        assert!(!root.join("mods/new.jar").exists());
        assert_eq!(read(&root.join("config/a.toml")), "setting=1");
        assert_eq!(read(&root.join("instance.json")), "{\"v\":1}");
        assert_eq!(read(&root.join("saves/World/level.dat")), "my world, played more");
        assert_eq!(read(&root.join("options.txt")), "fov:90");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn a_restore_can_itself_be_undone() {
        let (temp, root) = instance("undo");
        let dir = temp.join("snapshots");
        let original = create(&root, &dir, "first").unwrap();
        write(&root.join("mods/a.jar"), "A2");
        restore(&original, &root, &dir).unwrap();

        let all = list(&dir);
        assert!(all[0].label().contains("before-restore"), "{}", all[0].label());
        restore(&all[0], &root, &dir).unwrap();
        assert_eq!(read(&root.join("mods/a.jar")), "A2");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn keeps_only_the_newest_snapshots() {
        let (temp, root) = instance("keep");
        let dir = temp.join("snapshots");
        for i in 0..KEEP + 3 {
            create(&root, &dir, &format!("s{i}")).unwrap();
        }
        let all = list(&dir);
        assert_eq!(all.len(), KEEP);
        assert_eq!(all[0].label(), format!("s{}", KEEP + 2));
        assert_eq!(latest(&dir, 3).len(), 3);
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn refuses_snapshots_that_write_outside_the_instance() {
        let (temp, root) = instance("evil");
        let dir = temp.join("snapshots");
        std::fs::create_dir_all(&dir).unwrap();
        let evil = dir.join("20000101-000000-000-evil.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&evil).unwrap());
        z.start_file("../escaped.txt", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"gotcha").unwrap();
        z.finish().unwrap();

        let snapshot = list(&dir).remove(0);
        assert!(restore(&snapshot, &root, &dir).is_err());
        assert!(!temp.join("escaped.txt").exists());
        assert_eq!(read(&root.join("mods/a.jar")), "A1");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn restoring_removes_pack_folders_that_did_not_exist_yet() {
        let (temp, root) = instance("folders");
        std::fs::create_dir_all(root.join("defaultconfigs")).unwrap();
        let dir = temp.join("snapshots");
        let snapshot = create(&root, &dir, "before-update").unwrap();

        // the update brings a scripts folder and fills the empty defaultconfigs folder
        write(&root.join("kubejs/startup.js"), "new script");
        write(&root.join("defaultconfigs/new.toml"), "x=1");

        restore(&snapshot, &root, &dir).unwrap();

        assert!(!root.join("kubejs").exists());
        assert!(root.join("defaultconfigs").is_dir());
        assert!(!root.join("defaultconfigs/new.toml").exists());
        assert_eq!(read(&root.join("mods/a.jar")), "A1");
        assert_eq!(read(&root.join("saves/World/level.dat")), "my world");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn the_oldest_kept_snapshot_can_be_restored() {
        let (temp, root) = instance("oldest");
        let dir = temp.join("snapshots");
        let oldest = create(&root, &dir, "oldest").unwrap();
        write(&root.join("mods/a.jar"), "A2");
        for i in 1..KEEP {
            create(&root, &dir, &format!("s{i}")).unwrap();
        }
        assert_eq!(list(&dir).len(), KEEP);
        // saving the state before the restore pushes the oldest snapshot out of the kept ones
        restore(&oldest, &root, &dir).unwrap();
        assert_eq!(read(&root.join("mods/a.jar")), "A1");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn leaves_no_temporary_folders_behind() {
        let (temp, root) = instance("clean");
        let dir = temp.join("snapshots");
        restore(&create(&root, &dir, "x").unwrap(), &root, &dir).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(&temp)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn labels_are_read_from_the_file_name() {
        let s = Snapshot { file: PathBuf::from("/x/20260101-120000-123-before-update.zip"), created_at: 0, size: 0 };
        assert_eq!(s.label(), "before-update");
    }
}
