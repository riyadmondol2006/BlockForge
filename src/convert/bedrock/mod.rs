//! Bedrock Edition worlds: LevelDB access, keys, sub-chunk palettes, readers and writers.
//!
//! Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//!
//! This is the low level of the Bedrock side: everything here works on bytes and plain data, nothing knows about the
//! version-neutral model. The layers, bottom up:
//!
//! - [`db`]: [`db::BedrockDb`] over `rusty-leveldb` (raw deflate, zlib, snappy), plus an in-memory twin for tests.
//! - [`keys`]: chunk keys, the other well-known keys, column enumeration and the dimension id table.
//! - [`palette`]: sub-chunk (v8/v9) and Data3D/Data2D decoding and encoding.
//! - [`level_dat`] and [`data_version`]: the `level.dat` container and the table of known Bedrock versions.
//! - [`detect`]: finds a Bedrock world in a folder or a `.mcworld`/`.zip` archive.

pub mod data_version;
pub mod db;
pub mod detect;
pub mod keys;
pub mod level_dat;
mod nbt_le;
pub mod palette;
pub mod pretransform;
pub mod resolvers;

use std::fmt;

/// Every failure of the Bedrock low level. Nothing in this module panics on bad input.
#[derive(Debug)]
pub enum BedrockError {
    /// A file system error.
    Io(String),
    /// A LevelDB error (corruption, a locked or missing database, a write on a read-only handle).
    Db(String),
    /// Malformed NBT.
    Nbt(String),
    /// Malformed Bedrock data (a short buffer, an impossible bit width, a bad key).
    Format(String),
    /// Valid data that Chunker does not support (an old sub-chunk version, runtime palette ids).
    Unsupported(String),
    /// A bad or hostile archive.
    Archive(String),
}

/// The result type used across the Bedrock low level.
pub type Result<T> = std::result::Result<T, BedrockError>;

impl fmt::Display for BedrockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, msg) = match self {
            BedrockError::Io(m) => ("i/o error", m),
            BedrockError::Db(m) => ("database error", m),
            BedrockError::Nbt(m) => ("NBT error", m),
            BedrockError::Format(m) => ("malformed data", m),
            BedrockError::Unsupported(m) => ("unsupported", m),
            BedrockError::Archive(m) => ("archive error", m),
        };
        write!(f, "{kind}: {msg}")
    }
}

impl std::error::Error for BedrockError {}

impl From<std::io::Error> for BedrockError {
    fn from(e: std::io::Error) -> Self {
        BedrockError::Io(e.to_string())
    }
}

impl From<rusty_leveldb::Status> for BedrockError {
    fn from(e: rusty_leveldb::Status) -> Self {
        BedrockError::Db(format!("{:?}: {}", e.code, e.err))
    }
}

/// The real test world, if it is on this machine (tests skip cleanly when it is not).
#[cfg(test)]
pub(crate) fn test_world_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let p = std::path::Path::new(&home).join("Downloads").join("Mace, Spear, Sword PvP Map! V4.mcworld");
    p.is_file().then_some(p)
}

#[cfg(test)]
mod tests;
