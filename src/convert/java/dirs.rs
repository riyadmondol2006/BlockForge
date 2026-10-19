// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Where things live in a Java world folder (Chunker's `JavaLevelDirectoryResolver` and the 26.1
//! `LevelDirectoryResolver`), finding the dimensions of a world, and detecting a world from a folder or a `.zip`.
//!
//! Layout up to 1.21.11: the overworld is the world folder itself (`region/`, `entities/`, `poi/`), the nether is
//! `DIM-1`, the end `DIM1`, custom dimensions `dimensions/<namespace>/<key>`; `playerdata/`, `data/` (maps as
//! `map_N.dat`, `idcounts.dat`), `datapacks/`. From 26.1 every dimension is `dimensions/<namespace>/<key>` (with its
//! own `data/minecraft`), players are in `players/data/<uuid>.dat`, global data in `data/minecraft/` and maps in
//! `data/minecraft/maps/N.dat`.
//!
//! `session.lock` is never created, read or required here; extracting a zip skips it.

use super::format::JavaFormat;
use super::level_dat::{LevelDatError, LevelInfo, read_level_dat};
use super::region::{RegionKind, list_region_files};
use super::version::Version;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A dimension, addressed the way Java does.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum JavaDimension {
    Overworld,
    Nether,
    End,
    /// A custom dimension `namespace:key`.
    Custom(String, String),
}

impl JavaDimension {
    /// The namespaced id (`minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`, `ns:key`).
    pub fn identifier(&self) -> String {
        match self {
            JavaDimension::Overworld => "minecraft:overworld".into(),
            JavaDimension::Nether => "minecraft:the_nether".into(),
            JavaDimension::End => "minecraft:the_end".into(),
            JavaDimension::Custom(ns, key) => format!("{ns}:{key}"),
        }
    }

    /// Parse a namespaced id. Custom ids whose parts could escape the world folder are rejected.
    pub fn from_identifier(id: &str) -> Option<JavaDimension> {
        let (ns, key) = id.split_once(':')?;
        if !valid_id_part(ns, false) || !valid_id_part(key, true) {
            return None;
        }
        Some(match (ns, key) {
            ("minecraft", "overworld") => JavaDimension::Overworld,
            ("minecraft", "the_nether") => JavaDimension::Nether,
            ("minecraft", "the_end") => JavaDimension::End,
            _ => JavaDimension::Custom(ns.to_string(), key.to_string()),
        })
    }

    /// The numeric id used before 1.16 (0, -1, 1); custom dimensions have none.
    pub fn legacy_id(&self) -> Option<i32> {
        match self {
            JavaDimension::Overworld => Some(0),
            JavaDimension::Nether => Some(-1),
            JavaDimension::End => Some(1),
            JavaDimension::Custom(..) => None,
        }
    }
}

/// Namespace parts are `[a-z0-9_.-]`; key parts may also contain `/` between non-empty segments. Dots alone
/// (`.`, `..`) are never a segment.
fn valid_id_part(part: &str, allow_slash: bool) -> bool {
    if part.is_empty() || part.len() > 128 {
        return false;
    }
    let seg_ok = |s: &str| {
        !s.is_empty() && s != "." && s != ".." && s.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'))
    };
    if allow_slash { part.split('/').all(seg_ok) } else { seg_ok(part) }
}

/// Paths inside one world folder.
#[derive(Clone, Debug)]
pub struct LevelDirs {
    root: PathBuf,
    namespaced: bool,
}

impl LevelDirs {
    /// The layout for a format (`namespaced_dimension_dirs` is true from 26.1).
    pub fn new(root: impl Into<PathBuf>, format: &JavaFormat) -> LevelDirs {
        LevelDirs { root: root.into(), namespaced: format.namespaced_dimension_dirs }
    }

    /// An explicit layout choice.
    pub fn with_layout(root: impl Into<PathBuf>, namespaced: bool) -> LevelDirs {
        LevelDirs { root: root.into(), namespaced }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn level_dat(&self) -> PathBuf {
        self.root.join("level.dat")
    }

    /// Where `session.lock` would be. Informational: it is never created or required.
    pub fn session_lock(&self) -> PathBuf {
        self.root.join("session.lock")
    }

    /// The folder of a dimension (the base for `region/`, `entities/`, `poi/`).
    pub fn dimension_dir(&self, dim: &JavaDimension) -> PathBuf {
        if !self.namespaced {
            match dim {
                JavaDimension::Overworld => return self.root.clone(),
                JavaDimension::Nether => return self.root.join("DIM-1"),
                JavaDimension::End => return self.root.join("DIM1"),
                JavaDimension::Custom(..) => {}
            }
        }
        let id = dim.identifier();
        let (ns, key) = id.split_once(':').unwrap_or(("minecraft", &id));
        let mut path = self.root.join("dimensions").join(ns);
        for seg in key.split('/') {
            path.push(seg);
        }
        path
    }

    pub fn region_dir(&self, dim: &JavaDimension) -> PathBuf {
        RegionKind::Region.dir(&self.dimension_dir(dim))
    }

    pub fn entities_dir(&self, dim: &JavaDimension) -> PathBuf {
        RegionKind::Entities.dir(&self.dimension_dir(dim))
    }

    pub fn poi_dir(&self, dim: &JavaDimension) -> PathBuf {
        RegionKind::Poi.dir(&self.dimension_dir(dim))
    }

    /// The per-dimension vanilla data folder, `<dimension>/data/minecraft` (26.1+ only).
    pub fn dimension_data_dir(&self, dim: &JavaDimension) -> PathBuf {
        self.dimension_dir(dim).join("data").join("minecraft")
    }

    pub fn datapacks_dir(&self) -> PathBuf {
        self.root.join("datapacks")
    }

    /// Global data: `data/` before 26.1, `data/minecraft/` from 26.1.
    pub fn minecraft_data_dir(&self) -> PathBuf {
        if self.namespaced { self.root.join("data").join("minecraft") } else { self.root.join("data") }
    }

    /// Player files: `playerdata/` before 26.1, `players/data/` from 26.1.
    pub fn players_dir(&self) -> PathBuf {
        if self.namespaced { self.root.join("players").join("data") } else { self.root.join("playerdata") }
    }

    /// Item maps: `data/` before 26.1, `data/minecraft/maps/` from 26.1.
    pub fn maps_dir(&self) -> PathBuf {
        if self.namespaced { self.minecraft_data_dir().join("maps") } else { self.minecraft_data_dir() }
    }

    /// The file of map `id`: `map_<id>.dat` before 26.1, `<id>.dat` from 26.1.
    pub fn map_file(&self, id: i64) -> PathBuf {
        if self.namespaced { self.maps_dir().join(format!("{id}.dat")) } else { self.maps_dir().join(format!("map_{id}.dat")) }
    }

    /// The map id counter: `idcounts.dat` before 26.1, `last_id.dat` from 26.1.
    pub fn map_counter_file(&self) -> PathBuf {
        self.maps_dir().join(if self.namespaced { "last_id.dat" } else { "idcounts.dat" })
    }

    /// Parse a map file name into its id for this layout.
    pub fn parse_map_file_name(&self, name: &str) -> Option<i64> {
        let stem = name.strip_suffix(".dat")?;
        let digits = if self.namespaced { stem } else { stem.strip_prefix("map_")? };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok()
    }

    /// The file of a player: `<uuid>.dat` in [`LevelDirs::players_dir`].
    pub fn player_file(&self, uuid: &str) -> Option<PathBuf> {
        let ok = !uuid.is_empty() && uuid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
        ok.then(|| self.players_dir().join(format!("{uuid}.dat")))
    }

    /// The single-player file name of 26.1+, from `singleplayer_uuid` (four ints). All zeros when absent, like vanilla.
    pub fn uuid_from_ints(ints: &[i32]) -> Option<String> {
        if ints.len() != 4 {
            return None;
        }
        let hi = ((ints[0] as u32 as u64) << 32) | ints[1] as u32 as u64;
        let lo = ((ints[2] as u32 as u64) << 32) | ints[3] as u32 as u64;
        let h = format!("{:016x}{:016x}", hi, lo);
        Some(format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]))
    }

    /// Discover the dimensions that have a `region/` folder. The three vanilla dimensions are looked up in this
    /// layout first and in the other layout second (a 26.1 world that was copied around, a server layout, ...);
    /// custom dimensions are the `dimensions/<namespace>/<key>` folders with a `region/` folder.
    pub fn discover_dimensions(&self) -> Vec<DimensionInfo> {
        let other = LevelDirs { root: self.root.clone(), namespaced: !self.namespaced };
        let mut found: Vec<DimensionInfo> = Vec::new();
        for dim in [JavaDimension::Overworld, JavaDimension::Nether, JavaDimension::End] {
            for layout in [self, &other] {
                let dir = layout.dimension_dir(&dim);
                if dir.join("region").is_dir() {
                    found.push(DimensionInfo::inspect(dim.clone(), dir));
                    break;
                }
            }
        }
        // Custom dimensions: dimensions/<namespace>/<key>[/...]. Look two levels deep like Chunker, one more for
        // keys with a slash is not scanned.
        let dims = self.root.join("dimensions");
        let mut namespaces: Vec<_> = fs::read_dir(&dims).into_iter().flatten().flatten().collect();
        namespaces.sort_by_key(|e| e.file_name());
        for ns in namespaces {
            let (Some(ns_name), true) = (ns.file_name().to_str().map(str::to_string), ns.path().is_dir()) else { continue };
            let mut keys: Vec<_> = fs::read_dir(ns.path()).into_iter().flatten().flatten().collect();
            keys.sort_by_key(|e| e.file_name());
            for key in keys {
                let (Some(key_name), true) = (key.file_name().to_str().map(str::to_string), key.path().is_dir()) else { continue };
                let Some(dim) = JavaDimension::from_identifier(&format!("{ns_name}:{key_name}")) else { continue };
                if found.iter().any(|f| f.dimension == dim) || !key.path().join("region").is_dir() {
                    continue;
                }
                found.push(DimensionInfo::inspect(dim, key.path()));
            }
        }
        found
    }
}

/// A dimension found in a world folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DimensionInfo {
    pub dimension: JavaDimension,
    /// The dimension folder.
    pub dir: PathBuf,
    /// `.mca` files in `region/` (at least one sector long).
    pub region_files: usize,
    pub has_entities: bool,
    pub has_poi: bool,
}

impl DimensionInfo {
    fn inspect(dimension: JavaDimension, dir: PathBuf) -> DimensionInfo {
        let region_files = list_region_files(&RegionKind::Region.dir(&dir)).map(|l| l.len()).unwrap_or(0);
        DimensionInfo {
            dimension,
            has_entities: RegionKind::Entities.dir(&dir).is_dir(),
            has_poi: RegionKind::Poi.dir(&dir).is_dir(),
            region_files,
            dir,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Detection

/// A scratch directory deleted when dropped.
#[derive(Debug)]
pub struct ScratchDir {
    path: PathBuf,
}

static SCRATCH_COUNTER: AtomicUsize = AtomicUsize::new(0);

impl ScratchDir {
    /// Create a fresh directory under `parent`.
    pub fn new_in(parent: &Path) -> io::Result<ScratchDir> {
        fs::create_dir_all(parent)?;
        let n = SCRATCH_COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let path = parent.join(format!("java-world-{}-{}-{}", std::process::id(), n, nanos));
        fs::create_dir(&path)?;
        Ok(ScratchDir { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Keep the directory (do not delete it on drop) and return its path.
    pub fn keep(mut self) -> PathBuf {
        std::mem::take(&mut self.path)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Limits for extracting a zip.
#[derive(Clone, Copy, Debug)]
pub struct ZipLimits {
    pub max_entries: usize,
    pub max_total_bytes: u64,
    pub max_entry_bytes: u64,
}

impl Default for ZipLimits {
    fn default() -> ZipLimits {
        ZipLimits { max_entries: 500_000, max_total_bytes: 64 << 30, max_entry_bytes: 16 << 30 }
    }
}

/// Why a world could not be detected.
#[derive(Debug)]
pub enum DetectError {
    Io(io::Error),
    /// The path is not a folder or a `.zip`.
    NotAWorld(String),
    /// No `level.dat` (in the folder, or one folder down).
    NoLevelDat,
    LevelDat(LevelDatError),
    /// The level has no data version this port understands (not an Anvil world).
    UnknownVersion,
    /// The zip is unsafe or too big: reason.
    BadZip(String),
}

impl fmt::Display for DetectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DetectError::Io(e) => write!(f, "I/O error: {e}"),
            DetectError::NotAWorld(p) => write!(f, "{p} is not a world folder or zip"),
            DetectError::NoLevelDat => f.write_str("no level.dat found"),
            DetectError::LevelDat(e) => write!(f, "{e}"),
            DetectError::UnknownVersion => f.write_str("level.dat has no Java data version"),
            DetectError::BadZip(why) => write!(f, "unsafe or invalid zip: {why}"),
        }
    }
}

impl std::error::Error for DetectError {}

impl From<io::Error> for DetectError {
    fn from(e: io::Error) -> DetectError {
        DetectError::Io(e)
    }
}

/// What detection learned about a Java world.
#[derive(Debug)]
pub struct JavaWorldInfo {
    /// The world folder (the extracted copy for a zip).
    pub root: PathBuf,
    /// `LevelName`, else the folder name.
    pub name: String,
    pub data_version: i32,
    /// The registered game version the data version floors to.
    pub version: Version,
    /// `Version.Name` from `level.dat`.
    pub version_name: Option<String>,
    pub format: JavaFormat,
    pub dimensions: Vec<DimensionInfo>,
    pub level: LevelInfo,
    pub from_zip: bool,
    /// Owns the extracted copy of a zip; the folder is deleted when this is dropped.
    pub scratch: Option<ScratchDir>,
}

impl JavaWorldInfo {
    /// The directory helpers for this world.
    pub fn dirs(&self) -> LevelDirs {
        LevelDirs::new(&self.root, &self.format)
    }
}

/// Detect a Java world in a folder or `.zip`; a zip is extracted under the system temp folder.
pub fn detect_java(path: &Path) -> Option<JavaWorldInfo> {
    detect_java_checked(path, &std::env::temp_dir().join("blockforge-convert"), ZipLimits::default()).ok()
}

/// Like [`detect_java`] with the reason for a failure, a chosen scratch parent and zip limits.
pub fn detect_java_checked(path: &Path, scratch_parent: &Path, limits: ZipLimits) -> Result<JavaWorldInfo, DetectError> {
    let meta = fs::metadata(path)?;
    let (root, scratch, from_zip) = if meta.is_dir() {
        (path.to_path_buf(), None, false)
    } else if meta.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        let scratch = ScratchDir::new_in(scratch_parent)?;
        extract_zip(path, scratch.path(), limits)?;
        (scratch.path().to_path_buf(), Some(scratch), true)
    } else {
        return Err(DetectError::NotAWorld(path.display().to_string()));
    };
    let root = find_world_root(&root)?;
    let level = read_level_dat(&root.join("level.dat")).map_err(DetectError::LevelDat)?;
    let data_version = level.data_version.ok_or(DetectError::UnknownVersion)?;
    let format = JavaFormat::for_data_version(data_version);
    let dirs = LevelDirs::new(&root, &format);
    let dimensions = dirs.discover_dimensions();
    let name = level
        .level_name
        .clone()
        .filter(|n| !n.is_empty())
        .or_else(|| root.file_name().and_then(|n| n.to_str()).map(str::to_string))
        .unwrap_or_else(|| "World".to_string());
    Ok(JavaWorldInfo {
        root,
        name,
        data_version,
        version: format.version,
        version_name: level.version_name.clone(),
        format,
        dimensions,
        level,
        from_zip,
        scratch,
    })
}

/// The folder with `level.dat`: the folder itself, or its only child folder that has one (zips usually hold the
/// world in a top-level folder).
fn find_world_root(dir: &Path) -> Result<PathBuf, DetectError> {
    if dir.join("level.dat").is_file() {
        return Ok(dir.to_path_buf());
    }
    let mut hits = Vec::new();
    for entry in fs::read_dir(dir)?.flatten() {
        let p = entry.path();
        // __MACOSX and similar junk never hold a level.dat worth using.
        if entry.file_name().to_string_lossy().starts_with("__") {
            continue;
        }
        if p.is_dir() && p.join("level.dat").is_file() {
            hits.push(p);
        }
    }
    match hits.len() {
        1 => Ok(hits.remove(0)),
        _ => Err(DetectError::NoLevelDat),
    }
}

/// Extract a zip into `dest` safely: entries with absolute paths, `..`, drive prefixes or backslash tricks are an
/// error, symlinks are skipped, `session.lock` is skipped, and entry count and sizes are capped (counted while
/// copying, not trusted from the headers).
pub fn extract_zip(zip_path: &Path, dest: &Path, limits: ZipLimits) -> Result<(), DetectError> {
    let bad = |why: String| DetectError::BadZip(why);
    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| bad(e.to_string()))?;
    if archive.len() > limits.max_entries {
        return Err(bad(format!("{} entries (limit {})", archive.len(), limits.max_entries)));
    }
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| bad(e.to_string()))?;
        let name = entry.name().map_err(|e| bad(e.to_string()))?.to_string();
        if name.contains('\\') || name.contains('\0') {
            return Err(bad(format!("suspicious entry name {name:?}")));
        }
        let rel = Path::new(&name);
        let mut clean = PathBuf::new();
        for comp in rel.components() {
            match comp {
                Component::Normal(c) => clean.push(c),
                Component::CurDir => {}
                _ => return Err(bad(format!("entry {name:?} escapes the extraction folder"))),
            }
        }
        if clean.as_os_str().is_empty() {
            continue;
        }
        if let Some(mode) = entry.unix_mode() {
            if mode & 0o170000 == 0o120000 {
                continue; // symlink
            }
        }
        let target = dest.join(&clean);
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if clean.file_name().is_some_and(|n| n == "session.lock") {
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&target)?;
        let mut buf = vec![0u8; 1 << 16];
        let mut written: u64 = 0;
        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            written += n as u64;
            total += n as u64;
            if written > limits.max_entry_bytes || total > limits.max_total_bytes {
                return Err(bad("the zip expands beyond the size limit".into()));
            }
            out.write_all(&buf[..n])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
