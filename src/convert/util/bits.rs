// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Nibble arrays and bit-packed index arrays (palette storage).

/// The low (`lowest_bits`) or high nibble of a byte.
pub fn nibble_of(byte: u8, lowest_bits: bool) -> u8 {
    if lowest_bits { byte & 0xF } else { (byte >> 4) & 0xF }
}

/// `existing` with one nibble replaced by the low 4 bits of `input`.
pub fn update_nibble(existing: u8, input: u8, lowest_bits: bool) -> u8 {
    let other = nibble_of(existing, !lowest_bits);
    if lowest_bits { (other << 4) | (input & 0xF) } else { ((input & 0xF) << 4) | other }
}

/// Read entry `index` of a nibble array (even indices are the low nibble).
pub fn nibble_get(array: &[u8], index: usize) -> u8 {
    nibble_of(array[index >> 1], index & 1 == 0)
}

/// Write entry `index` of a nibble array.
pub fn nibble_set(array: &mut [u8], index: usize, value: u8) {
    let slot = &mut array[index >> 1];
    *slot = update_nibble(*slot, value, index & 1 == 0);
}

/// Chunker's `convertToNibbleArray`: 4096 values indexed `x * 256 + y * 16 + z` become the 2048 byte array of a
/// section, ordered `y * 256 + z * 16 + x`.
pub fn convert_to_nibble_array(xyz: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 2048];
    for i in 0..4096usize {
        let (x, z, y) = (i & 0xF, (i >> 4) & 0xF, (i >> 8) & 0xF);
        nibble_set(&mut out, i, xyz.get(x * 256 + y * 16 + z).copied().unwrap_or(0));
    }
    out
}

/// The number of bits needed to index `count` distinct values (0 for one or none).
pub fn bits_for_count(count: usize) -> u32 {
    if count <= 1 { 0 } else { usize::BITS - (count - 1).leading_zeros() }
}

/// Pack `values` of `bits` bits into longs the way Java 1.16+ does: entries never straddle two longs, the spare high
/// bits of each long stay zero, entry 0 is in the lowest bits. `bits` is 1..=32.
pub fn pack_padded(values: &[u32], bits: u32) -> Vec<i64> {
    assert!((1..=32).contains(&bits));
    let per = (64 / bits) as usize;
    let mask = (1u64 << bits) - 1;
    let mut out = vec![0i64; values.len().div_ceil(per)];
    for (i, &v) in values.iter().enumerate() {
        out[i / per] |= (((v as u64) & mask) << ((i % per) as u32 * bits)) as i64;
    }
    out
}

/// The inverse of [`pack_padded`]: `count` entries (missing longs read as zero).
pub fn unpack_padded(longs: &[i64], bits: u32, count: usize) -> Vec<u32> {
    assert!((1..=32).contains(&bits));
    let per = (64 / bits) as usize;
    let mask = (1u64 << bits) - 1;
    (0..count)
        .map(|i| {
            let word = longs.get(i / per).copied().unwrap_or(0) as u64;
            ((word >> ((i % per) as u32 * bits)) & mask) as u32
        })
        .collect()
}

/// Pack like Java before 1.16: entries are a continuous bit stream, so they may straddle two longs.
pub fn pack_spanning(values: &[u32], bits: u32) -> Vec<i64> {
    assert!((1..=32).contains(&bits));
    let mask = (1u64 << bits) - 1;
    let mut out = vec![0i64; (values.len() * bits as usize).div_ceil(64)];
    for (i, &v) in values.iter().enumerate() {
        let start = i * bits as usize;
        let (word, shift) = (start / 64, (start % 64) as u32);
        let v = (v as u64) & mask;
        out[word] |= (v << shift) as i64;
        if shift + bits > 64 {
            out[word + 1] |= (v >> (64 - shift)) as i64;
        }
    }
    out
}

/// The inverse of [`pack_spanning`].
pub fn unpack_spanning(longs: &[i64], bits: u32, count: usize) -> Vec<u32> {
    assert!((1..=32).contains(&bits));
    let mask = (1u64 << bits) - 1;
    (0..count)
        .map(|i| {
            let start = i * bits as usize;
            let (word, shift) = (start / 64, (start % 64) as u32);
            let mut v = (longs.get(word).copied().unwrap_or(0) as u64) >> shift;
            if shift + bits > 64 {
                v |= (longs.get(word + 1).copied().unwrap_or(0) as u64) << (64 - shift);
            }
            (v & mask) as u32
        })
        .collect()
}

/// Pack like Bedrock's storage: 32-bit words, entries never straddle two words, entry 0 in the lowest bits.
/// `bits` is 1..=16.
pub fn pack_words(values: &[u32], bits: u32) -> Vec<u32> {
    assert!((1..=16).contains(&bits));
    let per = (32 / bits) as usize;
    let mask = (1u32 << bits) - 1;
    let mut out = vec![0u32; values.len().div_ceil(per)];
    for (i, &v) in values.iter().enumerate() {
        out[i / per] |= (v & mask) << ((i % per) as u32 * bits);
    }
    out
}

/// The inverse of [`pack_words`].
pub fn unpack_words(words: &[u32], bits: u32, count: usize) -> Vec<u32> {
    assert!((1..=16).contains(&bits));
    let per = (32 / bits) as usize;
    let mask = (1u32 << bits) - 1;
    (0..count).map(|i| (words.get(i / per).copied().unwrap_or(0) >> ((i % per) as u32 * bits)) & mask).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nibbles() {
        assert_eq!(nibble_of(0xAB, true), 0xB);
        assert_eq!(nibble_of(0xAB, false), 0xA);
        assert_eq!(update_nibble(0xAB, 0x1F, true), 0xAF);
        assert_eq!(update_nibble(0xAB, 0x1, false), 0x1B);
        let mut a = vec![0u8; 4];
        for i in 0..8 {
            nibble_set(&mut a, i, i as u8 + 1);
        }
        assert_eq!(a, [0x21, 0x43, 0x65, 0x87]);
        assert_eq!((0..8).map(|i| nibble_get(&a, i)).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn nibble_array_layout() {
        // Chunker's loop: i = y*256 + z*16 + x reads input[x][y][z].
        let mut xyz = vec![0u8; 4096];
        xyz[3 * 256 + 5 * 16 + 7] = 9; // x = 3, y = 5, z = 7
        let out = convert_to_nibble_array(&xyz);
        assert_eq!(out.len(), 2048);
        let i = 5 * 256 + 7 * 16 + 3;
        assert_eq!(nibble_get(&out, i), 9);
        assert_eq!(out.iter().map(|b| b.count_ones()).sum::<u32>(), 2);
    }

    #[test]
    fn bit_widths() {
        assert_eq!(bits_for_count(0), 0);
        assert_eq!(bits_for_count(1), 0);
        assert_eq!(bits_for_count(2), 1);
        assert_eq!(bits_for_count(3), 2);
        assert_eq!(bits_for_count(16), 4);
        assert_eq!(bits_for_count(17), 5);
    }

    #[test]
    fn packing_round_trips() {
        for bits in 1..=16u32 {
            let n = 4096;
            let values: Vec<u32> = (0..n).map(|i| (i as u32 * 7 + 3) & ((1 << bits) - 1)).collect();
            assert_eq!(unpack_padded(&pack_padded(&values, bits), bits, n), values, "padded {bits}");
            assert_eq!(unpack_spanning(&pack_spanning(&values, bits), bits, n), values, "spanning {bits}");
            assert_eq!(unpack_words(&pack_words(&values, bits), bits, n), values, "words {bits}");
        }
    }

    #[test]
    fn packing_layout() {
        // 4 bits: 16 entries per long, entry 0 lowest. 5 bits: 12 per long with 4 spare bits.
        assert_eq!(pack_padded(&[1, 2, 3], 4), [0x321]);
        assert_eq!(pack_padded(&[31; 13], 5).len(), 2);
        assert_eq!(pack_padded(&[31; 12], 5)[0] as u64, (1u64 << 60) - 1);
        // Spanning packs 5-bit entries continuously: 13 entries need 65 bits.
        assert_eq!(pack_spanning(&[31; 13], 5).len(), 2);
        assert_eq!(pack_words(&[1, 2, 3], 4), [0x321]);
    }
}
