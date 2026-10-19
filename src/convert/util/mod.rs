// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Small shared helpers (versions, bit packing, LZ4 block streams, ...).
//!
//! Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//!
//! Not ported because Rust has the equivalent built in: `ByteBufferInputStream` (use `std::io::Cursor`) and
//! `SneakyThrows` (use `Result`).

pub mod bits;
pub mod collections;
pub mod json_text;
pub mod lz4_block;
pub mod pos;
pub mod text;
pub mod varint;
pub mod version;
pub mod xxhash;

pub use collections::{InvertibleMap, compare_collections};
pub use pos::{BlockPosition, LegacyIdentifier, Vec3};
pub use version::Version;
