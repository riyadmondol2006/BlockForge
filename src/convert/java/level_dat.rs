// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Reading `level.dat`: just what is needed to pick the format of a world (data version, game version, name).
//!
//! `level.dat` is gzip-compressed NBT. Like every `Tag.read*JavaNBT` helper in Chunker, reading unwraps a root child
//! compound named `data`, then `Data`, and works on that (`{Data: {...}}` is the usual shape; the 26.1 split files
//! use `{DataVersion, data: {...}}`).

use super::nbtio;
use super::version::{LAST_ANVIL_FILE_VERSION, Version, nearest_at_or_below};
use crate::convert::nbt::{Compound, Tag};
use std::fmt;
use std::fs;
use std::path::Path;

/// Largest `level.dat` this reads (real ones are a few KiB; with a lot of player data, a few MiB).
pub const MAX_LEVEL_DAT_BYTES: u64 = 64 << 20;

/// What can go wrong reading a `level.dat`.
#[derive(Debug)]
pub enum LevelDatError {
    Io(std::io::Error),
    TooLarge(u64),
    /// Not gzip/zlib/raw NBT, or not a compound at the root.
    Nbt(String),
}

impl fmt::Display for LevelDatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LevelDatError::Io(e) => write!(f, "cannot read level.dat: {e}"),
            LevelDatError::TooLarge(n) => write!(f, "level.dat is {n} bytes, too large"),
            LevelDatError::Nbt(e) => write!(f, "level.dat is not valid NBT: {e}"),
        }
    }
}

impl std::error::Error for LevelDatError {}

/// The fields of `level.dat` that decide the format.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelInfo {
    /// The data version: `Version.Id`, else `DataVersion`, else 0 for a 1.8.x level (Anvil `version` 19133).
    /// None when the file has no usable version at all (not an Anvil level).
    pub data_version: Option<i32>,
    /// `Version.Name` (e.g. "1.20.1"), when present.
    pub version_name: Option<String>,
    /// `Version.Snapshot` is set.
    pub snapshot: bool,
    /// `LevelName`.
    pub level_name: Option<String>,
    /// The Anvil `version` int (19133).
    pub anvil_version: Option<i32>,
    /// `LastPlayed` in milliseconds since the epoch.
    pub last_played: Option<i64>,
    /// The unwrapped data compound, for later phases.
    pub data: Compound,
}

impl LevelInfo {
    /// The game version of the registered row at or below the data version.
    pub fn game_version(&self) -> Option<Version> {
        self.data_version.map(|dv| nearest_at_or_below(dv).1)
    }
}

/// Unwrap `data`, then `Data`, from a root compound (else return it as it is).
pub fn unwrap_data(root: Compound) -> Compound {
    let mut current = root;
    for key in ["data", "Data"] {
        if matches!(current.get(key), Some(Tag::Compound(_))) {
            if let Some(Tag::Compound(inner)) = current.remove(key) {
                current = inner;
            }
        }
    }
    current
}

/// An integer tag of any width as i64.
pub fn tag_as_i64(tag: &Tag) -> Option<i64> {
    match tag {
        Tag::Byte(v) => Some(*v as i64),
        Tag::Short(v) => Some(*v as i64),
        Tag::Int(v) => Some(*v as i64),
        Tag::Long(v) => Some(*v),
        _ => None,
    }
}

fn int(c: &Compound, key: &str) -> Option<i64> {
    c.get(key).and_then(tag_as_i64)
}

/// Parse the bytes of a `level.dat` (gzip, zlib or raw NBT).
pub fn parse_level_dat(bytes: &[u8]) -> Result<LevelInfo, LevelDatError> {
    let raw = nbtio::decompress_auto(bytes).map_err(|e| LevelDatError::Nbt(e.to_string()))?;
    let (_, root) = nbtio::from_slice(&raw).map_err(|e| LevelDatError::Nbt(e.to_string()))?;
    let Tag::Compound(root) = root else {
        return Err(LevelDatError::Nbt("the root tag is not a compound".into()));
    };
    let data = unwrap_data(root);
    let version = data.get_compound("Version");
    let version_id = version.and_then(|v| int(v, "Id")).and_then(|v| i32::try_from(v).ok());
    let anvil_version = int(&data, "version").and_then(|v| i32::try_from(v).ok());
    let data_version = version_id
        .or_else(|| int(&data, "DataVersion").and_then(|v| i32::try_from(v).ok()))
        .or(if version.is_none() && anvil_version == Some(LAST_ANVIL_FILE_VERSION) { Some(0) } else { None });
    Ok(LevelInfo {
        data_version,
        version_name: version.and_then(|v| v.get_str("Name")).map(str::to_string),
        snapshot: version.and_then(|v| int(v, "Snapshot")).is_some_and(|s| s != 0),
        level_name: data.get_str("LevelName").map(str::to_string),
        anvil_version,
        last_played: int(&data, "LastPlayed"),
        data,
    })
}

/// Read and parse a `level.dat` file.
pub fn read_level_dat(path: &Path) -> Result<LevelInfo, LevelDatError> {
    let len = fs::metadata(path).map_err(LevelDatError::Io)?.len();
    if len > MAX_LEVEL_DAT_BYTES {
        return Err(LevelDatError::TooLarge(len));
    }
    parse_level_dat(&fs::read(path).map_err(LevelDatError::Io)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::java::testutil::TestDir;

    fn level(entries: Vec<(&str, Tag)>) -> Compound {
        let mut c = Compound::new();
        for (k, v) in entries {
            c.insert(k, v);
        }
        c
    }

    fn wrapped(data: Compound) -> Vec<u8> {
        let root = level(vec![("Data", Tag::Compound(data))]);
        nbtio::gzip(&nbtio::to_vec("", &Tag::Compound(root)))
    }

    #[test]
    fn modern_level() {
        let version =
            level(vec![("Id", Tag::Int(3465)), ("Name", Tag::String("1.20.1".into())), ("Snapshot", Tag::Byte(0))]);
        let data = level(vec![
            ("LevelName", Tag::String("My World".into())),
            ("Version", Tag::Compound(version)),
            ("DataVersion", Tag::Int(3465)),
            ("version", Tag::Int(19133)),
            ("LastPlayed", Tag::Long(1_700_000_000_000)),
        ]);
        let info = parse_level_dat(&wrapped(data)).unwrap();
        assert_eq!(info.data_version, Some(3465));
        assert_eq!(info.version_name.as_deref(), Some("1.20.1"));
        assert_eq!(info.level_name.as_deref(), Some("My World"));
        assert_eq!(info.game_version(), Some(Version::new(1, 20, 1)));
        assert_eq!(info.last_played, Some(1_700_000_000_000));
        assert!(!info.snapshot);
    }

    #[test]
    fn old_level_without_version_compound() {
        let ok = level(vec![("version", Tag::Int(19133)), ("LevelName", Tag::String("Old".into()))]);
        let info = parse_level_dat(&wrapped(ok)).unwrap();
        assert_eq!(info.data_version, Some(0));
        assert_eq!(info.game_version(), Some(Version::new(1, 8, 8)));
        // Any other Anvil number (or none) is not a level this port reads.
        let other = level(vec![("version", Tag::Int(19132))]);
        assert_eq!(parse_level_dat(&wrapped(other)).unwrap().data_version, None);
        let none = level(vec![("LevelName", Tag::String("x".into()))]);
        assert_eq!(parse_level_dat(&wrapped(none)).unwrap().data_version, None);
    }

    #[test]
    fn data_version_without_version_compound() {
        // 26.x levels: no usable Version compound, DataVersion is at the top of Data.
        let data = level(vec![("DataVersion", Tag::Int(5017)), ("version", Tag::Int(19133))]);
        assert_eq!(parse_level_dat(&wrapped(data)).unwrap().data_version, Some(5017));
    }

    #[test]
    fn unwrapping_and_encodings() {
        // 26.1 split files: {DataVersion, data: {...}}
        let inner = level(vec![("LevelName", Tag::String("Split".into()))]);
        let root = level(vec![("DataVersion", Tag::Int(4786)), ("data", Tag::Compound(inner))]);
        let unwrapped = unwrap_data(root);
        assert_eq!(unwrapped.get_str("LevelName"), Some("Split"));
        // A root without wrappers stays as it is.
        let plain = level(vec![("a", Tag::Int(1))]);
        assert_eq!(unwrap_data(plain.clone()), plain);
        // Raw and zlib NBT are accepted too.
        let data = level(vec![("LevelName", Tag::String("Raw".into())), ("version", Tag::Int(19133))]);
        let raw = nbtio::to_vec("", &Tag::Compound(level(vec![("Data", Tag::Compound(data))])));
        assert_eq!(parse_level_dat(&raw).unwrap().level_name.as_deref(), Some("Raw"));
        assert_eq!(parse_level_dat(&nbtio::zlib(&raw)).unwrap().level_name.as_deref(), Some("Raw"));
    }

    #[test]
    fn short_and_long_widths_are_accepted() {
        let version = level(vec![("Id", Tag::Short(922))]);
        let data = level(vec![("Version", Tag::Compound(version))]);
        assert_eq!(parse_level_dat(&wrapped(data)).unwrap().data_version, Some(922));
    }

    #[test]
    fn errors_not_panics() {
        assert!(parse_level_dat(b"").is_err());
        assert!(parse_level_dat(b"not nbt at all").is_err());
        assert!(parse_level_dat(&nbtio::gzip(b"\x0a\x00")).is_err());
        let int_root = nbtio::gzip(&nbtio::to_vec("", &Tag::Int(4)));
        assert!(matches!(parse_level_dat(&int_root), Err(LevelDatError::Nbt(_))));
        let dir = TestDir::new("leveldat");
        assert!(matches!(read_level_dat(&dir.path().join("level.dat")), Err(LevelDatError::Io(_))));
        let data = level(vec![("version", Tag::Int(19133))]);
        std::fs::write(dir.path().join("level.dat"), wrapped(data)).unwrap();
        assert_eq!(read_level_dat(&dir.path().join("level.dat")).unwrap().data_version, Some(0));
    }
}
