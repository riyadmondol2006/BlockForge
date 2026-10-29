// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Java Edition worlds: region files, palettes, level data, readers and writers.
//!
//! Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//!
//! The low-level layer (this is what exists so far):
//! - [`version`]: the data-version table (`DATA_VERSIONS`), floor lookup and the named thresholds (`DV_1_13` ...).
//! - [`format`]: [`JavaFormat`], every on-disk switch of a data version (replaces Chunker's 21-class chain).
//! - [`region`]: Anvil `.mca` reader ([`region::RegionFile`]) and streaming writer ([`region::RegionWriter`]),
//!   `.mcc` chunks, the `entities/` and `poi/` folders.
//! - [`palette`]: palette index packing (spanning and padded layouts), nibble arrays, heightmaps.
//! - [`dirs`]: folder layouts (before and from 26.1), dimension discovery, [`dirs::detect_java`] for folders and zips.
//! - [`level_dat`]: reading `level.dat` for the data version, game version and name.

pub mod dirs;
pub mod format;
pub mod level_dat;
pub mod level_writer;
mod nbtio;
pub mod palette;
pub mod pretransform;
pub mod region;
pub mod resolvers;
pub mod version;
pub mod writer;

#[cfg(test)]
mod real_world;
#[cfg(test)]
pub(crate) mod testutil;

pub use dirs::{DimensionInfo, JavaDimension, JavaWorldInfo, LevelDirs, detect_java};
pub use format::JavaFormat;
pub use region::{RegionFile, RegionWriter};
