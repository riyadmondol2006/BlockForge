// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! The few block state value enums with methods: `FacingDirection` and `FacingDirectionHorizontal`.

use super::generated::{FacingDirection, FacingDirectionHorizontal};

impl FacingDirection {
    /// From the 3D data value of legacy Java and Bedrock (`from3DByte`): 0 down, 1 up, 2 north, 3 south, 4 west,
    /// 5 east. The value is reduced `% 6` like Java, so negatives (and 6 and above) end up north or wrap.
    pub fn from_3d_byte(direction: i8) -> FacingDirection {
        match direction % 6 {
            0 => FacingDirection::DOWN,
            1 => FacingDirection::UP,
            2 => FacingDirection::NORTH,
            3 => FacingDirection::SOUTH,
            4 => FacingDirection::WEST,
            5 => FacingDirection::EAST,
            _ => FacingDirection::NORTH,
        }
    }

    /// From the 2D data value (`from2DByte`): 0 south, 1 west, 2 north, 3 east, reduced `% 4`.
    pub fn from_2d_byte(direction: i8) -> FacingDirection {
        match direction % 4 {
            0 => FacingDirection::SOUTH,
            1 => FacingDirection::WEST,
            2 => FacingDirection::NORTH,
            3 => FacingDirection::EAST,
            _ => FacingDirection::NORTH,
        }
    }

    /// The 3D data value (`to3DByte`).
    pub const fn to_3d_byte(self) -> i8 {
        match self {
            FacingDirection::SOUTH => 3,
            FacingDirection::WEST => 4,
            FacingDirection::NORTH => 2,
            FacingDirection::EAST => 5,
            FacingDirection::UP => 1,
            FacingDirection::DOWN => 0,
        }
    }

    /// The 2D data value (`to2DByte`); up and down are 0.
    pub const fn to_2d_byte(self) -> i8 {
        match self {
            FacingDirection::SOUTH => 0,
            FacingDirection::WEST => 1,
            FacingDirection::NORTH => 2,
            FacingDirection::EAST => 3,
            _ => 0,
        }
    }
}

impl FacingDirectionHorizontal {
    /// Block offset on the x axis.
    pub const fn x(self) -> i32 {
        match self {
            FacingDirectionHorizontal::NORTH | FacingDirectionHorizontal::SOUTH => 0,
            FacingDirectionHorizontal::EAST => 1,
            FacingDirectionHorizontal::WEST => -1,
        }
    }

    /// Block offset on the z axis.
    pub const fn z(self) -> i32 {
        match self {
            FacingDirectionHorizontal::NORTH => -1,
            FacingDirectionHorizontal::SOUTH => 1,
            FacingDirectionHorizontal::EAST | FacingDirectionHorizontal::WEST => 0,
        }
    }

    /// From the 2D data value (`from2DByte`): 0 south, 1 west, 2 north, 3 east, reduced `% 4`.
    pub fn from_2d_byte(direction: i8) -> FacingDirectionHorizontal {
        match direction % 4 {
            0 => FacingDirectionHorizontal::SOUTH,
            1 => FacingDirectionHorizontal::WEST,
            2 => FacingDirectionHorizontal::NORTH,
            3 => FacingDirectionHorizontal::EAST,
            _ => FacingDirectionHorizontal::NORTH,
        }
    }

    /// The 2D data value (`to2DByte`).
    pub const fn to_2d_byte(self) -> i8 {
        match self {
            FacingDirectionHorizontal::SOUTH => 0,
            FacingDirectionHorizontal::WEST => 1,
            FacingDirectionHorizontal::NORTH => 2,
            FacingDirectionHorizontal::EAST => 3,
        }
    }

    pub const fn rotate_clockwise(self) -> FacingDirectionHorizontal {
        match self {
            FacingDirectionHorizontal::NORTH => FacingDirectionHorizontal::EAST,
            FacingDirectionHorizontal::SOUTH => FacingDirectionHorizontal::WEST,
            FacingDirectionHorizontal::WEST => FacingDirectionHorizontal::NORTH,
            FacingDirectionHorizontal::EAST => FacingDirectionHorizontal::SOUTH,
        }
    }

    pub const fn rotate_anti_clockwise(self) -> FacingDirectionHorizontal {
        match self {
            FacingDirectionHorizontal::NORTH => FacingDirectionHorizontal::WEST,
            FacingDirectionHorizontal::SOUTH => FacingDirectionHorizontal::EAST,
            FacingDirectionHorizontal::WEST => FacingDirectionHorizontal::SOUTH,
            FacingDirectionHorizontal::EAST => FacingDirectionHorizontal::NORTH,
        }
    }

    /// Whether the two directions are at a right angle (`isAdjacent`).
    pub const fn is_adjacent(self, other: FacingDirectionHorizontal) -> bool {
        let diff = (self as i32 - other as i32).abs();
        diff == 1 || diff == 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facing_direction_bytes() {
        for d in FacingDirection::ALL {
            assert_eq!(FacingDirection::from_3d_byte(d.to_3d_byte()), d);
        }
        assert_eq!(FacingDirection::from_3d_byte(6), FacingDirection::DOWN);
        assert_eq!(FacingDirection::from_3d_byte(-1), FacingDirection::NORTH); // negative remainder: default
        assert_eq!(FacingDirection::from_2d_byte(0), FacingDirection::SOUTH);
        assert_eq!(FacingDirection::from_2d_byte(5), FacingDirection::WEST);
        assert_eq!(FacingDirection::from_2d_byte(-3), FacingDirection::NORTH);
        assert_eq!(FacingDirection::UP.to_2d_byte(), 0);
        assert_eq!(FacingDirection::EAST.to_2d_byte(), 3);
    }

    #[test]
    fn horizontal_directions() {
        use FacingDirectionHorizontal::*;
        assert_eq!(FacingDirectionHorizontal::ALL, [NORTH, EAST, SOUTH, WEST]);
        assert_eq!((NORTH.x(), NORTH.z()), (0, -1));
        assert_eq!((EAST.x(), EAST.z()), (1, 0));
        assert_eq!((SOUTH.x(), SOUTH.z()), (0, 1));
        assert_eq!((WEST.x(), WEST.z()), (-1, 0));
        for d in FacingDirectionHorizontal::ALL {
            assert_eq!(d.rotate_clockwise().rotate_anti_clockwise(), d);
            assert_eq!(d.rotate_clockwise().rotate_clockwise().rotate_clockwise().rotate_clockwise(), d);
            assert_eq!(FacingDirectionHorizontal::from_2d_byte(d.to_2d_byte()), d);
        }
        assert_eq!(NORTH.rotate_clockwise(), EAST);
        assert_eq!(NORTH.rotate_anti_clockwise(), WEST);
        assert!(NORTH.is_adjacent(EAST) && NORTH.is_adjacent(WEST));
        assert!(!NORTH.is_adjacent(SOUTH) && !NORTH.is_adjacent(NORTH));
    }
}
