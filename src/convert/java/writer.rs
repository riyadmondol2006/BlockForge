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

/// The block entity compound for a model block entity.
///
/// SWAP: `resolvers::block_entity::write_block_entity` still takes the private stand-in `JavaItems` of the be-java
/// branch, which cannot be built from here. Until that module uses `resolvers::itemstack::JavaItems`, this writes
/// only `{id, x, y, z}`; the whole body then becomes `write_block_entity(items, format, be)`.
pub(super) fn write_block_entity_impl(
    _items: &ItemsImpl,
    format: &JavaFormat,
    be: &crate::convert::model::BlockEntity,
) -> Option<crate::convert::nbt::Compound> {
    use crate::convert::nbt::{Compound, Tag};
    let id = super::resolvers::block_entity::java_block_entity_id(be.kind(), format);
    if id.is_empty() {
        return None;
    }
    let mut c = Compound::with_capacity(4);
    c.insert("id", Tag::String(id.to_string()));
    c.insert("x", Tag::Int(be.x));
    c.insert("y", Tag::Int(be.y));
    c.insert("z", Tag::Int(be.z));
    Some(c)
}

use super::dirs::{JavaDimension, LevelDirs};
use super::format::JavaFormat;
use super::nbtio;
use super::region::Compression;
use crate::convert::level::{ConversionSettings, ConvertError, Dimension, Level};
use crate::convert::model::Column;
use crate::convert::nbt::Tag;
use crate::convert::util::Version;
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
    stats: ColumnStats,
}

impl PreparedColumn {
    /// Size of the compressed chunk.
    pub fn compressed_len(&self) -> usize {
        self.data.len()
    }
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
    settings: ConversionSettings,
    level: Level,
    options: WriterOptions,
    regions: RegionSet,
    summary: WriterSummary,
    icon: Option<Vec<u8>>,
    player_items: std::collections::BTreeMap<i8, crate::convert::model::ItemStack>,
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
        Ok(JavaWorldWriter {
            dirs: LevelDirs::new(out, &format),
            blocks: BlockWriter::new(format.version, settings.custom_identifiers),
            biomes: BiomeWriter::new(format.version, settings),
            items: ItemsImpl::new(&format, format.version),
            settings: settings.clone(),
            level: level.clone(),
            regions: RegionSet::new(options.max_open_regions),
            options,
            summary: WriterSummary::default(),
            icon: None,
            player_items: std::collections::BTreeMap::new(),
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
    pub fn set_player_items(&mut self, items: std::collections::BTreeMap<i8, crate::convert::model::ItemStack>) {
        self.player_items = items;
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
            settings: &self.settings,
            source_from_minus_64: self.source_from_minus_64,
            tall_overworld: self.tall_overworld,
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
        Ok(Some(PreparedColumn {
            region_dir: self.dirs.region_dir(&java),
            chunk_x: column.position.x,
            chunk_z: column.position.z,
            compression: self.options.compression,
            data: self.options.compression.compress(&bytes),
            stats: encoded.stats,
        }))
    }

    /// Append a prepared column to its region file.
    pub fn write_prepared(&mut self, dim: &Dimension, prepared: PreparedColumn) -> Result<(), ConvertError> {
        self.regions
            .write_chunk(&prepared.region_dir, prepared.chunk_x, prepared.chunk_z, prepared.compression, &prepared.data)
            .map_err(io_error)?;
        let s = &mut self.summary;
        s.columns += 1;
        s.sections += prepared.stats.sections as u64;
        s.dropped_sections += prepared.stats.dropped_sections as u64;
        s.block_entities += prepared.stats.block_entities as u64;
        s.dropped_block_entities += prepared.stats.dropped_block_entities as u64;
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

    /// [`JavaWorldWriter::finish`] that also returns the counters.
    pub fn finish_with_summary(self) -> Result<WriterSummary, ConvertError> {
        let mut this = self;
        this.regions.flush().map_err(io_error)?;
        let summary = this.summary();
        super::level_writer::write_level_files(&super::level_writer::LevelFiles {
            format: &this.format,
            dirs: &this.dirs,
            level: &this.level,
            settings: &this.settings,
            items: &this.items,
            biomes: &this.biomes,
            player_items: &this.player_items,
            icon: this.icon.as_deref(),
            dimensions: &this.dimensions_seen,
            tall_overworld: this.tall_overworld,
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
