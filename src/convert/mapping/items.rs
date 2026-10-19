// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The item identifier engine: a port of Chunker's `ChunkerItemIdentifierResolver` and `ItemMapping`.
//!
//! Items use the same grouping idea as blocks but have no state groups. An item mapping pins edition input states
//! (usually the Bedrock/legacy `data` value) and produces a pivot item type plus properties: `S:<STATE>` keys are pivot
//! block states (for block items), `P:<name>` keys are Chunker item properties such as `spawn_egg_mob` or `potion`.
//! Values are the Chunker enum constant names, or typed text (`i:3`) for plain numbers.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

use super::engine::{Grouping, MappingError, Mode, Output, PivotInfo, PivotValue};
use super::{StateMap, StateValue};

/// A pivot item type: either a block type (block items keep their states) or an item-only type.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ItemKind {
    Block(Arc<str>),
    Item(Arc<str>),
}

impl ItemKind {
    /// Parse the data form `B:OAK_DOOR` / `I:STICK`.
    pub fn parse(text: &str) -> Option<ItemKind> {
        match text.split_once(':')? {
            ("B", n) => Some(ItemKind::Block(n.into())),
            ("I", n) => Some(ItemKind::Item(n.into())),
            _ => None,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            ItemKind::Block(n) | ItemKind::Item(n) => n,
        }
    }
}

/// An item in the version-neutral model: its type and the properties the mapping fixes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PivotItem {
    pub kind: ItemKind,
    pub props: BTreeMap<Arc<str>, PivotValue>,
}

impl PivotItem {
    pub fn new(kind: ItemKind, props: &[(&str, &str)]) -> PivotItem {
        PivotItem { kind, props: props.iter().map(|(k, v)| (Arc::from(*k), Arc::from(*v))).collect() }
    }
}

/// One item row.
#[derive(Clone, Debug)]
pub struct ItemMapping {
    pub id: Arc<str>,
    pub states: BTreeMap<Arc<str>, StateValue>,
    pub kind: ItemKind,
    pub props: BTreeMap<Arc<str>, PivotValue>,
}

impl ItemMapping {
    pub fn of(id: &str, kind: ItemKind) -> ItemMapping {
        ItemMapping { id: id.into(), states: BTreeMap::new(), kind, props: BTreeMap::new() }
    }

    /// Pin the legacy/Bedrock data value.
    pub fn data(mut self, value: i32) -> ItemMapping {
        self.states.insert("data".into(), StateValue::Int(value));
        self
    }

    pub fn prop(mut self, key: &str, value: &str) -> ItemMapping {
        self.props.insert(key.into(), value.into());
        self
    }
}

type ToTable = HashMap<Arc<str>, Grouping<Arc<str>, StateValue, ItemKind, Arc<str>, PivotValue>>;
type FromTable = HashMap<ItemKind, Grouping<Arc<str>, PivotValue, Arc<str>, Arc<str>, StateValue>>;
type ToCache = RwLock<HashMap<Arc<str>, HashMap<StateMap, Option<PivotItem>>>>;
type FromCache = RwLock<HashMap<ItemKind, HashMap<BTreeMap<Arc<str>, PivotValue>, Option<(Arc<str>, StateMap)>>>>;

/// Converts items between one edition/version and the pivot.
pub struct ItemResolver {
    pivot: Arc<PivotInfo>,
    /// Add `data=0` to written items without one (Chunker's `defaultData`, on for the legacy Java table).
    default_data: bool,
    to: ToTable,
    from: FromTable,
    to_cache: ToCache,
    from_cache: FromCache,
}

impl ItemResolver {
    pub fn empty(pivot: Arc<PivotInfo>, default_data: bool) -> ItemResolver {
        ItemResolver {
            pivot,
            default_data,
            to: HashMap::new(),
            from: HashMap::new(),
            to_cache: RwLock::default(),
            from_cache: RwLock::default(),
        }
    }

    /// Enter one mapping with the given register flavour (same flavours as blocks).
    pub fn register(&mut self, flag: super::engine::Flag, m: ItemMapping) -> Result<(), MappingError> {
        let (to_mode, from_mode) = flag.modes();
        let what = || format!("{} {:?} / {:?} {:?}", m.id, m.states, m.kind, m.props);
        if to_mode != Mode::Skip {
            let names: Vec<Arc<str>> = m.states.keys().cloned().collect();
            let values: Vec<StateValue> = m.states.values().cloned().collect();
            let output = Output { ident: m.kind.clone(), outputs: m.props.clone(), group: None };
            self.to.entry(m.id.clone()).or_default().insert(
                names.clone(),
                names.clone(),
                values,
                output,
                to_mode == Mode::Override,
                &what,
            )?;
        }
        if from_mode != Mode::Skip {
            let names: Vec<Arc<str>> = m.props.keys().cloned().collect();
            let values: Vec<PivotValue> = m.props.values().cloned().collect();
            let output = Output { ident: m.id.clone(), outputs: m.states.clone(), group: None };
            self.from.entry(m.kind.clone()).or_default().insert(
                names.clone(),
                names,
                values,
                output,
                from_mode == Mode::Override,
                &what,
            )?;
        }
        Ok(())
    }

    /// Whether an edition item id is known (`isSupported(String)`); block items fall through to the block table.
    pub fn is_supported_id(&self, id: &str) -> bool {
        self.to.contains_key(id)
    }

    pub fn is_supported_kind(&self, kind: &ItemKind) -> bool {
        self.from.contains_key(kind)
    }

    /// Number of (edition -> pivot, pivot -> edition) rows.
    pub fn row_counts(&self) -> (usize, usize) {
        (self.to.values().map(Grouping::len).sum(), self.from.values().map(Grouping::len).sum())
    }

    /// Edition item -> pivot item; `None` when the id is not an item (the caller then tries the block table).
    pub fn to_pivot(&self, id: &str, states: &StateMap) -> Option<PivotItem> {
        if let Some(hit) = self.to_cache.read().ok()?.get(id).and_then(|m| m.get(states)) {
            return hit.clone();
        }
        let result = self.to.get(id).and_then(|g| {
            g.resolve(&|n: &Arc<str>| states.get(n).cloned(), &|_, _| {}, None)
                .map(|(kind, props)| PivotItem { kind: kind.clone(), props })
        });
        if let Ok(mut cache) = self.to_cache.write() {
            cache.entry(id.into()).or_default().insert(states.clone(), result.clone());
        }
        result
    }

    /// Pivot item -> edition id and states (`data` etc.).
    pub fn from_pivot(&self, item: &PivotItem) -> Option<(Arc<str>, StateMap)> {
        if let Some(hit) = self.from_cache.read().ok()?.get(&item.kind).and_then(|m| m.get(&item.props)) {
            return hit.clone();
        }
        let result = self.resolve_from(item);
        if let Ok(mut cache) = self.from_cache.write() {
            cache.entry(item.kind.clone()).or_default().insert(item.props.clone(), result.clone());
        }
        result
    }

    fn resolve_from(&self, item: &PivotItem) -> Option<(Arc<str>, StateMap)> {
        let grouping = self.from.get(&item.kind)?;
        let lookup = |key: &Arc<str>| -> Option<PivotValue> {
            if let Some(v) = item.props.get(key) {
                return Some(v.clone());
            }
            // Block states fall back to the type's default, item properties do not.
            let state = key.strip_prefix("S:")?;
            match &item.kind {
                ItemKind::Block(ty) => self.pivot.default_for(ty, state),
                ItemKind::Item(_) => None,
            }
        };
        let (id, mut states) = grouping.resolve(&lookup, &|_, _| {}, None)?;
        if self.default_data && !states.contains_key("data") {
            states.insert("data".into(), StateValue::Int(0));
        }
        Some((id.clone(), states))
    }
}
