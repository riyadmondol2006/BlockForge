//! The version-neutral level and world model (settings, dimensions, player data) and the conversion task machinery.
//!
//! Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//!
//! Nothing here depends on the column model: positions are local `ChunkPos`/`RegionPos`, items are NBT.

pub mod convert_api;
pub mod dimension;
pub mod encoding;
#[allow(clippy::module_inception)]
pub mod level;
pub mod level_settings;
pub mod map;
pub mod player;
pub mod portal;
pub mod settings;
pub mod task;
pub mod world;

pub use convert_api::{ConvertError, ConvertRequest, ConvertSummary, NullSink, ProgressSink};
pub use dimension::{BEDROCK_CUSTOM_DIMENSION_ID_START, Dimension, DimensionRegistry};
pub use encoding::{EncodingType, TargetVersion, VersionTriple};
pub use level::Level;
pub use level_settings::{GeneratorType, LevelSettings};
pub use map::LevelMap;
pub use player::LevelPlayer;
pub use portal::Portal;
pub use settings::{ConversionSettings, ConversionSettingsBuilder, PruningConfig, PruningRegion};
pub use task::{CancelToken, Job, Progress, ProgressEvent, TaskContext, TaskReport, TaskRunner, Weight};
pub use world::{ChunkPos, RegionPos, World};
