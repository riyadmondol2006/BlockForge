// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! LevelDB keys of a Bedrock world: building, parsing and finding the chunk columns.
//!
//! Chunk keys are little-endian: `i32 x, i32 z, [i32 dimension], u8 tag, [i8 sub-chunk y]`. The dimension is left out
//! for the overworld, so chunk keys are 9/10 bytes long in the overworld and 13/14 bytes long elsewhere.

use super::db::BedrockDb;
use super::{BedrockError, Result, nbt_le};
use crate::convert::nbt::{Compound, Tag};
use std::collections::BTreeSet;

pub const ACTOR_PREFIX: &[u8] = b"actorprefix";
pub const BIOME_IDS_TABLE: &[u8] = b"BiomeIdsTable";
pub const DIGP_PREFIX: &[u8] = b"digp";
pub const DIMENSION_NAME_ID_TABLE: &[u8] = b"DimensionNameIdTable";
pub const LOCAL_PLAYER: &[u8] = b"~local_player";
pub const MAP_PREFIX: &[u8] = b"map_";
pub const PORTALS: &[u8] = b"portals";
pub const POS_TRACK_DB: &[u8] = b"PosTrackDB-0x";
pub const POS_TRACK_DB_LAST_ID: &[u8] = b"PositionTrackDB-LastId";
/// Keys real worlds have that Chunker ignores.
pub const AUTONOMOUS_ENTITIES: &[u8] = b"AutonomousEntities";
pub const OVERWORLD_LIMBO: &[u8] = b"Overworld";
pub const WORLD_CLOCKS: &[u8] = b"WorldClocks";
pub const BIOME_DATA: &[u8] = b"BiomeData";
pub const LEVEL_CHUNK_META_DATA_DICTIONARY: &[u8] = b"LevelChunkMetaDataDictionary";

/// The id of custom dimensions starts here (`DimensionRegistry.BEDROCK_CUSTOM_DIMENSION_ID_START`).
pub const CUSTOM_DIMENSION_ID_START: i32 = 1000;

/// A Bedrock dimension id as it is written in keys and NBT.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DimensionId(pub i32);

impl DimensionId {
    pub const OVERWORLD: DimensionId = DimensionId(0);
    pub const NETHER: DimensionId = DimensionId(1);
    pub const END: DimensionId = DimensionId(2);

    /// Overworld, nether or end.
    pub fn is_vanilla(self) -> bool {
        (0..=2).contains(&self.0)
    }

    /// A custom dimension (registered in `DimensionNameIdTable`).
    pub fn is_custom(self) -> bool {
        self.0 >= CUSTOM_DIMENSION_ID_START
    }

    /// The vanilla identifier (`minecraft:overworld` ...), if this is a vanilla dimension.
    pub fn vanilla_identifier(self) -> Option<&'static str> {
        match self.0 {
            0 => Some("minecraft:overworld"),
            1 => Some("minecraft:the_nether"),
            2 => Some("minecraft:the_end"),
            _ => None,
        }
    }
}

/// The record types a chunk can have (`LevelDBChunkType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ChunkTag {
    Data3D = 43,
    Version = 44,
    Data2D = 45,
    Data2DLegacy = 46,
    SubChunkPrefix = 47,
    LegacyTerrain = 48,
    BlockEntity = 49,
    Entity = 50,
    PendingTicks = 51,
    LegacyBlockExtraData = 52,
    BiomeState = 53,
    FinalizedState = 54,
    ConversionData = 55,
    BorderBlocks = 56,
    HardcodedSpawners = 57,
    RandomTicks = 58,
    Checksums = 59,
    GenerationSeed = 60,
    GeneratedPreCavesAndCliffsBlending = 61,
    BlendingBiomeHeight = 62,
    MetaDataHash = 63,
    BlendingData = 64,
    ActorDigestVersion = 65,
    LegacyVersion = 118,
}

impl ChunkTag {
    /// Every tag, in id order.
    pub const ALL: [ChunkTag; 24] = [
        ChunkTag::Data3D,
        ChunkTag::Version,
        ChunkTag::Data2D,
        ChunkTag::Data2DLegacy,
        ChunkTag::SubChunkPrefix,
        ChunkTag::LegacyTerrain,
        ChunkTag::BlockEntity,
        ChunkTag::Entity,
        ChunkTag::PendingTicks,
        ChunkTag::LegacyBlockExtraData,
        ChunkTag::BiomeState,
        ChunkTag::FinalizedState,
        ChunkTag::ConversionData,
        ChunkTag::BorderBlocks,
        ChunkTag::HardcodedSpawners,
        ChunkTag::RandomTicks,
        ChunkTag::Checksums,
        ChunkTag::GenerationSeed,
        ChunkTag::GeneratedPreCavesAndCliffsBlending,
        ChunkTag::BlendingBiomeHeight,
        ChunkTag::MetaDataHash,
        ChunkTag::BlendingData,
        ChunkTag::ActorDigestVersion,
        ChunkTag::LegacyVersion,
    ];

    /// The tag for a key byte, if it is a known one.
    pub fn from_byte(b: u8) -> Option<ChunkTag> {
        ChunkTag::ALL.iter().copied().find(|t| *t as u8 == b)
    }

    /// The byte written in keys.
    pub fn byte(self) -> u8 {
        self as u8
    }
}

/// A parsed chunk key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkKey {
    pub x: i32,
    pub z: i32,
    pub dim: DimensionId,
    pub tag: ChunkTag,
    /// The sub-chunk y index, only for [`ChunkTag::SubChunkPrefix`].
    pub sub_y: Option<i8>,
}

fn le_i32(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl ChunkKey {
    /// A column-level key (no sub-chunk index).
    pub fn new(dim: DimensionId, x: i32, z: i32, tag: ChunkTag) -> ChunkKey {
        ChunkKey { x, z, dim, tag, sub_y: None }
    }

    /// A sub-chunk key.
    pub fn sub_chunk(dim: DimensionId, x: i32, z: i32, y: i8) -> ChunkKey {
        ChunkKey { x, z, dim, tag: ChunkTag::SubChunkPrefix, sub_y: Some(y) }
    }

    /// Encode the key.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(14);
        v.extend_from_slice(&self.x.to_le_bytes());
        v.extend_from_slice(&self.z.to_le_bytes());
        if self.dim != DimensionId::OVERWORLD {
            v.extend_from_slice(&self.dim.0.to_le_bytes());
        }
        v.push(self.tag.byte());
        if let Some(y) = self.sub_y {
            v.push(y as u8);
        }
        v
    }

    /// Decode a chunk key. `None` for anything that is not one (other keys, unknown tags, wrong lengths).
    pub fn parse(key: &[u8]) -> Option<ChunkKey> {
        let len = key.len();
        if !matches!(len, 9 | 10 | 13 | 14) || !is_chunk_shaped(key) {
            return None;
        }
        let sub = len == 10 || len == 14;
        let dim = if len >= 13 { DimensionId(le_i32(key, 8)) } else { DimensionId::OVERWORLD };
        let tag_at = if sub { len - 2 } else { len - 1 };
        let tag = ChunkTag::from_byte(key[tag_at])?;
        // A sub-chunk index belongs to sub-chunk records only, and those always carry one.
        if sub != (tag == ChunkTag::SubChunkPrefix) {
            return None;
        }
        // The overworld never writes its id into the key.
        if len >= 13 && dim == DimensionId::OVERWORLD {
            return None;
        }
        Some(ChunkKey { x: le_i32(key, 0), z: le_i32(key, 4), dim, tag, sub_y: sub.then(|| key[len - 1] as i8) })
    }
}

/// The key is not one of the named non-chunk keys that can have a chunk-like length (`~local_player` is 13 bytes).
fn is_chunk_shaped(key: &[u8]) -> bool {
    !(key.starts_with(MAP_PREFIX)
        || key.starts_with(ACTOR_PREFIX)
        || key.starts_with(DIGP_PREFIX)
        || key == LOCAL_PLAYER)
}

/// `LevelDBKey.isColumnKey`: whether a key is used to discover chunk columns (a sub-chunk, Data2D, Data3D, Entity or
/// BlockEntity record).
pub fn is_column_key(key: &[u8]) -> bool {
    let len = key.len();
    if !matches!(len, 9 | 10 | 13 | 14) || !is_chunk_shaped(key) {
        return false;
    }
    let sub = len == 10 || len == 14;
    let tag = key[if sub { len - 2 } else { len - 1 }];
    if sub {
        return tag == ChunkTag::SubChunkPrefix.byte();
    }
    [ChunkTag::Data2D, ChunkTag::Data3D, ChunkTag::Entity, ChunkTag::BlockEntity].iter().any(|t| t.byte() == tag)
}

/// The key of a column-level record.
pub fn chunk_key(dim: DimensionId, x: i32, z: i32, tag: ChunkTag) -> Vec<u8> {
    ChunkKey::new(dim, x, z, tag).to_bytes()
}

/// The key of a sub-chunk record.
pub fn sub_chunk_key(dim: DimensionId, x: i32, z: i32, y: i8) -> Vec<u8> {
    ChunkKey::sub_chunk(dim, x, z, y).to_bytes()
}

/// `digp` + x + z [+ dimension]: the list of actor keys in a column.
pub fn digp_key(dim: DimensionId, x: i32, z: i32) -> Vec<u8> {
    let mut v = DIGP_PREFIX.to_vec();
    v.extend_from_slice(&x.to_le_bytes());
    v.extend_from_slice(&z.to_le_bytes());
    if dim != DimensionId::OVERWORLD {
        v.extend_from_slice(&dim.0.to_le_bytes());
    }
    v
}

/// Parse a `digp` key back into `(dimension, x, z)`.
pub fn parse_digp_key(key: &[u8]) -> Option<(DimensionId, i32, i32)> {
    let rest = key.strip_prefix(DIGP_PREFIX)?;
    match rest.len() {
        8 => Some((DimensionId::OVERWORLD, le_i32(rest, 0), le_i32(rest, 4))),
        12 => Some((DimensionId(le_i32(rest, 8)), le_i32(rest, 0), le_i32(rest, 4))),
        _ => None,
    }
}

/// `actorprefix` + the 8 byte storage key of one actor.
pub fn actor_key(storage_key: [u8; 8]) -> Vec<u8> {
    let mut v = ACTOR_PREFIX.to_vec();
    v.extend_from_slice(&storage_key);
    v
}

/// The storage key an actor is saved under (`ColumnWriter.generateStorageKeyForEntity`): the top 32 bits of the unique
/// id are flipped and `0x100000000` is added, written big-endian.
pub fn actor_storage_key(unique_id: i64) -> [u8; 8] {
    let key = 0x1_0000_0000i64.wrapping_add(unique_id ^ (0xFFFF_FFFF_0000_0000u64 as i64));
    key.to_be_bytes()
}

/// Split the value of a `digp` record into the 8 byte actor storage keys it lists (a trailing partial key is ignored).
pub fn parse_digp_value(value: &[u8]) -> Vec<[u8; 8]> {
    value.chunks_exact(8).map(|c| <[u8; 8]>::try_from(c).unwrap_or([0; 8])).collect()
}

/// The `map_<id>` key of a map.
pub fn map_key(id: i64) -> Vec<u8> {
    let mut v = MAP_PREFIX.to_vec();
    v.extend_from_slice(id.to_string().as_bytes());
    v
}

/// The `PosTrackDB-0x%08x` key of a lodestone tracking record.
pub fn pos_track_key(id: u32) -> Vec<u8> {
    format!("PosTrackDB-0x{id:08x}").into_bytes()
}

/// The position of a column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ColumnPos {
    pub dim: DimensionId,
    pub x: i32,
    pub z: i32,
}

/// The columns named by a list of keys, sorted by dimension, x, z and without duplicates.
pub fn columns_from_keys<'a>(keys: impl IntoIterator<Item = &'a [u8]>) -> Vec<ColumnPos> {
    let mut set = BTreeSet::new();
    for key in keys {
        if !is_column_key(key) {
            continue;
        }
        if let Some(k) = ChunkKey::parse(key) {
            set.insert(ColumnPos { dim: k.dim, x: k.x, z: k.z });
        }
    }
    set.into_iter().collect()
}

/// Every column that has any record at all (any chunk key or a `digp` key), a superset of [`columns_from_keys`]: it
/// includes columns that only hold a version byte, a finalized state or an actor digest.
pub fn all_columns_from_keys<'a>(keys: impl IntoIterator<Item = &'a [u8]>) -> Vec<ColumnPos> {
    let mut set = BTreeSet::new();
    for key in keys {
        if let Some(k) = ChunkKey::parse(key) {
            set.insert(ColumnPos { dim: k.dim, x: k.x, z: k.z });
        } else if let Some((dim, x, z)) = parse_digp_key(key) {
            set.insert(ColumnPos { dim, x, z });
        }
    }
    set.into_iter().collect()
}

/// Every column of a database (all dimensions) by Chunker's rule ([`is_column_key`]); scans all keys once.
pub fn enumerate_columns(db: &mut dyn BedrockDb) -> Result<Vec<ColumnPos>> {
    let keys = db.iter_keys()?;
    Ok(columns_from_keys(keys.iter().map(|k| k.as_slice())))
}

/// The `DimensionNameIdTable` record: custom dimension identifiers and their ids.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DimensionTable {
    /// `(identifier, bedrock id)` in the order stored.
    pub entries: Vec<(String, i32)>,
}

impl DimensionTable {
    /// Parse the record value (`{entries: {name: int}}`). Only custom ids (>= 1000) are kept, like Chunker.
    pub fn from_value(value: &[u8]) -> Result<DimensionTable> {
        let (_, root) = nbt_le::from_slice(value)?;
        let Tag::Compound(root) = root else {
            return Err(BedrockError::Format("DimensionNameIdTable is not a compound".into()));
        };
        let mut entries = Vec::new();
        if let Some(e) = root.get_compound("entries") {
            for (name, tag) in &e.0 {
                if let Tag::Int(id) = tag {
                    if *id >= CUSTOM_DIMENSION_ID_START {
                        entries.push((name.clone(), *id));
                    }
                }
            }
        }
        Ok(DimensionTable { entries })
    }

    /// Encode the record value.
    pub fn to_value(&self) -> Result<Vec<u8>> {
        let mut entries = Compound::new();
        for (name, id) in &self.entries {
            entries.insert(name.clone(), Tag::Int(*id));
        }
        let mut root = Compound::new();
        root.insert("entries", Tag::Compound(entries));
        nbt_le::to_vec("", &Tag::Compound(root))
    }

    /// The id registered for an identifier.
    pub fn id_of(&self, identifier: &str) -> Option<DimensionId> {
        self.entries.iter().find(|(n, _)| n == identifier).map(|(_, id)| DimensionId(*id))
    }

    /// The identifier of a dimension id (vanilla ids resolve to the vanilla identifiers).
    pub fn identifier_of(&self, dim: DimensionId) -> Option<&str> {
        dim.vanilla_identifier().or_else(|| self.entries.iter().find(|(_, id)| *id == dim.0).map(|(n, _)| n.as_str()))
    }

    /// The next free custom id (`DimensionRegistry.getNextCustomBedrockID`).
    pub fn next_custom_id(&self) -> DimensionId {
        let mut id = CUSTOM_DIMENSION_ID_START;
        while self.entries.iter().any(|(_, e)| *e == id) {
            id += 1;
        }
        DimensionId(id)
    }

    /// Register an identifier, returning its id (the existing one if it is registered already).
    pub fn register(&mut self, identifier: &str) -> DimensionId {
        if let Some(id) = self.id_of(identifier) {
            return id;
        }
        if let Some(vanilla) = (0..=2).map(DimensionId).find(|d| d.vanilla_identifier() == Some(identifier)) {
            return vanilla;
        }
        let id = self.next_custom_id();
        self.entries.push((identifier.to_string(), id.0));
        id
    }
}

/// Read the dimension table of a database, if it has one.
pub fn read_dimension_table(db: &mut dyn BedrockDb) -> Result<Option<DimensionTable>> {
    match db.get(DIMENSION_NAME_ID_TABLE)? {
        Some(v) => DimensionTable::from_value(&v).map(Some),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::bedrock::db::MemDb;

    #[test]
    fn overworld_keys() {
        let k = chunk_key(DimensionId::OVERWORLD, -5, 22, ChunkTag::Data3D);
        assert_eq!(k, [0xFB, 0xFF, 0xFF, 0xFF, 22, 0, 0, 0, 43]);
        assert_eq!(ChunkKey::parse(&k), Some(ChunkKey::new(DimensionId::OVERWORLD, -5, 22, ChunkTag::Data3D)));
        let s = sub_chunk_key(DimensionId::OVERWORLD, 1, -2, -4);
        assert_eq!(s.len(), 10);
        assert_eq!(&s[8..], &[47, 0xFC]);
        assert_eq!(ChunkKey::parse(&s), Some(ChunkKey::sub_chunk(DimensionId::OVERWORLD, 1, -2, -4)));
    }

    #[test]
    fn other_dimensions() {
        let k = chunk_key(DimensionId::NETHER, 3, 4, ChunkTag::Version);
        assert_eq!(k.len(), 13);
        assert_eq!(&k[8..12], &1i32.to_le_bytes());
        assert_eq!(ChunkKey::parse(&k).unwrap().dim, DimensionId::NETHER);
        let s = sub_chunk_key(DimensionId(1005), 0, 0, 3);
        assert_eq!(s.len(), 14);
        let p = ChunkKey::parse(&s).unwrap();
        assert_eq!((p.dim, p.sub_y), (DimensionId(1005), Some(3)));
        assert_eq!(p.to_bytes(), s);
    }

    #[test]
    fn every_tag_round_trips() {
        for t in ChunkTag::ALL {
            assert_eq!(ChunkTag::from_byte(t.byte()), Some(t));
            if t != ChunkTag::SubChunkPrefix {
                let k = chunk_key(DimensionId::END, 7, -7, t);
                assert_eq!(ChunkKey::parse(&k).map(|p| p.tag), Some(t));
            }
        }
        assert_eq!(ChunkTag::from_byte(0), None);
        assert_eq!(ChunkTag::ALL.len(), 24);
    }

    #[test]
    fn non_chunk_keys_are_rejected() {
        for k in [&b"~local_player"[..], b"map_-1234", b"BiomeData", b"Overworld", b"portals", b"digp12345678"] {
            assert!(!is_column_key(k), "{k:?}");
            assert_eq!(ChunkKey::parse(k), None, "{k:?}");
        }
        assert!(!is_column_key(&[]));
        // PendingTicks and FinalizedState are chunk keys but do not discover columns.
        let k = chunk_key(DimensionId::OVERWORLD, 0, 0, ChunkTag::FinalizedState);
        assert!(!is_column_key(&k));
        assert!(ChunkKey::parse(&k).is_some());
        // A sub-chunk key whose tag is not SubChunkPrefix is not a chunk key.
        let mut bad = chunk_key(DimensionId::OVERWORLD, 0, 0, ChunkTag::Data3D);
        bad.push(0);
        assert_eq!(ChunkKey::parse(&bad), None);
        assert!(!is_column_key(&bad));
    }

    #[test]
    fn digp_and_actor_keys() {
        let k = digp_key(DimensionId::OVERWORLD, 1, 2);
        assert_eq!(k.len(), 12);
        assert_eq!(parse_digp_key(&k), Some((DimensionId::OVERWORLD, 1, 2)));
        let k = digp_key(DimensionId::END, -1, -2);
        assert_eq!(k.len(), 16);
        assert_eq!(parse_digp_key(&k), Some((DimensionId::END, -1, -2)));
        assert!(!is_column_key(&k));
        // unique id (0xFFFFFFFF << 32) | 5 -> storage key 0x1_0000_0000 + 5
        let uid = ((0xFFFF_FFFFu64 << 32) | 5) as i64;
        assert_eq!(actor_storage_key(uid), [0, 0, 0, 1, 0, 0, 0, 5]);
        let ak = actor_key([1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(ak.starts_with(b"actorprefix"));
        assert_eq!(parse_digp_value(&[1, 2, 3, 4, 5, 6, 7, 8, 9]), vec![[1, 2, 3, 4, 5, 6, 7, 8]]);
        assert_eq!(map_key(-17), b"map_-17");
        assert_eq!(pos_track_key(0x2a), b"PosTrackDB-0x0000002a");
    }

    #[test]
    fn column_enumeration() {
        let mut db = MemDb::new();
        for (x, z) in [(0, 0), (1, 0), (0, 0)] {
            db.put(&chunk_key(DimensionId::OVERWORLD, x, z, ChunkTag::Data3D), b"d").unwrap();
            db.put(&sub_chunk_key(DimensionId::OVERWORLD, x, z, 0), b"s").unwrap();
            db.put(&chunk_key(DimensionId::OVERWORLD, x, z, ChunkTag::Version), b"v").unwrap();
        }
        db.put(&sub_chunk_key(DimensionId::NETHER, 5, 5, 1), b"s").unwrap();
        db.put(b"~local_player", b"p").unwrap();
        db.put(b"map_5", b"m").unwrap();
        let cols = enumerate_columns(&mut db).unwrap();
        assert_eq!(
            cols,
            vec![
                ColumnPos { dim: DimensionId::OVERWORLD, x: 0, z: 0 },
                ColumnPos { dim: DimensionId::OVERWORLD, x: 1, z: 0 },
                ColumnPos { dim: DimensionId::NETHER, x: 5, z: 5 },
            ]
        );
    }

    #[test]
    fn dimension_table() {
        let mut t = DimensionTable::default();
        let a = t.register("mymod:sky");
        let b = t.register("mymod:cave");
        assert_eq!((a, b), (DimensionId(1000), DimensionId(1001)));
        assert_eq!(t.register("mymod:sky"), a);
        assert_eq!(t.register("minecraft:the_end"), DimensionId::END);
        assert_eq!(t.identifier_of(DimensionId::NETHER), Some("minecraft:the_nether"));
        assert_eq!(t.identifier_of(b), Some("mymod:cave"));
        let back = DimensionTable::from_value(&t.to_value().unwrap()).unwrap();
        assert_eq!(back, t);
        assert!(DimensionTable::from_value(&[1, 2, 3]).is_err());
    }
}
