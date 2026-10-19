// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Whole-file helpers: the compression wrappers and headers around a root compound (Chunker's static helpers on
//! `Tag`, collapsed into a few generic functions).

use std::path::Path;

use super::compound::Compound;
use super::error::NbtError;
use super::io::{self, Flavor};
use super::tag::Tag;
use crate::convert::util::lz4_block;

/// How a root compound is wrapped on disk (the ids of Anvil's compression byte, except `None`'s 3 is a plain stream).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Compression {
    /// Anvil type 3 / `.dat` stored raw.
    None,
    /// Anvil type 1 / most `.dat` files.
    Gzip,
    /// Anvil type 2, the default.
    Zlib,
    /// Anvil type 4 (lz4-java's block stream).
    Lz4Block,
}

impl Compression {
    /// The compression of an Anvil chunk, from its type byte.
    pub fn from_anvil_id(id: u8) -> Option<Compression> {
        match id {
            1 => Some(Compression::Gzip),
            2 => Some(Compression::Zlib),
            3 => Some(Compression::None),
            4 => Some(Compression::Lz4Block),
            _ => None,
        }
    }

    /// The Anvil type byte.
    pub fn anvil_id(self) -> u8 {
        match self {
            Compression::Gzip => 1,
            Compression::Zlib => 2,
            Compression::None => 3,
            Compression::Lz4Block => 4,
        }
    }
}

/// Decompress `bytes` as `compression` says.
pub fn decompress(bytes: &[u8], compression: Compression) -> Result<Vec<u8>, NbtError> {
    match compression {
        Compression::None => Ok(bytes.to_vec()),
        Compression::Gzip => io::gunzip(bytes),
        Compression::Zlib => io::unzlib(bytes),
        Compression::Lz4Block => lz4_block::decode(bytes).map_err(|e| NbtError::Compression(e.to_string())),
    }
}

/// Compress `bytes` as `compression` says (zlib at the fastest level, as Chunker writes).
pub fn compress(bytes: &[u8], compression: Compression) -> Result<Vec<u8>, NbtError> {
    match compression {
        Compression::None => Ok(bytes.to_vec()),
        Compression::Gzip => io::gzip(bytes),
        Compression::Zlib => io::zlib(bytes),
        Compression::Lz4Block => Ok(lz4_block::encode(bytes)),
    }
}

fn into_compound(tag: Tag) -> Result<Compound, NbtError> {
    match tag {
        Tag::Compound(c) => Ok(c),
        other => Err(NbtError::Other(format!("NBT root is a {}, expected a compound", other.tag_type().name()))),
    }
}

/// Chunker's unwrapping of Java files: the compound `data` if there is one, else `Data` (level.dat), else the root.
pub fn unwrap_data(mut root: Compound) -> Compound {
    for key in ["data", "Data"] {
        if let Some(Tag::Compound(inner)) = root.remove(key) {
            return inner;
        }
    }
    root
}

/// Read a Java NBT blob, unwrapping `data`/`Data`. `Ok(None)` when the stream starts with an END tag.
pub fn read_java_nbt(bytes: &[u8], compression: Compression) -> Result<Option<Compound>, NbtError> {
    let raw = decompress(bytes, compression)?;
    match io::from_slice(&raw, Flavor::Java) {
        Ok((_, tag)) => Ok(Some(unwrap_data(into_compound(tag)?))),
        Err(NbtError::EmptyRoot) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Read a Java NBT blob and keep the root as it is (no unwrapping).
pub fn read_java_root(bytes: &[u8], compression: Compression) -> Result<Compound, NbtError> {
    let raw = decompress(bytes, compression)?;
    into_compound(io::from_slice(&raw, Flavor::Java)?.1)
}

/// Read a Java NBT file that may or may not be gzipped (checks for the gzip magic), unwrapping `data`/`Data`.
pub fn read_possible_gzip_java_file(path: impl AsRef<Path>) -> Result<Option<Compound>, NbtError> {
    let bytes = std::fs::read(path)?;
    let compression = if bytes.starts_with(&[0x1F, 0x8B]) { Compression::Gzip } else { Compression::None };
    read_java_nbt(&bytes, compression)
}

/// Read a Java NBT file with the given compression, unwrapping `data`/`Data`.
pub fn read_java_file(path: impl AsRef<Path>, compression: Compression) -> Result<Option<Compound>, NbtError> {
    read_java_nbt(&std::fs::read(path)?, compression)
}

/// Encode a root compound (empty name) as Java NBT. It is written as given: wrap it in `Data` yourself if needed.
pub fn write_java_nbt(root: &Compound, compression: Compression) -> Result<Vec<u8>, NbtError> {
    let raw = io::to_vec("", &Tag::Compound(root.clone()), Flavor::Java)?;
    compress(&raw, compression)
}

/// Write a Java NBT file.
pub fn write_java_file(path: impl AsRef<Path>, root: &Compound, compression: Compression) -> Result<(), NbtError> {
    Ok(std::fs::write(path, write_java_nbt(root, compression)?)?)
}

/// Read Bedrock NBT bytes without a header (LevelDB values). `Ok(None)` when empty or starting with END.
pub fn read_bedrock_nbt(bytes: &[u8]) -> Result<Option<Compound>, NbtError> {
    if bytes.is_empty() {
        return Ok(None);
    }
    match io::from_slice(bytes, Flavor::Bedrock) {
        Ok((_, tag)) => Ok(Some(into_compound(tag)?)),
        Err(NbtError::EmptyRoot) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Read a Bedrock `level.dat`: an 8-byte header (storage version, payload length, little-endian) and the NBT.
/// Returns the storage version and the root.
pub fn read_bedrock_level_dat(bytes: &[u8]) -> Result<(i32, Option<Compound>), NbtError> {
    let Some(head) = bytes.get(..8) else {
        return Err(NbtError::UnexpectedEof);
    };
    let version = i32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    Ok((version, read_bedrock_nbt(&bytes[8..])?))
}

/// Encode a root compound (empty name) as Bedrock NBT without a header.
pub fn write_bedrock_nbt(root: &Compound) -> Result<Vec<u8>, NbtError> {
    io::to_vec("", &Tag::Compound(root.clone()), Flavor::Bedrock)
}

/// Encode a Bedrock `level.dat`: storage version, payload length, then the payload.
pub fn write_bedrock_level_dat(storage_version: i32, root: &Compound) -> Result<Vec<u8>, NbtError> {
    let payload = write_bedrock_nbt(root)?;
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.extend_from_slice(&storage_version.to_le_bytes());
    out.extend_from_slice(&(payload.len() as i32).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}
