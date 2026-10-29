// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The settings of a conversion: Chunker's `WorldConverter` options, pruning and the overrides applied on top of the
//! level settings.
//!
//! Chunker's `ChunkerLevelSettings` (the 106 `level.dat` fields) is [`super::level_settings::LevelSettings`]; this
//! struct only holds what steers the conversion itself. [`ConversionSettings::default`] equals Chunker's defaults.
//!
//! What BlockForge exposes in its UI first (everything else stays at the default):
//! - the target edition and version ([`super::ConvertRequest::target`]),
//! - the dimensions to convert ([`ConversionSettings::dimension_mapping`], see
//!   [`ConversionSettingsBuilder::dimensions`]),
//! - keeping unknown NBT ([`ConversionSettings::allow_nbt_copying`], only legal between identical editions, see
//!   [`ConversionSettings::nbt_copying_allowed`]).
//!
//! Left out on purpose (out of scope, PORTING.md decision 7): user mapping files (`blockMappings`) and the
//! messenger's request plumbing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::dimension::{Dimension, DimensionRegistry};
use super::encoding::EncodingType;
use super::map::LevelMap;
use super::world::{ChunkPos, RegionPos};

/// A rectangle of chunks, inclusive (`PruningRegion`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PruningRegion {
    pub min_chunk_x: i32,
    pub min_chunk_z: i32,
    pub max_chunk_x: i32,
    pub max_chunk_z: i32,
}

impl PruningRegion {
    pub fn new(min_chunk_x: i32, min_chunk_z: i32, max_chunk_x: i32, max_chunk_z: i32) -> PruningRegion {
        PruningRegion { min_chunk_x, min_chunk_z, max_chunk_x, max_chunk_z }
    }

    fn contains(&self, chunk: ChunkPos) -> bool {
        chunk.x >= self.min_chunk_x
            && chunk.x <= self.max_chunk_x
            && chunk.z >= self.min_chunk_z
            && chunk.z <= self.max_chunk_z
    }
}

/// Which chunks of a dimension to keep (`PruningConfig`): the listed rectangles when `include`, everything but them
/// otherwise.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PruningConfig {
    pub include: bool,
    pub regions: Vec<PruningRegion>,
}

/// The settings of one conversion (the options of Chunker's `WorldConverter`).
#[derive(Clone, Debug)]
pub struct ConversionSettings {
    /// Per dimension identifier, which chunks to keep. `None`/empty keeps everything.
    pub pruning_configs: Option<BTreeMap<String, PruningConfig>>,
    /// Input dimension identifier to output identifier (resolved through [`Self::dimension_registry`]); an input
    /// dimension without an entry is dropped. `None` keeps every dimension as it is. EXPOSED IN THE UI.
    pub dimension_mapping: Option<BTreeMap<String, String>>,
    /// The known dimensions, including custom ones added by the request.
    pub dimension_registry: DimensionRegistry,
    /// Vanilla biome identifier to vanilla biome identifier replacements (`None` = identity).
    pub biome_mapping: Option<BTreeMap<String, String>>,
    /// A JSON overlay applied to the level settings after reading (`changedSettings`).
    pub changed_settings: Option<serde_json::Map<String, serde_json::Value>>,
    /// Replaces the maps of the level when set.
    pub maps: Option<Vec<LevelMap>>,
    /// Compact the Bedrock database after writing (`levelDBCompaction`).
    pub level_db_compaction: bool,
    pub process_maps: bool,
    pub process_items: bool,
    pub process_entities: bool,
    pub process_block_entities: bool,
    pub process_loot_tables: bool,
    pub process_biomes: bool,
    pub process_height_map: bool,
    pub process_lighting: bool,
    /// Resolve block states that depend on neighbours (fences, panes, walls, stairs...).
    pub process_column_pre_transform: bool,
    /// Keep the original NBT of block entities, entities and the level (`allowNBTCopying`). EXPOSED IN THE UI as
    /// "keep unknown NBT"; only takes effect when input and output edition are the same.
    pub allow_nbt_copying: bool,
    pub discard_empty_chunks: bool,
    pub prevent_y_biome_blending: bool,
    /// Allow identifiers that are not vanilla to pass through.
    pub custom_identifiers: bool,
    /// An opaque id of the caller's session.
    pub session_id: Option<String>,

    // ---- Players and level settings (players, maps and level overrides) -----------------------------------------
    /// What happens to the players of the world: who owns it, the ender chest and stats, whose inventory the owner
    /// gets and whether the other players' profiles are written too.
    pub player: PlayerOptions,
    /// Level settings forced on the converted world (difficulty, game mode, cheats, hardcore); applied after
    /// [`Self::changed_settings`]. Every one left at `None` keeps the source world's value.
    pub level_overrides: LevelOverrides,
}

/// Whose inventory the owner of the converted world starts with ([`PlayerOptions::inventory_source`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum InventorySource {
    /// The host's own (`~local_player`).
    #[default]
    Host,
    /// The inventory, ender chest and experience of another player's profile (`player_server_<uuid>`, the key or just
    /// the uuid) instead of the host's; position, spawn point, health and food stay the host's.
    Profile(String),
    /// The host's inventory, with its empty slots filled from that profile's inventory. Everything else (ender
    /// chest, experience) stays the host's.
    MergeHostAnd(String),
}

impl InventorySource {
    /// The uuid of the profile named by the choice, without the `player_server_` prefix; `None` for the host.
    pub fn profile_id(&self) -> Option<&str> {
        match self {
            InventorySource::Host => None,
            InventorySource::Profile(id) | InventorySource::MergeHostAnd(id) => {
                Some(id.strip_prefix("player_server_").unwrap_or(id))
            }
        }
    }
}

/// What is converted of the world's players ([`ConversionSettings::player`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerOptions {
    /// The Java account that owns the converted world. When set, the host's player file is named after it
    /// (`players/data/<uuid>.dat` from 26.1, `playerdata/<uuid>.dat` before), level.dat's `singleplayer_uuid`
    /// (26.1+) points at it and the player's own `UUID` is it. `None` keeps the anonymous single-player file, which
    /// whoever opens the world plays as.
    pub owner_uuid: Option<Uuid>,
    /// Write the host's ender chest (`EnderChestInventory`).
    pub include_ender_chest: bool,
    /// Write the host's experience, health, food, spawn point (bed or respawn anchor), effects and abilities;
    /// otherwise the player starts with full health and food and no experience.
    pub include_stats: bool,
    /// Whose inventory the owner gets.
    pub inventory_source: InventorySource,
    /// Also write the other players' profiles (`player_server_<uuid>`), each as the player file of its uuid (or of
    /// the Java account [`Self::guest_uuids`] gives it), so their inventories are not lost.
    pub export_guest_profiles: bool,
    /// The Java account to write a guest profile as, by profile uuid (`player_server_` prefix optional). A profile
    /// without an entry keeps its Bedrock uuid.
    pub guest_uuids: BTreeMap<String, Uuid>,
}

impl Default for PlayerOptions {
    fn default() -> Self {
        PlayerOptions {
            owner_uuid: None,
            include_ender_chest: true,
            include_stats: true,
            inventory_source: InventorySource::Host,
            export_guest_profiles: true,
            guest_uuids: BTreeMap::new(),
        }
    }
}

impl PlayerOptions {
    /// The Java account a guest profile is written as: its entry in [`Self::guest_uuids`], else its own uuid.
    pub fn guest_uuid(&self, profile_id: &str) -> Option<Uuid> {
        let bare = |id: &str| id.strip_prefix("player_server_").unwrap_or(id).to_ascii_lowercase();
        let id = bare(profile_id);
        self.guest_uuids
            .iter()
            .find(|(key, _)| bare(key) == id)
            .map(|(_, uuid)| *uuid)
            .or_else(|| Uuid::parse_str(&id).ok())
    }
}

/// The difficulty of a world (`Difficulty` in level.dat; `difficulty_settings.difficulty` from 26.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
    Peaceful,
    Easy,
    Normal,
    Hard,
}

impl Difficulty {
    /// The number both editions store.
    pub fn id(self) -> i32 {
        self as i32
    }
}

/// The default game mode of a world (`GameType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameType {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

impl GameType {
    /// The number Java stores.
    pub fn id(self) -> i32 {
        self as i32
    }
}

/// Level settings to force on the converted world ([`ConversionSettings::level_overrides`]). Java has no lock on
/// advancements for worlds that were in Creative or had cheats on, so nothing else needs changing for them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LevelOverrides {
    pub difficulty: Option<Difficulty>,
    /// Also becomes the game mode of every player written.
    pub game_type: Option<GameType>,
    /// Cheats (`allowCommands`).
    pub allow_commands: Option<bool>,
    pub hardcore: Option<bool>,
}

impl LevelOverrides {
    /// Whether any setting is forced.
    pub fn is_empty(&self) -> bool {
        *self == LevelOverrides::default()
    }

    /// Write the forced settings into `settings`.
    pub fn apply(&self, settings: &mut super::level_settings::LevelSettings) {
        if let Some(d) = self.difficulty {
            settings.difficulty = d.id();
        }
        if let Some(g) = self.game_type {
            settings.game_type = g.id();
        }
        if let Some(c) = self.allow_commands {
            settings.commands_enabled = c;
        }
        if let Some(h) = self.hardcore {
            settings.hardcore = h;
        }
    }
}

impl Default for ConversionSettings {
    fn default() -> Self {
        ConversionSettings {
            pruning_configs: None,
            dimension_mapping: None,
            dimension_registry: DimensionRegistry::new(),
            biome_mapping: None,
            changed_settings: None,
            maps: None,
            level_db_compaction: true,
            process_maps: true,
            process_items: true,
            process_entities: true,
            process_block_entities: true,
            process_loot_tables: true,
            process_biomes: true,
            process_height_map: true,
            process_lighting: true,
            process_column_pre_transform: true,
            allow_nbt_copying: false,
            discard_empty_chunks: false,
            prevent_y_biome_blending: false,
            custom_identifiers: true,
            session_id: None,
            player: PlayerOptions::default(),
            level_overrides: LevelOverrides::default(),
        }
    }
}

impl ConversionSettings {
    pub fn builder() -> ConversionSettingsBuilder {
        ConversionSettingsBuilder::default()
    }

    /// Whether NBT may be copied for this pair of editions (only between identical ones).
    pub fn nbt_copying_allowed(&self, input: EncodingType, output: EncodingType) -> bool {
        self.allow_nbt_copying && input == output
    }

    /// The dimension an input dimension is written as, or `None` when it is dropped (`getNewDimension`).
    pub fn new_dimension(&self, dimension: &Dimension) -> Option<Dimension> {
        match &self.dimension_mapping {
            None => Some(dimension.clone()),
            Some(mapping) => {
                let target = mapping.get(dimension.identifier())?;
                self.dimension_registry.by_identifier(target).cloned()
            }
        }
    }

    /// Whether the dimension is converted at all (`shouldProcessDimension`).
    pub fn should_process_dimension(&self, dimension: &Dimension) -> bool {
        self.new_dimension(dimension).is_some()
    }

    /// The replacement of a vanilla biome identifier (`getNewBiome`).
    pub fn new_biome<'a>(&'a self, biome: &'a str) -> &'a str {
        match &self.biome_mapping {
            Some(mapping) => mapping.get(biome).map_or(biome, String::as_str),
            None => biome,
        }
    }

    fn pruning_regions(&self, dimension: &Dimension) -> Option<&PruningConfig> {
        let configs = self.pruning_configs.as_ref().filter(|c| !c.is_empty())?;
        let config = configs.get(dimension.identifier())?;
        if config.regions.is_empty() { None } else { Some(config) }
    }

    /// Whether a region file is converted under the pruning rules (`shouldProcessRegion`): in include mode when it
    /// overlaps any rectangle, in exclude mode unless it lies fully inside one.
    pub fn should_process_region(&self, dimension: &Dimension, region: RegionPos) -> bool {
        let Some(config) = self.pruning_regions(dimension) else { return true };
        let min = region.chunk(0, 0);
        let max = region.chunk(31, 31);
        for rect in &config.regions {
            if config.include {
                let overlap = max.x >= rect.min_chunk_x
                    && min.x <= rect.max_chunk_x
                    && max.z >= rect.min_chunk_z
                    && min.z <= rect.max_chunk_z;
                if overlap {
                    return true;
                }
            } else {
                let contained = min.x >= rect.min_chunk_x
                    && max.x <= rect.max_chunk_x
                    && min.z >= rect.min_chunk_z
                    && max.z <= rect.max_chunk_z;
                if contained {
                    return false;
                }
            }
        }
        !config.include
    }

    /// Whether a column is converted under the pruning rules (`shouldProcessColumn`): the first rectangle containing
    /// the chunk decides, none means `!include`.
    pub fn should_process_column(&self, dimension: &Dimension, chunk: ChunkPos) -> bool {
        let Some(config) = self.pruning_regions(dimension) else { return true };
        if config.regions.iter().any(|rect| rect.contains(chunk)) {
            return config.include;
        }
        !config.include
    }

    /// Applies the keys of Chunker's CLI converter-settings JSON: `mapConversion`, `lootTableConversion`,
    /// `itemConversion`, `blockConnections`, `enableCompact`, `discardEmptyChunks`, `preventYBiomeBlending` and
    /// `customIdentifiers`. Other keys and non-boolean values are ignored.
    pub fn apply_cli_json(&mut self, json: &serde_json::Map<String, serde_json::Value>) {
        let flag = |key: &str| json.get(key).and_then(serde_json::Value::as_bool);
        if let Some(v) = flag("mapConversion") {
            self.process_maps = v;
        }
        if let Some(v) = flag("lootTableConversion") {
            self.process_loot_tables = v;
        }
        if let Some(v) = flag("itemConversion") {
            self.process_items = v;
        }
        if let Some(v) = flag("blockConnections") {
            self.process_column_pre_transform = v;
        }
        if let Some(v) = flag("enableCompact") {
            self.level_db_compaction = v;
        }
        if let Some(v) = flag("discardEmptyChunks") {
            self.discard_empty_chunks = v;
        }
        if let Some(v) = flag("preventYBiomeBlending") {
            self.prevent_y_biome_blending = v;
        }
        if let Some(v) = flag("customIdentifiers") {
            self.custom_identifiers = v;
        }
    }

    /// The level settings with [`Self::changed_settings`] and then [`Self::level_overrides`] applied (what the level
    /// handler does before writing).
    pub fn apply_to_level_settings(
        &self,
        settings: &super::level_settings::LevelSettings,
    ) -> Result<super::level_settings::LevelSettings, serde_json::Error> {
        let mut out = match &self.changed_settings {
            Some(changed) => settings.with_overrides(changed)?,
            None => settings.clone(),
        };
        self.level_overrides.apply(&mut out);
        Ok(out)
    }
}

/// Chained construction of [`ConversionSettings`].
#[derive(Clone, Debug, Default)]
pub struct ConversionSettingsBuilder {
    settings: ConversionSettings,
}

macro_rules! builder_flag {
    ($($(#[$m:meta])* $name:ident),* $(,)?) => {
        $( $(#[$m])* pub fn $name(mut self, value: bool) -> Self {
            self.settings.$name = value;
            self
        } )*
    };
}

impl ConversionSettingsBuilder {
    builder_flag!(
        level_db_compaction,
        process_maps,
        process_items,
        process_entities,
        process_block_entities,
        process_loot_tables,
        process_biomes,
        process_height_map,
        process_lighting,
        process_column_pre_transform,
        /// Keep unknown NBT (only effective between identical editions).
        allow_nbt_copying,
        discard_empty_chunks,
        prevent_y_biome_blending,
        custom_identifiers,
    );

    /// Converts exactly these dimensions, each as itself.
    pub fn dimensions<'a>(mut self, dimensions: impl IntoIterator<Item = &'a Dimension>) -> Self {
        let mut mapping = BTreeMap::new();
        for dimension in dimensions {
            if self.settings.dimension_registry.by_identifier(dimension.identifier()).is_none() {
                self.settings.dimension_registry.register(dimension.identifier().to_owned(), dimension.clone());
            }
            mapping.insert(dimension.identifier().to_owned(), dimension.identifier().to_owned());
        }
        self.settings.dimension_mapping = Some(mapping);
        self
    }

    /// Maps an input dimension identifier to an output one (the output must be in the registry).
    pub fn map_dimension(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.settings.dimension_mapping.get_or_insert_with(BTreeMap::new).insert(from.into(), to.into());
        self
    }

    pub fn dimension_registry(mut self, registry: DimensionRegistry) -> Self {
        self.settings.dimension_registry = registry;
        self
    }

    pub fn pruning(mut self, dimension: impl Into<String>, config: PruningConfig) -> Self {
        self.settings.pruning_configs.get_or_insert_with(BTreeMap::new).insert(dimension.into(), config);
        self
    }

    pub fn biome_mapping(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.settings.biome_mapping.get_or_insert_with(BTreeMap::new).insert(from.into(), to.into());
        self
    }

    pub fn changed_settings(mut self, overrides: serde_json::Map<String, serde_json::Value>) -> Self {
        self.settings.changed_settings = Some(overrides);
        self
    }

    pub fn maps(mut self, maps: Vec<LevelMap>) -> Self {
        self.settings.maps = Some(maps);
        self
    }

    pub fn player(mut self, options: PlayerOptions) -> Self {
        self.settings.player = options;
        self
    }

    pub fn level_overrides(mut self, overrides: LevelOverrides) -> Self {
        self.settings.level_overrides = overrides;
        self
    }

    pub fn session_id(mut self, id: impl Into<String>) -> Self {
        self.settings.session_id = Some(id.into());
        self
    }

    pub fn build(self) -> ConversionSettings {
        self.settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_chunker() {
        let s = ConversionSettings::default();
        assert!(s.level_db_compaction && s.process_maps && s.process_items && s.process_entities);
        assert!(s.process_block_entities && s.process_loot_tables && s.process_biomes && s.process_height_map);
        assert!(s.process_lighting && s.process_column_pre_transform && s.custom_identifiers);
        assert!(!s.allow_nbt_copying && !s.discard_empty_chunks && !s.prevent_y_biome_blending);
        assert!(s.pruning_configs.is_none() && s.dimension_mapping.is_none() && s.biome_mapping.is_none());
        assert_eq!(s.dimension_registry.dimensions().count(), 3);
    }

    #[test]
    fn dimension_mapping() {
        let none = ConversionSettings::default();
        assert_eq!(none.new_dimension(&Dimension::nether()), Some(Dimension::nether()));

        let s = ConversionSettings::builder().dimensions(&[Dimension::overworld(), Dimension::the_end()]).build();
        assert!(s.should_process_dimension(&Dimension::overworld()));
        assert!(!s.should_process_dimension(&Dimension::nether()));

        let s = ConversionSettings::builder().map_dimension("minecraft:the_nether", "minecraft:the_end").build();
        assert_eq!(s.new_dimension(&Dimension::nether()), Some(Dimension::the_end()));
        assert_eq!(s.new_dimension(&Dimension::overworld()), None);

        // A target that is not registered drops the dimension.
        let s = ConversionSettings::builder().map_dimension("minecraft:overworld", "mod:nowhere").build();
        assert_eq!(s.new_dimension(&Dimension::overworld()), None);
    }

    #[test]
    fn custom_dimension_through_builder() {
        let mut registry = DimensionRegistry::new();
        let custom = registry.register_from_world("mod:a", None);
        let s = ConversionSettings::builder().dimension_registry(registry).map_dimension("mod:a", "mod:a").build();
        assert_eq!(s.new_dimension(&custom), Some(custom));
    }

    #[test]
    fn biome_mapping() {
        let s = ConversionSettings::builder().biome_mapping("minecraft:desert", "minecraft:plains").build();
        assert_eq!(s.new_biome("minecraft:desert"), "minecraft:plains");
        assert_eq!(s.new_biome("minecraft:swamp"), "minecraft:swamp");
        assert_eq!(ConversionSettings::default().new_biome("minecraft:desert"), "minecraft:desert");
    }

    #[test]
    fn nbt_copying_needs_same_edition() {
        let s = ConversionSettings::builder().allow_nbt_copying(true).build();
        assert!(s.nbt_copying_allowed(EncodingType::Java, EncodingType::Java));
        assert!(!s.nbt_copying_allowed(EncodingType::Java, EncodingType::Bedrock));
        assert!(!ConversionSettings::default().nbt_copying_allowed(EncodingType::Java, EncodingType::Java));
    }

    #[test]
    fn pruning_include() {
        let cfg = PruningConfig { include: true, regions: vec![PruningRegion::new(0, 0, 40, 10)] };
        let s = ConversionSettings::builder().pruning("minecraft:overworld", cfg).build();
        let ow = Dimension::overworld();
        // Region 0,0 covers chunks 0..=31: inside. Region 1,0 covers 32..=63: overlaps (32..=40).
        assert!(s.should_process_region(&ow, RegionPos::new(0, 0)));
        assert!(s.should_process_region(&ow, RegionPos::new(1, 0)));
        assert!(!s.should_process_region(&ow, RegionPos::new(2, 0)));
        assert!(!s.should_process_region(&ow, RegionPos::new(0, -1)));
        assert!(s.should_process_column(&ow, ChunkPos::new(40, 10)));
        assert!(!s.should_process_column(&ow, ChunkPos::new(41, 10)));
        // Other dimensions are untouched.
        assert!(s.should_process_region(&Dimension::nether(), RegionPos::new(9, 9)));
        assert!(s.should_process_column(&Dimension::nether(), ChunkPos::new(999, 9)));
    }

    #[test]
    fn pruning_exclude() {
        let cfg = PruningConfig { include: false, regions: vec![PruningRegion::new(0, 0, 31, 31)] };
        let s = ConversionSettings::builder().pruning("minecraft:overworld", cfg).build();
        let ow = Dimension::overworld();
        // Only a region fully inside the rectangle is skipped.
        assert!(!s.should_process_region(&ow, RegionPos::new(0, 0)));
        assert!(s.should_process_region(&ow, RegionPos::new(1, 0)));
        assert!(!s.should_process_column(&ow, ChunkPos::new(5, 5)));
        assert!(s.should_process_column(&ow, ChunkPos::new(32, 5)));
        let partial = PruningConfig { include: false, regions: vec![PruningRegion::new(0, 0, 30, 31)] };
        let s = ConversionSettings::builder().pruning("minecraft:overworld", partial).build();
        assert!(s.should_process_region(&ow, RegionPos::new(0, 0)));
    }

    #[test]
    fn pruning_empty_config_keeps_everything() {
        let cfg = PruningConfig { include: true, regions: vec![] };
        let s = ConversionSettings::builder().pruning("minecraft:overworld", cfg).build();
        assert!(s.should_process_region(&Dimension::overworld(), RegionPos::new(3, 3)));
        assert!(s.should_process_column(&Dimension::overworld(), ChunkPos::new(3, 3)));
    }

    #[test]
    fn pruning_json() {
        let cfg: PruningConfig =
            serde_json::from_str(r#"{"include":true,"regions":[{"minChunkX":1,"maxChunkX":2}]}"#).unwrap();
        assert!(cfg.include);
        assert_eq!(cfg.regions, [PruningRegion::new(1, 0, 2, 0)]);
    }

    #[test]
    fn cli_json_flags() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"mapConversion":false,"itemConversion":false,"blockConnections":false,"enableCompact":false,
                "discardEmptyChunks":true,"preventYBiomeBlending":true,"customIdentifiers":false,
                "lootTableConversion":false,"other":1}"#,
        )
        .unwrap();
        let mut s = ConversionSettings::default();
        s.apply_cli_json(json.as_object().unwrap());
        assert!(!s.process_maps && !s.process_items && !s.process_column_pre_transform && !s.level_db_compaction);
        assert!(s.discard_empty_chunks && s.prevent_y_biome_blending && !s.custom_identifiers);
        assert!(!s.process_loot_tables);
        assert!(s.process_entities);
    }

    #[test]
    fn player_and_level_defaults_keep_everything() {
        let s = ConversionSettings::default();
        let p = &s.player;
        assert_eq!(p.owner_uuid, None);
        assert!(p.include_ender_chest && p.include_stats && p.export_guest_profiles);
        assert_eq!(p.inventory_source, InventorySource::Host);
        assert!(p.guest_uuids.is_empty());
        assert!(s.level_overrides.is_empty());
        let base = super::super::level_settings::LevelSettings { difficulty: 0, ..Default::default() };
        assert_eq!(s.apply_to_level_settings(&base).unwrap(), base);
    }

    #[test]
    fn level_overrides_apply_after_the_json_overlay() {
        let overrides = serde_json::json!({"Difficulty": 1, "LevelName": "Renamed"}).as_object().unwrap().clone();
        let s = ConversionSettings::builder()
            .changed_settings(overrides)
            .level_overrides(LevelOverrides {
                difficulty: Some(Difficulty::Hard),
                game_type: Some(GameType::Adventure),
                allow_commands: Some(true),
                hardcore: Some(true),
            })
            .build();
        let base = super::super::level_settings::LevelSettings {
            difficulty: 0,
            game_type: 0,
            commands_enabled: false,
            ..Default::default()
        };
        let out = s.apply_to_level_settings(&base).unwrap();
        assert_eq!((out.difficulty, out.game_type, out.commands_enabled, out.hardcore), (3, 2, true, true));
        assert_eq!(out.level_name, "Renamed");
        // Only what is set is forced.
        let s = ConversionSettings::builder()
            .level_overrides(LevelOverrides { allow_commands: Some(false), ..LevelOverrides::default() })
            .build();
        let base = super::super::level_settings::LevelSettings { difficulty: 2, ..Default::default() };
        let out = s.apply_to_level_settings(&base).unwrap();
        assert_eq!((out.difficulty, out.commands_enabled), (2, false));
        assert_eq!((Difficulty::Peaceful.id(), Difficulty::Normal.id(), GameType::Spectator.id()), (0, 2, 3));
    }

    #[test]
    fn inventory_source_and_guest_accounts() {
        assert_eq!(InventorySource::Host.profile_id(), None);
        assert_eq!(InventorySource::Profile("player_server_abc".into()).profile_id(), Some("abc"));
        assert_eq!(InventorySource::MergeHostAnd("abc".into()).profile_id(), Some("abc"));
        let guest = "874d8b7a-1111-4222-8333-444455556666";
        let account = Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
        let mut p = PlayerOptions::default();
        // Without an entry a profile keeps its own uuid; ids that are no uuid have none.
        assert_eq!(p.guest_uuid(&format!("player_server_{guest}")), Uuid::parse_str(guest).ok());
        assert_eq!(p.guest_uuid("not-a-uuid"), None);
        p.guest_uuids.insert(format!("player_server_{guest}"), account);
        assert_eq!(p.guest_uuid(guest), Some(account));
        assert_eq!(p.guest_uuid(&guest.to_uppercase()), Some(account));
        let s = ConversionSettings::builder().player(p.clone()).build();
        assert_eq!(s.player, p);
    }

    #[test]
    fn changed_settings_overlay() {
        let overrides = serde_json::json!({"LevelName": "Renamed", "Difficulty": 3}).as_object().unwrap().clone();
        let s = ConversionSettings::builder().changed_settings(overrides).build();
        let base = super::super::level_settings::LevelSettings::default();
        let out = s.apply_to_level_settings(&base).unwrap();
        assert_eq!(out.level_name, "Renamed");
        assert_eq!(out.difficulty, 3);
        assert_eq!(out.spawn_y, 32767);
        assert_eq!(ConversionSettings::default().apply_to_level_settings(&base).unwrap(), base);
    }
}
