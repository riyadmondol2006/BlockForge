// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Palettes (`chunk.palette`): a list of keys plus a per voxel index into it, over a cube of `D`x`D`x`D` voxels
//! (16 for blocks and per-block biomes, 4 for biome cells).
//!
//! One enum covers Chunker's three classes: [`Palette::Empty`] (`EmptyPalette`, no keys), [`Palette::Single`]
//! (`SingleValuePalette`, every voxel is the one key) and [`Palette::Packed`] (`ShortBasedPalette`). Indices are bit
//! packed (a typical section of up to 16 keys takes 2 KiB, Chunker's `short[16][16][16]` took 8 KiB) and stored
//! in Y, Z, X order: `index = (y * D + z) * D + x`.
//!
//! Chunker's quirks are kept on purpose:
//! * an index outside the key list is legal and reads as the caller's default; [`Palette::compact`] rewrites such
//!   voxels to the default value;
//! * `Single` answers every key lookup with its value, whatever the index;
//! * writing to `Empty` behaves like Chunker's `asWriteable()`: the palette becomes `Packed` with no keys and all
//!   indices 0, so the first key created is key 0 and *every* voxel then reads as it;
//! * [`Palette::contains_key`] looks at the keys only (unused keys count), [`Palette::contains_value`] looks at the
//!   voxels, except that a palette with one key answers by that key without scanning;
//! * `compact` assigns new keys in the order Chunker's loops meet them (x outermost, then y, then z), drops unused
//!   keys, merges equal keys and returns `Single` for one key and `Empty` for none.

/// Bit packed voxel indices. Entries never straddle a word: `64 / bits` entries per `u64`.
#[derive(Clone, Debug)]
pub struct PackedIndices {
    bits: u8,
    per_word: u8,
    len: usize,
    words: Box<[u64]>,
}

impl PackedIndices {
    /// `len` zero indices of `bits` bits each (1 to 16).
    pub fn zeroed(len: usize, bits: u8) -> PackedIndices {
        assert!((1..=16).contains(&bits), "bits out of range: {bits}");
        let per_word = 64 / bits;
        let words = len.div_ceil(per_word as usize);
        PackedIndices { bits, per_word, len, words: vec![0u64; words].into_boxed_slice() }
    }

    /// Pack a slice, using the fewest bits that hold its largest entry.
    pub fn from_slice(values: &[u16]) -> PackedIndices {
        let max = values.iter().copied().max().unwrap_or(0);
        let mut packed = PackedIndices::zeroed(values.len(), bits_for(max));
        for (i, v) in values.iter().enumerate() {
            packed.set_unchecked(i, *v);
        }
        packed
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Bits per entry.
    pub fn bits(&self) -> u8 {
        self.bits
    }

    /// Size of the packed data in bytes.
    pub fn byte_size(&self) -> usize {
        self.words.len() * 8
    }

    pub fn get(&self, i: usize) -> u16 {
        assert!(i < self.len, "index {i} out of range {}", self.len);
        let per = self.per_word as usize;
        let shift = (i % per) as u32 * self.bits as u32;
        ((self.words[i / per] >> shift) & self.mask()) as u16
    }

    /// Store `value`, repacking with more bits first when it does not fit.
    pub fn set(&mut self, i: usize, value: u16) {
        assert!(i < self.len, "index {i} out of range {}", self.len);
        let needed = bits_for(value);
        if needed > self.bits {
            self.repack(needed);
        }
        self.set_unchecked(i, value);
    }

    fn set_unchecked(&mut self, i: usize, value: u16) {
        let per = self.per_word as usize;
        let shift = (i % per) as u32 * self.bits as u32;
        let mask = self.mask();
        let word = &mut self.words[i / per];
        *word = (*word & !(mask << shift)) | ((value as u64) << shift);
    }

    fn mask(&self) -> u64 {
        (1u64 << self.bits) - 1
    }

    fn repack(&mut self, bits: u8) {
        let mut bigger = PackedIndices::zeroed(self.len, bits);
        for i in 0..self.len {
            bigger.set_unchecked(i, self.get(i));
        }
        *self = bigger;
    }

    /// All entries in storage order.
    pub fn iter(&self) -> impl Iterator<Item = u16> + '_ {
        (0..self.len).map(|i| self.get(i))
    }

    /// All entries as a vector.
    pub fn to_vec(&self) -> Vec<u16> {
        self.iter().collect()
    }

    /// The largest entry.
    pub fn max(&self) -> u16 {
        self.iter().max().unwrap_or(0)
    }
}

impl PartialEq for PackedIndices {
    fn eq(&self, other: &Self) -> bool {
        if self.len != other.len {
            return false;
        }
        if self.bits == other.bits {
            return self.words == other.words;
        }
        self.iter().eq(other.iter())
    }
}

impl Eq for PackedIndices {}

/// Fewest bits (at least 1) that can hold `value`.
fn bits_for(value: u16) -> u8 {
    (16 - value.leading_zeros()).max(1) as u8
}

/// A palette over `D`x`D`x`D` voxels. See the module docs for the semantics.
#[derive(Clone, Debug, PartialEq)]
pub enum Palette<T, const D: usize> {
    /// No keys: every read gives the caller's default.
    Empty,
    /// Every voxel is this value.
    Single(T),
    /// Keys and one index per voxel.
    Packed { keys: Vec<T>, indices: PackedIndices },
}

// Not derived: a derive would require `T: Default`.
#[allow(clippy::derivable_impls)]
impl<T, const D: usize> Default for Palette<T, D> {
    fn default() -> Self {
        Palette::Empty
    }
}

impl<T, const D: usize> Palette<T, D> {
    /// Number of voxels.
    pub const VOLUME: usize = D * D * D;

    /// Storage index of a voxel (Y, Z, X order).
    pub const fn index(x: usize, y: usize, z: usize) -> usize {
        (y * D + z) * D + x
    }

    /// A palette where every voxel is `value`.
    pub fn single(value: T) -> Palette<T, D> {
        Palette::Single(value)
    }

    /// Build a `Packed` palette from keys and one index per voxel (storage order, [`Palette::index`]).
    /// Indices may point outside the keys.
    pub fn from_parts(keys: Vec<T>, indices: &[u16]) -> Palette<T, D> {
        assert_eq!(indices.len(), Self::VOLUME, "expected {} indices", Self::VOLUME);
        Palette::Packed { keys, indices: PackedIndices::from_slice(indices) }
    }

    /// Build a `Packed` palette from keys and ready packed indices.
    pub fn from_packed(keys: Vec<T>, indices: PackedIndices) -> Palette<T, D> {
        assert_eq!(indices.len(), Self::VOLUME, "expected {} indices", Self::VOLUME);
        Palette::Packed { keys, indices }
    }

    /// Number of keys (`getKeyCount`).
    pub fn key_count(&self) -> usize {
        match self {
            Palette::Empty => 0,
            Palette::Single(_) => 1,
            Palette::Packed { keys, .. } => keys.len(),
        }
    }

    /// The keys.
    pub fn keys(&self) -> &[T] {
        match self {
            Palette::Empty => &[],
            Palette::Single(v) => std::slice::from_ref(v),
            Palette::Packed { keys, .. } => keys,
        }
    }

    /// The key at an index (`getKey(i)`): `None` when out of range; a `Single` palette answers its value for any
    /// index.
    pub fn key(&self, index: i32) -> Option<&T> {
        match self {
            Palette::Empty => None,
            Palette::Single(v) => Some(v),
            Palette::Packed { keys, .. } => usize::try_from(index).ok().and_then(|i| keys.get(i)),
        }
    }

    /// The key at an index or `default` (`getKey(i, default)`).
    pub fn key_or<'a>(&'a self, index: i32, default: &'a T) -> &'a T {
        self.key(index).unwrap_or(default)
    }

    /// The raw index stored for a voxel. `Empty` and `Single` have no index array: they report 0.
    pub fn index_at(&self, x: usize, y: usize, z: usize) -> u16 {
        match self {
            Palette::Packed { indices, .. } => indices.get(Self::index(x, y, z)),
            _ => 0,
        }
    }

    /// The value of a voxel, `None` when its index is outside the keys (`get(x, y, z)`).
    pub fn get(&self, x: usize, y: usize, z: usize) -> Option<&T> {
        match self {
            Palette::Empty => None,
            Palette::Single(v) => Some(v),
            Palette::Packed { keys, indices } => keys.get(indices.get(Self::index(x, y, z)) as usize),
        }
    }

    /// The value of a voxel or `default` (`get(x, y, z, default)`).
    pub fn get_or<'a>(&'a self, x: usize, y: usize, z: usize, default: &'a T) -> &'a T {
        self.get(x, y, z).unwrap_or(default)
    }

    /// Whether there are no keys (`isEmpty`).
    pub fn is_empty(&self) -> bool {
        match self {
            Palette::Empty => true,
            Palette::Single(_) => false,
            Palette::Packed { keys, .. } => keys.is_empty(),
        }
    }

    /// Whether any key matches (`containsKey`); unused keys count.
    pub fn contains_key(&self, predicate: impl Fn(&T) -> bool) -> bool {
        self.keys().iter().any(predicate)
    }

    /// Whether any voxel matches (`containsValue`). A palette with a single key answers by that key without
    /// looking at the voxels; voxels with an index outside the keys never match.
    pub fn contains_value(&self, predicate: impl Fn(&T) -> bool) -> bool {
        match self {
            Palette::Empty => false,
            Palette::Single(v) => predicate(v),
            Palette::Packed { keys, indices } => match keys.len() {
                0 => false,
                1 => predicate(&keys[0]),
                n => {
                    let matched: Vec<bool> = keys.iter().map(&predicate).collect();
                    if !matched.contains(&true) {
                        return false;
                    }
                    indices.iter().any(|i| (i as usize) < n && matched[i as usize])
                }
            },
        }
    }

    /// Transform every key, keeping the voxel indices (`map`). Consumes the palette, so unlike Java no array is
    /// shared with the source.
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> Palette<U, D> {
        match self {
            Palette::Empty => Palette::Empty,
            Palette::Single(v) => Palette::Single(f(v)),
            Palette::Packed { keys, indices } => Palette::Packed { keys: keys.into_iter().map(f).collect(), indices },
        }
    }

    /// Voxel indices in storage order (`Empty` and `Single` yield zeros).
    pub fn to_u16_vec(&self) -> Vec<u16> {
        match self {
            Palette::Packed { indices, .. } => indices.to_vec(),
            _ => vec![0; Self::VOLUME],
        }
    }

    /// The packed indices, if any.
    pub fn packed_indices(&self) -> Option<&PackedIndices> {
        match self {
            Palette::Packed { indices, .. } => Some(indices),
            _ => None,
        }
    }

    /// Convert to `Packed` in place like `asWriteable()`: `Empty` gets no keys and zero indices, `Single` gets its
    /// value as key 0.
    pub fn make_writeable(&mut self) {
        let keys = match std::mem::take(self) {
            Palette::Packed { keys, indices } => {
                *self = Palette::Packed { keys, indices };
                return;
            }
            Palette::Empty => Vec::new(),
            Palette::Single(v) => vec![v],
        };
        *self = Palette::Packed { keys, indices: PackedIndices::zeroed(Self::VOLUME, 1) };
    }

    /// Overwrite the index of a voxel (`setPaletteIndex`). The palette becomes writeable first.
    pub fn set_index(&mut self, x: usize, y: usize, z: usize, index: u16) {
        self.make_writeable();
        if let Palette::Packed { indices, .. } = self {
            indices.set(Self::index(x, y, z), index);
        }
    }
}

impl<T: Clone + PartialEq, const D: usize> Palette<T, D> {
    /// The index of `key`, adding it when missing (`getOrCreateKey`). The palette becomes writeable first.
    pub fn get_or_create_key(&mut self, key: T) -> u16 {
        self.make_writeable();
        let Palette::Packed { keys, .. } = self else { unreachable!("make_writeable") };
        let index = match keys.iter().position(|k| *k == key) {
            Some(i) => i,
            None => {
                keys.push(key);
                keys.len() - 1
            }
        };
        u16::try_from(index).expect("palette has more than 65535 keys")
    }

    /// Set a voxel (`set`), creating the key when needed.
    pub fn set(&mut self, x: usize, y: usize, z: usize, value: T) {
        let index = self.get_or_create_key(value);
        self.set_index(x, y, z, index);
    }

    /// Drop unused keys, merge equal ones and rewrite out-of-range voxels to `default` (`compact`); see the module
    /// docs for the exact rules.
    pub fn compact(self, default: &T) -> Palette<T, D> {
        let (keys, indices) = match self {
            Palette::Packed { keys, indices } => (keys, indices),
            other => return other,
        };
        if keys.is_empty() {
            return Palette::Empty;
        }
        const NONE: u16 = u16::MAX;
        let mut remap = vec![NONE; keys.len()];
        let mut new_keys: Vec<T> = Vec::new();
        let mut new_indices = vec![0u16; Self::VOLUME];
        for x in 0..D {
            for y in 0..D {
                for z in 0..D {
                    let at = Self::index(x, y, z);
                    let old = indices.get(at) as usize;
                    let valid = old < keys.len();
                    let mut new = if valid { remap[old] } else { NONE };
                    if new == NONE {
                        let value = if valid { &keys[old] } else { default };
                        let found = match new_keys.iter().position(|k| k == value) {
                            Some(i) => i,
                            None => {
                                new_keys.push(value.clone());
                                new_keys.len() - 1
                            }
                        };
                        new = found as u16;
                        if valid {
                            remap[old] = new;
                        }
                    }
                    new_indices[at] = new;
                }
            }
        }
        if new_keys.len() == 1 {
            return Palette::Single(new_keys.pop().expect("one key"));
        }
        Palette::Packed { keys: new_keys, indices: PackedIndices::from_slice(&new_indices) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type P = Palette<&'static str, 16>;
    type Small = Palette<&'static str, 4>;

    #[test]
    fn packed_indices_round_trip() {
        for bits in 1..=16u8 {
            let mut p = PackedIndices::zeroed(4096, bits);
            let max = ((1u32 << bits) - 1) as u16;
            for i in 0..4096 {
                p.set(i, (i as u32 % (max as u32 + 1)) as u16);
            }
            for i in 0..4096 {
                assert_eq!(p.get(i) as u32, i as u32 % (max as u32 + 1), "bits {bits} index {i}");
            }
            assert_eq!(p.bits(), bits);
        }
    }

    #[test]
    fn packed_indices_grow_and_size() {
        let mut p = PackedIndices::zeroed(4096, 1);
        p.set(5, 1);
        assert_eq!(p.bits(), 1);
        p.set(6, 9);
        assert_eq!(p.bits(), 4);
        assert_eq!((p.get(5), p.get(6), p.get(7)), (1, 9, 0));
        assert_eq!(p.byte_size(), 2048); // 2 KiB for up to 16 keys
        assert_eq!(PackedIndices::zeroed(64, 4).byte_size(), 32);
        let q = PackedIndices::from_slice(&[0, 3, 2, 7]);
        assert_eq!(q.bits(), 3);
        assert_eq!(q, PackedIndices::from_slice(&[0, 3, 2, 7]));
        let mut wide = PackedIndices::zeroed(4, 8);
        wide.set(1, 3);
        wide.set(2, 2);
        wide.set(3, 7);
        assert_eq!(q, wide); // equality is by value, not by width
    }

    #[test]
    fn empty_palette() {
        let p = P::Empty;
        assert!(p.is_empty());
        assert_eq!(p.key_count(), 0);
        assert_eq!(p.get(1, 2, 3), None);
        assert_eq!(*p.get_or(1, 2, 3, &"def"), "def");
        assert_eq!(p.key(0), None);
        assert!(!p.contains_key(|_| true));
        assert!(!p.contains_value(|_| true));
        assert_eq!(p.clone().compact(&"def"), P::Empty);
        assert_eq!(p.map(|k| k.len()), Palette::<usize, 16>::Empty);
    }

    #[test]
    fn single_palette() {
        let p = P::single("air");
        assert!(!p.is_empty());
        assert_eq!(p.key_count(), 1);
        assert_eq!(p.get(15, 15, 15), Some(&"air"));
        // Any key index answers with the value, even a negative or huge one.
        assert_eq!(p.key(-1), Some(&"air"));
        assert_eq!(p.key(99), Some(&"air"));
        assert!(p.contains_key(|k| *k == "air"));
        assert!(p.contains_value(|k| *k == "air"));
        assert!(!p.contains_value(|k| *k == "stone"));
        assert_eq!(p.clone().compact(&"x"), p);
        assert_eq!(p.map(|k| k.len()), Palette::<usize, 16>::Single(3));
    }

    #[test]
    fn set_on_single_keeps_old_value() {
        let mut p = P::single("air");
        p.set(1, 2, 3, "stone");
        assert_eq!(p.keys(), &["air", "stone"]);
        assert_eq!(p.get(1, 2, 3), Some(&"stone"));
        assert_eq!(p.get(0, 0, 0), Some(&"air"));
        assert_eq!(p.get(3, 2, 1), Some(&"air"));
        // Setting the same value again does not add a key.
        p.set(4, 4, 4, "stone");
        assert_eq!(p.key_count(), 2);
    }

    #[test]
    fn set_on_empty_floods_with_the_first_key() {
        // `EmptyPalette.asWriteable()` is a palette with no keys and all indices 0, so the first key created
        // becomes key 0 and every voxel reads as it.
        let mut p = P::Empty;
        p.set(1, 1, 1, "stone");
        assert_eq!(p.keys(), &["stone"]);
        assert_eq!(p.get(0, 0, 0), Some(&"stone"));
        assert_eq!(p.get(15, 15, 15), Some(&"stone"));
        // A second key: only the written voxel changes.
        p.set(2, 2, 2, "dirt");
        assert_eq!(p.get(2, 2, 2), Some(&"dirt"));
        assert_eq!(p.get(1, 1, 1), Some(&"stone"));
    }

    #[test]
    fn writeable_empty_reads_default() {
        let mut p = P::Empty;
        p.make_writeable();
        assert!(p.is_empty());
        assert_eq!(p.get(0, 0, 0), None);
        assert_eq!(p.key_count(), 0);
    }

    #[test]
    fn out_of_range_index_reads_default() {
        let mut p = P::from_parts(vec!["a", "b"], &vec![0u16; 4096]);
        p.set_index(3, 4, 5, 7);
        assert_eq!(p.get(3, 4, 5), None);
        assert_eq!(*p.get_or(3, 4, 5, &"def"), "def");
        assert_eq!(p.index_at(3, 4, 5), 7);
        assert_eq!(p.get(0, 0, 0), Some(&"a"));
        // Large indices are fine too.
        p.set_index(0, 0, 1, 60_000);
        assert_eq!(p.get(0, 0, 1), None);
    }

    #[test]
    fn key_lookup_ranges() {
        let p = P::from_parts(vec!["a", "b"], &vec![1u16; 4096]);
        assert_eq!(p.key(0), Some(&"a"));
        assert_eq!(p.key(1), Some(&"b"));
        assert_eq!(p.key(2), None);
        assert_eq!(p.key(-1), None);
        assert_eq!(*p.key_or(5, &"d"), "d");
    }

    #[test]
    fn contains_key_vs_value() {
        let mut idx = vec![0u16; 4096];
        idx[10] = 1;
        let p = P::from_parts(vec!["air", "stone", "unused"], &idx);
        assert!(p.contains_key(|k| *k == "unused")); // keys only
        assert!(!p.contains_value(|k| *k == "unused")); // voxels
        assert!(p.contains_value(|k| *k == "stone"));
        assert!(p.contains_value(|k| *k == "air"));
        assert!(!p.contains_value(|k| *k == "missing"));
    }

    #[test]
    fn contains_value_ignores_invalid_indices() {
        let mut idx = vec![9u16; 4096];
        idx[0] = 1;
        let p = P::from_parts(vec!["a", "b", "c"], &idx);
        assert!(p.contains_value(|k| *k == "b"));
        assert!(!p.contains_value(|k| *k == "a"));
        assert!(!p.contains_value(|k| *k == "c"));
    }

    #[test]
    fn contains_value_with_one_key_does_not_scan() {
        // All voxels point outside the single key, yet the key test decides (Chunker's shortcut).
        let p = P::from_parts(vec!["a"], &vec![5u16; 4096]);
        assert!(p.contains_value(|k| *k == "a"));
        assert!(!p.contains_value(|k| *k == "b"));
        let none = P::from_parts(Vec::new(), &vec![0u16; 4096]);
        assert!(!none.contains_value(|_| true));
        assert!(none.is_empty());
    }

    #[test]
    fn map_keeps_indices() {
        let mut idx = vec![0u16; 4096];
        idx[Palette::<&str, 16>::index(1, 2, 3)] = 1;
        let p = P::from_parts(vec!["a", "bb"], &idx);
        let m = p.map(|k| k.len());
        assert_eq!(m.keys(), &[1, 2]);
        assert_eq!(m.get(1, 2, 3), Some(&2));
        assert_eq!(m.get(0, 0, 0), Some(&1));
        // Mapping may merge keys without touching the indices.
        let merged = P::from_parts(vec!["a", "b"], &idx).map(|_| 0u8);
        assert_eq!(merged.key_count(), 2);
    }

    #[test]
    fn compact_drops_unused_and_merges_equal() {
        let mut idx = vec![2u16; 4096];
        idx[0] = 3;
        idx[1] = 0;
        let p = P::from_parts(vec!["a", "unused", "b", "a"], &idx);
        let c = p.compact(&"def");
        // Voxels meet key 0 ("a") and key 3 ("a" again, merged) and "b".
        assert_eq!(c.key_count(), 2);
        assert_eq!(c.get(0, 0, 0), Some(&"a")); // idx[0] = 3 -> "a"
        assert_eq!(c.get(1, 0, 0), Some(&"a")); // idx[1] = 0 -> "a"
        assert_eq!(c.get(5, 5, 5), Some(&"b"));
        assert!(!c.contains_key(|k| *k == "unused"));
    }

    #[test]
    fn compact_key_order_follows_x_y_z_loops() {
        // Voxel (x=0,y=1,z=0) is met before (x=1,y=0,z=0) when x is the outermost loop, although the YZX storage
        // order would put the second one first.
        let mut idx = vec![0u16; 4096];
        idx[Palette::<&str, 16>::index(0, 1, 0)] = 1;
        idx[Palette::<&str, 16>::index(1, 0, 0)] = 2;
        let p = P::from_parts(vec!["base", "p", "q"], &idx);
        let c = p.compact(&"def");
        // (0,0,0) = base is met first, then (0,1,0) = p, then (1,0,0) = q.
        assert_eq!(c.keys(), &["base", "p", "q"]);
        let mut idx2 = vec![0u16; 4096];
        idx2[Palette::<&str, 16>::index(0, 1, 0)] = 2;
        idx2[Palette::<&str, 16>::index(1, 0, 0)] = 1;
        let c2 = P::from_parts(vec!["base", "p", "q"], &idx2).compact(&"def");
        assert_eq!(c2.keys(), &["base", "q", "p"]);
    }

    #[test]
    fn compact_rewrites_out_of_range_to_default() {
        let mut idx = vec![0u16; 4096];
        idx[Palette::<&str, 16>::index(2, 0, 0)] = 77;
        let p = P::from_parts(vec!["a"], &idx);
        let c = p.compact(&"def");
        assert_eq!(c.keys(), &["a", "def"]);
        assert_eq!(c.get(2, 0, 0), Some(&"def"));
        assert_eq!(c.get(0, 0, 0), Some(&"a"));
        // The default may equal an existing key.
        let mut idx = vec![0u16; 4096];
        idx[5] = 9;
        let c = P::from_parts(vec!["a", "b"], &idx).compact(&"a");
        assert_eq!(c, P::Single("a"));
    }

    #[test]
    fn compact_returns_single_and_empty() {
        let p = P::from_parts(vec!["a", "b"], &vec![1u16; 4096]);
        assert_eq!(p.compact(&"def"), P::Single("b"));
        let q = P::from_parts(Vec::new(), &vec![0u16; 4096]);
        assert_eq!(q.compact(&"def"), P::Empty);
        // Out of range everywhere with keys present collapses to the default.
        let r = P::from_parts(vec!["a"], &vec![4u16; 4096]);
        assert_eq!(r.compact(&"def"), P::Single("def"));
    }

    #[test]
    fn compact_shrinks_bits() {
        let mut p = P::Empty;
        p.make_writeable();
        for i in 0..40 {
            let key: &'static str = Box::leak(format!("k{i}").into_boxed_str());
            p.set(i % 16, i / 16, 0, key);
        }
        let bits_before = p.packed_indices().unwrap().bits();
        assert!(bits_before >= 5);
        // Overwrite everything with two values: compaction must shrink to one bit.
        for x in 0..16 {
            for y in 0..16 {
                for z in 0..16 {
                    p.set(x, y, z, if (x + y + z) % 2 == 0 { "even" } else { "odd" });
                }
            }
        }
        let c = p.compact(&"def");
        assert_eq!(c.key_count(), 2);
        assert_eq!(c.packed_indices().unwrap().bits(), 1);
    }

    #[test]
    fn typical_section_is_2_kib() {
        let mut p = P::single("air");
        for i in 0..14 {
            let key: &'static str = Box::leak(format!("b{i}").into_boxed_str());
            p.set(i, 0, 0, key);
        }
        assert_eq!(p.key_count(), 15);
        assert_eq!(p.packed_indices().unwrap().bits(), 4);
        assert_eq!(p.packed_indices().unwrap().byte_size(), 2048);
    }

    #[test]
    fn four_cube_palette() {
        let mut p = Small::single("plains");
        p.set(3, 3, 3, "desert");
        assert_eq!(p.get(3, 3, 3), Some(&"desert"));
        assert_eq!(p.get(0, 0, 0), Some(&"plains"));
        assert_eq!(p.to_u16_vec().len(), 64);
        assert_eq!(Small::index(1, 2, 3), (2 * 4 + 3) * 4 + 1);
    }

    #[test]
    fn index_layout_is_yzx() {
        assert_eq!(P::index(1, 0, 0), 1);
        assert_eq!(P::index(0, 0, 1), 16);
        assert_eq!(P::index(0, 1, 0), 256);
    }

    #[test]
    fn to_u16_vec_round_trip() {
        let mut idx = vec![0u16; 4096];
        for (i, v) in idx.iter_mut().enumerate() {
            *v = (i % 5) as u16;
        }
        let p = P::from_parts(vec!["a", "b", "c", "d", "e"], &idx);
        assert_eq!(p.to_u16_vec(), idx);
        assert_eq!(P::single("a").to_u16_vec(), vec![0u16; 4096]);
    }

    #[test]
    fn get_or_create_key_reuses() {
        let mut p = P::single("a");
        assert_eq!(p.get_or_create_key("a"), 0);
        assert_eq!(p.get_or_create_key("b"), 1);
        assert_eq!(p.get_or_create_key("b"), 1);
        assert_eq!(p.key_count(), 2);
    }
}
