// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Private adapter between this module and the NBT layer: little-endian (Bedrock flavour) named tags, with the NBT
//! errors turned into [`BedrockError`].

use super::{BedrockError, Result};
use crate::convert::nbt::io::{self, Flavor};
use crate::convert::nbt::{NbtError, Tag};
use std::io::Cursor;

fn err(e: NbtError) -> BedrockError {
    BedrockError::Nbt(e.to_string())
}

/// Read one named tag from `c`, leaving the cursor right after it.
pub(crate) fn read_named(c: &mut Cursor<&[u8]>) -> Result<(String, Tag)> {
    io::read_named(c, Flavor::Bedrock).map_err(err)
}

/// Read every named tag in `bytes` back to back (block entities, legacy entities).
pub(crate) fn read_all_named(bytes: &[u8]) -> Result<Vec<(String, Tag)>> {
    io::read_compound_stream(bytes, Flavor::Bedrock).map_err(err)
}

/// Read a single named tag from the start of `bytes`.
pub(crate) fn from_slice(bytes: &[u8]) -> Result<(String, Tag)> {
    io::from_slice(bytes, Flavor::Bedrock).map_err(err)
}

/// Append one named tag to `out`.
pub(crate) fn write_named(out: &mut Vec<u8>, name: &str, tag: &Tag) -> Result<()> {
    io::write_named(out, name, tag, Flavor::Bedrock).map_err(err)
}

/// Encode one named tag.
pub(crate) fn to_vec(name: &str, tag: &Tag) -> Result<Vec<u8>> {
    io::to_vec(name, tag, Flavor::Bedrock).map_err(err)
}
