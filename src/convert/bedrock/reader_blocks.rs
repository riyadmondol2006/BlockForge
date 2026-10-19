// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! From palette entries to model blocks, and from sub-chunks to model sections.
//!
//! A palette entry `{name, states, version}` becomes a [`BlockRef`] in three steps, each cached:
//!
//! 1. the NBT states become a [`StateMap`] (`Byte` is a boolean that is true only for 1, `Int` and `String` as they
//!    are) plus the pseudo state `waterlogged` that Chunker adds to every Bedrock identifier;
//! 2. the mapping engine turns `(name, states)` into a pivot block (a type and states named after Chunker's
//!    constants), or knows nothing about it;
//! 3. the pivot block becomes a [`BlockIdentifier`]; blocks the engine does not know but whose namespace is not
//!    `minecraft:` pass through as custom blocks when custom identifiers are allowed, everything else is counted as
//!    unmapped and becomes air (Chunker's `logMissingMapping` + `AIR`).
//!
//! The cache is keyed by a 128 bit hash of the entry, so a repeated entry (the common case: the same dozen palette
//! entries over thousands of sections) costs a hash and a lookup, not a resolution. Resolved blocks are interned, so
//! equal blocks share one allocation and palette keys can be compared by pointer.

use std::collections::HashMap;
use std::hash::Hasher;
use std::sync::{Arc, Mutex, RwLock};

use crate::convert::bedrock::palette::{BlockStateRaw, PaletteData, RawSubChunk, STORAGE_SIZE};
use crate::convert::mapping::{BlockResolver, Edition, PivotBlock, StateMap, StateValue, Version};
use crate::convert::model::identifier::StateValue as ModelStateValue;
use crate::convert::model::{
    BlockIdentifier, BlockKind, BlockPalette, BlockRef, BlockType, Chunk, Palette, StateId, StateSet, air,
};
use crate::convert::nbt::{Compound, Tag};

/// The Bedrock block resolver of one world: tables of the world's version, the cache and the unmapped counters.
pub(super) struct BlockReader {
    resolver: BlockResolver,
    legacy: std::sync::OnceLock<BlockResolver>,
    version: Version,
    custom_allowed: bool,
    cache: RwLock<HashMap<u128, BlockRef>>,
    interned: Mutex<HashMap<BlockIdentifier, BlockRef>>,
    /// Block names (or `name[states]` for known names with unknown states) that had no mapping, with their counts.
    unmapped: Mutex<HashMap<String, u64>>,
    /// Pivot states the model does not know (`type.STATE=VALUE`), with their counts.
    dropped_states: Mutex<HashMap<String, u64>>,
}

/// A 128 bit hash of a palette entry: name, states in stored order, schema-independent, plus the waterlogged flag.
fn entry_hash(name: &str, states: &Compound, val: Option<i16>, waterlogged: bool) -> u128 {
    fn one(seed: u8, name: &str, states: &Compound, val: Option<i16>, waterlogged: bool) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        h.write_u8(seed);
        h.write_u8(waterlogged as u8);
        h.write_i32(val.map_or(-1, i32::from));
        h.write(name.as_bytes());
        h.write_u8(0);
        for (k, v) in states.iter() {
            h.write(k.as_bytes());
            h.write_u8(0);
            hash_tag(&mut h, v);
        }
        h.finish()
    }
    (one(1, name, states, val, waterlogged) as u128) << 64 | one(2, name, states, val, waterlogged) as u128
}

fn hash_tag(h: &mut impl Hasher, tag: &Tag) {
    match tag {
        Tag::Byte(v) => {
            h.write_u8(1);
            h.write_i8(*v)
        }
        Tag::Short(v) => {
            h.write_u8(2);
            h.write_i16(*v)
        }
        Tag::Int(v) => {
            h.write_u8(3);
            h.write_i32(*v)
        }
        Tag::Long(v) => {
            h.write_u8(4);
            h.write_i64(*v)
        }
        Tag::String(s) => {
            h.write_u8(8);
            h.write(s.as_bytes());
            h.write_u8(0)
        }
        // States are bytes, ints and strings; anything else only needs to hash to *something*.
        _ => h.write_u8(255),
    }
}

/// The NBT states of a palette entry as engine input (`StateValue.fromNBT`), plus `waterlogged`. Tags that cannot
/// be a state value (lists, floats) are skipped; shorts and longs count as ints.
pub(super) fn state_map(states: &Compound, waterlogged: bool) -> StateMap {
    let mut map = StateMap::new();
    map.insert(Arc::from("waterlogged"), StateValue::Bool(waterlogged));
    for (k, v) in states.iter() {
        let value = match v {
            Tag::Byte(b) => StateValue::Bool(*b == 1),
            Tag::Int(i) => StateValue::Int(*i),
            Tag::Short(i) => StateValue::Int(*i as i32),
            Tag::Long(i) => StateValue::Int(*i as i32),
            Tag::String(s) => StateValue::Str(Arc::from(s.as_str())),
            _ => continue,
        };
        map.insert(Arc::from(k.as_str()), value);
    }
    map
}

fn model_state_value(v: &StateValue) -> ModelStateValue {
    match v {
        StateValue::Bool(b) => ModelStateValue::Bool(*b),
        StateValue::Int(i) => ModelStateValue::Int(*i),
        StateValue::Str(s) => ModelStateValue::Str(s.clone()),
    }
}

impl BlockReader {
    /// Resolver for a world last opened with `version`.
    pub fn new(version: Version, custom_allowed: bool) -> BlockReader {
        BlockReader {
            resolver: BlockResolver::new(Edition::Bedrock, version),
            legacy: std::sync::OnceLock::new(),
            version,
            custom_allowed,
            cache: RwLock::default(),
            interned: Mutex::default(),
            unmapped: Mutex::default(),
            dropped_states: Mutex::default(),
        }
    }

    /// The block for a palette entry (`readBlock`): air when there is no mapping.
    pub fn resolve(&self, raw: &BlockStateRaw, waterlogged: bool) -> BlockRef {
        let key = entry_hash(&raw.name, &raw.states, raw.val, waterlogged);
        if let Ok(cache) = self.cache.read() {
            if let Some(hit) = cache.get(&key) {
                return hit.clone();
            }
        }
        let block = self.resolve_uncached(raw, waterlogged);
        let block = self.intern(block);
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(key, block.clone());
        }
        block
    }

    fn intern(&self, block: BlockIdentifier) -> BlockRef {
        let Ok(mut interned) = self.interned.lock() else { return Arc::new(block) };
        if let Some(existing) = interned.get(&block) {
            return existing.clone();
        }
        let shared = Arc::new(block.clone());
        interned.insert(block, shared.clone());
        shared
    }

    fn resolve_uncached(&self, raw: &BlockStateRaw, waterlogged: bool) -> BlockIdentifier {
        let mut states = state_map(&raw.states, waterlogged);
        // 1.12 style entries `{name, val}` carry a numeric data value and go through the legacy table.
        if let (Some(val), true) = (raw.val, raw.states.is_empty()) {
            states.insert(Arc::from("data"), StateValue::Int(val as i32));
            let legacy = self.legacy.get_or_init(|| BlockResolver::new_legacy(Edition::Bedrock, self.version));
            if let Some(block) = legacy.to_pivot(&raw.name, &states).and_then(|p| self.pivot_to_block(&p)) {
                return block;
            }
            states.remove("data");
        }
        if let Some(pivot) = self.resolver.to_pivot(&raw.name, &states) {
            if let Some(block) = self.pivot_to_block(&pivot) {
                return block;
            }
        } else if self.custom_allowed && !raw.name.starts_with("minecraft:") && raw.name.contains(':') {
            return BlockIdentifier::custom(
                &raw.name,
                states.iter().map(|(k, v)| (k.to_string(), model_state_value(v))),
            );
        }
        self.count_unmapped(raw, &states);
        BlockIdentifier::new(BlockType::AIR)
    }

    fn count_unmapped(&self, raw: &BlockStateRaw, states: &StateMap) {
        let label = if self.resolver.is_supported_id(&raw.name) {
            let shown: Vec<String> =
                states.iter().filter(|(k, _)| &***k != "waterlogged").map(|(k, v)| format!("{k}={v}")).collect();
            format!("{}[{}]", raw.name, shown.join(","))
        } else {
            raw.name.clone()
        };
        if let Ok(mut unmapped) = self.unmapped.lock() {
            *unmapped.entry(label).or_insert(0) += 1;
        }
    }

    /// A pivot block as a model block: the type by Chunker's constant name and every state by constant name and
    /// value name. A state the model does not have is dropped and counted; an unknown type is `None`.
    pub fn pivot_to_block(&self, pivot: &PivotBlock) -> Option<BlockIdentifier> {
        let ty = BlockType::from_name(&pivot.ty)?;
        let mut states = StateSet::new();
        for (name, value) in &pivot.states {
            let known = StateId::from_name(name).and_then(|id| id.value_ordinal(value).map(|v| (id, v)));
            match known {
                Some((id, ordinal)) => states.set_raw(id, ordinal),
                None => {
                    if let Ok(mut dropped) = self.dropped_states.lock() {
                        *dropped.entry(format!("{}.{name}={value}", pivot.ty)).or_insert(0) += 1;
                    }
                }
            }
        }
        Some(BlockIdentifier::from_parts(BlockKind::Vanilla(ty), states, None))
    }

    /// Blocks without a mapping, with how often they were met.
    pub fn unmapped(&self) -> HashMap<String, u64> {
        self.unmapped.lock().map(|m| m.clone()).unwrap_or_default()
    }

    /// States of mapped blocks that the model could not hold.
    pub fn dropped_states(&self) -> HashMap<String, u64> {
        self.dropped_states.lock().map(|m| m.clone()).unwrap_or_default()
    }

    /// The number of cached entries (tests).
    #[cfg(test)]
    pub fn cached(&self) -> usize {
        self.cache.read().map_or(0, |c| c.len())
    }
}

/// Position of storage index `i` (`x * 256 + z * 16 + y`) in the model's order (`(y * 16 + z) * 16 + x`).
#[inline]
fn model_index(i: usize) -> usize {
    let (x, z, y) = (i >> 8, (i >> 4) & 15, i & 15);
    (y << 8) | (z << 4) | x
}

/// Builds the key list of a palette while the voxels are visited: keys are unique (pointer equality works because
/// blocks are interned) and only those that a voxel uses are kept.
struct KeyBuilder {
    keys: Vec<BlockRef>,
}

impl KeyBuilder {
    fn new() -> KeyBuilder {
        KeyBuilder { keys: Vec::new() }
    }

    fn index_of(&mut self, block: &BlockRef) -> u16 {
        if let Some(i) = self.keys.iter().position(|k| Arc::ptr_eq(k, block) || **k == **block) {
            return i as u16;
        }
        self.keys.push(block.clone());
        (self.keys.len() - 1) as u16
    }
}

/// The model palette of a block layer, with the block of each entry from `entry_block` and optionally the
/// waterlogged flag of each voxel.
fn layer_palette(
    layer: &PaletteData<BlockStateRaw>,
    water: Option<&[bool]>,
    entry_block: &mut dyn FnMut(&BlockStateRaw, bool) -> BlockRef,
) -> BlockPalette {
    if layer.is_inherit() || layer.entries.is_empty() {
        return Palette::Empty;
    }
    let mut builder = KeyBuilder::new();
    let mut out = vec![0u16; STORAGE_SIZE];
    // (entry, waterlogged) -> key index, resolved lazily.
    let mut lookup: Vec<[Option<u16>; 2]> = vec![[None; 2]; layer.entries.len()];
    let mut air_key: Option<u16> = None;
    for (i, &entry) in layer.indices.iter().enumerate().take(STORAGE_SIZE) {
        let entry = entry as usize;
        let wl = water.is_some_and(|w| w[i]) as usize;
        let key = match lookup.get_mut(entry) {
            Some(slot) => match slot[wl] {
                Some(k) => k,
                None => {
                    let block = entry_block(&layer.entries[entry], wl == 1);
                    let k = builder.index_of(&block);
                    slot[wl] = Some(k);
                    k
                }
            },
            // An index outside the palette reads as air.
            None => *air_key.get_or_insert_with(|| builder.index_of(air())),
        };
        out[model_index(i)] = key;
    }
    // Drop the keys no voxel uses (entries that have no voxel).
    let mut used = vec![false; builder.keys.len()];
    for &k in &out {
        used[k as usize] = true;
    }
    if used.iter().filter(|u| **u).count() == 1 {
        let k = used.iter().position(|u| *u).unwrap_or(0);
        return Palette::Single(builder.keys[k].clone());
    }
    let mut remap = vec![0u16; builder.keys.len()];
    let mut keys = Vec::new();
    for (i, key) in builder.keys.iter().enumerate() {
        if used[i] {
            remap[i] = keys.len() as u16;
            keys.push(key.clone());
        }
    }
    for v in &mut out {
        *v = remap[*v as usize];
    }
    Palette::from_parts(keys, &out)
}

/// Build the model section of a decoded sub-chunk (`BedrockChunkReader.readLayers`): layer 0 holds the blocks and a
/// later layer the liquids, whose water marks the voxels of layer 0 as waterlogged.
pub(super) fn build_chunk(blocks: &BlockReader, sub: &RawSubChunk, y: i8) -> Chunk {
    let Some(layer0) = sub.layers.first() else { return Chunk::new(y) };
    // Chunker keeps the last layer after the first as the liquid layer.
    let liquid = if sub.layers.len() > 1 { sub.layers.last() } else { None };
    let mut water: Option<Vec<bool>> = None;
    if let Some(liquid) = liquid.filter(|l| !l.is_inherit() && !l.entries.is_empty()) {
        let is_water: Vec<bool> =
            liquid.entries.iter().map(|e| blocks.resolve(e, false).block_type() == Some(BlockType::WATER)).collect();
        if is_water.iter().any(|w| *w) {
            let mut flags = vec![false; STORAGE_SIZE];
            for (i, &idx) in liquid.indices.iter().enumerate().take(STORAGE_SIZE) {
                flags[i] = is_water.get(idx as usize).copied().unwrap_or(false);
            }
            water = Some(flags);
        }
    }
    let palette = layer_palette(layer0, water.as_deref(), &mut |raw, wl| blocks.resolve(raw, wl));
    Chunk::with_blocks(y, palette)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::bedrock::palette::RawPalette;

    fn raw(name: &str, states: &[(&str, Tag)]) -> BlockStateRaw {
        let mut c = Compound::new();
        for (k, v) in states {
            c.insert(*k, v.clone());
        }
        BlockStateRaw::new(name, c, 18168865)
    }

    fn reader() -> BlockReader {
        BlockReader::new(Version::new(1, 26, 50), true)
    }

    fn sub(layers: Vec<RawPalette>) -> RawSubChunk {
        RawSubChunk { version: 9, y: 2, layers }
    }

    #[test]
    fn index_transposition() {
        // storage (x=3, z=5, y=7) lands at the model's (x=3, y=7, z=5).
        let i = 3 * 256 + 5 * 16 + 7;
        assert_eq!(model_index(i), Palette::<(), 16>::index(3, 7, 5));
        let mut seen = vec![false; STORAGE_SIZE];
        for i in 0..STORAGE_SIZE {
            seen[model_index(i)] = true;
        }
        assert!(seen.iter().all(|s| *s));
    }

    #[test]
    fn states_become_a_state_map() {
        let mut c = Compound::new();
        c.insert("lit_bit", Tag::Byte(1));
        c.insert("off", Tag::Byte(0));
        c.insert("weird", Tag::Byte(2));
        c.insert("n", Tag::Int(7));
        c.insert("s", Tag::String("north".into()));
        c.insert("skipped", Tag::Float(1.0));
        let m = state_map(&c, true);
        assert_eq!(m.get("waterlogged"), Some(&StateValue::Bool(true)));
        assert_eq!(m.get("lit_bit"), Some(&StateValue::Bool(true)));
        assert_eq!(m.get("off"), Some(&StateValue::Bool(false)));
        // Chunker: a byte is true only for exactly 1.
        assert_eq!(m.get("weird"), Some(&StateValue::Bool(false)));
        assert_eq!(m.get("n"), Some(&StateValue::Int(7)));
        assert_eq!(m.get("s"), Some(&StateValue::str("north")));
        assert!(m.get("skipped").is_none());
    }

    #[test]
    fn known_blocks_resolve_and_are_cached() {
        let r = reader();
        let a = r.resolve(&raw("minecraft:stone", &[]), false);
        assert_eq!(a.block_type(), Some(BlockType::STONE));
        let b = r.resolve(&raw("minecraft:stone", &[]), false);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(r.cached(), 1);
        let air_block = r.resolve(&raw("minecraft:air", &[]), false);
        assert!(air_block.is_air());
        // Waterlogged is part of the identity.
        let w = r.resolve(&raw("minecraft:stone", &[]), true);
        assert_ne!(*a, *w);
        assert!(r.unmapped().is_empty());
    }

    #[test]
    fn unknown_blocks_count_and_become_air() {
        let r = reader();
        let b = r.resolve(&raw("minecraft:not_a_block", &[]), false);
        assert!(b.is_air());
        let _ = r.resolve(&raw("minecraft:not_a_block", &[("x", Tag::Int(1))]), false);
        let _ = r.resolve(&raw("minecraft:not_a_block", &[("x", Tag::Int(2))]), false);
        // Two different entries, both counted under the name.
        assert_eq!(r.unmapped().get("minecraft:not_a_block"), Some(&3));
    }

    #[test]
    fn custom_blocks_pass_through_when_allowed() {
        let r = reader();
        let b = r.resolve(&raw("mymod:glow_rock", &[("power", Tag::Int(3))]), false);
        assert_eq!(b.kind(), &BlockKind::Custom(Arc::from("mymod:glow_rock")));
        assert_eq!(b.states().get_custom("power"), Some(&ModelStateValue::Int(3)));
        assert!(r.unmapped().is_empty());
        let off = BlockReader::new(Version::new(1, 26, 50), false);
        assert!(off.resolve(&raw("mymod:glow_rock", &[]), false).is_air());
        assert_eq!(off.unmapped().get("mymod:glow_rock"), Some(&1));
        // minecraft: names never pass through.
        assert!(r.resolve(&raw("minecraft:glow_rock", &[]), false).is_air());
    }

    #[test]
    fn uniform_sections_become_single() {
        let r = reader();
        let layer = RawPalette::uniform(raw("minecraft:stone", &[]));
        let chunk = build_chunk(&r, &sub(vec![layer]), 2);
        assert_eq!(chunk.y, 2);
        assert!(matches!(&chunk.blocks, Palette::Single(b) if b.block_type() == Some(BlockType::STONE)));
        let empty = build_chunk(&r, &sub(vec![]), -1);
        assert!(empty.is_empty());
    }

    #[test]
    fn voxels_land_in_model_order() {
        let r = reader();
        let mut idx = vec![0u16; STORAGE_SIZE];
        idx[3 * 256 + 5 * 16 + 7] = 1; // x 3, z 5, y 7
        idx[15 * 256 + 15 * 16 + 15] = 2;
        let layer = RawPalette::from_indices(
            vec![raw("minecraft:air", &[]), raw("minecraft:stone", &[]), raw("minecraft:dirt", &[])],
            idx,
        )
        .unwrap();
        let chunk = build_chunk(&r, &sub(vec![layer]), 0);
        assert_eq!(chunk.blocks.get(3, 7, 5).unwrap().block_type(), Some(BlockType::STONE));
        assert_eq!(chunk.blocks.get(15, 15, 15).unwrap().block_type(), Some(BlockType::DIRT));
        assert!(chunk.blocks.get(3, 5, 7).unwrap().is_air());
        assert_eq!(chunk.blocks.key_count(), 3);
    }

    #[test]
    fn unused_and_duplicate_entries_are_merged() {
        let r = reader();
        // Two entries that both map to air (one unknown) and one that is never used.
        let mut idx = vec![0u16; STORAGE_SIZE];
        idx[10] = 1;
        let layer = RawPalette::from_indices(
            vec![raw("minecraft:air", &[]), raw("minecraft:unknown_thing", &[]), raw("minecraft:stone", &[])],
            idx,
        )
        .unwrap();
        let chunk = build_chunk(&r, &sub(vec![layer]), 0);
        // The unused stone entry is gone; the two airs (table air and the unmapped fallback) stay distinct keys.
        assert_eq!(chunk.blocks.key_count(), 2);
        assert!(chunk.is_empty());
        assert_eq!(r.unmapped().get("minecraft:unknown_thing"), Some(&1));
    }

    #[test]
    fn water_layer_waterlogs_blocks() {
        let r = reader();
        let mut idx = vec![0u16; STORAGE_SIZE];
        idx[0] = 1; // x0 z0 y0: stone
        idx[1] = 1; // x0 z0 y1: stone
        let blocks =
            RawPalette::from_indices(vec![raw("minecraft:air", &[]), raw("minecraft:stone", &[])], idx).unwrap();
        let mut widx = vec![0u16; STORAGE_SIZE];
        widx[1] = 1; // only y1 is waterlogged
        widx[2] = 1; // water in the air above
        let liquid = RawPalette::from_indices(
            vec![raw("minecraft:air", &[]), raw("minecraft:water", &[("liquid_depth", Tag::Int(0))])],
            widx,
        )
        .unwrap();
        let chunk = build_chunk(&r, &sub(vec![blocks, liquid]), 0);
        let dry = chunk.blocks.get(0, 0, 0).unwrap();
        let wet = chunk.blocks.get(0, 1, 0).unwrap();
        assert_eq!(dry.block_type(), Some(BlockType::STONE));
        assert_eq!(wet.block_type(), Some(BlockType::STONE));
        assert_ne!(**dry, **wet);
        use crate::convert::model::{Bool, states};
        assert_eq!(dry.get(states::WATERLOGGED), Some(Bool::FALSE));
        assert_eq!(wet.get(states::WATERLOGGED), Some(Bool::TRUE));
        // Water over air waterlogs the air.
        assert!(chunk.blocks.get(0, 2, 0).unwrap().is_air());
    }

    #[test]
    fn a_liquid_layer_without_water_changes_nothing() {
        let r = reader();
        let blocks = RawPalette::uniform(raw("minecraft:stone", &[]));
        let lava = RawPalette::uniform(raw("minecraft:lava", &[("liquid_depth", Tag::Int(0))]));
        let with = build_chunk(&r, &sub(vec![blocks.clone(), lava]), 0);
        let without = build_chunk(&r, &sub(vec![blocks]), 0);
        assert_eq!(with.blocks, without.blocks);
    }

    #[test]
    fn out_of_range_indices_read_as_air() {
        let r = reader();
        let mut layer = RawPalette::uniform(raw("minecraft:stone", &[]));
        layer.indices[100] = 7;
        let chunk = build_chunk(&r, &sub(vec![layer]), 0);
        assert!(chunk.blocks.get(0, 4, 6).unwrap().is_air());
        assert_eq!(chunk.blocks.get(0, 5, 6).unwrap().block_type(), Some(BlockType::STONE));
        assert_eq!(chunk.blocks.key_count(), 2);
    }
}
