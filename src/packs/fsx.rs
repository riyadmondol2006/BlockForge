//! File helpers with the same behaviour as the Java edition's `Utils` (copy over what is there, unzip with its
//! naming quirks, delete without following links), so folders come out the same in both editions.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use md5::Md5;
use sha1::{Digest, Sha1};

/// Copy a file. With `with_filename` the target is the full new path, otherwise it is the folder to put the file
/// in. An existing file is replaced.
pub fn copy_file(from: &Path, to: &Path, with_filename: bool) -> Result<PathBuf> {
    let target = if with_filename {
        to.to_path_buf()
    } else {
        to.join(from.file_name().with_context(|| format!("{} has no name", from.display()))?)
    };
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if target.exists() {
        let _ = std::fs::remove_file(&target);
    }
    std::fs::copy(from, &target).with_context(|| format!("copying {} to {}", from.display(), target.display()))?;
    Ok(target)
}

/// Copy a folder's contents into `to` (created if needed), replacing files that are already there. With
/// `copy_folder` the folder itself is put inside `to`.
pub fn copy_dir(from: &Path, to: &Path, copy_folder: bool) -> Result<()> {
    let target = if copy_folder {
        to.join(from.file_name().with_context(|| format!("{} has no name", from.display()))?)
    } else {
        to.to_path_buf()
    };
    copy_dir_inner(from, &target)
}

fn copy_dir_inner(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)?.flatten() {
            copy_dir_inner(&e.path(), &to.join(e.file_name()))?;
        }
    } else {
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::copy(from, to).with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
    }
    Ok(())
}

/// Delete a file or a whole folder. Links are removed, never followed. A path that isn't there is fine.
pub fn delete_path(p: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(p) else { return };
    if meta.is_dir() {
        let _ = std::fs::remove_dir_all(p);
    } else {
        let _ = std::fs::remove_file(p);
    }
}

/// Empty a folder but keep the folder.
pub fn delete_contents(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            delete_path(&e.path());
        }
    }
}

/// Delete the entries of `dir` whose names are (`delete = true`) or are not (`false`) in `names`.
pub fn delete_with_filter(dir: &Path, names: &[String], delete: bool) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if names.contains(&name) == delete {
            delete_path(&e.path());
        }
    }
}

/// Remove `start`'s empty parent folders, going up until `root` or a folder that has something in it.
pub fn delete_empty_parents(start: &Path, root: &Path) {
    let mut dir = start.parent().map(Path::to_path_buf);
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) || !d.is_dir() {
            break;
        }
        let empty = std::fs::read_dir(&d).map(|mut rd| rd.next().is_none()).unwrap_or(false);
        if !empty || std::fs::remove_dir(&d).is_err() {
            break;
        }
        dir = d.parent().map(Path::to_path_buf);
    }
}

/// Unzip `zip_path` into `out`. Matches the Java edition: `aux.class` is stored as `aux_class` (Windows cannot
/// have a file called aux), a file entry called `.minecraft` is skipped, existing files are replaced. Entries that
/// would land outside `out` are refused.
pub fn unzip(zip_path: &Path, out: &Path) -> Result<()> {
    let file = File::open(zip_path).with_context(|| format!("opening {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).with_context(|| format!("{} isn't a zip file", zip_path.display()))?;
    std::fs::create_dir_all(out)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Ok(raw) = entry.name().map(|n| n.to_string()) else { continue };
        // Windows cannot have a file called aux; the Java edition stores it as aux_class and zips it back as aux.class
        // (it drops the folder part as well, which breaks the class; here only the name changes)
        let name = if raw.ends_with("aux.class") {
            format!("{}aux_class", &raw[..raw.len() - "aux.class".len()])
        } else {
            raw.clone()
        };
        if !is_safe_relative(&name) {
            continue;
        }
        let dest = out.join(&name);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest)?;
            continue;
        }
        if raw == ".minecraft" {
            continue;
        }
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = File::create(&dest).with_context(|| format!("writing {}", dest.display()))?;
        io::copy(&mut entry, &mut f)?;
    }
    Ok(())
}

/// Whether a path from inside an archive stays below the folder it is unpacked into.
pub fn is_safe_relative(name: &str) -> bool {
    let p = Path::new(name);
    !p.is_absolute()
        && !p.components().any(|c| matches!(c, Component::ParentDir | Component::Prefix(_) | Component::RootDir))
}

/// Read one file out of a zip, if it is in there.
pub fn read_zip_entry(zip_path: &Path, name: &str) -> Option<Vec<u8>> {
    let file = File::open(zip_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name(name).ok()?;
    let mut out = Vec::new();
    entry.read_to_end(&mut out).ok()?;
    Some(out)
}

pub fn zip_contains(zip_path: &Path, name: &str) -> bool {
    File::open(zip_path).ok().and_then(|f| zip::ZipArchive::new(f).ok()).is_some_and(|mut a| a.by_name(name).is_ok())
}

/// Zip a folder's contents into `out` (`aux_class` goes back to `aux.class`).
pub fn zip_dir(dir: &Path, out: &Path) -> Result<()> {
    use zip::write::SimpleFileOptions;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = zip::ZipWriter::new(File::create(out).with_context(|| format!("creating {}", out.display()))?);
    let opts = SimpleFileOptions::default();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&d)?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            let rel = path.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
            let rel = if rel.ends_with("aux_class") { rel.replace("aux_class", "aux.class") } else { rel };
            if path.is_dir() {
                writer.add_directory(format!("{rel}/"), opts)?;
                stack.push(path);
            } else {
                writer.start_file(rel, opts)?;
                io::copy(&mut File::open(&path)?, &mut writer)?;
            }
        }
    }
    writer.finish()?;
    Ok(())
}

/// Merge two jars into one for old Minecraft versions that are patched by putting files into the game jar. Files
/// from `add` win over the same file in `main`; `META-INF` is left out so the signatures of the original do not
/// make the changed jar fail to load.
pub fn combine_jars(main: &Path, add: &Path, out: &Path) -> Result<()> {
    use std::collections::HashSet;
    use zip::write::SimpleFileOptions;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = zip::ZipWriter::new(File::create(out).with_context(|| format!("creating {}", out.display()))?);
    let opts = SimpleFileOptions::default();
    let mut added: HashSet<String> = HashSet::new();
    for source in [add, main] {
        let mut archive =
            zip::ZipArchive::new(File::open(source).with_context(|| format!("opening {}", source.display()))?)?;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let Ok(name) = entry.name().map(|n| n.to_string()) else { continue };
            if name.contains("META-INF") || added.contains(&name) {
                continue;
            }
            if entry.is_dir() {
                writer.add_directory(&name, opts)?;
            } else {
                writer.start_file(&name, opts)?;
                io::copy(&mut entry, &mut writer)?;
            }
            added.insert(name);
        }
    }
    writer.finish()?;
    Ok(())
}

pub fn md5_file(path: &Path) -> Result<String> {
    let mut h = Md5::new();
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

pub fn sha1_file(path: &Path) -> Result<String> {
    let mut h = Sha1::new();
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Replace the extension-less tail of `file` with its upper or lower case form, the way `caseAllFiles` asks
/// (`Mod.JAR` stays as it is: only the part before the last dot changes).
pub fn recase_name(file: &str, upper: bool) -> String {
    match file.rfind('.') {
        Some(dot) => {
            let (stem, ext) = file.split_at(dot);
            format!("{}{}", if upper { stem.to_uppercase() } else { stem.to_lowercase() }, ext)
        }
        None => {
            if upper {
                file.to_uppercase()
            } else {
                file.to_lowercase()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bf-packs-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn zip_round_trip_keeps_aux_class_and_blocks_escapes() {
        let dir = temp("zip");
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), "hello").unwrap();
        std::fs::write(src.join("sub").join("aux_class"), "x").unwrap();
        let zip_path = dir.join("t.zip");
        zip_dir(&src, &zip_path).unwrap();
        assert!(zip_contains(&zip_path, "sub/aux.class"));
        let out = dir.join("out");
        unzip(&zip_path, &out).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("a.txt")).unwrap(), "hello");
        assert!(out.join("sub").join("aux_class").exists());
        assert!(!is_safe_relative("../evil"));
        assert!(!is_safe_relative("/etc/passwd"));
        assert!(is_safe_relative("mods/a.jar"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn combining_jars_prefers_the_added_files_and_drops_meta_inf() {
        let dir = temp("jar");
        let make = |name: &str, files: &[(&str, &str)]| {
            let p = dir.join(name);
            let src = dir.join(format!("{name}-src"));
            for (f, c) in files {
                let fp = src.join(f);
                std::fs::create_dir_all(fp.parent().unwrap()).unwrap();
                std::fs::write(fp, c).unwrap();
            }
            zip_dir(&src, &p).unwrap();
            p
        };
        let main = make("main.jar", &[("a.class", "main"), ("b.class", "main"), ("META-INF/MANIFEST.MF", "m")]);
        let add = make("add.jar", &[("a.class", "added"), ("c.class", "added")]);
        let out = dir.join("out.jar");
        combine_jars(&main, &add, &out).unwrap();
        assert_eq!(read_zip_entry(&out, "a.class").unwrap(), b"added");
        assert_eq!(read_zip_entry(&out, "b.class").unwrap(), b"main");
        assert_eq!(read_zip_entry(&out, "c.class").unwrap(), b"added");
        assert!(read_zip_entry(&out, "META-INF/MANIFEST.MF").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn copy_and_filtered_delete() {
        let dir = temp("copy");
        let mods = dir.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        for n in ["a.jar", "b.jar", "mine.jar"] {
            std::fs::write(mods.join(n), n).unwrap();
        }
        delete_with_filter(&mods, &["a.jar".into(), "b.jar".into()], true);
        assert!(!mods.join("a.jar").exists());
        assert!(mods.join("mine.jar").exists());
        let copied = copy_file(&mods.join("mine.jar"), &dir.join("elsewhere"), false).unwrap();
        assert_eq!(copied, dir.join("elsewhere").join("mine.jar"));
        copy_dir(&mods, &dir.join("clone"), false).unwrap();
        assert!(dir.join("clone").join("mine.jar").exists());
        let nested = dir.join("x").join("y").join("z.txt");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(&nested, "z").unwrap();
        delete_path(&nested);
        delete_empty_parents(&nested, &dir);
        assert!(!dir.join("x").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn case_conversion_only_touches_the_name() {
        assert_eq!(recase_name("CoolMod-1.0.jar", true), "COOLMOD-1.0.jar");
        assert_eq!(recase_name("CoolMod-1.0.jar", false), "coolmod-1.0.jar");
    }
}
