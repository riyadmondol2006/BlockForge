// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Palette index packing (Chunker's `PaletteUtil`), nibble arrays and heightmap packing.
//!
//! Section values are ordered `x | z << s | y << 2s` (x fastest) with `s` = 4 for blocks (4096 values) and 2 for
//! biomes (64 values). Two layouts exist: before 1.16 the values span 64-bit longs; from 1.16 each long holds
//! `64 / bits` whole values and the rest of the long is padding.

use std::fmt;

/// Smallest bits per entry of a block palette.
pub const MIN_BITS_BLOCKS: u32 = 4;
/// Smallest bits per entry of a biome palette.
pub const MIN_BITS_BIOMES: u32 = 1;
/// Values in a block section (16 x 16 x 16).
pub const BLOCKS_PER_SECTION: usize = 4096;
/// Values in a biome section (4 x 4 x 4).
pub const BIOMES_PER_SECTION: usize = 64;
/// Values in a heightmap (16 x 16).
pub const HEIGHTMAP_VALUES: usize = 256;
/// The widest entry the packers handle (indices are `u16`).
pub const MAX_BITS: u32 = 16;

/// How values sit in the longs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Before 1.16: values run across the 64-bit boundaries.
    Spanning,
    /// 1.16 and later: whole values only, the high bits of each long are padding.
    Padded,
}

impl Layout {
    /// The layout a writer uses for a data version (spanning below 1.16).
    pub fn for_data_version(data_version: i32) -> Layout {
        if data_version >= super::version::DV_1_16 { Layout::Padded } else { Layout::Spanning }
    }
}

/// What went wrong when decoding packed values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteError {
    /// Bits per entry is 0 or more than 16.
    BadBits(u32),
    /// The long array is too short for the layout: (have, need).
    TooShort(usize, usize),
    /// No layout/bit width explains a long array of this length.
    UnknownLength(usize),
}

impl fmt::Display for PaletteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaletteError::BadBits(b) => write!(f, "unsupported bits per palette entry: {b}"),
            PaletteError::TooShort(have, need) => write!(f, "packed palette has {have} longs, needs {need}"),
            PaletteError::UnknownLength(n) => write!(f, "palette data of {n} longs matches no bit width"),
        }
    }
}

impl std::error::Error for PaletteError {}

/// Bits per entry for a palette of `key_count` keys: at least `min_bits`, enough to index every key.
pub fn bits_for(min_bits: u32, key_count: usize) -> u32 {
    let mut bits = min_bits.max(1);
    while bits < 32 && key_count > (1usize << bits) {
        bits += 1;
    }
    bits
}

/// How many whole values one long holds in the padded layout.
pub fn values_per_long(bits: u32) -> usize {
    (64 / bits.max(1)) as usize
}

/// How many longs `count` values of `bits` bits take.
pub fn packed_len(layout: Layout, count: usize, bits: u32) -> usize {
    match layout {
        Layout::Spanning => (count * bits as usize).div_ceil(64),
        Layout::Padded => count.div_ceil(values_per_long(bits)),
    }
}

/// Pack `values` (each masked to `bits`) into longs. `bits` must be 1..=16.
pub fn pack(layout: Layout, bits: u32, values: &[u16]) -> Result<Vec<i64>, PaletteError> {
    if bits == 0 || bits > MAX_BITS {
        return Err(PaletteError::BadBits(bits));
    }
    let mask = (1u64 << bits) - 1;
    let mut out = vec![0i64; packed_len(layout, values.len(), bits)];
    match layout {
        Layout::Spanning => {
            for (i, &v) in values.iter().enumerate() {
                let v = v as u64 & mask;
                let bit = i * bits as usize;
                let (start, sub) = (bit / 64, (bit % 64) as u32);
                out[start] |= (v << sub) as i64;
                if sub + bits > 64 {
                    out[start + 1] |= (v >> (64 - sub)) as i64;
                }
            }
        }
        Layout::Padded => {
            let per = values_per_long(bits);
            for (i, &v) in values.iter().enumerate() {
                out[i / per] |= ((v as u64 & mask) << ((i % per) as u32 * bits)) as i64;
            }
        }
    }
    Ok(out)
}

/// Unpack `count` values of `bits` bits. A long array that is too short is an error, a longer one is tolerated.
pub fn unpack(layout: Layout, bits: u32, count: usize, longs: &[i64]) -> Result<Vec<u16>, PaletteError> {
    if bits == 0 || bits > MAX_BITS {
        return Err(PaletteError::BadBits(bits));
    }
    let need = packed_len(layout, count, bits);
    if longs.len() < need {
        return Err(PaletteError::TooShort(longs.len(), need));
    }
    let mask = (1u64 << bits) - 1;
    let mut out = Vec::with_capacity(count);
    match layout {
        Layout::Spanning => {
            for i in 0..count {
                let bit = i * bits as usize;
                let (start, sub) = (bit / 64, (bit % 64) as u32);
                let mut v = (longs[start] as u64) >> sub;
                if sub + bits > 64 {
                    v |= (longs[start + 1] as u64) << (64 - sub);
                }
                out.push((v & mask) as u16);
            }
        }
        Layout::Padded => {
            let per = values_per_long(bits);
            for i in 0..count {
                out.push((((longs[i / per] as u64) >> ((i % per) as u32 * bits)) & mask) as u16);
            }
        }
    }
    Ok(out)
}

/// `writePaletteValues1_13` / `writePaletteValues1_16`: pack indices for a palette of `key_count` keys. `values`
/// None writes all zeros (a single-value palette still carries `data` for blocks).
pub fn pack_palette(
    layout: Layout,
    min_bits: u32,
    count: usize,
    key_count: usize,
    values: Option<&[u16]>,
) -> Result<Vec<i64>, PaletteError> {
    let bits = bits_for(min_bits, key_count);
    match values {
        Some(v) => pack(layout, bits, v),
        None => Ok(vec![0i64; packed_len(layout, count, bits)]),
    }
}

/// `readPaletteValues`: decode with the layout guessed from the array length, like Chunker. The bits come from the
/// key count; when the length does not fit the padded layout for those bits, the array is read as spanning with
/// `bits = len * 64 / count`. Beyond Chunker: a length that exactly fits some other (layout, bits) pair is read
/// that way first (see [`detect_by_length`]), which handles tools that write wider entries than the palette needs.
pub fn unpack_auto(min_bits: u32, count: usize, key_count: usize, longs: &[i64]) -> Result<Vec<u16>, PaletteError> {
    let bits = bits_for(min_bits, key_count);
    if bits <= MAX_BITS && longs.len() == packed_len(Layout::Padded, count, bits) {
        return unpack(Layout::Padded, bits, count, longs);
    }
    let spanning_bits = (longs.len() * 64 / count.max(1)) as u32;
    if (1..=MAX_BITS).contains(&spanning_bits) && longs.len() == packed_len(Layout::Spanning, count, spanning_bits) {
        return unpack(Layout::Spanning, spanning_bits, count, longs);
    }
    if let Some((layout, bits)) = detect_by_length(count, longs.len(), 1) {
        return unpack(layout, bits, count, longs);
    }
    // Chunker's lenient path: spanning with the floored width, tolerating trailing longs.
    unpack(Layout::Spanning, spanning_bits, count, longs)
}

/// Find a (layout, bits) pair whose packed length is exactly `len` for `count` values, trying the padded layout
/// first, with bits from `min_bits` up to 16.
pub fn detect_by_length(count: usize, len: usize, min_bits: u32) -> Option<(Layout, u32)> {
    for layout in [Layout::Padded, Layout::Spanning] {
        for bits in min_bits.max(1)..=MAX_BITS {
            if packed_len(layout, count, bits) == len {
                return Some((layout, bits));
            }
        }
    }
    None
}

/// Index of (x, y, z) in a section of side `1 << shift` (4 for blocks gives 16 wide, 2 gives 4 wide).
pub fn index(shift: u32, x: usize, y: usize, z: usize) -> usize {
    x | (z << shift) | (y << (shift * 2))
}

/// Index of a block in a 16^3 section.
pub fn block_index(x: usize, y: usize, z: usize) -> usize {
    index(4, x, y, z)
}

/// Index of a biome cell in a 4^3 section.
pub fn biome_index(x: usize, y: usize, z: usize) -> usize {
    index(2, x, y, z)
}

// ---- nibble arrays (light, legacy Data/Add) ----

/// Read nibble `i` (the low nibble of a byte comes first).
pub fn get_nibble(data: &[u8], i: usize) -> u8 {
    match data.get(i >> 1) {
        Some(b) if i & 1 == 0 => b & 0x0F,
        Some(b) => b >> 4,
        None => 0,
    }
}

/// Write nibble `i`; indexes past the end are ignored.
pub fn set_nibble(data: &mut [u8], i: usize, value: u8) {
    if let Some(b) = data.get_mut(i >> 1) {
        if i & 1 == 0 {
            *b = (*b & 0xF0) | (value & 0x0F);
        } else {
            *b = (*b & 0x0F) | ((value & 0x0F) << 4);
        }
    }
}

/// Expand a nibble array into one byte per value (`count` values; missing data reads as 0).
pub fn unpack_nibbles(data: &[u8], count: usize) -> Vec<u8> {
    (0..count).map(|i| get_nibble(data, i)).collect()
}

/// Pack values (each masked to 4 bits) into a nibble array of `values.len() / 2` bytes (rounded up).
pub fn pack_nibbles(values: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; values.len().div_ceil(2)];
    for (i, &v) in values.iter().enumerate() {
        set_nibble(&mut out, i, v);
    }
    out
}

/// Legacy (before 1.13) section arrays to `(block id, data)` per position: `Blocks` byte[4096] with the optional
/// `Add` nibble as the high 4 bits of the id, and the optional `Data` nibble. Arrays that are too short give an error.
pub fn legacy_blocks(blocks: &[u8], add: Option<&[u8]>, data: Option<&[u8]>) -> Result<Vec<(u16, u8)>, PaletteError> {
    if blocks.len() < BLOCKS_PER_SECTION {
        return Err(PaletteError::TooShort(blocks.len(), BLOCKS_PER_SECTION));
    }
    for nib in [add, data].into_iter().flatten() {
        if nib.len() < BLOCKS_PER_SECTION / 2 {
            return Err(PaletteError::TooShort(nib.len(), BLOCKS_PER_SECTION / 2));
        }
    }
    Ok((0..BLOCKS_PER_SECTION)
        .map(|i| {
            let hi = add.map_or(0, |a| get_nibble(a, i)) as u16;
            (blocks[i] as u16 | (hi << 8), data.map_or(0, |d| get_nibble(d, i)))
        })
        .collect())
}

/// The inverse of [`legacy_blocks`]: `(Blocks, Add, Data)` for ids up to 4095.
pub fn legacy_blocks_encode(values: &[(u16, u8)]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut blocks = vec![0u8; BLOCKS_PER_SECTION];
    let mut add = vec![0u8; BLOCKS_PER_SECTION / 2];
    let mut data = vec![0u8; BLOCKS_PER_SECTION / 2];
    for (i, &(id, d)) in values.iter().enumerate().take(BLOCKS_PER_SECTION) {
        blocks[i] = (id & 0xFF) as u8;
        set_nibble(&mut add, i, ((id >> 8) & 0xF) as u8);
        set_nibble(&mut data, i, d);
    }
    (blocks, add, data)
}

// ---- heightmaps ----

/// Bits per heightmap entry for a world of `height` blocks: enough for 0..=height (9 for the 384-high overworld).
pub fn heightmap_bits(height: u32) -> u32 {
    32 - height.leading_zeros()
}

/// Pack 256 heights (spanning before 1.16, padded after). `bits` is usually [`heightmap_bits`] (9).
pub fn pack_heightmap(layout: Layout, bits: u32, heights: &[u16]) -> Result<Vec<i64>, PaletteError> {
    pack(layout, bits, heights)
}

/// Unpack 256 heights.
pub fn unpack_heightmap(layout: Layout, bits: u32, longs: &[i64]) -> Result<Vec<u16>, PaletteError> {
    unpack(layout, bits, HEIGHTMAP_VALUES, longs)
}

/// Unpack a heightmap, taking the layout from the array length (37 longs of 9 bits padded, 36 spanning).
pub fn unpack_heightmap_auto(bits: u32, longs: &[i64]) -> Result<Vec<u16>, PaletteError> {
    if longs.len() == packed_len(Layout::Padded, HEIGHTMAP_VALUES, bits) {
        unpack(Layout::Padded, bits, HEIGHTMAP_VALUES, longs)
    } else {
        unpack(Layout::Spanning, bits, HEIGHTMAP_VALUES, longs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift64 so the tests need no rand crate.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    fn random_values(rng: &mut Rng, count: usize, bits: u32) -> Vec<u16> {
        let mask = (1u64 << bits) - 1;
        (0..count).map(|_| (rng.next() & mask) as u16).collect()
    }

    #[test]
    fn round_trip_every_width_and_layout() {
        let mut rng = Rng(0x9E3779B97F4A7C15);
        for layout in [Layout::Spanning, Layout::Padded] {
            for bits in 1..=16 {
                for count in [BLOCKS_PER_SECTION, BIOMES_PER_SECTION, HEIGHTMAP_VALUES, 1, 3, 65] {
                    let values = random_values(&mut rng, count, bits);
                    let packed = pack(layout, bits, &values).unwrap();
                    assert_eq!(packed.len(), packed_len(layout, count, bits));
                    assert_eq!(unpack(layout, bits, count, &packed).unwrap(), values, "{layout:?} bits {bits}");
                }
            }
        }
    }

    #[test]
    fn layouts_coincide_for_dividing_widths() {
        let mut rng = Rng(7);
        for bits in [1, 2, 4, 8, 16] {
            let values = random_values(&mut rng, BLOCKS_PER_SECTION, bits);
            assert_eq!(pack(Layout::Spanning, bits, &values).unwrap(), pack(Layout::Padded, bits, &values).unwrap());
        }
        // 5 bits differ: 342 longs padded against 320 spanning.
        assert_eq!(packed_len(Layout::Padded, 4096, 5), 342);
        assert_eq!(packed_len(Layout::Spanning, 4096, 5), 320);
    }

    #[test]
    fn known_vectors() {
        // 4 bits padded: value i goes to long i/16, shift 4*(i%16).
        let mut values = vec![0u16; 4096];
        values[0] = 0xF;
        values[1] = 0x3;
        values[16] = 0x9;
        let packed = pack(Layout::Padded, 4, &values).unwrap();
        assert_eq!(packed[0], 0x3F);
        assert_eq!(packed[1], 0x9);
        // 5 bits spanning: value 12 starts at bit 60 and crosses into the second long.
        let mut v = vec![0u16; 4096];
        v[12] = 0b10110;
        let s = pack(Layout::Spanning, 5, &v).unwrap();
        assert_eq!(s[0] as u64, 0b0110u64 << 60);
        assert_eq!(s[1], 0b1);
        // 5 bits padded: 12 values per long, so value 12 is the first of the next long.
        let p = pack(Layout::Padded, 5, &v).unwrap();
        assert_eq!(p[0], 0);
        assert_eq!(p[1], 0b10110);
    }

    #[test]
    fn bits_for_keys() {
        assert_eq!(bits_for(4, 1), 4);
        assert_eq!(bits_for(4, 16), 4);
        assert_eq!(bits_for(4, 17), 5);
        assert_eq!(bits_for(4, 4096), 12);
        assert_eq!(bits_for(1, 1), 1);
        assert_eq!(bits_for(1, 2), 1);
        assert_eq!(bits_for(1, 3), 2);
        assert_eq!(bits_for(1, 64), 6);
    }

    #[test]
    fn auto_detects_both_layouts() {
        let mut rng = Rng(42);
        for keys in [2usize, 5, 16, 17, 33, 100, 300, 1000, 4096] {
            let bits = bits_for(MIN_BITS_BLOCKS, keys);
            let values: Vec<u16> = (0..4096).map(|_| (rng.next() % keys as u64) as u16).collect();
            for layout in [Layout::Spanning, Layout::Padded] {
                let packed = pack_palette(layout, MIN_BITS_BLOCKS, 4096, keys, Some(&values)).unwrap();
                assert_eq!(
                    unpack_auto(MIN_BITS_BLOCKS, 4096, keys, &packed).unwrap(),
                    values,
                    "{keys} keys {bits} bits {layout:?}"
                );
            }
        }
        // Biomes: 64 values.
        for keys in [2usize, 3, 5, 9, 17, 33, 64] {
            let values: Vec<u16> = (0..64).map(|_| (rng.next() % keys as u64) as u16).collect();
            for layout in [Layout::Spanning, Layout::Padded] {
                let packed = pack_palette(layout, MIN_BITS_BIOMES, 64, keys, Some(&values)).unwrap();
                assert_eq!(unpack_auto(MIN_BITS_BIOMES, 64, keys, &packed).unwrap(), values, "{keys} keys {layout:?}");
            }
        }
    }

    #[test]
    fn auto_survives_wider_data_than_the_key_count() {
        // Some tools write more bits than the palette needs; the length still identifies the layout.
        let mut rng = Rng(99);
        let values = random_values(&mut rng, 4096, 6);
        let packed = pack(Layout::Padded, 6, &values).unwrap();
        assert_eq!(unpack_auto(MIN_BITS_BLOCKS, 4096, 20, &packed).unwrap(), values);
    }

    #[test]
    fn empty_values_write_zeros() {
        let packed = pack_palette(Layout::Padded, MIN_BITS_BLOCKS, 4096, 1, None).unwrap();
        assert_eq!(packed.len(), 256);
        assert!(packed.iter().all(|&l| l == 0));
        let spanning = pack_palette(Layout::Spanning, MIN_BITS_BLOCKS, 4096, 17, None).unwrap();
        assert_eq!(spanning.len(), 320);
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert_eq!(unpack(Layout::Padded, 0, 10, &[]), Err(PaletteError::BadBits(0)));
        assert_eq!(unpack(Layout::Padded, 17, 10, &[0; 10]), Err(PaletteError::BadBits(17)));
        assert_eq!(unpack(Layout::Padded, 4, 4096, &[0; 5]), Err(PaletteError::TooShort(5, 256)));
        assert!(unpack_auto(4, 4096, 20, &[]).is_err());
        assert!(unpack_auto(4, 4096, 20, &[1, 2, 3]).is_err());
        assert!(pack(Layout::Spanning, 20, &[1]).is_err());
        // A value wider than the width is masked, not smeared into its neighbours.
        let packed = pack(Layout::Padded, 4, &[0xFFFF, 0]).unwrap();
        assert_eq!(unpack(Layout::Padded, 4, 2, &packed).unwrap(), vec![0xF, 0]);
    }

    #[test]
    fn indexes() {
        assert_eq!(block_index(0, 0, 0), 0);
        assert_eq!(block_index(1, 0, 0), 1);
        assert_eq!(block_index(0, 0, 1), 16);
        assert_eq!(block_index(0, 1, 0), 256);
        assert_eq!(block_index(15, 15, 15), 4095);
        assert_eq!(biome_index(3, 3, 3), 63);
        assert_eq!(biome_index(0, 1, 0), 16);
        assert_eq!(biome_index(0, 0, 1), 4);
    }

    #[test]
    fn nibbles() {
        let mut data = vec![0u8; 2048];
        set_nibble(&mut data, 0, 0xA);
        set_nibble(&mut data, 1, 0x5);
        set_nibble(&mut data, 4095, 0xF);
        assert_eq!(data[0], 0x5A);
        assert_eq!(data[2047], 0xF0);
        assert_eq!(get_nibble(&data, 0), 0xA);
        assert_eq!(get_nibble(&data, 1), 0x5);
        assert_eq!(get_nibble(&data, 5000), 0, "past the end reads 0");
        let mut rng = Rng(5);
        let values: Vec<u8> = (0..4096).map(|_| (rng.next() & 0xF) as u8).collect();
        assert_eq!(unpack_nibbles(&pack_nibbles(&values), 4096), values);
    }

    #[test]
    fn legacy_arrays_round_trip() {
        let mut rng = Rng(11);
        let values: Vec<(u16, u8)> =
            (0..4096).map(|_| ((rng.next() % 4096) as u16, (rng.next() & 0xF) as u8)).collect();
        let (blocks, add, data) = legacy_blocks_encode(&values);
        assert_eq!(legacy_blocks(&blocks, Some(&add), Some(&data)).unwrap(), values);
        // Without Add/Data the ids stay below 256 and the data is 0.
        let plain = legacy_blocks(&blocks, None, None).unwrap();
        assert_eq!(plain[3], (values[3].0 & 0xFF, 0));
        assert!(legacy_blocks(&blocks[..100], None, None).is_err());
        assert!(legacy_blocks(&blocks, Some(&add[..10]), None).is_err());
    }

    #[test]
    fn heightmaps() {
        assert_eq!(heightmap_bits(384), 9);
        assert_eq!(heightmap_bits(256), 9);
        assert_eq!(heightmap_bits(255), 8);
        let mut rng = Rng(3);
        let heights = random_values(&mut rng, 256, 9);
        for layout in [Layout::Spanning, Layout::Padded] {
            let packed = pack_heightmap(layout, 9, &heights).unwrap();
            assert_eq!(packed.len(), if layout == Layout::Spanning { 36 } else { 37 });
            assert_eq!(unpack_heightmap(layout, 9, &packed).unwrap(), heights);
            assert_eq!(unpack_heightmap_auto(9, &packed).unwrap(), heights);
        }
    }

    #[test]
    fn layout_by_data_version() {
        assert_eq!(Layout::for_data_version(1631), Layout::Spanning);
        assert_eq!(Layout::for_data_version(2566), Layout::Padded);
        assert_eq!(Layout::for_data_version(2230), Layout::Spanning);
    }

    #[test]
    fn detect_by_length_basics() {
        assert_eq!(detect_by_length(4096, 256, 4), Some((Layout::Padded, 4)));
        assert_eq!(detect_by_length(4096, 320, 4), Some((Layout::Spanning, 5)));
        assert_eq!(detect_by_length(4096, 7, 4), None);
    }
}
