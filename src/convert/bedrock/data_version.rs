// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The table of Bedrock versions Chunker knows (`BedrockDataVersion`) and the rules that pick one.

/// A game version as `[major, minor, patch]`.
pub type GameVersion = [i32; 3];

/// One registered Bedrock version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BedrockVersion {
    /// The game version, `[1, 26, 50]`.
    pub game: GameVersion,
    /// The network protocol version (`NetworkVersion` in level.dat).
    pub protocol: i32,
    /// The `level.dat` storage version (8, 9 or 10); 9 and up also mean sub-chunk v9 and Data3D from 1.18.30.
    pub storage: i32,
    /// The extra value in the low byte of the block state version (only 1.20.0 uses one: 33).
    pub state_patch: i32,
}

macro_rules! v {
    ($proto:expr, $storage:expr, $maj:expr, $min:expr, $patch:expr) => {
        BedrockVersion { game: [$maj, $min, $patch], protocol: $proto, storage: $storage, state_patch: 0 }
    };
    ($proto:expr, $storage:expr, $sub:expr, $maj:expr, $min:expr, $patch:expr) => {
        BedrockVersion { game: [$maj, $min, $patch], protocol: $proto, storage: $storage, state_patch: $sub }
    };
}

/// Every registered version, oldest first (`BedrockDataVersion.register` calls, 1.12.0 to 1.26.60).
pub static VERSIONS: &[BedrockVersion] = &[
    v!(361, 8, 1, 12, 0),
    v!(388, 8, 1, 13, 0),
    v!(389, 8, 1, 14, 0),
    v!(389, 8, 1, 14, 20),
    v!(389, 8, 1, 14, 30),
    v!(390, 8, 1, 14, 60),
    v!(407, 8, 1, 16, 0),
    v!(408, 8, 1, 16, 20),
    v!(408, 8, 1, 16, 40),
    v!(419, 8, 1, 16, 100),
    v!(422, 8, 1, 16, 200),
    v!(428, 8, 1, 16, 210),
    v!(431, 8, 1, 16, 220),
    v!(440, 8, 1, 17, 0),
    v!(448, 8, 1, 17, 10),
    v!(465, 8, 1, 17, 30),
    v!(471, 8, 1, 17, 40),
    v!(475, 8, 1, 18, 0),
    v!(486, 8, 1, 18, 10),
    v!(503, 9, 1, 18, 30),
    v!(526, 9, 1, 19, 0),
    v!(534, 9, 1, 19, 10),
    v!(544, 9, 1, 19, 20),
    v!(554, 9, 1, 19, 30),
    v!(557, 10, 1, 19, 40),
    v!(560, 10, 1, 19, 50),
    v!(568, 10, 1, 19, 60),
    v!(575, 10, 1, 19, 70),
    v!(582, 10, 1, 19, 80),
    v!(588, 10, 33, 1, 20, 0),
    v!(594, 10, 1, 20, 10),
    v!(618, 10, 1, 20, 30),
    v!(622, 10, 1, 20, 40),
    v!(630, 10, 1, 20, 50),
    v!(649, 10, 1, 20, 60),
    v!(662, 10, 1, 20, 70),
    v!(671, 10, 1, 20, 80),
    v!(685, 10, 1, 21, 0),
    v!(712, 10, 1, 21, 20),
    v!(729, 10, 1, 21, 30),
    v!(748, 10, 1, 21, 40),
    v!(766, 10, 1, 21, 50),
    v!(776, 10, 1, 21, 60),
    v!(786, 10, 1, 21, 70),
    v!(800, 10, 1, 21, 80),
    v!(818, 10, 1, 21, 90),
    v!(819, 10, 1, 21, 93),
    v!(827, 10, 1, 21, 100),
    v!(843, 10, 1, 21, 110),
    v!(859, 10, 1, 21, 120),
    v!(898, 10, 1, 21, 130),
    v!(924, 10, 1, 26, 0),
    v!(944, 10, 1, 26, 10),
    v!(974, 10, 1, 26, 20),
    v!(1001, 10, 1, 26, 30),
    v!(2168, 10, 1, 26, 40),
    v!(2192, 10, 1, 26, 50),
    v!(2223, 10, 1, 26, 60),
];

/// The versions that have their own reader/writer package in Chunker (`BedrockEncoders.register`); the others use the
/// nearest one at or below.
pub static ENCODER_ROOTS: [GameVersion; 28] = [
    [1, 12, 0],
    [1, 13, 0],
    [1, 14, 0],
    [1, 16, 0],
    [1, 17, 0],
    [1, 17, 30],
    [1, 18, 0],
    [1, 18, 30],
    [1, 19, 0],
    [1, 19, 80],
    [1, 20, 0],
    [1, 20, 30],
    [1, 20, 50],
    [1, 20, 60],
    [1, 21, 0],
    [1, 21, 40],
    [1, 21, 50],
    [1, 21, 60],
    [1, 21, 80],
    [1, 21, 90],
    [1, 21, 100],
    [1, 21, 110],
    [1, 21, 120],
    [1, 26, 20],
    [1, 26, 30],
    [1, 26, 40],
    [1, 26, 50],
    [1, 26, 60],
];

/// All registered versions, oldest first.
pub fn versions() -> &'static [BedrockVersion] {
    VERSIONS
}

impl BedrockVersion {
    /// The oldest registered version (1.12.0).
    pub fn oldest() -> &'static BedrockVersion {
        &versions()[0]
    }

    /// The newest registered version.
    pub fn latest() -> &'static BedrockVersion {
        &versions()[versions().len() - 1]
    }

    /// The registered version nearest at or below `game` (the oldest one when `game` is older than all of them), like
    /// `BedrockDataVersion.getNearestVersion`.
    pub fn nearest_at_or_below(game: GameVersion) -> &'static BedrockVersion {
        let all = versions();
        all.iter().rev().find(|v| v.game <= game).unwrap_or(&all[0])
    }

    /// The registered version nearest at or below a network protocol version (`getNearestProtocol`).
    pub fn nearest_protocol(protocol: i32) -> &'static BedrockVersion {
        let all = versions();
        // Several versions can share a protocol (1.14.x): the last one registered for it wins, like a TreeMap put.
        let best = all.iter().filter(|v| v.protocol <= protocol).map(|v| v.protocol).max();
        match best {
            Some(p) => all.iter().rev().find(|v| v.protocol == p).unwrap_or(&all[0]),
            None => &all[0],
        }
    }

    /// The exact registered version, if any.
    pub fn exact(game: GameVersion) -> Option<&'static BedrockVersion> {
        versions().iter().find(|v| v.game == game)
    }

    /// `1.26.50`.
    pub fn name(&self) -> String {
        format!("{}.{}.{}", self.game[0], self.game[1], self.game[2])
    }

    /// The block state version written into palette entries: `major << 24 | minor << 16 | patch << 8 | sub`.
    pub fn state_version(&self) -> i32 {
        (self.game[0] << 24) | (self.game[1] << 16) | (self.game[2] << 8) | self.state_patch
    }

    /// The version whose reader/writer package Chunker uses for this one.
    pub fn encoder_root(&self) -> GameVersion {
        ENCODER_ROOTS.iter().rev().find(|r| **r <= self.game).copied().unwrap_or(ENCODER_ROOTS[0])
    }

    /// Whether this version is at or after `game`.
    pub fn at_least(&self, game: GameVersion) -> bool {
        self.game >= game
    }

    /// Sub-chunks are written as version 9 (with the y index inside) from 1.17.30 on.
    pub fn sub_chunk_version(&self) -> u8 {
        if self.at_least([1, 17, 30]) { 9 } else { 8 }
    }

    /// Whether biomes are stored in Data3D (1.17.30 and on) rather than Data2D.
    pub fn uses_data3d(&self) -> bool {
        self.at_least([1, 17, 30])
    }

    /// Whether entities live in `digp`/`actorprefix` records (1.18.30 and on) rather than the `Entity` chunk record.
    pub fn uses_actor_storage(&self) -> bool {
        self.at_least([1, 18, 30])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_and_complete() {
        let all = versions();
        assert_eq!(all.len(), 58);
        assert!(all.windows(2).all(|w| w[0].game < w[1].game));
        assert!(all.windows(2).all(|w| w[0].protocol <= w[1].protocol));
        assert_eq!(BedrockVersion::oldest().game, [1, 12, 0]);
        assert_eq!(BedrockVersion::latest().game, [1, 26, 60]);
        for r in ENCODER_ROOTS {
            assert!(BedrockVersion::exact(r).is_some(), "{r:?}");
        }
    }

    #[test]
    fn nearest_lookup_rounds_down() {
        let v = BedrockVersion::nearest_at_or_below([1, 26, 51]);
        assert_eq!((v.game, v.protocol, v.storage), ([1, 26, 50], 2192, 10));
        assert_eq!(BedrockVersion::nearest_at_or_below([1, 26, 50]).game, [1, 26, 50]);
        assert_eq!(BedrockVersion::nearest_at_or_below([1, 20, 15]).game, [1, 20, 10]);
        assert_eq!(BedrockVersion::nearest_at_or_below([1, 2, 0]).game, [1, 12, 0]);
        assert_eq!(BedrockVersion::nearest_at_or_below([2, 0, 0]).game, [1, 26, 60]);
        assert_eq!(BedrockVersion::nearest_at_or_below([1, 18, 20]).storage, 8);
        assert_eq!(BedrockVersion::nearest_at_or_below([1, 18, 30]).storage, 9);
        assert_eq!(BedrockVersion::nearest_protocol(1001).game, [1, 26, 30]);
        assert_eq!(BedrockVersion::nearest_protocol(389).game, [1, 14, 30]);
        assert_eq!(BedrockVersion::nearest_protocol(1).game, [1, 12, 0]);
    }

    #[test]
    fn state_versions_and_roots() {
        let v = BedrockVersion::exact([1, 20, 0]).unwrap();
        assert_eq!(v.state_version(), (1 << 24) | (20 << 16) | 33);
        assert_eq!(BedrockVersion::exact([1, 21, 60]).unwrap().state_version(), 0x0115_3C00);
        assert_eq!(BedrockVersion::exact([1, 26, 10]).unwrap().encoder_root(), [1, 21, 120]);
        assert_eq!(BedrockVersion::exact([1, 26, 50]).unwrap().encoder_root(), [1, 26, 50]);
        assert_eq!(BedrockVersion::exact([1, 19, 70]).unwrap().encoder_root(), [1, 19, 0]);
        assert_eq!(BedrockVersion::exact([1, 18, 30]).unwrap().name(), "1.18.30");
    }

    #[test]
    fn format_knobs() {
        let old = BedrockVersion::exact([1, 17, 0]).unwrap();
        let mid = BedrockVersion::exact([1, 17, 30]).unwrap();
        let new = BedrockVersion::exact([1, 18, 30]).unwrap();
        assert_eq!((old.sub_chunk_version(), mid.sub_chunk_version()), (8, 9));
        assert!(!old.uses_data3d() && mid.uses_data3d());
        assert!(!mid.uses_actor_storage() && new.uses_actor_storage());
    }
}
