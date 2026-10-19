// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! lz4-java's `LZ4BlockOutputStream` / `LZ4BlockInputStream` stream format, which is what Anvil compression type 4
//! stores. It is not the standard LZ4 frame format.
//!
//! A stream is a series of blocks, each with a 21 byte header: the magic `LZ4Block`, a token (method in the high
//! nibble, `0x10` raw or `0x20` LZ4; block size level in the low nibble, block size `1 << (level + 10)`), the
//! compressed length, the decompressed length and a checksum (all little-endian `i32`), then the payload. An empty
//! block (both lengths zero) ends the stream. The checksum is xxHash32 with seed `0x9747B28C` of the decompressed
//! block, truncated to 28 bits (lz4-java's `Checksum` adapter masks with `0xFFFFFFF`).

use std::fmt;

use super::xxhash::xxh32;

const MAGIC: &[u8; 8] = b"LZ4Block";
const HEADER_LEN: usize = 21;
const METHOD_RAW: u8 = 0x10;
const METHOD_LZ4: u8 = 0x20;
const SEED: u32 = 0x9747_B28C;
const COMPRESSION_LEVEL_BASE: u32 = 10;
const MAX_BLOCK_SIZE: usize = 1 << (COMPRESSION_LEVEL_BASE + 0x0F);
/// lz4-java's default block size (64 KiB), compression level 6.
const DEFAULT_LEVEL: u8 = 6;
/// Bound on the decoded size, against decompression bombs.
const MAX_OUTPUT: usize = 1 << 30;

/// A stream that could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lz4Error(pub &'static str);

impl fmt::Display for Lz4Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LZ4 block stream: {}", self.0)
    }
}

impl std::error::Error for Lz4Error {}

fn checksum(data: &[u8]) -> i32 {
    (xxh32(data, SEED) & 0x0FFF_FFFF) as i32
}

fn le_i32(b: &[u8]) -> i32 {
    i32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Decode a whole LZ4Block stream. Input that ends between blocks (no end marker) is accepted, as lz4-java does.
pub fn decode(input: &[u8]) -> Result<Vec<u8>, Lz4Error> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < input.len() {
        let head = input.get(pos..pos + HEADER_LEN).ok_or(Lz4Error("truncated block header"))?;
        if &head[..8] != MAGIC {
            return Err(Lz4Error("bad magic"));
        }
        let token = head[8];
        let method = token & 0xF0;
        let level = u32::from(token & 0x0F) + COMPRESSION_LEVEL_BASE;
        let compressed = le_i32(&head[9..]);
        let original = le_i32(&head[13..]);
        let check = le_i32(&head[17..]);
        pos += HEADER_LEN;

        let max_block = 1usize << level;
        if compressed < 0
            || original < 0
            || compressed as usize > max_block
            || original as usize > max_block
            || (original == 0 && compressed != 0)
            || (original != 0 && compressed == 0)
            || (method == METHOD_RAW && original != compressed)
        {
            return Err(Lz4Error("corrupted block lengths"));
        }
        if original == 0 {
            // The end marker.
            if check != 0 {
                return Err(Lz4Error("corrupted end marker"));
            }
            break;
        }
        let (compressed, original) = (compressed as usize, original as usize);
        let payload = input.get(pos..pos + compressed).ok_or(Lz4Error("truncated block"))?;
        pos += compressed;
        if out.len() + original > MAX_OUTPUT {
            return Err(Lz4Error("output too large"));
        }
        let start = out.len();
        match method {
            METHOD_RAW => out.extend_from_slice(payload),
            METHOD_LZ4 => {
                let block =
                    lz4_flex::block::decompress(payload, original).map_err(|_| Lz4Error("corrupted LZ4 data"))?;
                if block.len() != original {
                    return Err(Lz4Error("block decoded to the wrong size"));
                }
                out.extend_from_slice(&block);
            }
            _ => return Err(Lz4Error("unknown compression method")),
        }
        let h = xxh32(&out[start..], SEED);
        if check != checksum(&out[start..]) && check != h as i32 {
            return Err(Lz4Error("checksum mismatch"));
        }
    }
    Ok(out)
}

/// Encode `input` as an LZ4Block stream with 64 KiB blocks and the end marker.
pub fn encode(input: &[u8]) -> Vec<u8> {
    let block_size = 1usize << (u32::from(DEFAULT_LEVEL) + COMPRESSION_LEVEL_BASE);
    debug_assert!(block_size <= MAX_BLOCK_SIZE);
    let mut out = Vec::with_capacity(input.len() / 2 + 2 * HEADER_LEN);
    for chunk in input.chunks(block_size) {
        let compressed = lz4_flex::block::compress(chunk);
        let (method, payload): (u8, &[u8]) =
            if compressed.len() >= chunk.len() { (METHOD_RAW, chunk) } else { (METHOD_LZ4, &compressed) };
        push_header(&mut out, method | DEFAULT_LEVEL, payload.len(), chunk.len(), checksum(chunk));
        out.extend_from_slice(payload);
    }
    push_header(&mut out, METHOD_RAW | DEFAULT_LEVEL, 0, 0, 0);
    out
}

fn push_header(out: &mut Vec<u8>, token: u8, compressed: usize, original: usize, check: i32) {
    out.extend_from_slice(MAGIC);
    out.push(token);
    out.extend_from_slice(&(compressed as i32).to_le_bytes());
    out.extend_from_slice(&(original as i32).to_le_bytes());
    out.extend_from_slice(&check.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(n: usize) -> Vec<u8> {
        (0..n).map(|i| ((i * 31) ^ (i >> 5)) as u8 % 17).collect()
    }

    #[test]
    fn round_trips() {
        for n in [0, 1, 100, 65535, 65536, 65537, 300_000] {
            let data = sample(n);
            assert_eq!(decode(&encode(&data)).unwrap(), data, "n = {n}");
        }
    }

    #[test]
    fn incompressible_blocks_are_raw() {
        // A pseudo-random block does not compress, so it must be stored raw.
        let mut x = 0x1234_5678u32;
        let data: Vec<u8> = (0..4096)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 24) as u8
            })
            .collect();
        let enc = encode(&data);
        assert_eq!(enc[8] & 0xF0, METHOD_RAW);
        assert_eq!(decode(&enc).unwrap(), data);
    }

    #[test]
    fn layout_of_an_empty_stream() {
        let enc = encode(b"");
        assert_eq!(enc.len(), HEADER_LEN);
        assert_eq!(&enc[..8], b"LZ4Block");
        assert_eq!(enc[8], METHOD_RAW | DEFAULT_LEVEL);
        assert!(enc[9..].iter().all(|&b| b == 0));
    }

    #[test]
    fn hand_built_raw_block() {
        // One raw block holding "hello", then the end marker.
        let mut s = Vec::new();
        push_header(&mut s, METHOD_RAW | 6, 5, 5, checksum(b"hello"));
        s.extend_from_slice(b"hello");
        push_header(&mut s, METHOD_RAW | 6, 0, 0, 0);
        assert_eq!(decode(&s).unwrap(), b"hello");
        // Without the end marker it is accepted as well.
        assert_eq!(decode(&s[..HEADER_LEN + 5]).unwrap(), b"hello");
    }

    #[test]
    fn rejects_corruption() {
        let mut enc = encode(&sample(1000));
        assert!(decode(&enc[..30]).is_err());
        enc[0] = b'X';
        assert!(decode(&enc).is_err());
        let mut enc = encode(&sample(1000));
        let last = HEADER_LEN + 3;
        enc[last] ^= 0xFF;
        assert!(decode(&enc).is_err());
        let mut enc = encode(b"abc");
        enc[17] ^= 1; // checksum
        assert!(decode(&enc).is_err());
        // A huge claimed length must be refused, not allocated.
        let mut s = Vec::new();
        push_header(&mut s, METHOD_LZ4 | 15, 10, i32::MAX as usize, 0);
        assert!(decode(&s).is_err());
    }
}
