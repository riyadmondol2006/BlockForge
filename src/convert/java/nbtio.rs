// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! A small adapter over the NBT layer for this module: Java flavor only, infallible compression helpers, and
//! decompression with a caller-chosen size cap.

use crate::convert::nbt::io::{self, Flavor};
use crate::convert::nbt::{NbtError, Tag};
use crate::convert::util::lz4_block;
use std::io::Read;

fn read_limited(reader: impl Read, limit: u64, what: &str) -> Result<Vec<u8>, NbtError> {
    let mut out = Vec::new();
    reader.take(limit + 1).read_to_end(&mut out).map_err(|e| NbtError::Compression(format!("{what}: {e}")))?;
    if out.len() as u64 > limit {
        return Err(NbtError::Compression(format!("{what}: output is larger than {limit} bytes")));
    }
    Ok(out)
}

/// Decompress gzip, producing at most `limit` bytes.
pub fn gunzip(data: &[u8], limit: u64) -> Result<Vec<u8>, NbtError> {
    read_limited(flate2::read::GzDecoder::new(data), limit, "gzip")
}

/// Decompress zlib, producing at most `limit` bytes.
pub fn unzlib(data: &[u8], limit: u64) -> Result<Vec<u8>, NbtError> {
    read_limited(flate2::read::ZlibDecoder::new(data), limit, "zlib")
}

/// Decode an LZ4Block stream (the decoder caps its own output at 1 GiB; the caller's limit is checked after).
pub fn lz4_block_decode(data: &[u8], limit: u64) -> Result<Vec<u8>, NbtError> {
    let out = lz4_block::decode(data).map_err(|e| NbtError::Compression(e.to_string()))?;
    if out.len() as u64 > limit {
        return Err(NbtError::Compression(format!("LZ4: output is larger than {limit} bytes")));
    }
    Ok(out)
}

pub fn lz4_block_encode(data: &[u8]) -> Vec<u8> {
    lz4_block::encode(data)
}

/// Compress as gzip (compression only fails on memory exhaustion; that gives an empty vector).
pub fn gzip(data: &[u8]) -> Vec<u8> {
    io::gzip(data).unwrap_or_default()
}

/// Compress as zlib.
pub fn zlib(data: &[u8]) -> Vec<u8> {
    io::zlib(data).unwrap_or_default()
}

/// Gzip, zlib or raw, by looking at the first bytes.
pub fn decompress_auto(data: &[u8]) -> Result<Vec<u8>, NbtError> {
    io::decompress_auto(data)
}

/// Parse a named root tag from uncompressed Java NBT bytes.
pub fn from_slice(bytes: &[u8]) -> Result<(String, Tag), NbtError> {
    io::from_slice(bytes, Flavor::Java)
}

/// Serialize a named root tag as uncompressed Java NBT (an unwritable tree gives an empty vector).
pub fn to_vec(name: &str, tag: &Tag) -> Vec<u8> {
    io::to_vec(name, tag, Flavor::Java).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compression_helpers() {
        let data = vec![7u8; 10_000];
        assert_eq!(decompress_auto(&gzip(&data)).unwrap(), data);
        assert_eq!(decompress_auto(&zlib(&data)).unwrap(), data);
        assert_eq!(lz4_block_decode(&lz4_block_encode(&data), 1 << 20).unwrap(), data);
        assert!(gunzip(&gzip(&data), 100).is_err());
        assert!(unzlib(&zlib(&data), 100).is_err());
        assert!(lz4_block_decode(&lz4_block_encode(&data), 100).is_err());
        assert!(lz4_block_decode(&lz4_block_encode(&data)[..30], 1 << 20).is_err());
    }
}
