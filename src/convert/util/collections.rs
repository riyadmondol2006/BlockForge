// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! A two-way map (Chunker's `InvertibleMap`) and the collection ordering used for sorted palettes.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::Hash;

/// A map that also answers value to key. `put` keeps both directions in step.
#[derive(Clone, Debug)]
pub struct InvertibleMap<K, V> {
    pub forward: HashMap<K, V>,
    pub inverse: HashMap<V, K>,
}

impl<K, V> Default for InvertibleMap<K, V> {
    fn default() -> Self {
        InvertibleMap { forward: HashMap::new(), inverse: HashMap::new() }
    }
}

impl<K: Hash + Eq + Clone, V: Hash + Eq + Clone> InvertibleMap<K, V> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a pair in both directions. Returns the previous value of `key` or, when there was none, the previous
    /// key of `value`, as Chunker's `put` does.
    pub fn put(&mut self, key: K, value: V) -> Option<PutReplaced<K, V>> {
        let old_forward = self.forward.insert(key.clone(), value.clone());
        let old_inverse = self.inverse.insert(value, key);
        match (old_forward, old_inverse) {
            (Some(v), _) => Some(PutReplaced::Value(v)),
            (None, Some(k)) => Some(PutReplaced::Key(k)),
            (None, None) => None,
        }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.forward.get(key)
    }

    pub fn get_inverse(&self, value: &V) -> Option<&K> {
        self.inverse.get(value)
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.forward.contains_key(key)
    }

    pub fn contains_value(&self, value: &V) -> bool {
        self.inverse.contains_key(value)
    }

    pub fn len(&self) -> usize {
        self.forward.len()
    }

    pub fn is_empty(&self) -> bool {
        self.forward.is_empty()
    }
}

/// What [`InvertibleMap::put`] replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PutReplaced<K, V> {
    /// The key already had this value.
    Value(V),
    /// The value already belonged to this key.
    Key(K),
}

/// Chunker's `CollectionComparator`: shorter collections first, then element by element.
pub fn compare_collections<T: Ord>(a: &[T], b: &[T]) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.iter().cmp(b.iter()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_updates_both_directions() {
        let mut m = InvertibleMap::new();
        assert_eq!(m.put("a", 1), None);
        assert_eq!(m.put("a", 2), Some(PutReplaced::Value(1)));
        assert_eq!(m.put("b", 2), Some(PutReplaced::Key("a")));
        assert_eq!(m.get(&"a"), Some(&2));
        assert_eq!(m.get_inverse(&2), Some(&"b"));
        assert!(m.contains_value(&1)); // the stale inverse entry stays, as in Chunker
    }

    #[test]
    fn collections_order_by_size_first() {
        assert_eq!(compare_collections(&[9], &[1, 2]), Ordering::Less);
        assert_eq!(compare_collections(&[1, 3], &[1, 2]), Ordering::Greater);
        assert_eq!(compare_collections(&[1, 2], &[1, 2]), Ordering::Equal);
    }
}
