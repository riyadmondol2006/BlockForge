// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Sub-chunk and biome data of a column: decoding and encoding of Bedrock's paletted storages.
//!
//! A *storage* is `u8 header (bits << 1 | runtime)`, `ceil(4096 / (32 / bits))` little-endian u32 words of packed
//! indices (values never straddle a word), `i32 palette size` and the palette entries. Block storages hold named NBT
//! compounds `{name, states, version}`, biome storages hold `i32` biome ids. The index of block `(x, y, z)` is
//! `x * 256 + z * 16 + y`. Header bits 127 mean "empty / same as the previous section" and bits 0 mean a single value
//! without words or size.
//!
//! The functions work on plain bytes and return plain data, and encoding what was decoded reproduces the bytes.

use super::{BedrockError, Result, nbt_le};
use crate::convert::nbt::{Compound, Tag};
use std::io::Cursor;

/// Blocks (or biome cells) in a 16x16x16 storage.
pub const STORAGE_SIZE: usize = 4096;
/// Bit widths a storage can use (`PaletteUtil.BITS_PER_ENTRY`).
pub const BITS_PER_ENTRY: [u8; 8] = [1, 2, 3, 4, 5, 6, 8, 16];
/// Header bits of the special "empty / inherit" storage (`PaletteUtil.EMPTY_PALETTE_BITS`).
pub const EMPTY_PALETTE_BITS: u8 = 127;

fn fmt_err(m: impl Into<String>) -> BedrockError {
    BedrockError::Format(m.into())
}

/// The index of a block in a storage.
pub fn storage_index(x: usize, y: usize, z: usize) -> usize {
    ((x & 15) << 8) | ((z & 15) << 4) | (y & 15)
}

/// One decoded storage: palette entries plus the 4096 indices into them (in [`storage_index`] order).
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteData<T> {
    /// The bit width stored in the header (0 = single value, 127 = empty/inherit, else one of [`BITS_PER_ENTRY`]).
    pub bits: u8,
    /// The header's runtime flag (network encoding); disk block storages never set it, disk biome storages do.
    pub runtime: bool,
    pub entries: Vec<T>,
    /// 4096 indices into `entries` (all zero for an empty/single-value storage).
    pub indices: Vec<u16>,
}

impl<T> PaletteData<T> {
    /// A storage of one value everywhere, written the way Chunker does (1 bit, all-zero words).
    pub fn uniform(entry: T) -> PaletteData<T> {
        PaletteData { bits: 1, runtime: false, entries: vec![entry], indices: vec![0; STORAGE_SIZE] }
    }

    /// A storage from entries and indices, with the smallest bit width that fits.
    pub fn from_indices(entries: Vec<T>, indices: Vec<u16>) -> Result<PaletteData<T>> {
        if indices.len() != STORAGE_SIZE {
            return Err(fmt_err(format!("a storage has 4096 indices, got {}", indices.len())));
        }
        Ok(PaletteData { bits: minimal_bits(entries.len()), runtime: false, entries, indices })
    }

    /// The special empty/inherit storage (header bits 127).
    pub fn inherit() -> PaletteData<T> {
        PaletteData { bits: EMPTY_PALETTE_BITS, runtime: false, entries: Vec::new(), indices: vec![0; STORAGE_SIZE] }
    }

    /// Whether this is the empty/inherit storage.
    pub fn is_inherit(&self) -> bool {
        self.bits == EMPTY_PALETTE_BITS
    }

    /// The palette entry at a position (`None` for an empty storage).
    pub fn get(&self, x: usize, y: usize, z: usize) -> Option<&T> {
        self.entries.get(*self.indices.get(storage_index(x, y, z))? as usize)
    }

    /// Whether every position holds the same entry (or there are none).
    pub fn is_uniform(&self) -> bool {
        self.entries.len() <= 1 || self.indices.iter().all(|i| *i == self.indices[0])
    }
}

/// The smallest legal bit width for a palette of `len` entries (at least 1, like Chunker).
pub fn minimal_bits(len: usize) -> u8 {
    BITS_PER_ENTRY.iter().copied().find(|b| len <= 1usize << b).unwrap_or(16)
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end =
            self.pos.checked_add(n).filter(|e| *e <= self.data.len()).ok_or_else(|| fmt_err("data ends early"))?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn i32(&mut self) -> Result<i32> {
        let b = self.take(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
}

fn values_per_word(bits: u8) -> usize {
    32 / bits as usize
}

fn word_count(bits: u8) -> usize {
    STORAGE_SIZE.div_ceil(values_per_word(bits))
}

/// Read one storage; `read_entry(reader, runtime)` reads one palette entry.
fn read_storage<T>(
    r: &mut Reader<'_>,
    mut read_entry: impl FnMut(&mut Reader<'_>, bool) -> Result<T>,
) -> Result<PaletteData<T>> {
    let header = r.u8()?;
    let bits = header >> 1;
    let runtime = header & 1 != 0;
    if bits == EMPTY_PALETTE_BITS {
        return Ok(PaletteData { runtime, ..PaletteData::inherit() });
    }
    if bits != 0 && !BITS_PER_ENTRY.contains(&bits) {
        return Err(fmt_err(format!("storage with {bits} bits per entry")));
    }
    let words = if bits == 0 { 0 } else { word_count(bits) };
    let word_bytes = r.take(words * 4)?;
    let size = if bits == 0 { 1 } else { r.i32()? };
    let size = usize::try_from(size).map_err(|_| fmt_err("negative palette size"))?;
    // Every entry takes at least one byte, which bounds hostile sizes.
    if size > r.remaining() || size > 1 << 16 {
        return Err(fmt_err(format!("palette of {size} entries does not fit the data")));
    }
    let mut entries = Vec::with_capacity(size);
    for _ in 0..size {
        entries.push(read_entry(r, runtime)?);
    }
    let mut indices = vec![0u16; STORAGE_SIZE];
    if bits != 0 {
        let vpw = values_per_word(bits);
        let mask = (1u32 << bits) - 1;
        let mut i = 0;
        for chunk in word_bytes.chunks_exact(4) {
            let w = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            for k in 0..vpw {
                if i >= STORAGE_SIZE {
                    break;
                }
                let v = (w >> (k * bits as usize)) & mask;
                if !entries.is_empty() && v as usize >= entries.len() {
                    return Err(fmt_err(format!("index {v} outside a palette of {} entries", entries.len())));
                }
                indices[i] = v as u16;
                i += 1;
            }
        }
    }
    Ok(PaletteData { bits, runtime, entries, indices })
}

/// Write one storage; `write_entry(out, entry)` writes one palette entry.
fn write_storage<T>(
    out: &mut Vec<u8>,
    p: &PaletteData<T>,
    mut write_entry: impl FnMut(&mut Vec<u8>, &T) -> Result<()>,
) -> Result<()> {
    if p.is_inherit() {
        out.push(EMPTY_PALETTE_BITS << 1 | p.runtime as u8);
        return Ok(());
    }
    if p.indices.len() != STORAGE_SIZE {
        return Err(fmt_err(format!("a storage has 4096 indices, got {}", p.indices.len())));
    }
    if let Some(bad) = p.indices.iter().find(|i| **i as usize >= p.entries.len().max(1)) {
        return Err(fmt_err(format!("index {bad} outside a palette of {} entries", p.entries.len())));
    }
    let single_no_words = p.bits == 0 && p.entries.len() == 1;
    let bits = if single_no_words {
        0
    } else if BITS_PER_ENTRY.contains(&p.bits) && p.entries.len() <= 1usize << p.bits {
        p.bits
    } else {
        minimal_bits(p.entries.len())
    };
    out.push(bits << 1 | p.runtime as u8);
    if bits != 0 {
        let vpw = values_per_word(bits);
        let mut words = vec![0u32; word_count(bits)];
        for (i, idx) in p.indices.iter().enumerate() {
            words[i / vpw] |= (*idx as u32) << ((i % vpw) * bits as usize);
        }
        out.reserve(words.len() * 4);
        for w in words {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out.extend_from_slice(&(p.entries.len() as i32).to_le_bytes());
    }
    for e in &p.entries {
        write_entry(out, e)?;
    }
    Ok(())
}

/// A block state as stored in a block palette: `{name, states{...}, version}`.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockStateRaw {
    /// `minecraft:stone`.
    pub name: String,
    /// The block states, in stored order (empty for the legacy form).
    pub states: Compound,
    /// The block state schema version (`major << 24 | minor << 16 | patch << 8 | sub`).
    pub version: i32,
    /// The 1.12-style numeric data value of `{name, val}` entries that have no `states`.
    pub val: Option<i16>,
}

impl BlockStateRaw {
    pub fn new(name: &str, states: Compound, version: i32) -> BlockStateRaw {
        BlockStateRaw { name: name.to_string(), states, version, val: None }
    }

    /// The compound as Bedrock writes it.
    pub fn to_tag(&self) -> Tag {
        let mut c = Compound::new();
        c.insert("name", Tag::String(self.name.clone()));
        match self.val {
            Some(val) if self.states.0.is_empty() && self.version == 0 => {
                c.insert("val", Tag::Short(val));
            }
            val => {
                c.insert("states", Tag::Compound(self.states.clone()));
                c.insert("version", Tag::Int(self.version));
                if let Some(val) = val {
                    c.insert("val", Tag::Short(val));
                }
            }
        }
        Tag::Compound(c)
    }

    /// Read a palette compound.
    pub fn from_tag(tag: &Tag) -> Result<BlockStateRaw> {
        let Tag::Compound(c) = tag else {
            return Err(fmt_err("a block palette entry is not a compound"));
        };
        let name = c.get_str("name").ok_or_else(|| fmt_err("a block palette entry has no name"))?.to_string();
        let states = c.get_compound("states").cloned().unwrap_or_default();
        let version = c.get_i32("version").unwrap_or(0);
        let val = match c.get("val") {
            Some(Tag::Short(v)) => Some(*v),
            Some(Tag::Byte(v)) => Some(*v as i16),
            Some(Tag::Int(v)) => i16::try_from(*v).ok(),
            _ => None,
        };
        Ok(BlockStateRaw { name, states, version, val })
    }
}

/// A block storage (one layer of a sub-chunk).
pub type RawPalette = PaletteData<BlockStateRaw>;
/// A biome storage (one 16x16x16 section of Data3D); entries are biome ids.
pub type BiomePalette = PaletteData<i32>;

fn read_block_entry(r: &mut Reader<'_>, runtime: bool) -> Result<BlockStateRaw> {
    if runtime {
        return Err(BedrockError::Unsupported("runtime ids in a block palette".into()));
    }
    let mut cursor = Cursor::new(r.data);
    cursor.set_position(r.pos as u64);
    let (_, tag) = nbt_le::read_named(&mut cursor)?;
    r.pos = cursor.position() as usize;
    BlockStateRaw::from_tag(&tag)
}

fn write_block_entry(out: &mut Vec<u8>, e: &BlockStateRaw) -> Result<()> {
    nbt_le::write_named(out, "", &e.to_tag())
}

/// A decoded sub-chunk record (`SubChunkPrefix`).
#[derive(Clone, Debug, PartialEq)]
pub struct RawSubChunk {
    /// The format version of the record (8 or 9).
    pub version: u8,
    /// The sub-chunk y index: stored in the data for v9, taken from the key for v8.
    pub y: i8,
    /// Layer 0 holds the blocks, layer 1 the liquids (water means waterlogged).
    pub layers: Vec<RawPalette>,
}

impl RawSubChunk {
    /// Whether every layer is one entry that is air (such a sub-chunk can be skipped).
    pub fn is_all_air(&self) -> bool {
        self.layers.iter().all(|l| l.entries.iter().all(|e| e.name == "minecraft:air") || l.is_inherit())
    }
}

/// Decode a `SubChunkPrefix` value. `key_y` is the y index from the key, used for versions that do not store one.
///
/// Versions 8 and 9 are supported; anything older is an [`BedrockError::Unsupported`] error like in Chunker.
pub fn decode_sub_chunk(data: &[u8], key_y: i8) -> Result<RawSubChunk> {
    let mut r = Reader::new(data);
    let version = r.u8()?;
    let (layer_count, y) = match version {
        8 => (r.u8()?, key_y),
        9 => {
            let n = r.u8()?;
            (n, r.u8()? as i8)
        }
        v => return Err(BedrockError::Unsupported(format!("sub-chunk version {v} (only 8 and 9 are supported)"))),
    };
    let mut layers = Vec::with_capacity(layer_count as usize);
    for _ in 0..layer_count {
        layers.push(read_storage(&mut r, read_block_entry)?);
    }
    Ok(RawSubChunk { version, y, layers })
}

/// Encode a sub-chunk as `SubChunkPrefix` value bytes (v8 or v9 according to `sub.version`).
pub fn encode_sub_chunk(sub: &RawSubChunk) -> Result<Vec<u8>> {
    let count = u8::try_from(sub.layers.len()).map_err(|_| fmt_err("more than 255 layers"))?;
    let mut out = Vec::with_capacity(8 + sub.layers.len() * 1300);
    match sub.version {
        8 => out.extend_from_slice(&[8, count]),
        9 => out.extend_from_slice(&[9, count, sub.y as u8]),
        v => return Err(BedrockError::Unsupported(format!("writing sub-chunk version {v}"))),
    }
    for layer in &sub.layers {
        write_storage(&mut out, layer, write_block_entry)?;
    }
    Ok(out)
}

fn read_biome_entry(r: &mut Reader<'_>, _runtime: bool) -> Result<i32> {
    r.i32()
}

fn write_biome_entry(out: &mut Vec<u8>, e: &i32) -> Result<()> {
    out.extend_from_slice(&e.to_le_bytes());
    Ok(())
}

/// Read one biome storage (also used standalone by tests); returns it and the bytes consumed.
pub fn decode_biome_storage(data: &[u8]) -> Result<(BiomePalette, usize)> {
    let mut r = Reader::new(data);
    let p = read_storage(&mut r, read_biome_entry)?;
    Ok((p, r.pos))
}

/// Encode one biome storage.
pub fn encode_biome_storage(p: &BiomePalette) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    write_storage(&mut out, p, write_biome_entry)?;
    Ok(out)
}

/// The `Data3D` record (1.18+): a height map and the biome storages, lowest section first.
#[derive(Clone, Debug, PartialEq)]
pub struct Data3D {
    /// 256 heights, index `z * 16 + x`.
    pub heights: Vec<i16>,
    /// One biome storage per 16-block section from the bottom of the dimension (24 overworld, 8 nether, 16 end).
    /// Header 127 sections are [`PaletteData::is_inherit`].
    pub sections: Vec<BiomePalette>,
}

impl Data3D {
    /// The height at a column position.
    pub fn height(&self, x: usize, z: usize) -> i16 {
        self.heights.get((z & 15) * 16 + (x & 15)).copied().unwrap_or(0)
    }

    /// Replace every "same as previous" (127) section by a copy of the section below it, as Bedrock reads them. A
    /// leading inherit section (nothing below) stays as it is. The result has no inherit sections but possibly the
    /// first.
    pub fn resolved_sections(&self) -> Vec<BiomePalette> {
        let mut out: Vec<BiomePalette> = Vec::with_capacity(self.sections.len());
        for s in &self.sections {
            match (s.is_inherit(), out.last()) {
                (true, Some(prev)) => out.push(prev.clone()),
                _ => out.push(s.clone()),
            }
        }
        out
    }

    /// The biome id at a position of a section, following inheritance. `None` when there is no value.
    pub fn biome_at(&self, section: usize, x: usize, y: usize, z: usize) -> Option<i32> {
        let mut i = section.min(self.sections.len().checked_sub(1)?);
        // Walk down to the nearest section that has data; sections above the last stored one inherit it too.
        loop {
            let s = &self.sections[i];
            if !s.is_inherit() {
                return s.get(x, y, z).copied();
            }
            i = i.checked_sub(1)?;
        }
    }
}

/// Decode a `Data3D` record.
pub fn decode_data3d(data: &[u8]) -> Result<Data3D> {
    if data.len() < 512 {
        return Err(fmt_err(format!("Data3D of {} bytes has no full height map", data.len())));
    }
    let heights = data[..512].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
    let mut r = Reader::new(&data[512..]);
    let mut sections = Vec::new();
    while r.remaining() > 0 {
        sections.push(read_storage(&mut r, read_biome_entry)?);
    }
    Ok(Data3D { heights, sections })
}

/// Encode a `Data3D` record.
pub fn encode_data3d(d: &Data3D) -> Result<Vec<u8>> {
    if d.heights.len() != 256 {
        return Err(fmt_err(format!("a height map has 256 entries, got {}", d.heights.len())));
    }
    let mut out = Vec::with_capacity(512 + d.sections.len() * 200);
    for h in &d.heights {
        out.extend_from_slice(&h.to_le_bytes());
    }
    for s in &d.sections {
        write_storage(&mut out, s, write_biome_entry)?;
    }
    Ok(out)
}

/// The `Data2D` record (before 1.18): a height map and one biome id per column.
#[derive(Clone, Debug, PartialEq)]
pub struct Data2D {
    /// 256 heights, index `z * 16 + x`.
    pub heights: Vec<i16>,
    /// 256 biome ids in x-then-z order (`index = x * 16 + z`, as `readBiomesColumn` reads them).
    pub biomes: Vec<u8>,
}

/// Decode a `Data2D` record.
pub fn decode_data2d(data: &[u8]) -> Result<Data2D> {
    if data.len() < 768 {
        return Err(fmt_err(format!("Data2D of {} bytes is shorter than 768", data.len())));
    }
    let heights = data[..512].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
    Ok(Data2D { heights, biomes: data[512..768].to_vec() })
}

/// Encode a `Data2D` record.
pub fn encode_data2d(d: &Data2D) -> Result<Vec<u8>> {
    if d.heights.len() != 256 || d.biomes.len() != 256 {
        return Err(fmt_err("Data2D needs 256 heights and 256 biomes"));
    }
    let mut out = Vec::with_capacity(768);
    for h in &d.heights {
        out.extend_from_slice(&h.to_le_bytes());
    }
    out.extend_from_slice(&d.biomes);
    Ok(out)
}

/// Split a record made of back-to-back named compounds (`BlockEntity`, pre-1.18.30 `Entity`).
pub fn decode_nbt_concat(data: &[u8]) -> Result<Vec<(String, Tag)>> {
    nbt_le::read_all_named(data)
}

/// Join compounds into a back-to-back record.
pub fn encode_nbt_concat(tags: &[Tag]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for t in tags {
        nbt_le::write_named(&mut out, "", t)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str, n: i32) -> BlockStateRaw {
        let mut states = Compound::new();
        states.insert("k", Tag::Int(n));
        BlockStateRaw::new(name, states, 18168865)
    }

    fn pattern_indices(len: usize, seed: usize) -> Vec<u16> {
        (0..STORAGE_SIZE).map(|i| ((i * 7 + i / 13 + seed) % len) as u16).collect()
    }

    fn sub_chunk_with(len: usize, version: u8) -> RawSubChunk {
        let entries: Vec<_> = (0..len).map(|i| state(&format!("minecraft:b{i}"), i as i32)).collect();
        let mut p = RawPalette::from_indices(entries, pattern_indices(len, 3)).unwrap();
        // make sure every entry is used so the bit width is the one under test
        for i in 0..len {
            p.indices[i] = i as u16;
        }
        RawSubChunk { version, y: -3, layers: vec![p] }
    }

    #[test]
    fn minimal_bit_widths() {
        for (len, bits) in [(1, 1), (2, 1), (3, 2), (4, 2), (5, 3), (8, 3), (9, 4), (16, 4), (17, 5), (32, 5)] {
            assert_eq!(minimal_bits(len), bits, "{len}");
        }
        for (len, bits) in [(33, 6), (64, 6), (65, 8), (256, 8), (257, 16), (4096, 16)] {
            assert_eq!(minimal_bits(len), bits, "{len}");
        }
    }

    #[test]
    fn round_trip_every_bit_width() {
        for (len, bits) in [(2usize, 1u8), (4, 2), (5, 3), (16, 4), (17, 5), (33, 6), (65, 8), (300, 16)] {
            for version in [8u8, 9] {
                let sub = sub_chunk_with(len, version);
                assert_eq!(sub.layers[0].bits, bits);
                let bytes = encode_sub_chunk(&sub).unwrap();
                let words = word_count(bits);
                assert!(bytes.len() > words * 4);
                let back = decode_sub_chunk(&bytes, -3).unwrap();
                assert_eq!(back, sub, "len {len} v{version}");
                assert_eq!(encode_sub_chunk(&back).unwrap(), bytes, "bytes len {len} v{version}");
            }
        }
    }

    #[test]
    fn word_packing_matches_bedrock() {
        // 3-bit entries: 10 values per word, 2 bits of padding; 410 words.
        assert_eq!((values_per_word(3), word_count(3)), (10, 410));
        assert_eq!((values_per_word(5), word_count(5)), (6, 683));
        assert_eq!((values_per_word(6), word_count(6)), (5, 820));
        assert_eq!((values_per_word(1), word_count(1)), (32, 128));
        assert_eq!((values_per_word(16), word_count(16)), (2, 2048));
        // A hand-built 1-bit storage: block 0 (x0 y0 z0) and block 33 set.
        let mut indices = vec![0u16; STORAGE_SIZE];
        indices[0] = 1;
        indices[33] = 1;
        let p = PaletteData::from_indices(vec![1i32, 2], indices).unwrap();
        let bytes = encode_biome_storage(&p).unwrap();
        assert_eq!(bytes[0], 1 << 1);
        // word 0 = bit 0, word 1 = bit 1 (33 - 32)
        assert_eq!(&bytes[1..5], &1u32.to_le_bytes());
        assert_eq!(&bytes[5..9], &2u32.to_le_bytes());
        let size_at = 1 + 128 * 4;
        assert_eq!(&bytes[size_at..size_at + 4], &2i32.to_le_bytes());
        assert_eq!(decode_biome_storage(&bytes).unwrap().0, p);
    }

    #[test]
    fn index_order_is_x_z_y() {
        assert_eq!(storage_index(0, 0, 0), 0);
        assert_eq!(storage_index(0, 1, 0), 1);
        assert_eq!(storage_index(0, 0, 1), 16);
        assert_eq!(storage_index(1, 0, 0), 256);
        assert_eq!(storage_index(15, 15, 15), 4095);
        let mut p = PaletteData::from_indices(vec!["a", "b"], vec![0; STORAGE_SIZE]).unwrap();
        p.indices[storage_index(3, 4, 5)] = 1;
        assert_eq!(p.get(3, 4, 5), Some(&"b"));
        assert_eq!(p.get(3, 5, 4), Some(&"a"));
    }

    #[test]
    fn v9_stores_y_and_v8_uses_the_key() {
        let mut sub = sub_chunk_with(2, 9);
        sub.y = -4;
        let b9 = encode_sub_chunk(&sub).unwrap();
        assert_eq!(&b9[..3], &[9, 1, 0xFC]);
        assert_eq!(decode_sub_chunk(&b9, 7).unwrap().y, -4);
        sub.version = 8;
        let b8 = encode_sub_chunk(&sub).unwrap();
        assert_eq!(&b8[..2], &[8, 1]);
        assert_eq!(decode_sub_chunk(&b8, 7).unwrap().y, 7);
    }

    #[test]
    fn single_value_and_empty_storages() {
        // Chunker's single value: 1 bit, zero words, one entry.
        let sub = RawSubChunk {
            version: 9,
            y: 0,
            layers: vec![RawPalette::uniform(state("minecraft:stone", 0)), RawPalette::inherit()],
        };
        let bytes = encode_sub_chunk(&sub).unwrap();
        let back = decode_sub_chunk(&bytes, 0).unwrap();
        assert_eq!(back, sub);
        assert!(back.layers[1].is_inherit());
        assert_eq!(*bytes.last().unwrap(), 127 << 1);
        // The 0-bit form (no words, no size int).
        let mut raw = vec![9u8, 1, 0, 0]; // bits 0, not runtime
        write_block_entry(&mut raw, &state("minecraft:dirt", 1)).unwrap();
        let d = decode_sub_chunk(&raw, 0).unwrap();
        assert_eq!(d.layers[0].bits, 0);
        assert_eq!(d.layers[0].entries.len(), 1);
        assert!(d.layers[0].is_uniform());
        assert_eq!(encode_sub_chunk(&d).unwrap(), raw);
        // An empty palette (size 0) in a 1-bit storage.
        let mut raw = vec![9u8, 1, 0, 2];
        raw.extend(std::iter::repeat_n(0u8, 128 * 4));
        raw.extend_from_slice(&0i32.to_le_bytes());
        let d = decode_sub_chunk(&raw, 0).unwrap();
        assert!(d.layers[0].entries.is_empty());
        assert!(d.is_all_air());
        assert_eq!(encode_sub_chunk(&d).unwrap(), raw);
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        let good = encode_sub_chunk(&sub_chunk_with(5, 9)).unwrap();
        for cut in [0, 1, 2, 3, 4, 100, good.len() / 2, good.len() - 1] {
            assert!(decode_sub_chunk(&good[..cut], 0).is_err(), "cut {cut}");
        }
        for v in [0u8, 1, 2, 3, 4, 5, 6, 7, 10, 255] {
            let e = decode_sub_chunk(&[v, 1, 0], 0).unwrap_err();
            assert!(matches!(e, BedrockError::Unsupported(_)), "v{v}: {e}");
        }
        // 7 bits is not a width
        assert!(decode_sub_chunk(&[9, 1, 0, 7 << 1], 0).is_err());
        // runtime ids in a block palette
        let mut runtime = good.clone();
        runtime[3] |= 1;
        assert!(matches!(decode_sub_chunk(&runtime, 0), Err(BedrockError::Unsupported(_))));
        // an absurd palette size
        let mut huge = vec![9u8, 1, 0, 1 << 1];
        huge.extend(std::iter::repeat_n(0u8, 128 * 4));
        huge.extend_from_slice(&i32::MAX.to_le_bytes());
        assert!(decode_sub_chunk(&huge, 0).is_err());
        // an index outside the palette: 2 entries but a 2-bit value of 3 in word 0
        let mut idx = vec![9u8, 1, 0, 2 << 1];
        let mut words = vec![0u8; word_count(2) * 4];
        words[0] = 3;
        idx.extend_from_slice(&words);
        idx.extend_from_slice(&2i32.to_le_bytes());
        write_block_entry(&mut idx, &state("a", 0)).unwrap();
        write_block_entry(&mut idx, &state("b", 0)).unwrap();
        assert!(decode_sub_chunk(&idx, 0).is_err());
        // encoding refuses bad data too
        let mut bad = sub_chunk_with(3, 9);
        bad.layers[0].indices[5] = 9;
        assert!(encode_sub_chunk(&bad).is_err());
        bad.layers[0].indices.pop();
        assert!(encode_sub_chunk(&bad).is_err());
        let mut v7 = sub_chunk_with(3, 9);
        v7.version = 7;
        assert!(encode_sub_chunk(&v7).is_err());
    }

    #[test]
    fn legacy_palette_entries() {
        let e = BlockStateRaw { name: "minecraft:wool".into(), states: Compound::new(), version: 0, val: Some(5) };
        let mut sub = sub_chunk_with(2, 8);
        sub.layers[0].entries[1] = e.clone();
        let back = decode_sub_chunk(&encode_sub_chunk(&sub).unwrap(), -3).unwrap();
        assert_eq!(back.layers[0].entries[1], e);
        assert!(BlockStateRaw::from_tag(&Tag::Int(1)).is_err());
    }

    #[test]
    fn data3d_round_trip_and_inherit() {
        let mut sections = vec![
            PaletteData { bits: 0, runtime: true, entries: vec![1], indices: vec![0; STORAGE_SIZE] },
            PaletteData { runtime: true, ..PaletteData::inherit() },
        ];
        let mut idx = vec![0u16; STORAGE_SIZE];
        idx[storage_index(2, 3, 4)] = 1;
        let mut p = PaletteData::from_indices(vec![7, 8], idx).unwrap();
        p.runtime = true;
        sections.push(p);
        sections.push(PaletteData { runtime: true, ..PaletteData::inherit() });
        let heights: Vec<i16> = (0..256).map(|i| i as i16 - 64).collect();
        let d = Data3D { heights, sections };
        let bytes = encode_data3d(&d).unwrap();
        let back = decode_data3d(&bytes).unwrap();
        assert_eq!(back, d);
        assert_eq!(encode_data3d(&back).unwrap(), bytes);
        assert_eq!(back.height(3, 2), 2 * 16 + 3 - 64);
        // 127 = same as the section below
        let r = back.resolved_sections();
        assert_eq!(r[1].entries, vec![1]);
        assert_eq!(r[3].entries, vec![7, 8]);
        assert_eq!(back.biome_at(1, 0, 0, 0), Some(1));
        assert_eq!(back.biome_at(2, 2, 3, 4), Some(8));
        assert_eq!(back.biome_at(3, 2, 3, 4), Some(8));
        assert_eq!(back.biome_at(3, 0, 0, 0), Some(7));
        assert_eq!(back.biome_at(99, 0, 0, 0), Some(7));
        // truncated
        assert!(decode_data3d(&bytes[..100]).is_err());
        assert!(decode_data3d(&bytes[..bytes.len() - 2]).is_err());
    }

    #[test]
    fn data2d_and_concat() {
        let d = Data2D { heights: vec![5; 256], biomes: (0..=255).collect() };
        let bytes = encode_data2d(&d).unwrap();
        assert_eq!(bytes.len(), 768);
        assert_eq!(decode_data2d(&bytes).unwrap(), d);
        assert!(decode_data2d(&bytes[..767]).is_err());
        let mut a = Compound::new();
        a.insert("id", Tag::String("Chest".into()));
        a.insert("x", Tag::Int(-5));
        let tags = vec![Tag::Compound(a.clone()), Tag::Compound(a)];
        let enc = encode_nbt_concat(&tags).unwrap();
        let dec = decode_nbt_concat(&enc).unwrap();
        assert_eq!(dec.len(), 2);
        assert_eq!(dec[1].1, tags[1]);
        assert!(decode_nbt_concat(&enc[..enc.len() - 2]).is_err());
    }
}
