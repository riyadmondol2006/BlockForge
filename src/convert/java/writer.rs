// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Writing the model as a Java world (`JavaWorldWriter`, `JavaColumnWriter`, `JavaChunkWriter` and the `v1_13` ...
//! `v26_3` writer overrides).
//!
//! ```text
//! let mut writer = JavaWorldWriter::create(out, Version::new(1, 21, 4), &level, &settings)?;
//! for column in columns { writer.write_column(&dimension, &column)?; }
//! writer.finish_dimension(&dimension)?;
//! writer.finish()?;                       // level.dat and the rest; `abort()` instead removes what was written
//! ```
//!
//! Columns are encoded, compressed and appended to their region file at once, so memory is one column plus a few
//! open files (see [`writer_region`]); `level.dat` is written last, so a world that is cut short has none and is
//! never mistaken for a finished one. The folder must be new or empty.
//!
//! Supported targets are 1.13 and later (the legacy numeric block ids of 1.12 and before are not written).

#[path = "writer_biomes.rs"]
pub(super) mod writer_biomes;
#[path = "writer_blocks.rs"]
mod writer_blocks;
#[path = "writer_chunk.rs"]
mod writer_chunk;
#[path = "writer_region.rs"]
mod writer_region;
#[cfg(test)]
#[path = "writer_tests.rs"]
mod writer_tests;

pub(super) use super::resolvers::itemstack::JavaItems as ItemsImpl;

/// The block entity compound for a model block entity, its items written with `hooks`.
///
pub(super) fn write_block_entity_impl(
    items: &ItemsImpl,
    format: &JavaFormat,
    be: &crate::convert::model::BlockEntity,
    hooks: &dyn ItemHooks,
) -> Option<crate::convert::nbt::Compound> {
    super::resolvers::block_entity::write_block_entity_with(items, format, be, hooks)
}

use super::dirs::{JavaDimension, LevelDirs};
use super::format::JavaFormat;
use super::nbtio;
use super::region::Compression;
use super::resolvers::hooks::WorldItemHooks;
use super::resolvers::itemstack::ItemHooks;
use crate::convert::level::{ConversionSettings, ConvertError, Dimension, Level, LevelPlayer};
use crate::convert::model::{BlockType, Column, ItemStack};
use crate::convert::nbt::{Compound, Tag};
use crate::convert::util::Version;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use writer_biomes::BiomeWriter;
use writer_blocks::BlockWriter;
use writer_chunk::{ColumnStats, EncodeContext, encode_column};
use writer_region::RegionSet;

/// Tunables of a [`JavaWorldWriter`].
#[derive(Clone, Debug)]
pub struct WriterOptions {
    /// Region files open at once; the least recently used is closed (and reopened if needed again).
    pub max_open_regions: usize,
    /// How chunks are compressed (zlib, like vanilla).
    pub compression: Compression,
}

impl Default for WriterOptions {
    fn default() -> Self {
        WriterOptions { max_open_regions: 16, compression: Compression::Zlib }
    }
}

/// What a finished (or running) writer did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriterSummary {
    pub columns: u64,
    pub sections: u64,
    /// Sections outside the height of the target version.
    pub dropped_sections: u64,
    pub block_entities: u64,
    pub dropped_block_entities: u64,
    pub entities: u64,
    /// Entities the target has no type for.
    pub dropped_entities: u64,
    /// Columns of dimensions the target version cannot hold (custom dimensions before 1.16, invalid names).
    pub skipped_columns: u64,
    /// Blocks written as air because the tables have no Java form for them, with the count of blocks.
    pub unmapped_blocks: Vec<(String, u64)>,
}

/// A column that is encoded and compressed but not yet in a region file.
pub struct PreparedColumn {
    region_dir: PathBuf,
    chunk_x: i32,
    chunk_z: i32,
    compression: Compression,
    data: Vec<u8>,
    /// The compressed `poi/` chunk when the column holds nether portal blocks.
    poi: Option<(PathBuf, Vec<u8>)>,
    /// The compressed `entities/` chunk (1.17+) when the column has entities.
    entities: Option<(PathBuf, Vec<u8>)>,
    stats: ColumnStats,
}

impl PreparedColumn {
    /// Size of the compressed chunk.
    pub fn compressed_len(&self) -> usize {
        self.data.len()
    }
}

/// A player other than the owner, written as the player file of `uuid` (`players/data/` from 26.1, `playerdata/`
/// before).
#[derive(Clone, Debug)]
pub struct GuestPlayer {
    pub uuid: uuid::Uuid,
    pub player: LevelPlayer,
    /// The inventory by slot, like [`JavaWorldWriter::set_player_items`].
    pub items: BTreeMap<i8, ItemStack>,
    pub ender_items: BTreeMap<i8, ItemStack>,
}

/// Writes a Java world folder.
pub struct JavaWorldWriter {
    out: PathBuf,
    /// The folder did not exist before: abort removes it, otherwise only its contents.
    created_root: bool,
    format: JavaFormat,
    dirs: LevelDirs,
    blocks: BlockWriter,
    biomes: BiomeWriter,
    items: ItemsImpl,
    /// Passed to every item the writer writes; see [`JavaWorldWriter::item_hooks`].
    item_hooks: WorldItemHooks,
    settings: ConversionSettings,
    level: Level,
    options: WriterOptions,
    regions: RegionSet,
    summary: WriterSummary,
    icon: Option<Vec<u8>>,
    player_items: BTreeMap<i8, ItemStack>,
    ender_items: BTreeMap<i8, ItemStack>,
    guests: Vec<GuestPlayer>,
    /// The reader's item and block resolvers, for entities kept as source NBT (Bedrock actors).
    entity_source: Option<std::sync::Arc<dyn crate::convert::model::EntitySource>>,
    dimensions_seen: Vec<Dimension>,
    source_from_minus_64: bool,
    tall_overworld: bool,
}

fn io_error(error: io::Error) -> ConvertError {
    ConvertError::Io(error)
}

impl JavaWorldWriter {
    /// Start a world in `out` for the Java version `target`. `out` must not exist or be an empty folder.
    pub fn create(
        out: &Path,
        target: Version,
        level: &Level,
        settings: &ConversionSettings,
    ) -> Result<JavaWorldWriter, ConvertError> {
        JavaWorldWriter::create_with(out, target, level, settings, WriterOptions::default())
    }

    /// [`JavaWorldWriter::create`] with explicit options.
    pub fn create_with(
        out: &Path,
        target: Version,
        level: &Level,
        settings: &ConversionSettings,
        options: WriterOptions,
    ) -> Result<JavaWorldWriter, ConvertError> {
        let format = JavaFormat::for_version(target);
        if format.legacy_ids {
            return Err(ConvertError::Unsupported(format!(
                "Java {target} (before 1.13) is not supported as a target, use 1.13 or later"
            )));
        }
        let created_root = check_output_folder(out)?;
        fs::create_dir_all(out).map_err(io_error)?;
        let tall_overworld =
            format.caves_and_cliffs == super::format::CavesAndCliffs::Datapack && level.settings.caves_and_cliffs;
        let items = ItemsImpl::new(&format, format.version);
        Ok(JavaWorldWriter {
            dirs: LevelDirs::new(out, &format),
            blocks: BlockWriter::new(format.version, settings.custom_identifiers),
            biomes: BiomeWriter::new(format.version, settings),
            item_hooks: WorldItemHooks::new(&items, &format, level.maps.len()),
            items,
            settings: settings.clone(),
            level: level.clone(),
            regions: RegionSet::new(options.max_open_regions),
            options,
            summary: WriterSummary::default(),
            icon: None,
            player_items: BTreeMap::new(),
            ender_items: BTreeMap::new(),
            guests: Vec::new(),
            entity_source: None,
            dimensions_seen: Vec::new(),
            source_from_minus_64: level.settings.caves_and_cliffs,
            tall_overworld,
            out: out.to_path_buf(),
            created_root,
            format,
        })
    }

    /// The format switches of the target.
    pub fn format(&self) -> &JavaFormat {
        &self.format
    }

    /// The world folder.
    pub fn path(&self) -> &Path {
        &self.out
    }

    /// A PNG or JPEG to become `icon.png` (resized to 64x64).
    pub fn set_icon(&mut self, image: Vec<u8>) {
        self.icon = Some(image);
    }

    /// The single-player inventory by slot (-106 off hand, 100..103 armor), written into the player data.
    pub fn set_player_items(&mut self, items: BTreeMap<i8, ItemStack>) {
        self.player_items = items;
    }

    /// The single player's ender chest by slot (0..26), written into the player data as `EnderItems`.
    pub fn set_ender_items(&mut self, items: BTreeMap<i8, ItemStack>) {
        self.ender_items = items;
    }

    /// Another player to write a player file for. A guest with the owner's uuid is not written.
    pub fn add_guest_player(&mut self, guest: GuestPlayer) {
        self.guests.retain(|g| g.uuid != guest.uuid);
        self.guests.push(guest);
    }

    /// The item writer of the target version.
    pub fn items(&self) -> &ItemsImpl {
        &self.items
    }

    /// The hooks every item of this world must be written with ([`ItemsImpl::write_with`]): they give `filled_map`
    /// items the id of their map and write the contents of shulker boxes and the like. Code that converts items
    /// outside of chunks' block entities and the player (entities) should use these too.
    pub fn item_hooks(&self) -> &WorldItemHooks {
        &self.item_hooks
    }

    /// The reader's item and block resolvers: without them the entities read from a Bedrock world (mobs, minecarts,
    /// paintings...) are not written. Item frames need none.
    pub fn set_entity_source(&mut self, source: std::sync::Arc<dyn crate::convert::model::EntitySource>) {
        self.entity_source = Some(source);
    }

    /// Blocks that had no Java form and were written as air (with how many), most frequent first.
    pub fn unmapped_blocks(&self) -> Vec<(String, u64)> {
        self.blocks.unmapped()
    }

    /// Counters so far.
    pub fn summary(&self) -> WriterSummary {
        let mut s = self.summary.clone();
        s.unmapped_blocks = self.blocks.unmapped();
        s
    }

    /// Whether columns of `dim` can be written for this target.
    pub fn supports_dimension(&self, dim: &Dimension) -> bool {
        self.java_dimension(dim).is_some()
    }

    /// How many times a closed region file had to be opened again (the least recently used ones are closed).
    pub fn reopened_regions(&self) -> usize {
        self.regions.reopened()
    }

    /// Region files open right now (at most [`WriterOptions::max_open_regions`]).
    pub fn open_regions(&self) -> usize {
        self.regions.open_count()
    }

    fn java_dimension(&self, dim: &Dimension) -> Option<JavaDimension> {
        let java = JavaDimension::from_identifier(dim.identifier())?;
        // Before 1.16 only the three vanilla dimensions exist.
        if matches!(java, JavaDimension::Custom(..)) && !self.format.world_gen_settings {
            return None;
        }
        Some(java)
    }

    /// Encode and compress a column without touching the disk; many columns can be prepared at once and written in
    /// any order with [`JavaWorldWriter::write_prepared`]. `Ok(None)` when the dimension cannot be written.
    pub fn prepare_column(&self, dim: &Dimension, column: &Column) -> Result<Option<PreparedColumn>, ConvertError> {
        let Some(java) = self.java_dimension(dim) else { return Ok(None) };
        let ctx = EncodeContext {
            format: &self.format,
            blocks: &self.blocks,
            biomes: &self.biomes,
            items: &self.items,
            hooks: &self.item_hooks,
            settings: &self.settings,
            source_from_minus_64: self.source_from_minus_64,
            tall_overworld: self.tall_overworld,
            entity_source: self.entity_source.as_deref(),
        };
        let encoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| encode_column(&ctx, dim, column)))
            .map_err(|_| {
                ConvertError::Internal(format!(
                    "encoding the column ({}, {}) failed unexpectedly",
                    column.position.x, column.position.z
                ))
            })?;
        let bytes = nbtio::to_vec("", &Tag::Compound(encoded.root));
        if bytes.is_empty() {
            return Err(ConvertError::Internal(format!(
                "the column ({}, {}) could not be serialized",
                column.position.x, column.position.z
            )));
        }
        let entities = encoded.entities.map(|root| {
            let bytes = nbtio::to_vec("", &Tag::Compound(root));
            (self.dirs.entities_dir(&java), self.options.compression.compress(&bytes))
        });
        Ok(Some(PreparedColumn {
            region_dir: self.dirs.region_dir(&java),
            chunk_x: column.position.x,
            chunk_z: column.position.z,
            compression: self.options.compression,
            data: self.options.compression.compress(&bytes),
            poi: self.poi_chunk(dim, &java, column),
            entities,
            stats: encoded.stats,
        }))
    }

    /// Nether portal records of a column (`writePOI`): every portal block above the frame bottoms listed in the level.
    fn poi_chunk(&self, dim: &Dimension, java: &JavaDimension, column: &Column) -> Option<(PathBuf, Vec<u8>)> {
        if self.level.portals.is_empty() || self.format.row_data_version < super::version::DV_1_14 {
            return None;
        }
        let chunk = crate::convert::level::ChunkPos::new(column.position.x, column.position.z);
        let mut sections: BTreeMap<i32, Vec<Tag>> = BTreeMap::new();
        for portal in self.level.portals.iter().filter(|p| p.dimension == *dim) {
            for (x, y0, z) in portal.horizontal_blocks(chunk) {
                let mut y = y0;
                // never taller than a world: a column whose lookups wrap round must not keep this going for ever
                let mut steps = 0;
                while steps < 512
                    && y < i32::MAX
                    && column.block(x, y, z).block_type() == Some(BlockType::NETHER_PORTAL)
                {
                    steps += 1;
                    let mut record = Compound::with_capacity(3);
                    record.insert("type", Tag::String("minecraft:nether_portal".into()));
                    record.insert("pos", Tag::IntArray(vec![x, y, z]));
                    record.insert("free_tickets", Tag::Int(0));
                    sections.entry(y >> 4).or_default().push(Tag::Compound(record));
                    y += 1;
                }
            }
        }
        if sections.is_empty() {
            return None;
        }
        let mut out = Compound::with_capacity(sections.len());
        for (y, records) in sections {
            let mut section = Compound::with_capacity(2);
            section.insert("Valid", Tag::Byte(1));
            section.insert("Records", Tag::List(records));
            out.insert(y.to_string(), Tag::Compound(section));
        }
        let mut root = Compound::with_capacity(2);
        root.insert("Sections", Tag::Compound(out));
        root.insert("DataVersion", Tag::Int(self.format.row_data_version));
        let bytes = nbtio::to_vec("", &Tag::Compound(root));
        Some((self.dirs.poi_dir(java), self.options.compression.compress(&bytes)))
    }

    /// Append a prepared column to its region file.
    pub fn write_prepared(&mut self, dim: &Dimension, prepared: PreparedColumn) -> Result<(), ConvertError> {
        self.regions
            .write_chunk(&prepared.region_dir, prepared.chunk_x, prepared.chunk_z, prepared.compression, &prepared.data)
            .map_err(io_error)?;
        for (dir, data) in prepared.poi.iter().chain(&prepared.entities) {
            self.regions
                .write_chunk(dir, prepared.chunk_x, prepared.chunk_z, prepared.compression, data)
                .map_err(io_error)?;
        }
        let s = &mut self.summary;
        s.columns += 1;
        s.sections += prepared.stats.sections as u64;
        s.dropped_sections += prepared.stats.dropped_sections as u64;
        s.block_entities += prepared.stats.block_entities as u64;
        s.dropped_block_entities += prepared.stats.dropped_block_entities as u64;
        s.entities += prepared.stats.entities as u64;
        s.dropped_entities += prepared.stats.dropped_entities as u64;
        if !self.dimensions_seen.contains(dim) {
            self.dimensions_seen.push(dim.clone());
        }
        Ok(())
    }

    /// Write one column. Columns can arrive in any order; each goes straight to its region file.
    pub fn write_column(&mut self, dim: &Dimension, column: &Column) -> Result<(), ConvertError> {
        match self.prepare_column(dim, column)? {
            Some(prepared) => self.write_prepared(dim, prepared),
            None => {
                self.summary.skipped_columns += 1;
                Ok(())
            }
        }
    }

    /// Close the region files of a dimension (all its columns are written).
    pub fn finish_dimension(&mut self, dim: &Dimension) -> Result<(), ConvertError> {
        let Some(java) = self.java_dimension(dim) else { return Ok(()) };
        self.regions.flush_dimension(&self.dirs.dimension_dir(&java)).map_err(io_error)
    }

    /// Write `level.dat` and the other level files, and close everything.
    pub fn finish(self) -> Result<(), ConvertError> {
        self.finish_with_summary().map(|_| ())
    }

    /// [`JavaWorldWriter::finish`] that also returns the counters. When the last files can't be written (a full
    /// disk), what was written is removed like [`JavaWorldWriter::abort`] does: a world without its `level.dat` is
    /// no use, and it would keep the folder from being used again ("not empty").
    pub fn finish_with_summary(self) -> Result<WriterSummary, ConvertError> {
        let mut this = self;
        match this.write_last_files() {
            Ok(summary) => Ok(summary),
            Err(e) => {
                this.abort();
                Err(e)
            }
        }
    }

    fn write_last_files(&mut self) -> Result<WriterSummary, ConvertError> {
        self.regions.flush().map_err(io_error)?;
        let summary = self.summary();
        super::level_writer::write_level_files(&super::level_writer::LevelFiles {
            format: &self.format,
            dirs: &self.dirs,
            level: &self.level,
            settings: &self.settings,
            items: &self.items,
            hooks: &self.item_hooks,
            biomes: &self.biomes,
            player_items: &self.player_items,
            ender_items: &self.ender_items,
            guests: &self.guests,
            icon: self.icon.as_deref(),
            dimensions: &self.dimensions_seen,
            tall_overworld: self.tall_overworld,
        })?;
        Ok(summary)
    }

    /// Give up: close the files and remove everything this writer put in the folder (or the folder itself when it
    /// created it). Safe to call after an error; the folder was empty when the writer started.
    pub fn abort(self) {
        let mut this = self;
        this.regions.discard();
        if this.created_root {
            let _ = fs::remove_dir_all(&this.out);
        } else if let Ok(entries) = fs::read_dir(&this.out) {
            for entry in entries.flatten() {
                let path = entry.path();
                let _ = if path.is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
            }
        }
    }
}

/// The output must not exist, or be an empty folder. Returns whether it does not exist yet.
fn check_output_folder(out: &Path) -> Result<bool, ConvertError> {
    match fs::metadata(out) {
        Ok(meta) if meta.is_dir() => {
            let mut entries = fs::read_dir(out).map_err(io_error)?;
            if entries.next().is_some() {
                return Err(ConvertError::Io(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} is not empty, choose a new folder", out.display()),
                )));
            }
            Ok(false)
        }
        Ok(_) => Err(ConvertError::Io(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} exists and is not a folder", out.display()),
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(io_error(e)),
    }
}
