// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Column and region coordinates and the four neighbour edges used by pre-transforms.

/// A column position in chunk units (`ChunkCoordPair`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ChunkCoord {
    pub x: i32,
    pub z: i32,
}

impl ChunkCoord {
    pub const fn new(x: i32, z: i32) -> ChunkCoord {
        ChunkCoord { x, z }
    }

    /// Index inside the 32x32 region (`to10BitIndex`): `(x & 31) << 5 | (z & 31)`.
    pub const fn to_10_bit_index(self) -> u16 {
        (((self.x & 31) << 5) | (self.z & 31)) as u16
    }

    /// Both coordinates packed into a long (`toLong`): x in the high 32 bits, z in the low 32 bits.
    pub const fn to_long(self) -> i64 {
        ((self.x as i64) << 32) | (self.z as u32 as i64)
    }

    /// Inverse of [`ChunkCoord::to_long`].
    pub const fn from_long(value: i64) -> ChunkCoord {
        ChunkCoord { x: (value >> 32) as i32, z: value as i32 }
    }

    /// Whether the block position (x, z) is inside this column (`isInside`).
    pub const fn is_inside(self, x: i32, z: i32) -> bool {
        x >> 4 == self.x && z >> 4 == self.z
    }

    /// The region containing the column (`getRegionX`, `getRegionZ`).
    pub const fn region(self) -> RegionCoord {
        RegionCoord { x: self.x >> 5, z: self.z >> 5 }
    }

    /// Whether the column is on the outer ring of its region.
    pub const fn is_region_edge(self) -> bool {
        let (x, z) = (self.x & 31, self.z & 31);
        x == 0 || x == 31 || z == 0 || z == 31
    }
}

/// A region position (32x32 columns).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct RegionCoord {
    pub x: i32,
    pub z: i32,
}

impl RegionCoord {
    pub const fn new(x: i32, z: i32) -> RegionCoord {
        RegionCoord { x, z }
    }

    /// The column at a local position inside the region (`getChunk`).
    pub const fn chunk(self, local_x: i32, local_z: i32) -> ChunkCoord {
        ChunkCoord { x: self.x << 5 | local_x, z: self.z << 5 | local_z }
    }

    /// Whether the column lies in this region (`isInside`).
    pub const fn contains(self, chunk: ChunkCoord) -> bool {
        chunk.x >> 5 == self.x && chunk.z >> 5 == self.z
    }
}

/// One of the four horizontal neighbours of a column (`Edge`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Edge {
    PositiveX,
    NegativeX,
    PositiveZ,
    NegativeZ,
}

impl Edge {
    pub const ALL: [Edge; 4] = [Edge::PositiveX, Edge::NegativeX, Edge::PositiveZ, Edge::NegativeZ];

    /// Column offset of the neighbour.
    pub const fn offset(self) -> (i32, i32) {
        match self {
            Edge::PositiveX => (1, 0),
            Edge::NegativeX => (-1, 0),
            Edge::PositiveZ => (0, 1),
            Edge::NegativeZ => (0, -1),
        }
    }

    /// The edge for an offset, if it is one of the four (`fromOffset`).
    pub const fn from_offset(x: i32, z: i32) -> Option<Edge> {
        match (x, z) {
            (1, 0) => Some(Edge::PositiveX),
            (-1, 0) => Some(Edge::NegativeX),
            (0, 1) => Some(Edge::PositiveZ),
            (0, -1) => Some(Edge::NegativeZ),
            _ => None,
        }
    }

    /// The position of the neighbour.
    pub const fn relative(self, pos: ChunkCoord) -> ChunkCoord {
        let (x, z) = self.offset();
        ChunkCoord { x: pos.x + x, z: pos.z + z }
    }

    pub const fn opposite(self) -> Edge {
        match self {
            Edge::PositiveX => Edge::NegativeX,
            Edge::NegativeX => Edge::PositiveX,
            Edge::PositiveZ => Edge::NegativeZ,
            Edge::NegativeZ => Edge::PositiveZ,
        }
    }
}

/// A set of edges as a bit mask (`EnumSet<Edge>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct EdgeSet(u8);

impl EdgeSet {
    pub const EMPTY: EdgeSet = EdgeSet(0);
    pub const ALL: EdgeSet = EdgeSet(0b1111);

    pub const fn contains(self, edge: Edge) -> bool {
        self.0 >> edge as u8 & 1 == 1
    }

    pub fn insert(&mut self, edge: Edge) {
        self.0 |= 1 << edge as u8;
    }

    pub const fn with(mut self, edge: Edge) -> EdgeSet {
        self.0 |= 1 << edge as u8;
        self
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn union(self, other: EdgeSet) -> EdgeSet {
        EdgeSet(self.0 | other.0)
    }

    pub fn iter(self) -> impl Iterator<Item = Edge> {
        Edge::ALL.into_iter().filter(move |e| self.contains(*e))
    }
}

impl FromIterator<Edge> for EdgeSet {
    fn from_iter<T: IntoIterator<Item = Edge>>(iter: T) -> Self {
        let mut set = EdgeSet::EMPTY;
        for e in iter {
            set.insert(e);
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coords() {
        let c = ChunkCoord::new(-1, 33);
        assert_eq!(c.to_10_bit_index(), (31 << 5) | 1);
        assert_eq!(c.region(), RegionCoord::new(-1, 1));
        assert!(c.is_region_edge());
        assert!(!ChunkCoord::new(5, 5).is_region_edge());
        assert_eq!(ChunkCoord::from_long(c.to_long()), c);
        assert_eq!(ChunkCoord::new(0, 1).to_long(), 1);
        assert_eq!(ChunkCoord::new(1, 0).to_long(), 1 << 32);
        assert_eq!(ChunkCoord::new(0, -1).to_long(), 0xFFFF_FFFF);
        assert!(c.is_inside(-16, 33 * 16));
        assert!(!c.is_inside(0, 33 * 16));
        assert_eq!(RegionCoord::new(-1, 1).chunk(31, 1), ChunkCoord::new(-1, 33));
        assert!(RegionCoord::new(-1, 1).contains(c));
    }

    #[test]
    fn edges() {
        for e in Edge::ALL {
            assert_eq!(e.opposite().opposite(), e);
            let (x, z) = e.offset();
            assert_eq!(Edge::from_offset(x, z), Some(e));
            assert_eq!(e.opposite().relative(e.relative(ChunkCoord::new(3, 4))), ChunkCoord::new(3, 4));
        }
        assert_eq!(Edge::from_offset(1, 1), None);
        let set: EdgeSet = [Edge::NegativeX, Edge::PositiveZ].into_iter().collect();
        assert!(set.contains(Edge::NegativeX) && !set.contains(Edge::PositiveX));
        assert_eq!(set.iter().count(), 2);
    }
}
