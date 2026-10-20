// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Small value types: block positions, vectors and legacy ids.

use std::cmp::Ordering;

/// A block position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BlockPosition {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPosition {
    pub const fn new(x: i32, y: i32, z: i32) -> BlockPosition {
        BlockPosition { x, y, z }
    }

    /// Chunker's `BY_Y_X_Z` comparator: by y, then x, then z.
    pub fn cmp_y_x_z(&self, other: &BlockPosition) -> Ordering {
        self.y.cmp(&other.y).then(self.x.cmp(&other.x)).then(self.z.cmp(&other.z))
    }
}

/// Orders by y, then x, then z (`BlockPosition.BY_Y_X_Z`), which is what sorting positions should use.
impl PartialOrd for BlockPosition {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BlockPosition {
    fn cmp(&self, other: &Self) -> Ordering {
        self.cmp_y_x_z(other)
    }
}

/// A position or velocity with fractional components.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z }
    }
}

/// A pre-flattening block or item id: a numeric id and a data value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LegacyIdentifier {
    pub id: i32,
    pub data: i8,
}

impl LegacyIdentifier {
    pub const fn new(id: i32, data: i8) -> LegacyIdentifier {
        LegacyIdentifier { id, data }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_sort_by_y_then_x_then_z() {
        let mut v = vec![
            BlockPosition::new(1, 2, 0),
            BlockPosition::new(0, 2, 5),
            BlockPosition::new(9, 1, 9),
            BlockPosition::new(0, 2, 1),
        ];
        v.sort();
        assert_eq!(
            v,
            [
                BlockPosition::new(9, 1, 9),
                BlockPosition::new(0, 2, 1),
                BlockPosition::new(0, 2, 5),
                BlockPosition::new(1, 2, 0)
            ]
        );
    }
}
