// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Gaps in our tables that the validation tests found and that are documented in `docs/porting/COVERAGE.md`.

/// The reason a failing `(check, edition, id)` is a known gap, if it is one.
pub(crate) fn known_gap(_check: &str, _edition: &str, _id: &str) -> Option<&'static str> {
    None
}
