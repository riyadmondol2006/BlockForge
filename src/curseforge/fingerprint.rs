//! CurseForge's file fingerprint: MurmurHash2 (32 bit, seed 1) of the file with the whitespace bytes
//! (tab, line feed, carriage return, space) taken out. The Java edition does the same in `Hashing.murmur`.

use std::path::Path;

use anyhow::{Context, Result};

const M: u32 = 0x5bd1_e995;
const R: u32 = 24;
const SEED: u32 = 1;

/// MurmurHash2 (the 32 bit version by Austin Appleby) of `data` with `seed`.
pub fn murmur2(data: &[u8], seed: u32) -> u32 {
    let mut h: u32 = seed ^ (data.len() as u32);
    let mut chunks = data.chunks_exact(4);
    for c in &mut chunks {
        let mut k = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);
        h = h.wrapping_mul(M);
        h ^= k;
    }
    let tail = chunks.remainder();
    if tail.len() >= 3 {
        h ^= (tail[2] as u32) << 16;
    }
    if tail.len() >= 2 {
        h ^= (tail[1] as u32) << 8;
    }
    if !tail.is_empty() {
        h ^= tail[0] as u32;
        h = h.wrapping_mul(M);
    }
    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^= h >> 15;
    h
}

/// The fingerprint of some bytes: whitespace removed, then murmur2 with seed 1.
pub fn fingerprint_bytes(data: &[u8]) -> u32 {
    let stripped: Vec<u8> = data.iter().copied().filter(|b| !matches!(b, 9 | 10 | 13 | 32)).collect();
    murmur2(&stripped, SEED)
}

/// The fingerprint of a file on disk.
pub fn fingerprint_file(path: &Path) -> Result<u32> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(fingerprint_bytes(&data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn murmur2_matches_reference_values() {
        // computed with a separate implementation of the algorithm (Python, same constants)
        assert_eq!(murmur2(b"", 1), 1_540_447_798);
        assert_eq!(murmur2(b"a", 1), 626_045_324);
        assert_eq!(murmur2(b"abcd", 1), 3_376_380_438);
        assert_eq!(murmur2(b"hello world", 1), 2_213_174_766);
        assert_eq!(murmur2(b"The quick brown fox jumps over the lazy dog", 0), 556_214_736);
    }

    #[test]
    fn whitespace_does_not_change_the_fingerprint() {
        assert_eq!(fingerprint_bytes(b"he llo\tw\r\norld"), fingerprint_bytes(b"helloworld"));
        assert_eq!(fingerprint_bytes(b"helloworld"), murmur2(b"helloworld", 1));
    }

    /// A jar the Java edition identified on CurseForge: its `fileFingerprint` is 3100092788. Runs only when that
    /// jar is on this computer.
    #[test]
    fn known_jar_has_curseforge_fingerprint() {
        let home = std::env::var("HOME").unwrap_or_default();
        let jar = std::path::Path::new(&home)
            .join(".local/share/atlauncher/instances/Minecraft263withFabric/mods/xaerominimap-fabric-26.3-26.5.1.jar");
        if !jar.is_file() {
            eprintln!("sample jar not found, skipping");
            return;
        }
        assert_eq!(fingerprint_file(&jar).unwrap(), 3_100_092_788);
    }
}
