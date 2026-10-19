//! The identifier tables (blocks, items, biomes, entities) and the engine that applies them.
//!
//! Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//!
//! Chunker keeps its tables as Java code. They are executed once by `tools/convert/dump` and packed into
//! `data/convert/*.z` (see `tools/convert/README.md`); [`data`] loads that data lazily and [`engine`], [`items`] and
//! [`tables`] interpret it, so that both directions work: `Java/Bedrock (name, states) <-> pivot (type, states)`.
//!
//! The pivot is Chunker's own model, kept as strings for now: a block type is its Chunker constant name
//! (`"OAK_DOOR"`), a state is the Chunker `VanillaBlockStates` constant name (`"FACING_HORIZONTAL"`, not the Java
//! property `facing`, which several constants share) and a value is the Chunker enum constant name (`"NORTH"`,
//! `"TRUE"`). Edition-side values are typed ([`StateValue`]): Java properties are always strings, Bedrock states are
//! bool, int or string, and Bedrock's `waterlogged` travels as a pseudo state like in Chunker.

pub mod data;
pub mod engine;
pub mod items;
pub mod tables;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

pub use engine::{BlockMapping, BlockResolver, Flag, MappingError, PivotBlock, PivotInfo};
pub use items::{ItemKind, ItemResolver, PivotItem};
pub use tables::IdTable;

/// The two game editions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edition {
    Java,
    Bedrock,
}

/// A game version as `major.minor.patch` (Java `1.21.4` or `26.3.0`, Bedrock `1.21.50`).
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

    /// Parse `1`, `1.21` or `1.21.4` (missing parts are zero).
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = text.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
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

/// A raw edition-side state value (Chunker's `StateValue`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StateValue {
    Bool(bool),
    Int(i32),
    Str(Arc<str>),
}

impl StateValue {
    /// Parse the typed text form used by the data files: `b:true`, `i:5`, `s:north`.
    pub fn parse_typed(text: &str) -> Option<StateValue> {
        let (kind, rest) = text.split_once(':')?;
        match kind {
            "b" => Some(StateValue::Bool(rest == "true")),
            "i" | "l" => rest.parse().ok().map(StateValue::Int),
            "s" => Some(StateValue::Str(rest.into())),
            _ => None,
        }
    }

    /// Shorthand for a string value.
    pub fn str(text: &str) -> StateValue {
        StateValue::Str(text.into())
    }
}

impl fmt::Display for StateValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateValue::Bool(b) => write!(f, "b:{b}"),
            StateValue::Int(i) => write!(f, "i:{i}"),
            StateValue::Str(s) => write!(f, "s:{s}"),
        }
    }
}

/// Edition-side states of one block: Java properties, Bedrock states (plus `waterlogged` on Bedrock).
pub type StateMap = BTreeMap<Arc<str>, StateValue>;

/// Build a [`StateMap`] from pairs, mostly for tests and callers with literals.
pub fn states(pairs: &[(&str, StateValue)]) -> StateMap {
    pairs.iter().map(|(k, v)| (Arc::from(*k), v.clone())).collect()
}

#[cfg(test)]
mod tests;
