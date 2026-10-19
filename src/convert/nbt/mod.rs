//! NBT: the tag tree Minecraft stores almost everything in.
//!
//! Derived from Chunker's `nbt` package (https://github.com/HiveGamesOSS/Chunker, Copyright (c) 2024 Hive Games, MIT
//! License). This file only fixes the type shapes the other modules are written against; the reading, writing and the
//! rest of the API are filled in by the NBT phase of PORTING.md and must keep these names and signatures.

use indexmap::IndexMap;

/// One NBT tag.
#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<Tag>),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

/// A compound: named tags that keep the order they were inserted in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compound(pub IndexMap<String, Tag>);

impl Compound {
    pub fn new() -> Compound {
        Compound(IndexMap::new())
    }
    pub fn get(&self, key: &str) -> Option<&Tag> {
        self.0.get(key)
    }
    pub fn insert(&mut self, key: impl Into<String>, tag: Tag) -> Option<Tag> {
        self.0.insert(key.into(), tag)
    }
    pub fn remove(&mut self, key: &str) -> Option<Tag> {
        self.0.shift_remove(key)
    }
    pub fn contains_key(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }
    pub fn get_compound(&self, key: &str) -> Option<&Compound> {
        match self.0.get(key) {
            Some(Tag::Compound(c)) => Some(c),
            _ => None,
        }
    }
    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.0.get(key) {
            Some(Tag::String(s)) => Some(s),
            _ => None,
        }
    }
    pub fn get_i32(&self, key: &str) -> Option<i32> {
        match self.0.get(key) {
            Some(Tag::Int(v)) => Some(*v),
            _ => None,
        }
    }
}
