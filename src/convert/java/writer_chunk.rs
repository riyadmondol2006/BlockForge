// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! A model column as the chunk NBT of a Java version (`JavaColumnWriter`, `JavaChunkWriter` and the `v1_13` ...
//! `v1_18` overrides): sections with packed block palettes, biomes, block entities and entities.
//!
//! What is written (1.13 and later):
//! - 1.18+: a flat root `{DataVersion, xPos, zPos, yPos, Status, LastUpdate, InhabitedTime, isLightOn, sections,
//!   block_entities}`; each section `{Y, block_states{palette, data}, biomes{palette[, data]}}`.
//! - 1.13 - 1.17: the same inside `Level` with `Sections`/`TileEntities`, `Palette`/`BlockStates` in the section and
//!   biomes as one int array per column (1.13, 1.14: 256 entries, 1.15+: 4x4x4 cells).
//! - Heightmaps and light are left out on purpose: the game recomputes both (`isLightOn` is 0, like a column that
//!   has never been lit). Chunker does the same.
//! - Entities (`resolvers::entity`): from 1.17 in their own chunk for `entities/`
//!   (`{DataVersion, Position, Entities}`), before that as `Level.Entities`.

use super::writer_biomes::BiomeWriter;
use super::writer_blocks::{BlockWriter, JavaBlock};
use super::{ItemsImpl, write_block_entity_impl};
use crate::convert::java::format::{BiomeKind, JavaFormat, LightFlag, SectionLayout};
use crate::convert::java::palette::{BIOMES_PER_SECTION, BLOCKS_PER_SECTION, pack_palette};
use crate::convert::java::region::entities_root;
use crate::convert::java::resolvers::entity::{EntityContext, write_entity};
use crate::convert::java::resolvers::itemstack::ItemHooks;
use crate::convert::level::{ConversionSettings, Dimension};
use crate::convert::model::{Biome, Biomes, BlockPalette, CellPalette, Chunk, Column, EntitySource, Palette};
use crate::convert::nbt::{Compound, Tag};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

/// Everything the encoder needs; shared by all columns of a world.
pub struct EncodeContext<'a> {
    pub format: &'a JavaFormat,
    pub blocks: &'a BlockWriter,
    pub biomes: &'a BiomeWriter,
    pub items: &'a ItemsImpl,
    /// The world's item hooks (map ids, block entity data of items); pass them to every item written.
    pub hooks: &'a dyn ItemHooks,
    pub settings: &'a ConversionSettings,
    /// The source world's lowest section is -4 (Chunker's `CavesAndCliffs` setting): biome sections are offset by 4.
    pub source_from_minus_64: bool,
    /// 1.17 only: the height pack is installed, so the overworld has sections -4 to 19.
    pub tall_overworld: bool,
    /// The reader's item and block resolvers, for the entities kept as source NBT.
    pub entity_source: Option<&'a dyn EntitySource>,
}

/// What happened while encoding a column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColumnStats {
    pub sections: usize,
    /// Sections outside the versions's height range.
    pub dropped_sections: usize,
    pub block_entities: usize,
    /// Block entities on air or on blocks without a Java form, outside the height range, or with no handler.
    pub dropped_block_entities: usize,
    pub entities: usize,
    /// Entities the target version has no type for (or that lost what they are made of, like an item entity
    /// without an item).
    pub dropped_entities: usize,
}

/// A column ready to be compressed.
pub struct EncodedColumn {
    /// The chunk root compound (including `DataVersion`).
    pub root: Compound,
    /// The `entities/` chunk root (1.17+), when the column has entities.
    pub entities: Option<Compound>,
    pub stats: ColumnStats,
}

fn byte(v: i8) -> Tag {
    Tag::Byte(v)
}

/// Compact a palette (drop unused keys, merge equal ones) while translating its keys.
struct Compacted<K> {
    keys: Vec<K>,
    /// One index per voxel into `keys`; `None` when every voxel is key 0.
    indices: Option<Vec<u16>>,
    /// How many voxels used each old key, and (last entry) how many pointed outside the palette.
    uses: Vec<u32>,
}

/// `mapped[i]` is the translation of old key `i`; voxels pointing outside the keys read `fallback`.
fn compact<K: Clone + Eq + Hash>(
    mapped: &[K],
    fallback: &K,
    old_indices: Option<Vec<u16>>,
    volume: usize,
) -> Compacted<K> {
    let mut uses = vec![0u32; mapped.len() + 1];
    let Some(old) = old_indices else {
        uses[0] = if mapped.is_empty() { 0 } else { volume as u32 };
        return Compacted { keys: vec![mapped.first().unwrap_or(fallback).clone()], indices: None, uses };
    };
    let mut keys: Vec<K> = Vec::new();
    let mut positions: HashMap<K, u16> = HashMap::new();
    let mut remap: Vec<u16> = vec![u16::MAX; mapped.len() + 1];
    let mut out = Vec::with_capacity(old.len());
    for &i in &old {
        let slot = (i as usize).min(mapped.len());
        uses[slot] += 1;
        let mut new = remap[slot];
        if new == u16::MAX {
            let key = mapped.get(slot).unwrap_or(fallback);
            new = *positions.entry(key.clone()).or_insert_with(|| {
                keys.push(key.clone());
                (keys.len() - 1) as u16
            });
            remap[slot] = new;
        }
        out.push(new);
    }
    if keys.len() == 1 {
        return Compacted { keys, indices: None, uses };
    }
    Compacted { keys, indices: Some(out), uses }
}

fn longs(values: Vec<i64>) -> Tag {
    Tag::LongArray(values)
}

/// `block_states` / `Palette` + `BlockStates` of one section.
fn encode_blocks(ctx: &EncodeContext<'_>, palette: &BlockPalette, out: &mut Compound) {
    let fmt = ctx.format;
    let renamed = fmt.renamed_block_state_fields;
    let air = ctx.blocks.air().clone();
    let (keys, indices): (&[crate::convert::model::BlockRef], Option<Vec<u16>>) = match palette {
        Palette::Empty => (&[], None),
        Palette::Single(_) => (palette.keys(), None),
        Palette::Packed { keys, indices } => (keys, Some(indices.to_vec())),
    };
    let resolved: Vec<Option<Arc<JavaBlock>>> = keys.iter().map(|k| ctx.blocks.resolve(k)).collect();
    let mapped: Vec<Arc<JavaBlock>> = resolved.iter().map(|r| r.clone().unwrap_or_else(|| air.clone())).collect();
    let packed = compact(&mapped, &air, indices, BLOCKS_PER_SECTION);
    for (i, r) in resolved.iter().enumerate() {
        if r.is_none() && packed.uses[i] > 0 {
            ctx.blocks.note_unmapped(&keys[i], packed.uses[i] as u64);
        }
    }
    let list = Tag::List(packed.keys.iter().map(|b| Tag::Compound(b.to_nbt(renamed))).collect());
    let data = pack_palette(
        fmt.palette_layout,
        fmt.min_bits_blocks,
        BLOCKS_PER_SECTION,
        packed.keys.len(),
        packed.indices.as_deref(),
    )
    .unwrap_or_default();
    match fmt.sections {
        SectionLayout::NestedStates => {
            let mut states = Compound::with_capacity(2);
            states.insert("palette", list);
            states.insert("data", longs(data));
            out.insert("block_states", Tag::Compound(states));
        }
        _ => {
            out.insert("Palette", list);
            out.insert("BlockStates", longs(data));
        }
    }
}

/// The `biomes` compound of one section (1.18+).
fn encode_section_biomes(ctx: &EncodeContext<'_>, cells: &CellPalette, fallback: &str) -> Compound {
    let fmt = ctx.format;
    let fallback_name: Arc<str> = Arc::from(fallback);
    let (keys, indices): (&[Biome], Option<Vec<u16>>) = match cells {
        Palette::Empty => (&[], None),
        Palette::Single(_) => (cells.keys(), None),
        Palette::Packed { keys, indices } => (keys, Some(indices.to_vec())),
    };
    let mapped: Vec<Arc<str>> = keys.iter().map(|b| ctx.biomes.name(b, fallback)).collect();
    let packed = compact(&mapped, &fallback_name, indices, BIOMES_PER_SECTION);
    let mut out = Compound::with_capacity(2);
    out.insert("palette", Tag::List(packed.keys.iter().map(|n| Tag::String(n.to_string())).collect()));
    if packed.keys.len() > 1 {
        let data = pack_palette(
            fmt.palette_layout,
            fmt.min_bits_biomes,
            BIOMES_PER_SECTION,
            packed.keys.len(),
            packed.indices.as_deref(),
        )
        .unwrap_or_default();
        out.insert("data", longs(data));
    }
    out
}

/// `Some(biomes)` when the model's biome data is well-formed enough to read without indexing past its end.
fn usable_biomes(biomes: &Option<Biomes>) -> Option<&Biomes> {
    match biomes {
        Some(b @ Biomes::Cluster(v)) if v.len() >= 1024 => Some(b),
        Some(b @ Biomes::Column(v)) if v.len() >= 256 => Some(b),
        Some(b @ (Biomes::Palette(_) | Biomes::ClusterPalette(_))) => Some(b),
        _ => None,
    }
}

/// The int array of a pre-1.18 column.
fn encode_legacy_biomes(ctx: &EncodeContext<'_>, biomes: &Biomes, fallback: &str, sections: i32, first: i32) -> Tag {
    let offset = if ctx.source_from_minus_64 { 4 } else { 0 };
    let fb = Biome::from_identifier(fallback);
    match ctx.format.biomes {
        BiomeKind::IntArray1024 => {
            let mut out: Vec<i32> = Vec::with_capacity(sections as usize * 64);
            for section in first..first + sections {
                let cells = biomes.as_4x4_palette(section + offset);
                for i in 0..64usize {
                    let (x, y, z) = (i & 3, i >> 4, (i >> 2) & 3);
                    let biome = cells.get_or(x, y, z, &fb);
                    out.push(ctx.biomes.id(biome, fallback));
                }
            }
            Tag::IntArray(out)
        }
        _ => {
            // One entry per column (1.13, 1.14): the cell at sea level.
            let cells = biomes.as_4x4_palette(4 + offset);
            let mut out = Vec::with_capacity(256);
            for i in 0..256usize {
                let (x, z) = (i & 15, i >> 4);
                out.push(ctx.biomes.id(cells.get_or(x >> 2, 0, z >> 2, &fb), fallback));
            }
            Tag::IntArray(out)
        }
    }
}

fn status(fmt: &JavaFormat) -> String {
    let status = fmt.chunk_status.unwrap_or("full");
    if fmt.sections == SectionLayout::NestedStates { format!("minecraft:{status}") } else { status.to_string() }
}

/// Whether section `y` can be written for this dimension and version.
fn section_supported(ctx: &EncodeContext<'_>, dim: &Dimension, y: i8) -> bool {
    let fmt = ctx.format;
    if !fmt.extended_height {
        return (0..16).contains(&y);
    }
    if fmt.sections != SectionLayout::NestedStates && !(ctx.tall_overworld && dim.identifier() == "minecraft:overworld")
    {
        // 1.17 without the height pack is 256 high.
        return (0..16).contains(&y);
    }
    true
}

/// The section range biome padding fills (`getDimensionBiomeHeight`).
fn padded_range(dim: &Dimension) -> (i32, i32) {
    match dim.identifier() {
        "minecraft:the_nether" | "minecraft:the_end" => (0, 15),
        _ => (-4, 19),
    }
}

/// The sections written so far.
#[derive(Default)]
struct SectionsOut {
    list: Vec<Tag>,
    lowest: Option<i8>,
    highest: Option<i8>,
}

fn push_section(
    ctx: &EncodeContext<'_>,
    biomes: Option<&Biomes>,
    fallback: &str,
    out: &mut SectionsOut,
    chunk: Option<&Chunk>,
    y: i8,
    stats: &mut ColumnStats,
) {
    let mut section = Compound::with_capacity(4);
    section.insert("Y", byte(y));
    match chunk {
        Some(chunk) => encode_blocks(ctx, &chunk.blocks, &mut section),
        None => encode_blocks(ctx, &Palette::Single(crate::convert::model::air().clone()), &mut section),
    }
    if ctx.format.sections == SectionLayout::NestedStates {
        let offset = if ctx.source_from_minus_64 { 4 } else { 0 };
        let cells = match biomes {
            Some(b) => b.as_4x4_palette(y as i32 + offset),
            None => Palette::Empty,
        };
        section.insert("biomes", Tag::Compound(encode_section_biomes(ctx, &cells, fallback)));
    }
    out.lowest = Some(out.lowest.map_or(y, |l| l.min(y)));
    out.highest = Some(out.highest.map_or(y, |h| h.max(y)));
    out.list.push(Tag::Compound(section));
    stats.sections += 1;
}

/// Write a model column as the chunk NBT of the target version.
pub fn encode_column(ctx: &EncodeContext<'_>, dim: &Dimension, column: &Column) -> EncodedColumn {
    let fmt = ctx.format;
    let nested = fmt.sections == SectionLayout::NestedStates;
    let mut stats = ColumnStats::default();
    let fallback = dim.fallback_biome();
    let biomes = usable_biomes(&column.biomes);

    // ---- sections ----
    let mut out = SectionsOut::default();
    for (&y, chunk) in &column.chunks {
        if !section_supported(ctx, dim, y) {
            stats.dropped_sections += 1;
            continue;
        }
        if ctx.settings.discard_empty_chunks && chunk.is_empty() {
            continue;
        }
        push_section(ctx, biomes, fallback, &mut out, Some(chunk), y, &mut stats);
    }
    // Sections above the last one get an air section so their biomes do not blend with the default (Chunker's
    // `preventYBiomeBlending`).
    if nested && ctx.settings.prevent_y_biome_blending && !column.chunks.is_empty() {
        let (bottom, top) = padded_range(dim);
        let last = out.highest.map_or(bottom - 1, i32::from);
        for y in (last + 1)..=top {
            if let Ok(y) = i8::try_from(y) {
                push_section(ctx, biomes, fallback, &mut out, None, y, &mut stats);
            }
        }
    }
    let (sections, lowest) = (out.list, out.lowest);

    // ---- block entities ----
    let (min_y, max_y) = if fmt.extended_height { (i32::MIN, i32::MAX) } else { (0, 255) };
    let mut block_entities: Vec<Tag> = Vec::new();
    for be in &column.block_entities {
        stats.block_entities += 1;
        if be.y < min_y || be.y > max_y || !column.position.is_inside(be.x, be.z) {
            stats.dropped_block_entities += 1;
            continue;
        }
        // A block entity on air (or on a block that became air) is not written.
        let block = column.block(be.x, be.y, be.z);
        if block.is_air() || ctx.blocks.resolve(block).is_none_or(|b| b.is_air()) {
            stats.dropped_block_entities += 1;
            continue;
        }
        let compound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            write_block_entity_impl(ctx.items, ctx.format, be, ctx.hooks)
        }))
        .unwrap_or(None);
        match compound {
            Some(c) => block_entities.push(Tag::Compound(c)),
            None => stats.dropped_block_entities += 1,
        }
    }

    // ---- entities ----
    let entities = encode_entities(ctx, column, &mut stats);

    // ---- the column compound ----
    let mut level = Compound::with_capacity(13);
    if nested {
        level.insert("DataVersion", Tag::Int(fmt.row_data_version));
    }
    level.insert("xPos", Tag::Int(column.position.x));
    level.insert("zPos", Tag::Int(column.position.z));
    if fmt.writes_y_pos {
        level.insert("yPos", Tag::Int(lowest.map_or(0, i32::from)));
    }
    level.insert("LastUpdate", Tag::Long(0));
    level.insert("InhabitedTime", Tag::Long(0));
    level.insert("Status", Tag::String(status(fmt)));
    // The game lights the column when it loads it.
    level.insert(
        match fmt.light_flag {
            LightFlag::IsLightOn => "isLightOn",
            LightFlag::LightPopulated => "LightPopulated",
        },
        byte(0),
    );
    if !nested {
        if let Some(b) = biomes {
            let (first, count) =
                if fmt.extended_height && ctx.tall_overworld && dim.identifier() == "minecraft:overworld" {
                    (-4, 24)
                } else {
                    (0, 16)
                };
            level.insert("Biomes", encode_legacy_biomes(ctx, b, fallback, count, first));
        }
    }
    level.insert(fmt.sections_key, Tag::List(sections));
    level.insert(fmt.block_entities_key, Tag::List(block_entities));
    let entities = if fmt.entities_separate {
        (!entities.is_empty())
            .then(|| entities_root(fmt.row_data_version, column.position.x, column.position.z, Tag::List(entities)))
    } else {
        level.insert("Entities", Tag::List(entities));
        None
    };

    let root = if fmt.level_wrapper {
        let mut root = Compound::with_capacity(2);
        root.insert("DataVersion", Tag::Int(fmt.row_data_version));
        root.insert("Level", Tag::Compound(level));
        root
    } else {
        level
    };
    EncodedColumn { root, entities, stats }
}

/// The entity compounds of a column (a failing entity is dropped, not the column).
fn encode_entities(ctx: &EncodeContext<'_>, column: &Column, stats: &mut ColumnStats) -> Vec<Tag> {
    if !ctx.settings.process_entities || column.entities.is_empty() {
        return Vec::new();
    }
    let renamed = ctx.format.renamed_block_state_fields;
    let block_state = |block: &crate::convert::model::BlockIdentifier| {
        let java = ctx.blocks.resolve(&Arc::new(block.clone()))?;
        (!java.is_air()).then(|| java.to_nbt(renamed))
    };
    let entity_ctx = EntityContext {
        items: ctx.items,
        format: ctx.format,
        hooks: ctx.hooks,
        source: ctx.entity_source,
        block_state: &block_state,
        // the account the world is converted for owns the host's pets, unless the pets are given to another one
        pet_owner: ctx.settings.pet_owner_uuid.or(ctx.settings.player.owner_uuid),
    };
    let mut out = Vec::with_capacity(column.entities.len());
    for entity in &column.entities {
        stats.entities += 1;
        let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| write_entity(&entity_ctx, entity)))
            .unwrap_or(None);
        match written {
            Some(c) => out.push(Tag::Compound(c)),
            None => stats.dropped_entities += 1,
        }
    }
    out
}
