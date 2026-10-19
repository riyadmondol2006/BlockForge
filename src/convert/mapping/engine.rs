// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The block identifier engine: a port of Chunker's `ChunkerBlockIdentifierResolver`, `StateMappingGroup`,
//! `StateMapping`, `TypeMapping` and `BlockMapping`.
//!
//! A [`BlockResolver`] holds two lookup tables, edition -> pivot and pivot -> edition. Each maps an identifier to a
//! [`Grouping`]: the mappings of that identifier, bucketed by the set of pinned state names and tried largest set
//! first. A hit yields the output identifier, its pinned output states, and then the states derived by the mapping's
//! [`StateGroup`] and by the resolver-wide extra group (Bedrock `waterlogged`, Java default `waterlogged=false`).

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, RwLock};

use super::{StateMap, StateValue};

/// Pivot value (Chunker enum constant name, e.g. `NORTH`).
pub type PivotValue = Arc<str>;

// ---- pivot description -------------------------------------------------------------------------------------------

/// One Chunker `BlockState`: its Java-style name, default and value list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateInfo {
    pub name: Arc<str>,
    pub default: PivotValue,
    pub values: Vec<PivotValue>,
}

/// What the engine needs to know about the pivot: the states and which states each block type has. Defaults are only
/// applied for states the type has (`ChunkerBlockIdentifier.getState`).
#[derive(Clone, Debug, Default)]
pub struct PivotInfo {
    pub states: HashMap<Arc<str>, StateInfo>,
    pub types: HashMap<Arc<str>, Vec<Arc<str>>>,
}

impl PivotInfo {
    /// The default value of `state` for `ty`, if the type has that state.
    pub fn default_for(&self, ty: &str, state: &str) -> Option<PivotValue> {
        let set = self.types.get(ty)?;
        if !set.iter().any(|s| &**s == state) {
            return None;
        }
        self.states.get(state).map(|s| s.default.clone())
    }

    /// The sort name of a state (Chunker orders pinned states by `BlockState.getName`).
    fn order_name(&self, state: &Arc<str>) -> Arc<str> {
        self.states.get(state).map_or_else(|| state.clone(), |s| s.name.clone())
    }
}

/// A block in the version-neutral model.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PivotBlock {
    /// Chunker block type constant, e.g. `OAK_DOOR`.
    pub ty: Arc<str>,
    /// Chunker state constant -> value constant; only the states that are set (the rest are defaults).
    pub states: BTreeMap<Arc<str>, PivotValue>,
}

impl PivotBlock {
    pub fn new(ty: &str, states: &[(&str, &str)]) -> PivotBlock {
        PivotBlock { ty: ty.into(), states: states.iter().map(|(k, v)| (Arc::from(*k), Arc::from(*v))).collect() }
    }
}

// ---- type mappings and state groups ------------------------------------------------------------------------------

/// Error raised while building tables (the Java code throws `IllegalArgumentException` at the same places).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MappingError {
    /// `Insert` hit an existing key.
    Duplicate(String),
    /// `Override` found nothing to override.
    NothingToOverride(String),
    /// A state group names a type mapping or group that does not exist.
    Unknown(String),
    /// A type mapping row repeats an input and an output.
    DuplicateTypeMapping(String),
}

impl fmt::Display for MappingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MappingError::Duplicate(s) => write!(f, "duplicate state lookup entry: {s}"),
            MappingError::NothingToOverride(s) => write!(f, "no existing mapping to override: {s}"),
            MappingError::Unknown(s) => write!(f, "unknown reference: {s}"),
            MappingError::DuplicateTypeMapping(s) => write!(f, "duplicate type mapping: {s}"),
        }
    }
}

impl std::error::Error for MappingError {}

/// Value conversion between edition input values and pivot values (`TypeMapping`). Several inputs may share one output
/// (lossy legacy values); `output_to_input` then names the one that is written back.
#[derive(Clone, Debug, Default)]
pub struct TypeMapping {
    pub input_to_output: HashMap<Vec<StateValue>, Vec<PivotValue>>,
    pub output_to_input: HashMap<Vec<PivotValue>, Vec<StateValue>>,
    pub default_input: Option<Vec<StateValue>>,
    pub default_output: Option<Vec<PivotValue>>,
}

impl TypeMapping {
    pub fn new() -> TypeMapping {
        TypeMapping::default()
    }

    /// `TypeMapping.Builder.mapping`: put-if-absent in both directions; an error only if both already existed.
    pub fn mapping(&mut self, inputs: Vec<StateValue>, outputs: Vec<PivotValue>) -> Result<(), MappingError> {
        let a = self.input_to_output.contains_key(&inputs);
        let b = self.output_to_input.contains_key(&outputs);
        if a && b {
            return Err(MappingError::DuplicateTypeMapping(format!("{inputs:?} -> {outputs:?}")));
        }
        self.input_to_output.entry(inputs.clone()).or_insert_with(|| outputs.clone());
        self.output_to_input.entry(outputs).or_insert(inputs);
        Ok(())
    }

    /// `TypeMapping.Builder.defaults`.
    pub fn defaults(&mut self, input: Vec<StateValue>, output: Vec<PivotValue>) {
        self.default_input = Some(input);
        self.default_output = Some(output);
    }
}

/// One edition state (or several combined) <-> one pivot state (or several combined) (`StateMapping`).
#[derive(Clone, Debug)]
pub struct StateMapping {
    pub input_names: Vec<Arc<str>>,
    pub output_states: Vec<Arc<str>>,
    pub ty: Arc<TypeMapping>,
}

impl StateMapping {
    /// Edition -> pivot. `lookup` reads an edition state (never defaulted).
    fn apply_input(
        &self,
        lookup: &dyn Fn(&str) -> Option<StateValue>,
        outputs: &mut BTreeMap<Arc<str>, PivotValue>,
        allow_default: bool,
    ) {
        let mut values = Vec::with_capacity(self.input_names.len());
        let mut complete = true;
        for name in &self.input_names {
            match lookup(name) {
                Some(v) => values.push(v),
                None => {
                    complete = false;
                    break;
                }
            }
        }
        let mut out = if complete { self.ty.input_to_output.get(&values) } else { None };
        if out.is_none() && allow_default {
            out = self.ty.default_output.as_ref();
        }
        let Some(out) = out else { return };
        for (k, v) in self.output_states.iter().zip(out) {
            outputs.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    /// Pivot -> edition. `lookup(state, default_allowed)` reads a pivot state.
    fn apply_output(
        &self,
        lookup: &dyn Fn(&str, bool) -> Option<PivotValue>,
        outputs: &mut StateMap,
        allow_default: bool,
    ) {
        let use_default = allow_default && self.ty.default_input.is_none();
        let mut values = Vec::with_capacity(self.output_states.len());
        let mut complete = true;
        for state in &self.output_states {
            match lookup(state, use_default) {
                Some(v) => values.push(v),
                None => {
                    complete = false;
                    break;
                }
            }
        }
        let mut out = if complete { self.ty.output_to_input.get(&values) } else { None };
        if out.is_none() && allow_default {
            out = self.ty.default_input.as_ref();
        }
        let Some(out) = out else { return };
        for (k, v) in self.input_names.iter().zip(out) {
            outputs.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
}

/// The states derived for a block once its identifier matched (`StateMappingGroup`).
#[derive(Clone, Debug, Default)]
pub struct StateGroup {
    pub states: Vec<StateMapping>,
    pub default_inputs: BTreeMap<Arc<str>, StateValue>,
    pub default_outputs: BTreeMap<Arc<str>, PivotValue>,
}

impl StateGroup {
    /// Edition -> pivot: fill `outputs` (put-if-absent) from the edition states, then from the defaults.
    pub fn apply_input(
        &self,
        lookup: &dyn Fn(&str) -> Option<StateValue>,
        outputs: &mut BTreeMap<Arc<str>, PivotValue>,
    ) {
        for sm in &self.states {
            let allow_default = !sm.output_states.iter().all(|s| self.default_outputs.contains_key(s));
            sm.apply_input(lookup, outputs, allow_default);
        }
        for (k, v) in &self.default_outputs {
            outputs.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    /// Pivot -> edition: fill `outputs` (put-if-absent) from the pivot states, then from the defaults.
    pub fn apply_output(&self, lookup: &dyn Fn(&str, bool) -> Option<PivotValue>, outputs: &mut StateMap) {
        for sm in &self.states {
            let allow_default = !sm.input_names.iter().all(|s| self.default_inputs.contains_key(s));
            sm.apply_output(lookup, outputs, allow_default);
        }
        for (k, v) in &self.default_inputs {
            outputs.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
}

// ---- grouping ----------------------------------------------------------------------------------------------------

/// What a mapping resolves to: the output identifier, its pinned states and its state group.
#[derive(Clone, Debug)]
pub struct Output<O, OK, OV> {
    pub ident: O,
    pub outputs: BTreeMap<OK, OV>,
    pub group: Option<Arc<StateGroup>>,
}

pub(crate) struct GroupEntry<IK, IV, O, OK, OV> {
    names: Vec<IK>,
    /// Sort key of each name (Java compares `String` / `BlockState.getName`).
    order: Vec<Arc<str>>,
    rows: HashMap<Vec<IV>, Output<O, OK, OV>>,
}

/// The mappings of one identifier, grouped by the set of pinned state names, largest set first (`InputStateGrouping`).
pub struct Grouping<IK, IV, O, OK, OV> {
    pub(crate) entries: Vec<GroupEntry<IK, IV, O, OK, OV>>,
}

impl<IK, IV, O, OK, OV> Default for Grouping<IK, IV, O, OK, OV> {
    fn default() -> Self {
        Grouping { entries: Vec::new() }
    }
}

impl<IK: PartialEq + Clone, IV: Eq + std::hash::Hash, O, OK, OV> Grouping<IK, IV, O, OK, OV> {
    /// `createMapping`: `names` and `values` are the pinned input states in sorted-name order; `order` holds the sort
    /// name of each. With `override_existing` an existing row must be replaced, otherwise it must not exist.
    pub(crate) fn insert(
        &mut self,
        names: Vec<IK>,
        order: Vec<Arc<str>>,
        values: Vec<IV>,
        output: Output<O, OK, OV>,
        override_existing: bool,
        what: &dyn Fn() -> String,
    ) -> Result<(), MappingError> {
        let idx = match self.entries.iter().position(|e| e.names == names) {
            Some(i) => i,
            None => {
                self.entries.push(GroupEntry { names: names.clone(), order, rows: HashMap::new() });
                // Largest set first, then names in descending order (reverse of Chunker's CollectionComparator).
                self.entries.sort_by(|a, b| (b.order.len(), &b.order).cmp(&(a.order.len(), &a.order)));
                self.entries.iter().position(|e| e.names == names).unwrap_or(0)
            }
        };
        match self.entries[idx].rows.entry(values) {
            Entry::Occupied(mut slot) if override_existing => {
                slot.insert(output);
            }
            Entry::Vacant(slot) if !override_existing => {
                slot.insert(output);
            }
            Entry::Occupied(_) => return Err(MappingError::Duplicate(what())),
            Entry::Vacant(_) => return Err(MappingError::NothingToOverride(what())),
        }
        Ok(())
    }

    /// Whether `insert` of this key would succeed.
    pub(crate) fn check(
        &self,
        names: &[IK],
        values: &[IV],
        override_existing: bool,
        what: &dyn Fn() -> String,
    ) -> Result<(), MappingError> {
        let exists = self.entries.iter().find(|e| e.names == names).is_some_and(|e| e.rows.contains_key(values));
        match (override_existing, exists) {
            (true, false) => Err(MappingError::NothingToOverride(what())),
            (false, true) => Err(MappingError::Duplicate(what())),
            _ => Ok(()),
        }
    }

    /// `resolve`: the first group (largest name set first) whose states are all readable and match a row.
    pub(crate) fn resolve(
        &self,
        get: &dyn Fn(&IK) -> Option<IV>,
        apply: &dyn Fn(&StateGroup, &mut BTreeMap<OK, OV>),
        extra: Option<&StateGroup>,
    ) -> Option<(&O, BTreeMap<OK, OV>)>
    where
        OK: Ord + Clone,
        OV: Clone,
    {
        'groups: for entry in &self.entries {
            let mut values = Vec::with_capacity(entry.names.len());
            for name in &entry.names {
                match get(name) {
                    Some(v) => values.push(v),
                    None => continue 'groups,
                }
            }
            let Some(row) = entry.rows.get(&values) else { continue };
            let mut states = row.outputs.clone();
            if let Some(group) = &row.group {
                apply(group, &mut states);
            }
            if let Some(extra) = extra {
                apply(extra, &mut states);
            }
            return Some((&row.ident, states));
        }
        None
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.iter().map(|e| e.rows.len()).sum()
    }
}

// ---- block mappings -----------------------------------------------------------------------------------------------

/// How a mapping is entered into each direction (Chunker's `register*` methods).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Add; it is an error if the key exists.
    Insert,
    /// Replace an existing entry; it is an error if there is none.
    Override,
    /// Do not touch this direction.
    Skip,
}

/// The eight `register*` flavours: `(edition -> pivot, pivot -> edition)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    /// `register`
    Register,
    /// `registerDuplicateInput`: many pivot types write to one edition id; only pivot -> edition is added.
    DuplicateInput,
    /// `registerDuplicateOutput`: an alias accepted on read; only edition -> pivot is added.
    DuplicateOutput,
    /// `registerOverrideOutput`: add edition -> pivot, replace pivot -> edition.
    OverrideOutput,
    /// `registerOverrideInput`: replace edition -> pivot, add pivot -> edition.
    OverrideInput,
    /// `registerOverrideInputOutput`: replace both.
    OverrideInputOutput,
    /// `registerDuplicateOverrideInput`: replace edition -> pivot only.
    DuplicateOverrideInput,
    /// `registerDuplicateOverrideOutput`: replace pivot -> edition only.
    DuplicateOverrideOutput,
}

impl Flag {
    /// `(edition -> pivot, pivot -> edition)`.
    pub fn modes(self) -> (Mode, Mode) {
        use Mode::{Insert as I, Override as O, Skip as S};
        match self {
            Flag::Register => (I, I),
            Flag::DuplicateInput => (S, I),
            Flag::DuplicateOutput => (I, S),
            Flag::OverrideOutput => (I, O),
            Flag::OverrideInput => (O, I),
            Flag::OverrideInputOutput => (O, O),
            Flag::DuplicateOverrideInput => (O, S),
            Flag::DuplicateOverrideOutput => (S, O),
        }
    }
}

/// One table row (`BlockMapping`): an edition id with pinned edition states <-> a pivot type with pinned pivot states.
#[derive(Clone, Debug)]
pub struct BlockMapping {
    pub id: Arc<str>,
    pub states: BTreeMap<Arc<str>, StateValue>,
    pub ty: Arc<str>,
    pub pivot_states: BTreeMap<Arc<str>, PivotValue>,
    pub group: Option<Arc<StateGroup>>,
}

impl BlockMapping {
    pub fn of(id: &str, ty: &str) -> BlockMapping {
        BlockMapping {
            id: id.into(),
            states: BTreeMap::new(),
            ty: ty.into(),
            pivot_states: BTreeMap::new(),
            group: None,
        }
    }

    pub fn group(mut self, group: &Arc<StateGroup>) -> BlockMapping {
        self.group = Some(group.clone());
        self
    }

    /// Pin an edition state.
    pub fn when(mut self, name: &str, value: StateValue) -> BlockMapping {
        self.states.insert(name.into(), value);
        self
    }

    /// Pin a pivot state.
    pub fn with(mut self, state: &str, value: &str) -> BlockMapping {
        self.pivot_states.insert(state.into(), value.into());
        self
    }
}

type ToTable = HashMap<Arc<str>, Grouping<Arc<str>, StateValue, Arc<str>, Arc<str>, PivotValue>>;
type FromTable = HashMap<Arc<str>, Grouping<Arc<str>, PivotValue, Arc<str>, Arc<str>, StateValue>>;
type ToCache = RwLock<HashMap<Arc<str>, HashMap<StateMap, Option<PivotBlock>>>>;
type FromCache = RwLock<HashMap<Arc<str>, HashMap<BTreeMap<Arc<str>, PivotValue>, Option<(Arc<str>, StateMap)>>>>;

/// Converts blocks between one edition/version and the pivot. Build one per world side and share it between threads;
/// lookups are cached.
pub struct BlockResolver {
    pivot: Arc<PivotInfo>,
    to: ToTable,
    from: FromTable,
    extra: Option<Arc<StateGroup>>,
    to_cache: ToCache,
    from_cache: FromCache,
}

impl BlockResolver {
    /// An empty resolver for hand-built tables (see [`BlockResolver::register`]).
    pub fn empty(pivot: Arc<PivotInfo>) -> BlockResolver {
        BlockResolver {
            pivot,
            to: HashMap::new(),
            from: HashMap::new(),
            extra: None,
            to_cache: RwLock::default(),
            from_cache: RwLock::default(),
        }
    }

    /// Set the group applied to every block (`extraStateMappingGroup`); only one is allowed.
    pub fn extra_group(&mut self, group: Arc<StateGroup>) {
        assert!(self.extra.is_none(), "Only a single extra state mapping group is allowed.");
        self.extra = Some(group);
    }

    /// Enter one mapping with the given register flavour. Unlike Chunker, a failing register changes nothing.
    pub fn register(&mut self, flag: Flag, m: BlockMapping) -> Result<(), MappingError> {
        let (to_mode, from_mode) = flag.modes();
        let what = || format!("{} {:?} / {} {:?}", m.id, m.states, m.ty, m.pivot_states);
        let to_names: Vec<Arc<str>> = m.states.keys().cloned().collect();
        let to_values: Vec<StateValue> = m.states.values().cloned().collect();
        let from_names: Vec<Arc<str>> = m.pivot_states.keys().cloned().collect();
        let from_values: Vec<PivotValue> = m.pivot_states.values().cloned().collect();
        let empty_to = Grouping::default();
        let empty_from = Grouping::default();
        if to_mode != Mode::Skip {
            self.to.get(&m.id).unwrap_or(&empty_to).check(&to_names, &to_values, to_mode == Mode::Override, &what)?;
        }
        if from_mode != Mode::Skip {
            self.from.get(&m.ty).unwrap_or(&empty_from).check(
                &from_names,
                &from_values,
                from_mode == Mode::Override,
                &what,
            )?;
        }
        if to_mode != Mode::Skip {
            let output = Output { ident: m.ty.clone(), outputs: m.pivot_states.clone(), group: m.group.clone() };
            self.to.entry(m.id.clone()).or_default().insert(
                to_names.clone(),
                to_names,
                to_values,
                output,
                to_mode == Mode::Override,
                &what,
            )?;
        }
        if from_mode != Mode::Skip {
            let order: Vec<Arc<str>> = from_names.iter().map(|n| self.pivot.order_name(n)).collect();
            let output = Output { ident: m.id.clone(), outputs: m.states.clone(), group: m.group.clone() };
            self.from.entry(m.ty.clone()).or_default().insert(
                from_names,
                order,
                from_values,
                output,
                from_mode == Mode::Override,
                &what,
            )?;
        }
        Ok(())
    }

    /// `removeOutputMapping`: forget how a pivot type is written.
    pub fn remove_output_mapping(&mut self, ty: &str) {
        self.from.remove(ty);
    }

    /// Whether an edition id is known (`isSupported(String)`).
    pub fn is_supported_id(&self, id: &str) -> bool {
        self.to.contains_key(id)
    }

    /// Whether a pivot type can be written (`isSupported(ChunkerBlockType)`).
    pub fn is_supported_type(&self, ty: &str) -> bool {
        self.from.contains_key(ty)
    }

    /// Number of (edition -> pivot, pivot -> edition) rows.
    pub fn row_counts(&self) -> (usize, usize) {
        (self.to.values().map(Grouping::len).sum(), self.from.values().map(Grouping::len).sum())
    }

    /// Edition block -> pivot block, `None` if the table has no mapping (the caller logs it and uses air).
    /// Bedrock callers include the pseudo state `waterlogged`.
    pub fn to_pivot(&self, id: &str, states: &StateMap) -> Option<PivotBlock> {
        if let Some(hit) = self.to_cache.read().ok()?.get(id).and_then(|m| m.get(states)) {
            return hit.clone();
        }
        let result = self.resolve_to(id, states);
        if let Ok(mut cache) = self.to_cache.write() {
            cache.entry(id.into()).or_default().insert(states.clone(), result.clone());
        }
        result
    }

    fn resolve_to(&self, id: &str, states: &StateMap) -> Option<PivotBlock> {
        let grouping = self.to.get(id)?;
        let lookup = |name: &str| states.get(name).cloned();
        let (ty, out) = grouping.resolve(
            &|name: &Arc<str>| states.get(name).cloned(),
            &|group, outputs| group.apply_input(&lookup, outputs),
            self.extra.as_deref(),
        )?;
        Some(PivotBlock { ty: ty.clone(), states: out })
    }

    /// Pivot block -> edition id and states (`None` if the pivot type cannot be written). Bedrock results include
    /// `waterlogged`, which the caller moves out of the NBT states.
    pub fn from_pivot(&self, block: &PivotBlock) -> Option<(Arc<str>, StateMap)> {
        if let Some(hit) = self.from_cache.read().ok()?.get(&block.ty).and_then(|m| m.get(&block.states)) {
            return hit.clone();
        }
        let result = self.resolve_from(block);
        if let Ok(mut cache) = self.from_cache.write() {
            cache.entry(block.ty.clone()).or_default().insert(block.states.clone(), result.clone());
        }
        result
    }

    fn resolve_from(&self, block: &PivotBlock) -> Option<(Arc<str>, StateMap)> {
        let grouping = self.from.get(&block.ty)?;
        let lookup = |state: &str, default_allowed: bool| -> Option<PivotValue> {
            match block.states.get(state) {
                Some(v) => Some(v.clone()),
                None if default_allowed => self.pivot.default_for(&block.ty, state),
                None => None,
            }
        };
        let (id, out) = grouping.resolve(
            &|state: &Arc<str>| lookup(state, true),
            &|group, outputs| group.apply_output(&lookup, outputs),
            self.extra.as_deref(),
        )?;
        Some((id.clone(), out))
    }
}
