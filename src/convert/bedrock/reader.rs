// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Reading Bedrock worlds into the model: [`BedrockWorldReader`] turns a world folder into [`Level`] and [`Column`]s.
//!
//! This is the merge of Chunker's `BedrockLevelReader`, `BedrockWorldReader`, `BedrockColumnReader` and
//! `BedrockChunkReader` (and the version packages that change them) as plain functions over one object:
//!
//! - [`open`](BedrockWorldReader::open) copies the database to a scratch directory (see `db`), starts the database
//!   thread (`reader_io`), lists the columns once and reads the custom dimension table;
//! - [`level`](BedrockWorldReader::level) is `level.dat` plus `~local_player`, the maps and the portals
//!   (`level_reader`); [`player_profiles`](BedrockWorldReader::player_profiles) are the other players
//!   (`player_server_<uuid>`), and [`read_item`](BedrockWorldReader::read_item) turns their items into the model;
//! - [`dimensions`](BedrockWorldReader::dimensions) and [`columns`](BedrockWorldReader::columns) are what the
//!   database holds, filtered by the conversion settings (dimension mapping and pruning);
//! - [`read_column`](BedrockWorldReader::read_column) reads one column: its sub-chunks (`reader_blocks`), biomes and
//!   height map (`reader_biomes`), block entities and the block fix-ups that go with them (`resolvers::block_entity::read_into_column`), then
//!   hands it to the pre-transform of the world's version.
//!
//! The reader is `Sync`. Every database access goes through one thread that owns the LevelDB handle
//! (`rusty-leveldb` handles are `!Send` and need `&mut`), so any number of workers may call `read_column` at once:
//! they queue for the raw bytes of a column, which is quick, and decode in parallel, which is the slow part. A call
//! holds one column in memory plus whatever the database caches (8 MiB).
//!
//! Not read in this version: entities (`digp`/`actorprefix` and the pre-1.18.30 `Entity` record; item frames and
//! paintings included), pending and random ticks, the legacy terrain record and sub-chunk versions before 8. The
//! chunk `Version` and `FinalizedState` records are checked for sanity and otherwise ignored, like Chunker does.

#[path = "reader_biomes.rs"]
mod reader_biomes;
#[path = "reader_blocks.rs"]
mod reader_blocks;
#[path = "reader_io.rs"]
mod reader_io;
#[cfg(test)]
#[path = "reader_tests.rs"]
mod reader_tests;

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use self::reader_biomes::{BiomeReader, height_map};
use self::reader_blocks::{BlockReader, build_chunk};
use self::reader_io::{DbHandle, DbOpener, Want};
use super::BedrockError;
use super::data_version::{BedrockVersion, GameVersion};
use super::db::{BedrockDb, LevelDb};
use super::detect::BedrockWorldInfo;
use super::keys::{self, DimensionId, DimensionTable};
use super::level_dat::LevelDat;
use super::level_reader::{
    PlayerIdentity, parse_biome_ids, parse_maps, parse_player, parse_player_identity, parse_portals,
    read_level_settings,
};
use super::palette::{decode_data2d, decode_data3d, decode_nbt_concat, decode_sub_chunk};
use super::resolvers::block_entity::read_into_column;
use super::resolvers::itemstack::BedrockItems;
use crate::convert::level::player::PlayerItem;
use crate::convert::level::{
    ChunkPos, ConversionSettings, ConvertError, Dimension, DimensionRegistry, Level, LevelPlayer, LevelSettings,
    RegionPos,
};
use crate::convert::mapping::Version;
use crate::convert::model::{Biome, ChunkCoord, Column, EffectType, ItemStack};
use crate::convert::nbt::{Compound, Tag};

/// The most warning messages kept (the rest is only counted).
const MAX_WARNINGS: usize = 500;

/// A dimension that has columns in the database.
struct DimensionColumns {
    dimension: Dimension,
    id: DimensionId,
    /// Sorted by x, then z.
    columns: Vec<(i32, i32)>,
}

/// Counters of what the reader met, for diagnostics and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReaderStats {
    /// Columns returned by `read_column`.
    pub columns: u64,
    /// Sub-chunks read.
    pub sub_chunks: u64,
    /// Block entities that arrived in a column.
    pub block_entities: u64,
    /// Columns whose `FinalizedState` says generation was not finished.
    pub unfinalized: u64,
}

/// A player other than the host (`player_server_<uuid>`).
#[derive(Clone, Debug)]
pub struct PlayerProfile {
    /// The uuid of the key (`player_server_<uuid>` without the prefix).
    pub id: String,
    /// The `player_<id>` records that point at this profile (a signed-in player has one with an `MsaId`).
    pub identities: Vec<PlayerIdentity>,
    /// The player, parsed like `~local_player` (items stay Bedrock item compounds).
    pub player: LevelPlayer,
}

impl PlayerProfile {
    /// Whether a player signed in to an account owned the profile (Bedrock keeps no gamertag either way).
    pub fn signed_in(&self) -> bool {
        self.identities.iter().any(|i| i.msa_id.is_some())
    }

    /// Stacks in the inventory, armor and off hand.
    pub fn item_count(&self) -> usize {
        self.player.inventory.len()
    }
}

/// Reads one Bedrock world. See the module documentation.
pub struct BedrockWorldReader {
    db: DbHandle,
    version: &'static BedrockVersion,
    settings: ConversionSettings,
    level_dat: LevelDat,
    level_settings: LevelSettings,
    registry: DimensionRegistry,
    dimensions: Vec<DimensionColumns>,
    blocks: BlockReader,
    biomes: BiomeReader,
    items: BedrockItems,
    /// Added to the y of version 8 sub-chunks in the overworld (-4 for 1.17.0 to 1.17.20 caves and cliffs worlds).
    chunk_y_offset: i32,
    warnings: Mutex<Vec<String>>,
    warnings_dropped: AtomicU64,
    unmapped_block_entities: Mutex<HashMap<String, u64>>,
    unmapped_items: std::sync::Arc<Mutex<HashMap<String, u64>>>,
    columns_read: AtomicU64,
    sub_chunks_read: AtomicU64,
    block_entities_read: AtomicU64,
    unfinalized: AtomicU64,
}

fn convert_error(e: BedrockError) -> ConvertError {
    match e {
        BedrockError::Io(m) => ConvertError::Io(std::io::Error::other(m)),
        BedrockError::Unsupported(m) => ConvertError::Unsupported(m),
        other => ConvertError::Malformed(other.to_string()),
    }
}

fn mapping_version(game: GameVersion) -> Version {
    let part = |v: i32| v.max(0) as u32;
    Version::new(part(game[0]), part(game[1]), part(game[2]))
}

impl BedrockWorldReader {
    /// Open a detected world (`detect_bedrock`). The database is copied first; the world is never modified.
    pub fn open(info: &BedrockWorldInfo, settings: &ConversionSettings) -> Result<BedrockWorldReader, ConvertError> {
        let level_dat = LevelDat::read(&info.level_dat_path()).map_err(convert_error)?;
        let version = BedrockVersion::nearest_at_or_below(info.version.game);
        let game = info.game_version.unwrap_or(info.version.game);
        let db_path = info.db_path();
        let opener: DbOpener =
            Box::new(move || LevelDb::open_read(&db_path).map(|db| Box::new(db) as Box<dyn BedrockDb>));
        Self::from_parts(opener, level_dat, version, game, settings)
    }

    /// Assemble a reader from an already known `level.dat`, version and database (tests build worlds in memory).
    pub(super) fn from_parts(
        opener: DbOpener,
        level_dat: LevelDat,
        version: &'static BedrockVersion,
        game: GameVersion,
        settings: &ConversionSettings,
    ) -> Result<BedrockWorldReader, ConvertError> {
        let (db, positions) = DbHandle::start(opener).map_err(convert_error)?;
        let warnings = Mutex::new(Vec::new());

        // Custom dimensions (`readCustomDimensions`).
        let mut registry = settings.dimension_registry.clone();
        match db.get(keys::DIMENSION_NAME_ID_TABLE) {
            Ok(Some(value)) => match DimensionTable::from_value(&value) {
                Ok(table) => {
                    for (name, id) in &table.entries {
                        registry.register_from_world(name, Some(*id));
                    }
                }
                Err(e) => push_warning(&warnings, format!("Failed to read custom dimensions: {e}")),
            },
            Ok(None) => {}
            Err(e) => push_warning(&warnings, format!("Failed to read custom dimensions: {e}")),
        }

        // Group the columns by the dimensions the registry knows; keys of unknown dimensions are not chunks.
        let mut by_dimension: BTreeMap<i32, Vec<(i32, i32)>> = BTreeMap::new();
        for p in positions {
            by_dimension.entry(p.dim.0).or_default().push((p.x, p.z));
        }
        let sentinel = Dimension::overworld();
        let mut dimensions = Vec::new();
        for (id, mut columns) in by_dimension {
            let dimension = registry.from_bedrock(id, &sentinel);
            if dimension.bedrock_id() != id {
                continue;
            }
            columns.sort_unstable();
            columns.dedup();
            dimensions.push(DimensionColumns { dimension, id: DimensionId(id), columns });
        }

        let level_settings = read_level_settings(&level_dat.root, level_dat.storage_version, version, &warnings);
        let root = version.encoder_root();
        let chunk_y_offset = if ([1, 17, 0]..[1, 17, 30]).contains(&root)
            && level_settings.caves_and_cliffs
            && level_settings.generator_type == crate::convert::level::GeneratorType::Normal
        {
            -4
        } else {
            0
        };

        let mapping = mapping_version(game);
        let mut biomes = BiomeReader::new(mapping);
        if version.at_least([1, 21, 110]) {
            match db.get(keys::BIOME_IDS_TABLE) {
                Ok(Some(value)) => match parse_biome_ids(&value) {
                    Ok(list) => biomes.load_custom(list),
                    Err(e) => push_warning(&warnings, format!("Failed to read the custom biome list: {e}")),
                },
                Ok(None) => {}
                Err(e) => push_warning(&warnings, format!("Failed to read the custom biome list: {e}")),
            }
        }

        // Items name maps by `map_uuid`; the Java writer numbers the maps from 0 in the order the level lists them
        // (sorted by id, the way `level()` parses them), so an item's map index is its map's place in that order.
        let mut map_indexes: HashMap<i64, i32> = HashMap::new();
        if settings.process_maps {
            match db.prefix(keys::MAP_PREFIX) {
                Ok(entries) => {
                    let maps = parse_maps(&entries, &registry, false, &mut Vec::new());
                    map_indexes = maps.iter().enumerate().map(|(index, map)| (map.id, index as i32)).collect();
                }
                Err(e) => push_warning(&warnings, format!("Failed to list the maps: {e}")),
            }
        }

        let unmapped_items: std::sync::Arc<Mutex<HashMap<String, u64>>> = Arc::default();
        let log = unmapped_items.clone();
        let items = BedrockItems::new(version)
            .with_map_indexes(move |uuid| map_indexes.get(&uuid).copied())
            .with_custom_identifiers(settings.custom_identifiers)
            .with_original_nbt(settings.allow_nbt_copying)
            .with_loot_tables(settings.process_loot_tables)
            .with_missing_logger(move |kind, what| {
                if let Ok(mut m) = log.lock() {
                    *m.entry(format!("{kind:?}: {what}")).or_insert(0) += 1;
                }
            });

        Ok(BedrockWorldReader {
            db,
            version,
            settings: settings.clone(),
            level_dat,
            level_settings,
            registry,
            dimensions,
            blocks: BlockReader::new(mapping, settings.custom_identifiers),
            biomes,
            items,
            chunk_y_offset,
            warnings,
            warnings_dropped: AtomicU64::new(0),
            unmapped_block_entities: Mutex::default(),
            unmapped_items,
            columns_read: AtomicU64::new(0),
            sub_chunks_read: AtomicU64::new(0),
            block_entities_read: AtomicU64::new(0),
            unfinalized: AtomicU64::new(0),
        })
    }

    /// A reader over a world held in memory, as the game `game` saved it (tests of the pipeline).
    #[cfg(test)]
    pub(crate) fn from_memory(
        db: super::db::MemDb,
        level_dat: LevelDat,
        game: GameVersion,
        settings: &ConversionSettings,
    ) -> Result<BedrockWorldReader, ConvertError> {
        let opener: DbOpener = Box::new(move || Ok(Box::new(db) as Box<dyn BedrockDb>));
        Self::from_parts(opener, level_dat, BedrockVersion::nearest_at_or_below(game), game, settings)
    }

    /// The registered version the world's tables were chosen by.
    pub fn version(&self) -> &'static BedrockVersion {
        self.version
    }

    /// The dimension registry with the custom dimensions of the world added. A pipeline that maps dimensions should
    /// use this one (the registry in the settings does not know them).
    pub fn dimension_registry(&self) -> &DimensionRegistry {
        &self.registry
    }

    fn warn(&self, message: String) {
        push_warning_counted(&self.warnings, &self.warnings_dropped, message);
    }

    /// `level.dat` and the level-wide records of the database as a [`Level`]: the settings, the local player (items
    /// stay Bedrock item compounds), the maps (if the settings process them), the portals and the original
    /// `level.dat` root.
    pub fn level(&self) -> Result<Level, ConvertError> {
        let mut level = Level { settings: self.level_settings.clone(), ..Level::default() };
        level.original_level_data = Some(self.level_dat.root.clone());

        match self.db.get(keys::LOCAL_PLAYER) {
            Ok(Some(bytes)) => match super::nbt_le::from_slice(&bytes) {
                Ok((_, Tag::Compound(player))) => {
                    level.player =
                        Some(parse_player(&player, &self.registry, self.settings.process_items, &|id| self.effect(id)));
                }
                Ok(_) => self.warn("~local_player is not a compound".into()),
                Err(e) => self.warn(format!("~local_player: {e}")),
            },
            Ok(None) => {}
            Err(e) => return Err(convert_error(e)),
        }

        if self.settings.process_maps {
            let entries = self.db.prefix(keys::MAP_PREFIX).map_err(convert_error)?;
            let mut problems = Vec::new();
            level.maps = parse_maps(&entries, &self.registry, self.settings.allow_nbt_copying, &mut problems);
            for p in problems {
                self.warn(p);
            }
        }

        match self.db.get(keys::PORTALS) {
            Ok(Some(bytes)) => match parse_portals(&bytes, &self.registry) {
                Ok(portals) => level.portals = portals,
                Err(e) => self.warn(format!("portals: {e}")),
            },
            Ok(None) => {}
            Err(e) => return Err(convert_error(e)),
        }
        Ok(level)
    }

    /// The players other than the host (`player_server_<uuid>`), sorted by id, with the `player_<id>` records that
    /// point at them. Profiles that fail to parse are reported in the warnings.
    pub fn player_profiles(&self) -> Result<Vec<PlayerProfile>, ConvertError> {
        let mut identities: Vec<PlayerIdentity> = Vec::new();
        for (key, value) in self.db.prefix(keys::PLAYER_PREFIX).map_err(convert_error)? {
            if key.starts_with(keys::PLAYER_SERVER_PREFIX) {
                continue;
            }
            match parse_player_identity(&value) {
                Ok(identity) => identities.push(identity),
                Err(e) => self.warn(format!("{}: {e}", String::from_utf8_lossy(&key))),
            }
        }
        let mut profiles = Vec::new();
        for (key, value) in self.db.prefix(keys::PLAYER_SERVER_PREFIX).map_err(convert_error)? {
            let name = String::from_utf8_lossy(&key).into_owned();
            let id = name[keys::PLAYER_SERVER_PREFIX.len()..].to_string();
            match super::nbt_le::from_slice(&value) {
                Ok((_, Tag::Compound(player))) => profiles.push(PlayerProfile {
                    identities: identities.iter().filter(|i| i.server_id.as_deref() == Some(&name)).cloned().collect(),
                    player: parse_player(&player, &self.registry, self.settings.process_items, &|id| self.effect(id)),
                    id,
                }),
                Ok(_) => self.warn(format!("{name} is not a compound")),
                Err(e) => self.warn(format!("{name}: {e}")),
            }
        }
        profiles.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(profiles)
    }

    /// A Bedrock item compound of a player (`PlayerItem`) as a model stack: `None` for air and for items without an
    /// equivalent. Maps keep their link (see `with_map_indexes` in `from_parts`).
    pub fn read_item(&self, item: &PlayerItem) -> Option<ItemStack> {
        self.items.read(item).filter(|stack| !stack.is_air())
    }

    /// [`Self::read_item`] for every slot.
    pub fn read_items(&self, items: &BTreeMap<i8, PlayerItem>) -> BTreeMap<i8, ItemStack> {
        items.iter().filter_map(|(slot, item)| Some((*slot, self.read_item(item)?))).collect()
    }

    fn effect(&self, id: i32) -> Option<EffectType> {
        let effect = self.items.tables.effect(id);
        if effect.is_none() {
            self.items.missing(super::resolvers::itemstack::Missing::Effect, &id.to_string());
        }
        effect
    }

    /// The dimensions that have columns and that the settings convert, overworld first, then by id.
    pub fn dimensions(&self) -> Vec<Dimension> {
        self.dimensions
            .iter()
            .filter(|d| self.settings.should_process_dimension(&d.dimension))
            .map(|d| d.dimension.clone())
            .collect()
    }

    /// The positions of the columns of a dimension that have chunk data (Chunker's `isColumnKey` rule), sorted by x
    /// then z, without the ones the pruning settings leave out.
    pub fn columns(&self, dim: &Dimension) -> Vec<(i32, i32)> {
        let Some(entry) = self.dimensions.iter().find(|d| d.id.0 == dim.bedrock_id()) else { return Vec::new() };
        entry
            .columns
            .iter()
            .copied()
            .filter(|&(x, z)| {
                let chunk = ChunkPos::new(x, z);
                self.settings.should_process_region(dim, RegionPos::new(x >> 5, z >> 5))
                    && self.settings.should_process_column(dim, chunk)
            })
            .collect()
    }

    /// Read one column. `None` when the database holds nothing for it. Problems inside a column (a damaged
    /// sub-chunk, an unsupported version) skip that part and are reported by [`warnings`](Self::warnings); only a
    /// database failure is an error.
    pub fn read_column(&self, dim: &Dimension, x: i32, z: i32) -> Result<Option<Column>, ConvertError> {
        let s = &self.settings;
        let id = DimensionId(dim.bedrock_id());
        let want = Want {
            biomes: s.process_biomes || s.process_height_map,
            block_entities: s.process_block_entities,
            data3d: self.version.uses_data3d(),
        };
        let records = self.db.column(id, x, z, want).map_err(convert_error)?;
        if records.is_empty() {
            return Ok(None);
        }
        let mut column = Column::new(ChunkCoord::new(x, z));
        let at = || format!("column {x},{z} of {}", dim.identifier());

        // Sub-chunks.
        for (key_y, bytes) in &records.sub_chunks {
            match decode_sub_chunk(bytes, *key_y) {
                Ok(sub) => {
                    let mut y = i32::from(sub.y);
                    if sub.version == 8 && id == DimensionId::OVERWORLD {
                        y += self.chunk_y_offset;
                    }
                    let Ok(y) = i8::try_from(y) else {
                        self.warn(format!("{}: sub-chunk y {y} is out of range", at()));
                        continue;
                    };
                    column.chunks.insert(y, build_chunk(&self.blocks, &sub, y));
                    self.sub_chunks_read.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => self.warn(format!("{}: sub-chunk {key_y}: {e}", at())),
            }
        }

        // Biomes and height map.
        let fallback = Biome::from_identifier(dim.fallback_biome());
        if let Some(data) = &records.data3d {
            match decode_data3d(data) {
                Ok(d) => {
                    if s.process_height_map {
                        column.height_map = Some(height_map(&d.heights));
                    }
                    if s.process_biomes {
                        column.biomes = Some(self.biomes.data3d_biomes(&d, &fallback));
                    }
                }
                Err(e) => self.warn(format!("{}: Data3D: {e}", at())),
            }
        } else if let Some(data) = &records.data2d {
            match decode_data2d(data) {
                Ok(d) => {
                    if s.process_height_map {
                        column.height_map = Some(height_map(&d.heights));
                    }
                    if s.process_biomes {
                        column.biomes = Some(self.biomes.data2d_biomes(&d, &fallback));
                    }
                }
                Err(e) => self.warn(format!("{}: Data2D: {e}", at())),
            }
        }

        // Block entities, then the blocks they fix up.
        if let Some(data) = &records.block_entities {
            match decode_nbt_concat(data) {
                Ok(tags) => {
                    let compounds: Vec<Compound> = tags
                        .into_iter()
                        .filter_map(|(_, t)| if let Tag::Compound(c) = t { Some(c) } else { None })
                        .collect();
                    let missing = read_into_column(&self.items, &compounds, &mut column, s.allow_nbt_copying);
                    if !missing.is_empty() {
                        if let Ok(mut unmapped) = self.unmapped_block_entities.lock() {
                            for id in missing {
                                *unmapped.entry(id).or_insert(0) += 1;
                            }
                        }
                    }
                    self.block_entities_read.fetch_add(column.block_entities.len() as u64, Ordering::Relaxed);
                }
                Err(e) => self.warn(format!("{}: block entities: {e}", at())),
            }
        }

        self.check_status(&records, &at);
        super::pretransform::solve(self.version, &mut column, s.process_column_pre_transform);
        self.columns_read.fetch_add(1, Ordering::Relaxed);
        Ok(Some(column))
    }

    /// The chunk `Version` and `FinalizedState` records are ignored but must look right.
    fn check_status(&self, records: &reader_io::ColumnRecords, at: &dyn Fn() -> String) {
        if let Some(v) = &records.version {
            if v.len() != 1 {
                self.warn(format!("{}: chunk Version record has {} bytes", at(), v.len()));
            }
        }
        if let Some(f) = &records.finalized {
            let state = match f.len() {
                1 => Some(i32::from(f[0])),
                4 => Some(i32::from_le_bytes([f[0], f[1], f[2], f[3]])),
                n => {
                    self.warn(format!("{}: FinalizedState record has {n} bytes", at()));
                    None
                }
            };
            // 2 is "done"; lower values were not fully generated (chunk population still pending).
            if state.is_some_and(|s| s < 2) {
                self.unfinalized.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Blocks that had no mapping (air was used), by name (or `name[states]` when only the states were unknown),
    /// with how many palette entries were met.
    pub fn unmapped_blocks(&self) -> HashMap<String, u64> {
        self.blocks.unmapped()
    }

    /// Biome ids that had no mapping (the dimension's fallback biome was used).
    pub fn unmapped_biomes(&self) -> HashMap<i32, u64> {
        self.biomes.unmapped()
    }

    /// Block entity ids no handler knows.
    pub fn unmapped_block_entities(&self) -> HashMap<String, u64> {
        self.unmapped_block_entities.lock().map(|m| m.clone()).unwrap_or_default()
    }

    /// Items, enchantments, effects and similar ids that had no mapping while reading block entities, as
    /// `Kind: id`, with counts. Block mappings missed by block entity fix-ups are in here as `Block: ...`.
    pub fn unmapped_items(&self) -> HashMap<String, u64> {
        self.unmapped_items.lock().map(|m| m.clone()).unwrap_or_default()
    }

    /// Pivot states the model could not hold (`TYPE.STATE=VALUE`), with counts.
    pub fn dropped_states(&self) -> HashMap<String, u64> {
        self.blocks.dropped_states()
    }

    /// Non-fatal problems (Chunker's `logNonFatalException`), at most 500 messages.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = self.warnings.lock().map(|w| w.clone()).unwrap_or_default();
        let dropped = self.warnings_dropped.load(Ordering::Relaxed);
        if dropped > 0 {
            out.push(format!("... and {dropped} more"));
        }
        out
    }

    /// What was read so far.
    pub fn stats(&self) -> ReaderStats {
        ReaderStats {
            columns: self.columns_read.load(Ordering::Relaxed),
            sub_chunks: self.sub_chunks_read.load(Ordering::Relaxed),
            block_entities: self.block_entities_read.load(Ordering::Relaxed),
            unfinalized: self.unfinalized.load(Ordering::Relaxed),
        }
    }
}

fn push_warning(warnings: &Mutex<Vec<String>>, message: String) {
    if let Ok(mut w) = warnings.lock() {
        if w.len() < MAX_WARNINGS {
            w.push(message);
        }
    }
}

fn push_warning_counted(warnings: &Mutex<Vec<String>>, dropped: &AtomicU64, message: String) {
    if let Ok(mut w) = warnings.lock() {
        if w.len() < MAX_WARNINGS {
            w.push(message);
            return;
        }
    }
    dropped.fetch_add(1, Ordering::Relaxed);
}

#[allow(dead_code)]
fn assert_sync() {
    fn is_sync<T: Sync + Send>() {}
    is_sync::<BedrockWorldReader>();
}
