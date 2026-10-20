// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Block types and block identifiers (`ChunkerBlockType`, `ChunkerBlockIdentifier`, `PreservedIdentifier`).

use std::sync::{Arc, LazyLock};

use super::blockentity::BlockEntityKind;
use super::generated::{BLOCK_INFO, BlockInfo, BlockType, Bool, STATE_SETS, StateId, states};
use super::identifier::{Identifier, StateValue};
use super::state::{State, StateEnum, StateMask, StateSet};

/// A group of block types (`ChunkerVanillaBlockGroups`); the members are a bit in [`BlockInfo::groups`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockGroup(pub u16);

impl BlockGroup {
    /// Whether the block type belongs to the group.
    pub const fn contains(self, ty: BlockType) -> bool {
        BLOCK_INFO[ty as usize].groups & self.0 != 0
    }
}

impl BlockType {
    /// Static facts about the block type.
    pub const fn info(self) -> &'static BlockInfo {
        &BLOCK_INFO[self as usize]
    }

    /// Map colour (0xRRGGBB, 0 when the block has none).
    pub const fn rgb_color(self) -> u32 {
        self.info().rgb
    }

    /// Whether every face is solid.
    pub const fn is_all_faces_solid(self) -> bool {
        self.info().solid
    }

    /// The states the type supports (always includes `WATERLOGGED`).
    pub const fn states(self) -> &'static StateMask {
        &STATE_SETS[self.info().set as usize]
    }

    /// Whether the type supports the state.
    pub const fn supports(self, state: StateId) -> bool {
        self.states().contains(state)
    }

    /// The block entity the type uses, if any.
    pub const fn block_entity(self) -> Option<BlockEntityKind> {
        self.info().entity
    }

    /// Whether the type is in the group.
    pub const fn in_group(self, group: BlockGroup) -> bool {
        group.contains(self)
    }

    pub const fn is_air(self) -> bool {
        matches!(self, BlockType::AIR)
    }
}

/// A block type: a vanilla one or a custom block known only by its identifier (`ChunkerCustomBlockType`).
///
/// Custom types compare by identifier alone; the states a custom block carries are part of its
/// [`BlockIdentifier`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Vanilla(BlockType),
    Custom(Arc<str>),
}

impl BlockKind {
    pub fn rgb_color(&self) -> u32 {
        match self {
            BlockKind::Vanilla(t) => t.rgb_color(),
            BlockKind::Custom(_) => 0,
        }
    }

    pub fn is_all_faces_solid(&self) -> bool {
        match self {
            BlockKind::Vanilla(t) => t.is_all_faces_solid(),
            BlockKind::Custom(_) => false,
        }
    }

    /// Whether the type supports the vanilla state (custom types only support `WATERLOGGED`).
    pub fn supports(&self, state: StateId) -> bool {
        match self {
            BlockKind::Vanilla(t) => t.supports(state),
            BlockKind::Custom(_) => state == StateId::WATERLOGGED,
        }
    }

    pub fn block_entity(&self) -> Option<BlockEntityKind> {
        match self {
            BlockKind::Vanilla(t) => t.block_entity(),
            BlockKind::Custom(_) => None,
        }
    }

    pub fn vanilla(&self) -> Option<BlockType> {
        match self {
            BlockKind::Vanilla(t) => Some(*t),
            BlockKind::Custom(_) => None,
        }
    }
}

impl From<BlockType> for BlockKind {
    fn from(value: BlockType) -> Self {
        BlockKind::Vanilla(value)
    }
}

/// A user supplied custom mapping carried from the reader to the writer (`PreservedIdentifier`). The side that did
/// not create it applies it when writing: `from_reader == true` means the reader created it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PreservedIdentifier {
    pub from_reader: bool,
    pub identifier: Identifier,
}

/// A block: type, present states and an optional preserved identifier.
///
/// Equality and hashing cover all three, so two otherwise equal blocks with different preserved identifiers are
/// distinct palette keys. Present states are only those set explicitly; [`BlockIdentifier::get`] falls back to the
/// state default when the type supports it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BlockIdentifier {
    kind: BlockKind,
    states: StateSet,
    preserved: Option<Arc<PreservedIdentifier>>,
}

/// A shared block identifier: the key type of block palettes.
pub type BlockRef = Arc<BlockIdentifier>;

static AIR: LazyLock<BlockRef> = LazyLock::new(|| Arc::new(BlockIdentifier::new(BlockType::AIR)));

/// The shared plain air block (`ChunkerBlockIdentifier.AIR`).
pub fn air() -> &'static BlockRef {
    &AIR
}

impl BlockIdentifier {
    /// A block without explicit states.
    pub fn new(ty: BlockType) -> BlockIdentifier {
        BlockIdentifier { kind: BlockKind::Vanilla(ty), states: StateSet::new(), preserved: None }
    }

    /// A block with the given states.
    pub fn with_states(ty: BlockType, states: StateSet) -> BlockIdentifier {
        BlockIdentifier { kind: BlockKind::Vanilla(ty), states, preserved: None }
    }

    /// Every part given.
    pub fn from_parts(
        kind: BlockKind,
        states: StateSet,
        preserved: Option<Arc<PreservedIdentifier>>,
    ) -> BlockIdentifier {
        BlockIdentifier { kind, states, preserved }
    }

    /// A custom block (`ChunkerBlockIdentifier.custom`). A state named `waterlogged` becomes the vanilla
    /// `WATERLOGGED` state (true for `true`, `"true"` or `1`, false otherwise); other states stay custom.
    pub fn custom<K, I>(id: impl AsRef<str>, states: I) -> BlockIdentifier
    where
        K: AsRef<str>,
        I: IntoIterator<Item = (K, StateValue)>,
    {
        let mut set = StateSet::new();
        for (name, value) in states {
            if name.as_ref() == "waterlogged" {
                let on = matches!(&value, StateValue::Bool(true) | StateValue::Int(1))
                    || matches!(&value, StateValue::Str(s) if &**s == "true");
                set.set(states::WATERLOGGED, if on { Bool::TRUE } else { Bool::FALSE });
            } else {
                set.set_custom(name, value);
            }
        }
        BlockIdentifier { kind: BlockKind::Custom(Arc::from(id.as_ref())), states: set, preserved: None }
    }

    pub fn kind(&self) -> &BlockKind {
        &self.kind
    }

    /// The vanilla block type, if the block is not custom.
    pub fn block_type(&self) -> Option<BlockType> {
        self.kind.vanilla()
    }

    /// Whether the type is `AIR` (`isAir`).
    pub fn is_air(&self) -> bool {
        self.kind == BlockKind::Vanilla(BlockType::AIR)
    }

    /// The states that are present (`getPresentStates`).
    pub fn states(&self) -> &StateSet {
        &self.states
    }

    pub fn preserved(&self) -> Option<&Arc<PreservedIdentifier>> {
        self.preserved.as_ref()
    }

    pub fn rgb_color(&self) -> u32 {
        self.kind.rgb_color()
    }

    pub fn has_rgb_color(&self) -> bool {
        self.rgb_color() != 0
    }

    /// Whether the state is explicitly present (`containsState`).
    pub fn contains_state(&self, state: StateId) -> bool {
        self.states.contains(state)
    }

    /// The value of a state: the present value, else the default if the type supports the state
    /// (`getState(state)`).
    pub fn get<T: StateEnum>(&self, state: State<T>) -> Option<T> {
        self.get_with(state, true)
    }

    /// Only the explicitly present value (`getState(state, false)`).
    pub fn get_present<T: StateEnum>(&self, state: State<T>) -> Option<T> {
        self.get_with(state, false)
    }

    /// `getState(state, returnDefaultValue)`.
    pub fn get_with<T: StateEnum>(&self, state: State<T>, return_default: bool) -> Option<T> {
        match self.states.get(state) {
            Some(v) => Some(v),
            None if return_default && self.kind.supports(state.id()) => Some(state.default_value()),
            None => None,
        }
    }

    /// A copy with the state set (`copyWith`).
    pub fn with<T: StateEnum>(&self, state: State<T>, value: T) -> BlockIdentifier {
        let mut copy = self.clone();
        copy.states.set(state, value);
        copy
    }

    /// A copy without the state (`copyWithout`).
    pub fn without<T: StateEnum>(&self, state: State<T>) -> BlockIdentifier {
        let mut copy = self.clone();
        copy.states.remove(state.id());
        copy
    }

    /// Set a state in place.
    pub fn set<T: StateEnum>(&mut self, state: State<T>, value: T) {
        self.states.set(state, value);
    }

    /// A copy with another preserved identifier.
    pub fn with_preserved(&self, preserved: Option<Arc<PreservedIdentifier>>) -> BlockIdentifier {
        BlockIdentifier { kind: self.kind.clone(), states: self.states.clone(), preserved }
    }

    /// `[name=value,...]` of the present states, vanilla first (by id), then custom (by name); `[]` when none.
    pub fn state_string(&self) -> String {
        if self.states.is_empty() {
            return "[]".to_string();
        }
        let mut parts: Vec<String> = Vec::with_capacity(self.states.len());
        for (id, raw) in self.states.vanilla() {
            parts.push(format!("{}={}", id.state_name(), id.value_name(*raw).unwrap_or("?")));
        }
        for (name, value) in self.states.custom() {
            parts.push(format!("{name}={value}"));
        }
        format!("[{}]", parts.join(","))
    }
}

impl From<BlockType> for BlockIdentifier {
    fn from(ty: BlockType) -> Self {
        BlockIdentifier::new(ty)
    }
}
