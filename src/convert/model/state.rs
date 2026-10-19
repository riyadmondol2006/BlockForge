// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Block states: typed handles, state sets of block types and the per-identifier state map.
//!
//! A block state is identified by a [`StateId`] (generated, 133 of them; names such as `age` or `facing` are shared
//! by several ids with different value types). Its value is one of the generated value enums; a vanilla value is
//! stored as its ordinal (`u8`). Custom blocks may carry extra named states with [`StateValue`]s.

use std::marker::PhantomData;
use std::sync::Arc;

use super::generated::{STATE_DEFS, StateDef, StateId};
use super::identifier::StateValue;

/// A block state value enum (`Axis`, `Bool`, `FacingDirection`, ...). The order of `VALUES` is the Java ordinal order.
pub trait StateEnum: Copy + Eq + 'static {
    /// All values, in ordinal order.
    const VALUES: &'static [Self];
    /// Java constant names, in ordinal order.
    const VALUE_NAMES: &'static [&'static str];

    /// The ordinal as stored in a state set.
    fn value_ordinal(self) -> u8;

    /// The value with the given ordinal.
    fn from_value_ordinal(ordinal: u8) -> Option<Self> {
        Self::VALUES.get(ordinal as usize).copied()
    }
}

/// A typed handle to a block state (`BlockState<T>`): the id plus the value type.
pub struct State<T> {
    id: StateId,
    marker: PhantomData<fn() -> T>,
}

impl<T> State<T> {
    pub const fn new(id: StateId) -> State<T> {
        State { id, marker: PhantomData }
    }

    pub const fn id(self) -> StateId {
        self.id
    }
}

impl<T: StateEnum> State<T> {
    /// The default value of the state.
    pub fn default_value(self) -> T {
        T::VALUES[self.id.def().default as usize]
    }
}

impl<T> Clone for State<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for State<T> {}

impl<T> PartialEq for State<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T> Eq for State<T> {}

impl<T> std::hash::Hash for State<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state)
    }
}

impl<T> std::fmt::Debug for State<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "State({})", self.id.name())
    }
}

impl StateId {
    /// The static definition (state name, default, value names).
    pub fn def(self) -> &'static StateDef {
        &STATE_DEFS[self as usize]
    }

    /// The state name as written in identifiers (`facing`, `age`, ...); several ids can share one.
    pub fn state_name(self) -> &'static str {
        self.def().name
    }

    /// Number of values the state can take.
    pub fn value_count(self) -> usize {
        self.def().values.len()
    }

    /// The ordinal of the value called `name` (a Java constant name such as `NORTH`).
    pub fn value_ordinal(self, name: &str) -> Option<u8> {
        self.def().values.iter().position(|v| *v == name).map(|i| i as u8)
    }

    /// Name of the value with the given ordinal.
    pub fn value_name(self, ordinal: u8) -> Option<&'static str> {
        self.def().values.get(ordinal as usize).copied()
    }

    /// Every id whose state name is `name` (`age` has 8, `facing` 3, ...), in id order.
    pub fn with_state_name(name: &str) -> impl Iterator<Item = StateId> + '_ {
        StateId::ALL.into_iter().filter(move |s| s.def().name == name)
    }
}

/// A set of state ids as a bit mask (133 ids fit in 3 words).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct StateMask(pub [u64; 3]);

impl StateMask {
    pub const EMPTY: StateMask = StateMask([0; 3]);

    pub const fn contains(&self, id: StateId) -> bool {
        let i = id as usize;
        self.0[i / 64] >> (i % 64) & 1 == 1
    }

    pub const fn with(mut self, id: StateId) -> StateMask {
        let i = id as usize;
        self.0[i / 64] |= 1 << (i % 64);
        self
    }

    pub const fn union(self, other: StateMask) -> StateMask {
        StateMask([self.0[0] | other.0[0], self.0[1] | other.0[1], self.0[2] | other.0[2]])
    }

    pub const fn len(&self) -> usize {
        (self.0[0].count_ones() + self.0[1].count_ones() + self.0[2].count_ones()) as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.0[0] == 0 && self.0[1] == 0 && self.0[2] == 0
    }

    /// The ids in the set, ascending.
    pub fn iter(&self) -> impl Iterator<Item = StateId> + '_ {
        (0..StateId::COUNT).filter(|i| self.0[i / 64] >> (i % 64) & 1 == 1).filter_map(StateId::from_ordinal)
    }
}

/// The states present on one block identifier: vanilla ids with their value ordinals, sorted by id, plus named
/// custom states sorted by name. Sorted storage makes `==` and `Hash` independent of insertion order.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct StateSet {
    vanilla: Vec<(StateId, u8)>,
    custom: Vec<(Arc<str>, StateValue)>,
}

impl StateSet {
    pub fn new() -> StateSet {
        StateSet::default()
    }

    /// Number of states present (vanilla and custom).
    pub fn len(&self) -> usize {
        self.vanilla.len() + self.custom.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vanilla.is_empty() && self.custom.is_empty()
    }

    /// Vanilla states as (id, value ordinal), ascending by id.
    pub fn vanilla(&self) -> &[(StateId, u8)] {
        &self.vanilla
    }

    /// Custom states, ascending by name.
    pub fn custom(&self) -> &[(Arc<str>, StateValue)] {
        &self.custom
    }

    pub fn contains(&self, id: StateId) -> bool {
        self.get_raw(id).is_some()
    }

    /// The raw value ordinal of a vanilla state, if present.
    pub fn get_raw(&self, id: StateId) -> Option<u8> {
        self.vanilla.binary_search_by_key(&id, |e| e.0).ok().map(|i| self.vanilla[i].1)
    }

    /// Set the raw value ordinal of a vanilla state.
    pub fn set_raw(&mut self, id: StateId, value: u8) {
        match self.vanilla.binary_search_by_key(&id, |e| e.0) {
            Ok(i) => self.vanilla[i].1 = value,
            Err(i) => self.vanilla.insert(i, (id, value)),
        }
    }

    /// Remove a vanilla state; returns whether it was present.
    pub fn remove(&mut self, id: StateId) -> bool {
        match self.vanilla.binary_search_by_key(&id, |e| e.0) {
            Ok(i) => {
                self.vanilla.remove(i);
                true
            }
            Err(_) => false,
        }
    }

    /// The typed value of a state, if present.
    pub fn get<T: StateEnum>(&self, state: State<T>) -> Option<T> {
        self.get_raw(state.id).and_then(T::from_value_ordinal)
    }

    /// Set a typed value.
    pub fn set<T: StateEnum>(&mut self, state: State<T>, value: T) {
        self.set_raw(state.id, value.value_ordinal());
    }

    pub fn get_custom(&self, name: &str) -> Option<&StateValue> {
        self.custom.binary_search_by(|e| (*e.0).cmp(name)).ok().map(|i| &self.custom[i].1)
    }

    /// Set a custom (non vanilla) state.
    pub fn set_custom(&mut self, name: impl AsRef<str>, value: StateValue) {
        match self.custom.binary_search_by(|e| (*e.0).cmp(name.as_ref())) {
            Ok(i) => self.custom[i].1 = value,
            Err(i) => self.custom.insert(i, (Arc::from(name.as_ref()), value)),
        }
    }

    pub fn remove_custom(&mut self, name: &str) -> bool {
        match self.custom.binary_search_by(|e| (*e.0).cmp(name)) {
            Ok(i) => {
                self.custom.remove(i);
                true
            }
            Err(_) => false,
        }
    }
}
