// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! xxHash32, the checksum of lz4-java's block stream.

const P1: u32 = 2_654_435_761;
const P2: u32 = 2_246_822_519;
const P3: u32 = 3_266_489_917;
const P4: u32 = 668_265_263;
const P5: u32 = 374_761_393;

fn round(acc: u32, input: u32) -> u32 {
    acc.wrapping_add(input.wrapping_mul(P2)).rotate_left(13).wrapping_mul(P1)
}

fn read_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// The xxHash32 of `data` with `seed`.
pub fn xxh32(data: &[u8], seed: u32) -> u32 {
    let mut rest = data;
    let mut h = if data.len() >= 16 {
        let mut v1 = seed.wrapping_add(P1).wrapping_add(P2);
        let mut v2 = seed.wrapping_add(P2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(P1);
        while rest.len() >= 16 {
            v1 = round(v1, read_u32(&rest[0..]));
            v2 = round(v2, read_u32(&rest[4..]));
            v3 = round(v3, read_u32(&rest[8..]));
            v4 = round(v4, read_u32(&rest[12..]));
            rest = &rest[16..];
        }
        v1.rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18))
    } else {
        seed.wrapping_add(P5)
    };
    h = h.wrapping_add(data.len() as u32);
    while rest.len() >= 4 {
        h = h.wrapping_add(read_u32(rest).wrapping_mul(P3)).rotate_left(17).wrapping_mul(P4);
        rest = &rest[4..];
    }
    for &b in rest {
        h = h.wrapping_add((b as u32).wrapping_mul(P5)).rotate_left(11).wrapping_mul(P1);
    }
    h ^= h >> 15;
    h = h.wrapping_mul(P2);
    h ^= h >> 13;
    h = h.wrapping_mul(P3);
    h ^ (h >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(xxh32(b"", 0), 0x02CC_5D05);
        assert_eq!(xxh32(b"a", 0), 0x550D_7456);
        assert_eq!(xxh32(b"abc", 0), 0x32D1_53FF);
        assert_eq!(xxh32(b"Nobody inspects the spammish repetition", 0), 0xE229_3B2F);
        assert_eq!(xxh32(b"", 1), 0x0B2C_B792);
    }
}
