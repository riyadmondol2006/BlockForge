// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Finding a Bedrock world in a folder or in a `.mcworld`/`.zip` archive.
//!
//! A Bedrock world is a folder with `level.dat` and a `db/` LevelDB folder. Chunker only reads folders; archives are
//! extracted to a scratch directory here, with limits on size and entry count and no way to write outside it.

use super::data_version::{BedrockVersion, GameVersion};
use super::db::ScratchDir;
use super::level_dat::LevelDat;
use super::{BedrockError, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// Largest total size extracted from an archive.
pub const MAX_EXTRACT_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Most entries an archive may hold.
pub const MAX_EXTRACT_ENTRIES: usize = 200_000;

/// Limits applied while extracting an archive.
#[derive(Clone, Copy, Debug)]
pub struct ExtractLimits {
    pub max_total_bytes: u64,
    pub max_entries: usize,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        ExtractLimits { max_total_bytes: MAX_EXTRACT_BYTES, max_entries: MAX_EXTRACT_ENTRIES }
    }
}

/// A Bedrock world that was found.
#[derive(Debug)]
pub struct BedrockWorldInfo {
    /// `levelname.txt`, else `LevelName` of `level.dat`, else the folder or file name.
    pub name: String,
    /// The registered version the world's format maps to.
    pub version: BedrockVersion,
    /// The game version `level.dat` records (`lastOpenedWithVersion`), if any.
    pub game_version: Option<GameVersion>,
    /// The `level.dat` header's storage version.
    pub storage_version: i32,
    /// `NetworkVersion`.
    pub network_version: Option<i32>,
    /// The folder holding `level.dat` and `db/` (inside the scratch directory for archives).
    pub root: PathBuf,
    /// Keeps the extracted archive alive; dropping the info removes the scratch directory.
    guard: Option<ScratchDir>,
}

impl BedrockWorldInfo {
    /// The `db/` folder.
    pub fn db_path(&self) -> PathBuf {
        self.root.join("db")
    }

    /// The `level.dat` file.
    pub fn level_dat_path(&self) -> PathBuf {
        self.root.join("level.dat")
    }

    /// Whether the world was extracted from an archive into a scratch directory.
    pub fn is_extracted(&self) -> bool {
        self.guard.is_some()
    }
}

/// Whether a folder is a Bedrock world root.
pub fn is_world_root(dir: &Path) -> bool {
    dir.join("level.dat").is_file() && dir.join("db").join("CURRENT").is_file()
}

/// The world root at `dir` or in its single sub-folder holding a world (zips often nest everything in a folder).
fn find_root(dir: &Path) -> Option<PathBuf> {
    if is_world_root(dir) {
        return Some(dir.to_path_buf());
    }
    let mut found = None;
    for e in std::fs::read_dir(dir).ok()? {
        let p = e.ok()?.path();
        if p.is_dir() && is_world_root(&p) {
            if found.is_some() {
                return None; // ambiguous
            }
            found = Some(p);
        }
    }
    found
}

/// Detect a Bedrock world at `path`: a world folder, or a `.mcworld`/`.zip` archive (extracted to the system temp
/// directory). `None` when it is not a Bedrock world or cannot be read.
pub fn detect_bedrock(path: &Path) -> Option<BedrockWorldInfo> {
    detect_bedrock_in(path, None, &ExtractLimits::default())
}

/// [`detect_bedrock`] with an explicit parent for the scratch directory and explicit extraction limits.
pub fn detect_bedrock_in(path: &Path, scratch_parent: Option<&Path>, limits: &ExtractLimits) -> Option<BedrockWorldInfo> {
    if path.is_dir() {
        let root = find_root(path)?;
        return describe(root, None, path);
    }
    if !path.is_file() {
        return None;
    }
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if ext != "mcworld" && ext != "zip" {
        return None;
    }
    if !archive_has_world(path).ok()? {
        return None;
    }
    let scratch = match scratch_parent {
        Some(p) => ScratchDir::new_in(p, "world").ok()?,
        None => ScratchDir::new("world").ok()?,
    };
    extract_zip(path, scratch.path(), limits).ok()?;
    let root = find_root(scratch.path())?;
    describe(root, Some(scratch), path)
}

fn describe(root: PathBuf, guard: Option<ScratchDir>, original: &Path) -> Option<BedrockWorldInfo> {
    let level = LevelDat::read(&root.join("level.dat")).ok()?;
    let info = level.info();
    let from_file = std::fs::read_to_string(root.join("levelname.txt"))
        .ok()
        .and_then(|s| s.lines().next().map(|l| l.trim().to_string()))
        .filter(|s| !s.is_empty());
    let name = from_file
        .or(info.level_name.filter(|s| !s.is_empty()))
        .or_else(|| original.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_default();
    Some(BedrockWorldInfo {
        name,
        version: *level.bedrock_version(),
        game_version: level.game_version(),
        storage_version: level.storage_version,
        network_version: info.network_version,
        root,
        guard,
    })
}

fn archive_err(e: impl std::fmt::Display) -> BedrockError {
    BedrockError::Archive(e.to_string())
}

/// Whether an archive holds `level.dat` next to `db/CURRENT` (at the root or one folder down).
fn archive_has_world(path: &Path) -> Result<bool> {
    let mut archive = zip::ZipArchive::new(File::open(path)?).map_err(archive_err)?;
    let mut level_dirs: Vec<PathBuf> = Vec::new();
    let mut current_dirs: Vec<PathBuf> = Vec::new();
    for i in 0..archive.len().min(MAX_EXTRACT_ENTRIES + 1) {
        let f = archive.by_index_raw(i).map_err(archive_err)?;
        let Some(name) = f.enclosed_name() else { continue };
        let depth = name.components().count();
        if depth > 3 {
            continue;
        }
        match name.file_name().and_then(|n| n.to_str()) {
            Some("level.dat") => level_dirs.push(name.parent().map(Path::to_path_buf).unwrap_or_default()),
            Some("CURRENT") => {
                if let Some(db) = name.parent() {
                    if db.file_name().is_some_and(|n| n == "db") {
                        current_dirs.push(db.parent().map(Path::to_path_buf).unwrap_or_default());
                    }
                }
            }
            _ => {}
        }
    }
    Ok(level_dirs.iter().any(|d| current_dirs.contains(d)))
}

/// Extract a zip into `dest` (which must exist). Entries are confined to `dest`: absolute paths, `..` and symbolic
/// links are skipped, and the extraction stops with an error past the limits.
pub fn extract_zip(zip_path: &Path, dest: &Path, limits: &ExtractLimits) -> Result<()> {
    let mut archive = zip::ZipArchive::new(File::open(zip_path)?).map_err(archive_err)?;
    if archive.len() > limits.max_entries {
        return Err(BedrockError::Archive(format!("archive has {} entries (limit {})", archive.len(), limits.max_entries)));
    }
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(archive_err)?;
        let Some(rel) = f.enclosed_name() else { continue };
        // Belt and braces on top of `enclosed_name`: only plain components.
        if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            continue;
        }
        if f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            continue; // a symbolic link
        }
        let out = dest.join(&rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let remaining = limits.max_total_bytes.saturating_sub(total);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&out)
            .map_err(|e| BedrockError::Archive(format!("cannot create {}: {e}", rel.display())))?;
        let written = copy_limited(&mut f, &mut file, remaining)?;
        total += written;
        if total > limits.max_total_bytes {
            return Err(BedrockError::Archive(format!("archive expands past {} bytes", limits.max_total_bytes)));
        }
    }
    Ok(())
}

/// Copy at most `limit + 1` bytes (one over, so the caller can tell the limit was passed).
fn copy_limited(from: &mut impl Read, to: &mut impl Write, limit: u64) -> Result<u64> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = from.read(&mut buf).map_err(archive_err)?;
        if n == 0 {
            return Ok(total);
        }
        to.write_all(&buf[..n])?;
        total += n as u64;
        if total > limit {
            return Ok(total);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::bedrock::db::{BedrockDb, DbOptions, LevelDb};
    use crate::convert::bedrock::keys::{self, ChunkTag, DimensionId};
    use crate::convert::bedrock::level_dat::LevelDat;
    use crate::convert::nbt::{Compound, Tag};

    /// A minimal world folder with a database and a level.dat.
    fn make_world(dir: &Path, version: [i32; 3], level_name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let mut root = Compound::new();
        root.insert("LevelName", Tag::String(level_name.into()));
        root.insert("lastOpenedWithVersion", Tag::List(vec![version[0], version[1], version[2], 0, 0].into_iter().map(Tag::Int).collect()));
        LevelDat { storage_version: 10, root_name: String::new(), root }.write(&dir.join("level.dat")).unwrap();
        let mut db = LevelDb::create_new(&dir.join("db"), &DbOptions::default()).unwrap();
        db.put(&keys::chunk_key(DimensionId::OVERWORLD, 0, 0, ChunkTag::Data3D), b"x").unwrap();
        db.compact().unwrap();
    }

    fn zip_dir(src: &Path, zip_path: &Path, prefix: &str) {
        let mut z = zip::ZipWriter::new(File::create(zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        fn walk(z: &mut zip::ZipWriter<File>, dir: &Path, base: &Path, prefix: &str, opts: zip::write::SimpleFileOptions) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                if p.is_dir() {
                    walk(z, &p, base, prefix, opts);
                } else {
                    z.start_file(format!("{prefix}{rel}"), opts).unwrap();
                    z.write_all(&std::fs::read(&p).unwrap()).unwrap();
                }
            }
        }
        walk(&mut z, src, src, prefix, opts);
        z.finish().unwrap();
    }

    #[test]
    fn folder_and_archive_detection() {
        let tmp = ScratchDir::new("detect").unwrap();
        let world = tmp.path().join("world");
        make_world(&world, [1, 21, 44], "Hello");
        std::fs::write(world.join("levelname.txt"), "My Name\nsecond line\n").unwrap();
        let info = detect_bedrock(&world).expect("folder");
        assert_eq!(info.name, "My Name");
        assert_eq!(info.version.game, [1, 21, 40]);
        assert_eq!(info.game_version, Some([1, 21, 44]));
        assert!(!info.is_extracted());
        assert!(info.db_path().join("CURRENT").is_file());
        // a parent folder with a single world inside
        assert!(detect_bedrock(tmp.path()).is_some());

        for (prefix, ext) in [("", "mcworld"), ("inner/", "zip")] {
            let archive = tmp.path().join(format!("w.{ext}"));
            zip_dir(&world, &archive, prefix);
            let scratch_parent = tmp.path().join("scratch");
            std::fs::create_dir_all(&scratch_parent).unwrap();
            let info = detect_bedrock_in(&archive, Some(&scratch_parent), &ExtractLimits::default()).expect("archive");
            assert_eq!(info.name, "My Name");
            assert!(info.is_extracted());
            assert!(info.root.starts_with(&scratch_parent));
            let root = info.root.clone();
            assert!(root.join("level.dat").is_file());
            drop(info);
            assert!(!root.exists(), "scratch removed with the info");
        }
    }

    #[test]
    fn non_worlds_are_not_detected() {
        let tmp = ScratchDir::new("detect-no").unwrap();
        assert!(detect_bedrock(&tmp.path().join("missing")).is_none());
        assert!(detect_bedrock(tmp.path()).is_none());
        // a Java-like folder
        let java = tmp.path().join("java");
        std::fs::create_dir_all(java.join("region")).unwrap();
        std::fs::write(java.join("level.dat"), [0x1F, 0x8B, 8, 0]).unwrap();
        assert!(detect_bedrock(&java).is_none());
        // a folder with a db but a broken level.dat
        let broken = tmp.path().join("broken");
        make_world(&broken, [1, 20, 0], "x");
        std::fs::write(broken.join("level.dat"), b"junk").unwrap();
        assert!(detect_bedrock(&broken).is_none());
        // an unrelated zip and a non-zip with the right extension
        let z = tmp.path().join("other.zip");
        let mut w = zip::ZipWriter::new(File::create(&z).unwrap());
        w.start_file("readme.txt", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(b"hi").unwrap();
        w.finish().unwrap();
        assert!(detect_bedrock(&z).is_none());
        let fake = tmp.path().join("fake.mcworld");
        std::fs::write(&fake, b"not a zip").unwrap();
        assert!(detect_bedrock(&fake).is_none());
        assert!(detect_bedrock(&tmp.path().join("a.txt")).is_none());
    }

    #[test]
    fn extraction_is_confined_and_limited() {
        let tmp = ScratchDir::new("extract").unwrap();
        let z = tmp.path().join("evil.zip");
        let mut w = zip::ZipWriter::new(File::create(&z).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        for name in ["../escape.txt", "/abs.txt", "a/../../escape2.txt", "ok/file.txt"] {
            w.start_file(name, o).unwrap();
            w.write_all(b"data").unwrap();
        }
        w.add_symlink("link", "/etc", o).unwrap();
        w.finish().unwrap();
        let dest = tmp.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        extract_zip(&z, &dest, &ExtractLimits::default()).unwrap();
        assert!(dest.join("ok/file.txt").is_file());
        assert!(!tmp.path().join("escape.txt").exists());
        assert!(!tmp.path().join("escape2.txt").exists());
        assert!(!Path::new("/abs.txt").exists());
        assert!(!dest.join("link").exists());

        // size cap, measured on the bytes really written
        let big = tmp.path().join("big.zip");
        let mut w = zip::ZipWriter::new(File::create(&big).unwrap());
        w.start_file("zeros", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
        w.write_all(&vec![0u8; 1 << 20]).unwrap();
        w.finish().unwrap();
        let dest2 = tmp.path().join("dest2");
        std::fs::create_dir_all(&dest2).unwrap();
        let tiny = ExtractLimits { max_total_bytes: 1000, max_entries: 10 };
        assert!(matches!(extract_zip(&big, &dest2, &tiny), Err(BedrockError::Archive(_))));
        // entry cap
        let dest3 = tmp.path().join("dest3");
        std::fs::create_dir_all(&dest3).unwrap();
        let few = ExtractLimits { max_total_bytes: 1 << 30, max_entries: 3 };
        assert!(extract_zip(&z, &dest3, &few).is_err());
    }

    #[test]
    fn the_real_world_is_detected() {
        let Some(path) = crate::convert::bedrock::test_world_path() else {
            eprintln!("test world not present, skipping");
            return;
        };
        let info = detect_bedrock(&path).expect("test world");
        assert_eq!(info.name, "Mace, Spear, Sword PvP Map! V4");
        assert_eq!(info.game_version, Some([1, 26, 51]));
        assert_eq!(info.version.game, [1, 26, 50]);
        assert_eq!((info.storage_version, info.network_version), (10, Some(1001)));
        assert!(info.is_extracted());
    }
}
