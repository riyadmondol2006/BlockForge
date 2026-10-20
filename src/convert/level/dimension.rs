// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Dimensions and the registry that maps identifiers and ids to them (`Dimension`, `DimensionRegistry`).
//!
//! Deviation from Chunker: Java compares dimensions by reference; here a [`Dimension`] is a cheap `Arc` handle with
//! value equality (all fields). That is equivalent because the registry never holds two dimensions with the same
//! identifier. The fallback biome is the vanilla biome identifier (`minecraft:plains`) until the model's biome enum
//! lands.

use std::sync::{Arc, LazyLock};

use indexmap::IndexMap;

use crate::convert::nbt::Tag;

/// The id the first custom Bedrock dimension gets.
pub const BEDROCK_CUSTOM_DIMENSION_ID_START: i32 = 1000;

#[derive(Debug, PartialEq, Eq, Hash)]
struct DimensionData {
    java_id: Option<i32>,
    bedrock_id: i32,
    identifier: String,
    fallback_biome: String,
    biome_height: i32,
}

/// A dimension: ids in both editions, its identifier, the biome used when none maps and the biome section count.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Dimension(Arc<DimensionData>);

static OVERWORLD: LazyLock<Dimension> =
    LazyLock::new(|| Dimension::new(Some(0), 0, "minecraft:overworld", "minecraft:plains", 24));
static NETHER: LazyLock<Dimension> =
    LazyLock::new(|| Dimension::new(Some(-1), 1, "minecraft:the_nether", "minecraft:nether_wastes", 8));
static THE_END: LazyLock<Dimension> =
    LazyLock::new(|| Dimension::new(Some(1), 2, "minecraft:the_end", "minecraft:the_end", 16));

impl Dimension {
    pub fn new(
        java_id: Option<i32>,
        bedrock_id: i32,
        identifier: impl Into<String>,
        fallback_biome: impl Into<String>,
        biome_height: i32,
    ) -> Dimension {
        Dimension(Arc::new(DimensionData {
            java_id,
            bedrock_id,
            identifier: identifier.into(),
            fallback_biome: fallback_biome.into(),
            biome_height,
        }))
    }

    pub fn overworld() -> Dimension {
        OVERWORLD.clone()
    }
    pub fn nether() -> Dimension {
        NETHER.clone()
    }
    pub fn the_end() -> Dimension {
        THE_END.clone()
    }

    /// The legacy Java id (`None` for custom dimensions).
    pub fn java_id(&self) -> Option<i32> {
        self.0.java_id
    }
    pub fn bedrock_id(&self) -> i32 {
        self.0.bedrock_id
    }
    pub fn identifier(&self) -> &str {
        &self.0.identifier
    }
    /// The vanilla biome identifier used when a biome cannot be mapped.
    pub fn fallback_biome(&self) -> &str {
        &self.0.fallback_biome
    }
    /// The number of 16-block biome sections.
    pub fn biome_height(&self) -> i32 {
        self.0.biome_height
    }

    /// True for the overworld, the nether and the end.
    pub fn is_vanilla(&self) -> bool {
        *self == *OVERWORLD || *self == *NETHER || *self == *THE_END
    }

    fn with_bedrock_id(&self, bedrock_id: i32) -> Dimension {
        Dimension::new(None, bedrock_id, self.identifier(), self.fallback_biome(), self.biome_height())
    }
}

/// All known dimensions, by identifier, Java id and Bedrock id.
#[derive(Clone, Debug)]
pub struct DimensionRegistry {
    by_identifier: IndexMap<String, Dimension>,
    by_java_id: IndexMap<i32, Dimension>,
    by_bedrock_id: IndexMap<i32, Dimension>,
}

impl Default for DimensionRegistry {
    fn default() -> Self {
        DimensionRegistry::new()
    }
}

impl DimensionRegistry {
    /// A registry with the three vanilla dimensions.
    pub fn new() -> DimensionRegistry {
        let mut registry = DimensionRegistry {
            by_identifier: IndexMap::new(),
            by_java_id: IndexMap::new(),
            by_bedrock_id: IndexMap::new(),
        };
        for dimension in [Dimension::overworld(), Dimension::nether(), Dimension::the_end()] {
            registry.register(dimension.identifier().to_owned(), dimension);
        }
        registry
    }

    /// All dimensions, in registration order.
    pub fn dimensions(&self) -> impl Iterator<Item = &Dimension> {
        self.by_identifier.values()
    }

    /// Registers a dimension, replacing one with the same identifier.
    pub fn register(&mut self, identifier: String, dimension: Dimension) {
        if let Some(replaced) = self.by_identifier.insert(identifier, dimension.clone()) {
            if self.by_bedrock_id.get(&replaced.bedrock_id()) == Some(&replaced) {
                self.by_bedrock_id.shift_remove(&replaced.bedrock_id());
            }
            if let Some(java_id) = replaced.java_id() {
                if self.by_java_id.get(&java_id) == Some(&replaced) {
                    self.by_java_id.shift_remove(&java_id);
                }
            }
        }
        self.by_bedrock_id.insert(dimension.bedrock_id(), dimension.clone());
        if let Some(java_id) = dimension.java_id() {
            self.by_java_id.insert(java_id, dimension);
        }
    }

    /// Creates (without registering) a custom dimension: plains fallback, 24 biome sections, no Java id.
    pub fn create_custom(&self, identifier: &str, bedrock_id: Option<i32>) -> Dimension {
        let id = bedrock_id.unwrap_or_else(|| self.next_custom_bedrock_id());
        Dimension::new(None, id, identifier, "minecraft:plains", 24)
    }

    /// The first free Bedrock id from [`BEDROCK_CUSTOM_DIMENSION_ID_START`].
    pub fn next_custom_bedrock_id(&self) -> i32 {
        let mut id = BEDROCK_CUSTOM_DIMENSION_ID_START;
        while self.by_bedrock_id.contains_key(&id) {
            id += 1;
        }
        id
    }

    pub fn by_identifier(&self, identifier: &str) -> Option<&Dimension> {
        self.by_identifier.get(identifier)
    }

    pub fn from_java(&self, id: i32, fallback: &Dimension) -> Dimension {
        self.by_java_id.get(&id).unwrap_or(fallback).clone()
    }

    pub fn from_bedrock(&self, id: i32, fallback: &Dimension) -> Dimension {
        self.by_bedrock_id.get(&id).unwrap_or(fallback).clone()
    }

    /// A Java `Dimension` tag: an identifier string, or a legacy byte/int id.
    pub fn from_java_nbt(&self, tag: Option<&Tag>, fallback: &Dimension) -> Dimension {
        match tag {
            Some(Tag::String(id)) => self.by_identifier.get(&id.to_lowercase()).unwrap_or(fallback).clone(),
            Some(Tag::Byte(id)) => self.from_java(*id as i32, fallback),
            Some(Tag::Int(id)) => self.from_java(*id as i8 as i32, fallback),
            _ => fallback.clone(),
        }
    }

    /// A Bedrock dimension tag: an identifier string, or a byte/int id.
    pub fn from_bedrock_nbt(&self, tag: Option<&Tag>, fallback: &Dimension) -> Dimension {
        match tag {
            Some(Tag::String(id)) => self.by_identifier.get(&id.to_lowercase()).unwrap_or(fallback).clone(),
            Some(Tag::Byte(id)) => self.from_bedrock(*id as i32, fallback),
            Some(Tag::Int(id)) => self.from_bedrock(*id, fallback),
            _ => fallback.clone(),
        }
    }

    /// Registers a dimension a world declares, resolving id conflicts: vanilla ids and dimensions are never rebound;
    /// a custom dimension already using the id gets a new free one. Returns the final dimension.
    pub fn register_from_world(&mut self, identifier: &str, bedrock_id: Option<i32>) -> Dimension {
        if let Some(declared) = self.by_identifier.get(identifier).cloned() {
            if declared.is_vanilla() {
                return declared;
            }
            let Some(wanted) = bedrock_id else { return declared };
            if declared.bedrock_id() == wanted {
                return declared;
            }
            if let Some(holder) = self.by_bedrock_id.get(&wanted).cloned() {
                if holder.is_vanilla() {
                    return holder;
                }
                let moved = holder.with_bedrock_id(self.next_custom_bedrock_id());
                self.register(holder.identifier().to_owned(), moved);
            }
            let dimension = declared.with_bedrock_id(wanted);
            self.register(identifier.to_owned(), dimension.clone());
            return dimension;
        }
        if let Some(wanted) = bedrock_id {
            if let Some(holder) = self.by_bedrock_id.get(&wanted).cloned() {
                if holder.is_vanilla() {
                    return holder;
                }
                let moved = holder.with_bedrock_id(self.next_custom_bedrock_id());
                self.register(holder.identifier().to_owned(), moved);
            }
        }
        let dimension = self.create_custom(identifier, bedrock_id);
        self.register(identifier.to_owned(), dimension.clone());
        dimension
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_dimensions() {
        let registry = DimensionRegistry::new();
        assert_eq!(registry.dimensions().count(), 3);
        let nether = registry.by_identifier("minecraft:the_nether").unwrap();
        assert_eq!((nether.java_id(), nether.bedrock_id(), nether.biome_height()), (Some(-1), 1, 8));
        assert!(nether.is_vanilla());
        assert_eq!(Dimension::the_end().fallback_biome(), "minecraft:the_end");
        assert_eq!(registry.from_java(1, &Dimension::overworld()), Dimension::the_end());
        assert_eq!(registry.from_bedrock(7, &Dimension::overworld()), Dimension::overworld());
    }

    #[test]
    fn nbt_lookup() {
        let registry = DimensionRegistry::new();
        let fb = Dimension::overworld();
        let nether = Dimension::nether();
        assert_eq!(registry.from_java_nbt(Some(&Tag::String("Minecraft:The_Nether".into())), &fb), nether);
        assert_eq!(registry.from_java_nbt(Some(&Tag::Int(-1)), &fb), nether);
        assert_eq!(registry.from_java_nbt(Some(&Tag::Byte(-1)), &fb), nether);
        assert_eq!(registry.from_java_nbt(Some(&Tag::Float(1.0)), &nether), nether);
        assert_eq!(registry.from_java_nbt(None, &nether), nether);
        assert_eq!(registry.from_bedrock_nbt(Some(&Tag::Int(2)), &fb), Dimension::the_end());
        assert_eq!(registry.from_bedrock_nbt(Some(&Tag::Byte(1)), &fb), nether);
        assert_eq!(registry.from_bedrock_nbt(Some(&Tag::Int(99)), &fb), fb);
    }

    #[test]
    fn custom_dimensions_get_free_ids() {
        let mut registry = DimensionRegistry::new();
        let a = registry.register_from_world("mod:a", None);
        assert_eq!((a.bedrock_id(), a.java_id()), (1000, None));
        assert_eq!(a.biome_height(), 24);
        let b = registry.register_from_world("mod:b", None);
        assert_eq!(b.bedrock_id(), 1001);
        // Same identifier again: unchanged.
        assert_eq!(registry.register_from_world("mod:a", None), a);
        assert_eq!(registry.next_custom_bedrock_id(), 1002);
    }

    #[test]
    fn id_conflicts_move_the_other_custom_dimension() {
        let mut registry = DimensionRegistry::new();
        let a = registry.register_from_world("mod:a", None); // 1000
        let b = registry.register_from_world("mod:b", Some(1000)); // takes a's id
        assert_eq!(b.bedrock_id(), 1000);
        let a_now = registry.by_identifier("mod:a").unwrap();
        assert_ne!(a_now.bedrock_id(), 1000);
        assert_eq!(a_now.bedrock_id(), 1001);
        assert_ne!(*a_now, a);
        assert_eq!(registry.from_bedrock(1000, &Dimension::overworld()), b);
        assert_eq!(registry.from_bedrock(1001, &Dimension::overworld()), *a_now);
        // Re-declaring an existing dimension with another id rebinds it.
        let a2 = registry.register_from_world("mod:a", Some(1005));
        assert_eq!(a2.bedrock_id(), 1005);
        assert_eq!(registry.from_bedrock(1001, &Dimension::overworld()), Dimension::overworld());
    }

    #[test]
    fn vanilla_is_never_rebound() {
        let mut registry = DimensionRegistry::new();
        assert_eq!(registry.register_from_world("minecraft:overworld", Some(5)), Dimension::overworld());
        // A custom dimension asking for the nether's id gets the nether.
        assert_eq!(registry.register_from_world("mod:x", Some(1)), Dimension::nether());
        assert!(registry.by_identifier("mod:x").is_none());
        // Explicit id for a new custom dimension.
        let c = registry.register_from_world("mod:c", Some(2000));
        assert_eq!(c.bedrock_id(), 2000);
        assert!(!c.is_vanilla());
    }

    #[test]
    fn register_replaces_by_identifier() {
        let mut registry = DimensionRegistry::new();
        let custom = Dimension::new(Some(5), 50, "mod:j", "minecraft:plains", 24);
        registry.register("mod:j".into(), custom.clone());
        assert_eq!(registry.from_java(5, &Dimension::overworld()), custom);
        let moved = Dimension::new(Some(6), 51, "mod:j", "minecraft:plains", 24);
        registry.register("mod:j".into(), moved.clone());
        assert_eq!(registry.from_java(5, &Dimension::overworld()), Dimension::overworld());
        assert_eq!(registry.from_bedrock(50, &Dimension::overworld()), Dimension::overworld());
        assert_eq!(registry.from_bedrock(51, &Dimension::overworld()), moved);
    }
}
