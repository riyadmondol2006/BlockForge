// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Model blocks to Java block states (`JavaResolvers.writeBlock`): the identifier tables of the target version
//! applied through the pivot, with a cache, the `Name`/`Properties` compound and a count of what had no mapping.

use crate::convert::mapping::{BlockResolver, Edition, PivotBlock, StateValue as MapValue};
use crate::convert::model::{BlockIdentifier, BlockKind, BlockRef, StateEnum, StateValue, states};
use crate::convert::nbt::{Compound, Tag};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock};

/// One Java palette entry: block id and property strings (sorted by property name).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JavaBlock {
    pub name: Arc<str>,
    pub props: Vec<(Arc<str>, Arc<str>)>,
}

impl JavaBlock {
    pub fn plain(name: &str) -> JavaBlock {
        JavaBlock { name: Arc::from(name), props: Vec::new() }
    }

    pub fn is_air(&self) -> bool {
        &*self.name == "minecraft:air"
    }

    /// The palette entry compound: `{Name, Properties}`, or `{id, properties}` from 26.3.
    pub fn to_nbt(&self, renamed_fields: bool) -> Compound {
        let mut c = Compound::with_capacity(2);
        c.insert(if renamed_fields { "id" } else { "Name" }, Tag::String(self.name.to_string()));
        if !self.props.is_empty() {
            let mut props = Compound::with_capacity(self.props.len());
            for (k, v) in &self.props {
                props.insert(k.to_string(), Tag::String(v.to_string()));
            }
            c.insert(if renamed_fields { "properties" } else { "Properties" }, Tag::Compound(props));
        }
        c
    }
}

fn value_text(value: &MapValue) -> String {
    match value {
        MapValue::Bool(b) => b.to_string(),
        MapValue::Int(i) => i.to_string(),
        MapValue::Str(s) => s.to_string(),
    }
}

fn model_value_text(value: &StateValue) -> String {
    value.to_string()
}

/// Converts model blocks for one Java version; cheap to share between threads.
pub struct BlockWriter {
    resolver: BlockResolver,
    cache: RwLock<HashMap<BlockRef, Option<Arc<JavaBlock>>>>,
    custom_identifiers: bool,
    unmapped: Mutex<BTreeMap<String, u64>>,
    air: Arc<JavaBlock>,
}

impl BlockWriter {
    /// The writer for a game version (the registered release the data version floors to).
    pub fn new(version: crate::convert::util::Version, custom_identifiers: bool) -> BlockWriter {
        let v = crate::convert::mapping::Version::new(
            version.major.max(0) as u32,
            version.minor.max(0) as u32,
            version.patch.max(0) as u32,
        );
        BlockWriter {
            resolver: BlockResolver::new(Edition::Java, v),
            cache: RwLock::new(HashMap::new()),
            custom_identifiers,
            unmapped: Mutex::new(BTreeMap::new()),
            air: Arc::new(JavaBlock::plain("minecraft:air")),
        }
    }

    pub fn air(&self) -> &Arc<JavaBlock> {
        &self.air
    }

    /// The Java block for a model block; `None` when the tables have no mapping for it.
    pub fn resolve(&self, block: &BlockRef) -> Option<Arc<JavaBlock>> {
        if block.is_air() && block.preserved().is_none() {
            return Some(self.air.clone());
        }
        if let Ok(cache) = self.cache.read() {
            if let Some(hit) = cache.get(block) {
                return hit.clone();
            }
        }
        let resolved = self.resolve_uncached(block).map(Arc::new);
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(block.clone(), resolved.clone());
        }
        resolved
    }

    /// A readable name for a block with no mapping (the type constant, or the custom identifier).
    pub fn describe(block: &BlockIdentifier) -> String {
        match block.kind() {
            BlockKind::Vanilla(t) => t.name().to_string(),
            BlockKind::Custom(id) => id.to_string(),
        }
    }

    /// Count `n` blocks that were replaced by air.
    pub fn note_unmapped(&self, block: &BlockIdentifier, n: u64) {
        if let Ok(mut map) = self.unmapped.lock() {
            *map.entry(Self::describe(block)).or_insert(0) += n;
        }
    }

    /// What had no mapping so far, with the number of blocks, most frequent first.
    pub fn unmapped(&self) -> Vec<(String, u64)> {
        let mut out: Vec<(String, u64)> =
            self.unmapped.lock().map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect()).unwrap_or_default();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    fn resolve_uncached(&self, block: &BlockIdentifier) -> Option<JavaBlock> {
        // A mapping the user preserved from the reader side is applied directly.
        if let Some(preserved) = block.preserved() {
            if preserved.from_reader {
                let id = &preserved.identifier;
                let mut props: Vec<(Arc<str>, Arc<str>)> =
                    id.states.iter().map(|(k, v)| (k.clone(), Arc::from(model_value_text(v)))).collect();
                props.sort();
                return Some(JavaBlock { name: id.name.clone(), props });
            }
        }
        match block.kind() {
            BlockKind::Vanilla(ty) => {
                let mut pivot_states: BTreeMap<Arc<str>, Arc<str>> = BTreeMap::new();
                for (id, raw) in block.states().vanilla() {
                    let Some(value) = id.value_name(*raw) else { continue };
                    pivot_states.insert(Arc::from(id.name()), Arc::from(value));
                }
                let pivot = PivotBlock { ty: Arc::from(ty.name()), states: pivot_states };
                let (id, states) = self.resolver.from_pivot(&pivot)?;
                let mut props: Vec<(Arc<str>, Arc<str>)> =
                    states.iter().map(|(k, v)| (k.clone(), Arc::from(value_text(v)))).collect();
                props.sort();
                Some(JavaBlock { name: id, props })
            }
            BlockKind::Custom(id) => {
                if !self.custom_identifiers || !id.contains(':') || id.starts_with("minecraft:") {
                    return None;
                }
                let mut props: Vec<(Arc<str>, Arc<str>)> = Vec::new();
                if let Some(w) = block.states().get(states::WATERLOGGED) {
                    props.push((Arc::from("waterlogged"), Arc::from(if w.value_ordinal() == 1 { "true" } else { "false" })));
                }
                for (k, v) in block.states().custom() {
                    props.push((k.clone(), Arc::from(model_value_text(v))));
                }
                props.sort();
                Some(JavaBlock { name: id.clone(), props })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::model::{BlockType, generated::FacingDirectionHorizontal};

    fn writer() -> BlockWriter {
        BlockWriter::new(crate::convert::util::Version::new(1, 21, 4), true)
    }

    #[test]
    fn plain_and_stateful_blocks() {
        let w = writer();
        let stone: BlockRef = Arc::new(BlockIdentifier::new(BlockType::STONE));
        let b = w.resolve(&stone).unwrap();
        assert_eq!(&*b.name, "minecraft:stone");
        assert!(b.props.is_empty());
        let stairs = BlockIdentifier::new(BlockType::OAK_STAIRS).with(states::FACING_HORIZONTAL, FacingDirectionHorizontal::EAST);
        let b = w.resolve(&Arc::new(stairs)).unwrap();
        assert_eq!(&*b.name, "minecraft:oak_stairs");
        let props: Vec<_> = b.props.iter().map(|(k, v)| format!("{k}={v}")).collect();
        assert!(props.contains(&"facing=east".to_string()), "{props:?}");
        assert!(props.contains(&"half=bottom".to_string()), "{props:?}");
    }

    #[test]
    fn custom_blocks_pass_through_when_namespaced() {
        let w = writer();
        let custom: BlockRef = Arc::new(BlockIdentifier::custom("mod:thing", [("age", StateValue::Int(3))]));
        let b = w.resolve(&custom).unwrap();
        assert_eq!(&*b.name, "mod:thing");
        assert_eq!(b.props, vec![(Arc::from("age"), Arc::from("3"))]);
        let fake: BlockRef = Arc::new(BlockIdentifier::custom("stone2", []));
        assert!(w.resolve(&fake).is_none());
        let off = BlockWriter::new(crate::convert::util::Version::new(1, 21, 4), false);
        assert!(off.resolve(&custom).is_none());
    }
}
