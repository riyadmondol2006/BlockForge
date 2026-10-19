// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The Java data-version table (Chunker's `JavaDataVersion`) and the data-version thresholds the format switches use.
//!
//! Only release builds are listed (1.9 = 169 ... 26.3 = 5017); snapshots round down to the release before them.

use std::fmt;

/// A game version (`major.minor.patch`), ordered like Chunker's `Version`.
///
/// This is a local twin of `util::Version`; the two have the same shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Version {
        Version { major, minor, patch }
    }

    /// True when this version is at least `major.minor.patch`.
    pub fn at_least(&self, major: u32, minor: u32, patch: u32) -> bool {
        *self >= Version::new(major, minor, patch)
    }

    /// Parse `1.20.4`, `1.21` or `26.1` (missing parts are 0). Returns None on anything else.
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = text.trim().split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = match parts.next() {
            Some(p) => p.parse().ok()?,
            None => 0,
        };
        let patch = match parts.next() {
            Some(p) => p.parse().ok()?,
            None => 0,
        };
        if parts.next().is_some() {
            return None;
        }
        Some(Version { major, minor, patch })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The `version` int of the last Anvil `level.dat` (1.8.x worlds have no `Version` compound, only this).
pub const LAST_ANVIL_FILE_VERSION: i32 = 19133;

// Data versions where the on-disk format changes (the keys of Chunker's encoder registry), plus a few more
// that switches use. Named after the first game version that has them.
pub const DV_1_8_8: i32 = 0;
pub const DV_1_11: i32 = 819;
pub const DV_1_13: i32 = 1519;
pub const DV_1_14: i32 = 1952;
pub const DV_1_15: i32 = 2225;
pub const DV_1_16: i32 = 2566;
pub const DV_1_17: i32 = 2724;
pub const DV_1_18: i32 = 2860;
pub const DV_1_19_3: i32 = 3218;
pub const DV_1_20: i32 = 3463;
pub const DV_1_20_3: i32 = 3698;
pub const DV_1_20_5: i32 = 3837;
pub const DV_1_21: i32 = 3953;
pub const DV_1_21_2: i32 = 4080;
pub const DV_1_21_4: i32 = 4189;
pub const DV_1_21_5: i32 = 4325;
pub const DV_1_21_6: i32 = 4435;
pub const DV_1_21_9: i32 = 4554;
pub const DV_1_21_11: i32 = 4671;
pub const DV_26_1: i32 = 4786;
pub const DV_26_2: i32 = 4903;
pub const DV_26_3: i32 = 5017;

/// (data version, game version) for every registered release, in the order of `JavaDataVersion`.
pub const DATA_VERSIONS: &[(i32, Version)] = &[
    (0, Version::new(1, 8, 8)),
    (169, Version::new(1, 9, 0)),
    (175, Version::new(1, 9, 1)),
    (176, Version::new(1, 9, 2)),
    (183, Version::new(1, 9, 3)),
    (184, Version::new(1, 9, 4)),
    (510, Version::new(1, 10, 0)),
    (511, Version::new(1, 10, 1)),
    (512, Version::new(1, 10, 2)),
    (819, Version::new(1, 11, 0)),
    (921, Version::new(1, 11, 1)),
    (922, Version::new(1, 11, 2)),
    (1139, Version::new(1, 12, 0)),
    (1241, Version::new(1, 12, 1)),
    (1343, Version::new(1, 12, 2)),
    (1519, Version::new(1, 13, 0)),
    (1628, Version::new(1, 13, 1)),
    (1631, Version::new(1, 13, 2)),
    (1952, Version::new(1, 14, 0)),
    (1957, Version::new(1, 14, 1)),
    (1963, Version::new(1, 14, 2)),
    (1968, Version::new(1, 14, 3)),
    (1976, Version::new(1, 14, 4)),
    (2225, Version::new(1, 15, 0)),
    (2227, Version::new(1, 15, 1)),
    (2230, Version::new(1, 15, 2)),
    (2566, Version::new(1, 16, 0)),
    (2567, Version::new(1, 16, 1)),
    (2578, Version::new(1, 16, 2)),
    (2580, Version::new(1, 16, 3)),
    (2584, Version::new(1, 16, 4)),
    (2586, Version::new(1, 16, 5)),
    (2724, Version::new(1, 17, 0)),
    (2730, Version::new(1, 17, 1)),
    (2860, Version::new(1, 18, 0)),
    (2865, Version::new(1, 18, 1)),
    (2975, Version::new(1, 18, 2)),
    (3105, Version::new(1, 19, 0)),
    (3117, Version::new(1, 19, 1)),
    (3120, Version::new(1, 19, 2)),
    (3218, Version::new(1, 19, 3)),
    (3337, Version::new(1, 19, 4)),
    (3463, Version::new(1, 20, 0)),
    (3465, Version::new(1, 20, 1)),
    (3578, Version::new(1, 20, 2)),
    (3698, Version::new(1, 20, 3)),
    (3700, Version::new(1, 20, 4)),
    (3837, Version::new(1, 20, 5)),
    (3839, Version::new(1, 20, 6)),
    (3953, Version::new(1, 21, 0)),
    (3955, Version::new(1, 21, 1)),
    (4080, Version::new(1, 21, 2)),
    (4082, Version::new(1, 21, 3)),
    (4189, Version::new(1, 21, 4)),
    (4325, Version::new(1, 21, 5)),
    (4435, Version::new(1, 21, 6)),
    (4438, Version::new(1, 21, 7)),
    (4440, Version::new(1, 21, 8)),
    (4554, Version::new(1, 21, 9)),
    (4556, Version::new(1, 21, 10)),
    (4671, Version::new(1, 21, 11)),
    (4786, Version::new(26, 1, 0)),
    (4788, Version::new(26, 1, 1)),
    (4790, Version::new(26, 1, 2)),
    (4903, Version::new(26, 2, 0)),
    (5017, Version::new(26, 3, 0)),
];

/// The registered row at or below `data_version`; an older number than the table gives the oldest row (1.8.8) and
/// a newer one the latest row (like `JavaDataVersion.getNearestVersion`).
pub fn nearest_at_or_below(data_version: i32) -> (i32, Version) {
    // The table is sorted by data version, so a binary search finds the floor.
    match DATA_VERSIONS.binary_search_by_key(&data_version, |row| row.0) {
        Ok(i) => DATA_VERSIONS[i],
        Err(0) => DATA_VERSIONS[0],
        Err(i) => DATA_VERSIONS[i - 1],
    }
}

/// The registered row for a game version, rounded down (like `JavaDataVersion.getNearestVersion(Version)`).
pub fn nearest_for_version(version: Version) -> (i32, Version) {
    let mut best = DATA_VERSIONS[0];
    for row in DATA_VERSIONS {
        if row.1 <= version {
            best = *row;
        }
    }
    best
}

/// The newest registered data version.
pub fn latest() -> (i32, Version) {
    DATA_VERSIONS[DATA_VERSIONS.len() - 1]
}

/// The oldest registered data version (1.8.8, "data version" 0).
pub fn oldest() -> (i32, Version) {
    DATA_VERSIONS[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_shape() {
        assert_eq!(DATA_VERSIONS.len(), 66);
        assert!(DATA_VERSIONS.windows(2).all(|w| w[0].0 < w[1].0), "sorted by data version");
        assert!(DATA_VERSIONS.windows(2).all(|w| w[0].1 < w[1].1), "sorted by game version");
    }

    #[test]
    fn every_row_finds_itself() {
        for row in DATA_VERSIONS {
            assert_eq!(nearest_at_or_below(row.0), *row);
            assert_eq!(nearest_for_version(row.1), *row);
        }
    }

    #[test]
    fn older_and_newer() {
        assert_eq!(nearest_at_or_below(-1), oldest());
        assert_eq!(nearest_at_or_below(5018), latest());
        assert_eq!(nearest_at_or_below(i32::MAX), latest());
    }

    #[test]
    fn ranges_floor() {
        // JavaDataVersionTests.testVersionRange: every number below the next row maps to this row.
        for w in DATA_VERSIONS.windows(2) {
            for dv in w[0].0..w[1].0 {
                assert_eq!(nearest_at_or_below(dv).0, w[0].0);
            }
        }
        assert_eq!(nearest_at_or_below(100).0, 0);
        assert_eq!(nearest_at_or_below(2586).1, Version::new(1, 16, 5));
        assert_eq!(nearest_at_or_below(3954).1, Version::new(1, 21, 0));
        assert_eq!(nearest_at_or_below(4000).1, Version::new(1, 21, 1));
    }

    #[test]
    fn thresholds_are_rows() {
        for dv in [
            DV_1_8_8, DV_1_11, DV_1_13, DV_1_14, DV_1_15, DV_1_16, DV_1_17, DV_1_18, DV_1_19_3, DV_1_20, DV_1_20_3,
            DV_1_20_5, DV_1_21, DV_1_21_2, DV_1_21_4, DV_1_21_5, DV_1_21_6, DV_1_21_9, DV_1_21_11, DV_26_1, DV_26_2,
            DV_26_3,
        ] {
            assert_eq!(nearest_at_or_below(dv).0, dv, "{dv} must be a registered row");
        }
    }

    #[test]
    fn version_parse_and_order() {
        assert_eq!(Version::parse("1.20.4"), Some(Version::new(1, 20, 4)));
        assert_eq!(Version::parse("26.1"), Some(Version::new(26, 1, 0)));
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert_eq!(Version::parse("1.x"), None);
        assert!(Version::new(1, 21, 0).at_least(1, 20, 5));
        assert!(!Version::new(1, 12, 2).at_least(1, 13, 0));
        assert_eq!(Version::new(1, 21, 0).to_string(), "1.21.0");
    }
}
