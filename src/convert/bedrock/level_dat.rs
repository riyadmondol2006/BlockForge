// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The Bedrock `level.dat` container and the few fields the format choice depends on.
//!
//! The file is `i32 LE storage version`, `i32 LE payload length` and then an uncompressed little-endian NBT root.

use super::data_version::{BedrockVersion, GameVersion};
use super::{BedrockError, Result, nbt_le};
use crate::convert::nbt::{Compound, Tag};
use std::path::Path;

/// A parsed `level.dat`.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelDat {
    /// The header's storage version (8, 9 or 10).
    pub storage_version: i32,
    /// The root tag's name (empty in worlds written by the game).
    pub root_name: String,
    pub root: Compound,
}

/// What `level.dat` says about the version and name of the world.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LevelDatInfo {
    /// `LevelName`.
    pub level_name: Option<String>,
    /// `lastOpenedWithVersion`: the game that last saved the world (`[1, 26, 51, 1, 0]`).
    pub last_opened_with: Option<Vec<i32>>,
    /// `MinimumCompatibleClientVersion`.
    pub minimum_compatible: Option<Vec<i32>>,
    /// The `StorageVersion` tag (not the header's).
    pub storage_version_tag: Option<i32>,
    /// `NetworkVersion` (the protocol).
    pub network_version: Option<i32>,
    /// `InventoryVersion` (a string such as `1.26.33`).
    pub inventory_version: Option<String>,
}

fn int_list(c: &Compound, key: &str) -> Option<Vec<i32>> {
    match c.get(key)? {
        Tag::List(items) => {
            let v: Vec<i32> = items.iter().filter_map(|t| if let Tag::Int(i) = t { Some(*i) } else { None }).collect();
            (v.len() == items.len() && !v.is_empty()).then_some(v)
        }
        _ => None,
    }
}

impl LevelDat {
    /// Parse the bytes of a `level.dat`.
    pub fn from_bytes(bytes: &[u8]) -> Result<LevelDat> {
        if bytes.len() < 9 {
            return Err(BedrockError::Format("level.dat is shorter than its header".into()));
        }
        let storage_version = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let declared = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if !(1..=64).contains(&storage_version) {
            return Err(BedrockError::Format(format!("level.dat storage version {storage_version} is not plausible")));
        }
        let body = &bytes[8..];
        // The declared length should match; if it is shorter the extra bytes are ignored, if longer it is a cut file.
        let payload = match usize::try_from(declared) {
            Ok(n) if n <= body.len() && n > 0 => &body[..n],
            Ok(n) if n > body.len() => {
                return Err(BedrockError::Format(format!("level.dat declares {n} bytes but has {}", body.len())));
            }
            _ => body,
        };
        let (root_name, tag) = nbt_le::from_slice(payload)?;
        match tag {
            Tag::Compound(root) => Ok(LevelDat { storage_version, root_name, root }),
            _ => Err(BedrockError::Format("level.dat root is not a compound".into())),
        }
    }

    /// Read `level.dat` from disk.
    pub fn read(path: &Path) -> Result<LevelDat> {
        LevelDat::from_bytes(&std::fs::read(path)?)
    }

    /// Encode the container.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let payload = nbt_le::to_vec(&self.root_name, &Tag::Compound(self.root.clone()))?;
        let len = i32::try_from(payload.len()).map_err(|_| BedrockError::Format("level.dat too large".into()))?;
        let mut out = Vec::with_capacity(payload.len() + 8);
        out.extend_from_slice(&self.storage_version.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&payload);
        Ok(out)
    }

    /// Write `level.dat` to disk.
    pub fn write(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_bytes()?).map_err(Into::into)
    }

    /// The version and name fields.
    pub fn info(&self) -> LevelDatInfo {
        let r = &self.root;
        LevelDatInfo {
            level_name: r.get_str("LevelName").map(str::to_string),
            last_opened_with: int_list(r, "lastOpenedWithVersion"),
            minimum_compatible: int_list(r, "MinimumCompatibleClientVersion"),
            storage_version_tag: r.get_i32("StorageVersion"),
            network_version: r.get_i32("NetworkVersion"),
            inventory_version: r.get_str("InventoryVersion").map(str::to_string),
        }
    }

    /// The game version Chunker would pick (`BedrockDataVersion.detect`): `lastOpenedWithVersion`, else
    /// `MinimumCompatibleClientVersion`, else nothing (the caller then assumes the oldest).
    pub fn game_version(&self) -> Option<GameVersion> {
        let info = self.info();
        let v = info.last_opened_with.or(info.minimum_compatible)?;
        (v.len() >= 3).then(|| [v[0], v[1], v[2]])
    }

    /// The registered version for this world: the nearest at or below its game version, the oldest if it has none.
    pub fn bedrock_version(&self) -> &'static BedrockVersion {
        match self.game_version() {
            Some(g) => BedrockVersion::nearest_at_or_below(g),
            None => BedrockVersion::oldest(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LevelDat {
        let mut root = Compound::new();
        root.insert("LevelName", Tag::String("Test World".into()));
        root.insert("StorageVersion", Tag::Int(10));
        root.insert("NetworkVersion", Tag::Int(1001));
        root.insert("RandomSeed", Tag::Long(1045923935239469310));
        root.insert("lastOpenedWithVersion", Tag::List(vec![1, 26, 51, 1, 0].into_iter().map(Tag::Int).collect()));
        root.insert("MinimumCompatibleClientVersion", Tag::List(vec![1, 26, 50, 0, 0].into_iter().map(Tag::Int).collect()));
        LevelDat { storage_version: 10, root_name: String::new(), root }
    }

    #[test]
    fn container_round_trip() {
        let l = sample();
        let bytes = l.to_bytes().unwrap();
        assert_eq!(&bytes[..4], &10i32.to_le_bytes());
        assert_eq!(&bytes[4..8], &((bytes.len() - 8) as i32).to_le_bytes());
        let back = LevelDat::from_bytes(&bytes).unwrap();
        assert_eq!(back, l);
        assert_eq!(back.to_bytes().unwrap(), bytes);
    }

    #[test]
    fn version_fields() {
        let l = sample();
        let info = l.info();
        assert_eq!(info.level_name.as_deref(), Some("Test World"));
        assert_eq!(info.last_opened_with, Some(vec![1, 26, 51, 1, 0]));
        assert_eq!((info.storage_version_tag, info.network_version), (Some(10), Some(1001)));
        assert_eq!(l.game_version(), Some([1, 26, 51]));
        assert_eq!(l.bedrock_version().game, [1, 26, 50]);
        // falls back to the minimum compatible client version
        let mut l2 = l.clone();
        l2.root.remove("lastOpenedWithVersion");
        assert_eq!(l2.game_version(), Some([1, 26, 50]));
        // and to the oldest version
        l2.root.remove("MinimumCompatibleClientVersion");
        assert_eq!(l2.game_version(), None);
        assert_eq!(l2.bedrock_version().game, [1, 12, 0]);
    }

    #[test]
    fn bad_files_are_errors() {
        let bytes = sample().to_bytes().unwrap();
        assert!(LevelDat::from_bytes(&[]).is_err());
        assert!(LevelDat::from_bytes(&bytes[..8]).is_err());
        assert!(LevelDat::from_bytes(&bytes[..bytes.len() - 10]).is_err());
        // gzip magic (a Java level.dat) has an implausible storage version
        assert!(LevelDat::from_bytes(&[0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 0]).is_err());
        let mut wrong = bytes.clone();
        wrong[8] = 9; // root is not a compound
        assert!(LevelDat::from_bytes(&wrong).is_err());
    }
}
