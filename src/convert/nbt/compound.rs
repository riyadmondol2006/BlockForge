// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! [`Compound`]: the ordered string-keyed map of tags, with typed accessors.
//!
//! Every value kind has four accessors:
//! - `get_x(key) -> Option<_>`: `None` when the key is missing or holds another type;
//! - `get_x_or(key, default) -> _`: the same with a fallback;
//! - `opt_x(key) -> Result<Option<_>>`: `Ok(None)` when missing, an error when the type is wrong. This is what
//!   Chunker's `getX(name, default)` does (it throws on a type mismatch even with a default);
//! - `require_x(key) -> Result<_>`: an error when missing or of another type (Chunker's `getX(name)`).

use indexmap::IndexMap;

use super::error::NbtError;
use super::tag::{FromTag, Tag, TagType};

/// A compound: named tags that keep the order they were inserted in.
///
/// Equality ignores the order, as `Map.equals` does in Chunker. Replacing a key keeps its position; removing one keeps
/// the order of the others.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compound(pub IndexMap<String, Tag>);

macro_rules! prim_accessors {
    ($(($t:ty, $var:ident, $label:expr, $get:ident, $get_or:ident, $opt:ident, $req:ident)),* $(,)?) => {$(
        #[doc = concat!("The ", $label, " under `key`, `None` when missing or of another type.")]
        pub fn $get(&self, key: &str) -> Option<$t> {
            match self.0.get(key) {
                Some(Tag::$var(v)) => Some(*v),
                _ => None,
            }
        }
        #[doc = concat!("The ", $label, " under `key`, or `default` when missing or of another type.")]
        pub fn $get_or(&self, key: &str, default: $t) -> $t {
            self.$get(key).unwrap_or(default)
        }
        #[doc = concat!("The ", $label, " under `key`: `Ok(None)` when missing, an error when of another type.")]
        pub fn $opt(&self, key: &str) -> Result<Option<$t>, NbtError> {
            match self.0.get(key) {
                None => Ok(None),
                Some(Tag::$var(v)) => Ok(Some(*v)),
                Some(other) => Err(wrong(key, $label, other)),
            }
        }
        #[doc = concat!("The ", $label, " under `key`; an error when missing or of another type.")]
        pub fn $req(&self, key: &str) -> Result<$t, NbtError> {
            self.$opt(key)?.ok_or_else(|| NbtError::Missing(key.to_owned()))
        }
    )*};
}

macro_rules! ref_accessors {
    ($(($t:ty, $var:ident, $label:expr, $get:ident, $get_or:ident, $opt:ident, $req:ident, $conv:expr)),* $(,)?) => {$(
        #[doc = concat!("The ", $label, " under `key`, `None` when missing or of another type.")]
        pub fn $get(&self, key: &str) -> Option<&$t> {
            match self.0.get(key) {
                Some(Tag::$var(v)) => Some($conv(v)),
                _ => None,
            }
        }
        #[doc = concat!("The ", $label, " under `key`, or `default` when missing or of another type.")]
        pub fn $get_or<'a>(&'a self, key: &str, default: &'a $t) -> &'a $t {
            self.$get(key).unwrap_or(default)
        }
        #[doc = concat!("The ", $label, " under `key`: `Ok(None)` when missing, an error when of another type.")]
        pub fn $opt(&self, key: &str) -> Result<Option<&$t>, NbtError> {
            match self.0.get(key) {
                None => Ok(None),
                Some(Tag::$var(v)) => Ok(Some($conv(v))),
                Some(other) => Err(wrong(key, $label, other)),
            }
        }
        #[doc = concat!("The ", $label, " under `key`; an error when missing or of another type.")]
        pub fn $req(&self, key: &str) -> Result<&$t, NbtError> {
            self.$opt(key)?.ok_or_else(|| NbtError::Missing(key.to_owned()))
        }
    )*};
}

fn wrong(key: &str, expected: &'static str, found: &Tag) -> NbtError {
    NbtError::WrongType { key: key.to_owned(), expected, found: found.tag_type() }
}

fn id<T: ?Sized>(v: &T) -> &T {
    v
}

fn as_slice<T>(v: &Vec<T>) -> &[T] {
    v
}

impl Compound {
    pub fn new() -> Compound {
        Compound(IndexMap::new())
    }

    /// An empty compound with room for `capacity` entries.
    pub fn with_capacity(capacity: usize) -> Compound {
        Compound(IndexMap::with_capacity(capacity))
    }

    pub fn get(&self, key: &str) -> Option<&Tag> {
        self.0.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Tag> {
        self.0.get_mut(key)
    }

    pub fn insert(&mut self, key: impl Into<String>, tag: Tag) -> Option<Tag> {
        self.0.insert(key.into(), tag)
    }

    /// Insert any value that converts to a tag (`1i32`, `"text"`, `true`, a [`Compound`], ...); returns the old tag.
    pub fn put(&mut self, key: impl Into<String>, value: impl Into<Tag>) -> Option<Tag> {
        self.0.insert(key.into(), value.into())
    }

    /// Builder form of [`Compound::put`].
    pub fn with(mut self, key: impl Into<String>, value: impl Into<Tag>) -> Compound {
        self.put(key, value);
        self
    }

    /// Remove a key, keeping the order of the others.
    pub fn remove(&mut self, key: &str) -> Option<Tag> {
        self.0.shift_remove(key)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn iter(&self) -> indexmap::map::Iter<'_, String, Tag> {
        self.0.iter()
    }

    pub fn iter_mut(&mut self) -> indexmap::map::IterMut<'_, String, Tag> {
        self.0.iter_mut()
    }

    pub fn keys(&self) -> indexmap::map::Keys<'_, String, Tag> {
        self.0.keys()
    }

    pub fn values(&self) -> indexmap::map::Values<'_, String, Tag> {
        self.0.values()
    }

    prim_accessors!(
        (i8, Byte, "byte", get_i8, get_i8_or, opt_i8, require_i8),
        (i16, Short, "short", get_i16, get_i16_or, opt_i16, require_i16),
        (i32, Int, "int", get_i32, get_i32_or, opt_i32, require_i32),
        (i64, Long, "long", get_i64, get_i64_or, opt_i64, require_i64),
        (f32, Float, "float", get_f32, get_f32_or, opt_f32, require_f32),
        (f64, Double, "double", get_f64, get_f64_or, opt_f64, require_f64),
    );

    ref_accessors!(
        (str, String, "string", get_str, get_str_or, opt_str, require_str, String::as_str),
        (Compound, Compound, "compound", get_compound, get_compound_or, opt_compound, require_compound, id),
        (
            [i8],
            ByteArray,
            "byte array",
            get_byte_array,
            get_byte_array_or,
            opt_byte_array,
            require_byte_array,
            as_slice
        ),
        ([i32], IntArray, "int array", get_int_array, get_int_array_or, opt_int_array, require_int_array, as_slice),
        (
            [i64],
            LongArray,
            "long array",
            get_long_array,
            get_long_array_or,
            opt_long_array,
            require_long_array,
            as_slice
        ),
    );

    /// A byte under `key` as a boolean (non-zero is true), `None` when missing or not a byte.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get_i8(key).map(|v| v != 0)
    }

    /// A boolean byte, or `default` when missing or not a byte.
    pub fn get_bool_or(&self, key: &str, default: bool) -> bool {
        self.get_bool(key).unwrap_or(default)
    }

    /// A boolean byte: `Ok(None)` when missing, an error when of another type.
    pub fn opt_bool(&self, key: &str) -> Result<Option<bool>, NbtError> {
        Ok(self.opt_i8(key)?.map(|v| v != 0))
    }

    /// A boolean byte; an error when missing or of another type.
    pub fn require_bool(&self, key: &str) -> Result<bool, NbtError> {
        Ok(self.require_i8(key)? != 0)
    }

    /// Any integral tag (byte, short, int, long) under `key` widened to `i64`.
    pub fn get_integer(&self, key: &str) -> Option<i64> {
        self.0.get(key).and_then(Tag::as_integer)
    }

    /// Any numeric tag under `key` as `f64`.
    pub fn get_number(&self, key: &str) -> Option<f64> {
        self.0.get(key).and_then(Tag::as_number)
    }

    pub fn get_compound_mut(&mut self, key: &str) -> Option<&mut Compound> {
        match self.0.get_mut(key) {
            Some(Tag::Compound(c)) => Some(c),
            _ => None,
        }
    }

    /// The compound under `key`, created (replacing a tag of another type) when it is not there.
    pub fn get_or_create_compound(&mut self, key: &str) -> &mut Compound {
        if !matches!(self.0.get(key), Some(Tag::Compound(_))) {
            self.0.insert(key.to_owned(), Tag::Compound(Compound::new()));
        }
        match self.0.get_mut(key) {
            Some(Tag::Compound(c)) => c,
            _ => unreachable!("compound was just inserted"),
        }
    }

    /// The list under `key`, `None` when missing or not a list.
    pub fn get_list(&self, key: &str) -> Option<&[Tag]> {
        match self.0.get(key) {
            Some(Tag::List(v)) => Some(v),
            _ => None,
        }
    }

    pub fn get_list_mut(&mut self, key: &str) -> Option<&mut Vec<Tag>> {
        match self.0.get_mut(key) {
            Some(Tag::List(v)) => Some(v),
            _ => None,
        }
    }

    /// The list under `key` checked against the element type `ty` as Chunker's `getList` does: `Ok(None)` when the
    /// key is missing (or an empty long array, which some data uses for an absent list), an error when it is another
    /// tag, or a list of another element type. Empty lists match any type.
    pub fn opt_list_of(&self, key: &str, ty: TagType) -> Result<Option<&[Tag]>, NbtError> {
        match self.0.get(key) {
            None => Ok(None),
            Some(Tag::LongArray(a)) if a.is_empty() => Ok(None),
            Some(Tag::List(items)) => {
                match items.first() {
                    Some(first) if first.tag_type() != ty => {
                        return Err(NbtError::WrongType {
                            key: key.to_owned(),
                            expected: list_label(ty),
                            found: first.tag_type(),
                        });
                    }
                    _ => {}
                }
                Ok(Some(items))
            }
            Some(other) => Err(wrong(key, "list", other)),
        }
    }

    /// Like [`Compound::opt_list_of`], but a missing list is an error.
    pub fn require_list_of(&self, key: &str, ty: TagType) -> Result<&[Tag], NbtError> {
        self.opt_list_of(key, ty)?.ok_or_else(|| NbtError::Missing(key.to_owned()))
    }

    /// The values of a list of one element type; an error when missing or of another type.
    pub fn require_list_values<V: FromTag>(&self, key: &str) -> Result<Vec<V>, NbtError> {
        Ok(self.require_list_of(key, V::TYPE)?.iter().filter_map(V::from_tag).collect())
    }

    /// The values of a list of one element type, `default` when missing; an error when of another type.
    pub fn opt_list_values<V: FromTag>(&self, key: &str) -> Result<Option<Vec<V>>, NbtError> {
        Ok(self.opt_list_of(key, V::TYPE)?.map(|l| l.iter().filter_map(V::from_tag).collect()))
    }

    /// Insert every entry of another compound (later keys replace earlier ones).
    pub fn extend(&mut self, other: Compound) {
        self.0.extend(other.0);
    }
}

fn list_label(ty: TagType) -> &'static str {
    match ty {
        TagType::End => "empty list",
        TagType::Byte => "list of byte",
        TagType::Short => "list of short",
        TagType::Int => "list of int",
        TagType::Long => "list of long",
        TagType::Float => "list of float",
        TagType::Double => "list of double",
        TagType::ByteArray => "list of byte array",
        TagType::String => "list of string",
        TagType::List => "list of list",
        TagType::Compound => "list of compound",
        TagType::IntArray => "list of int array",
        TagType::LongArray => "list of long array",
    }
}

impl IntoIterator for Compound {
    type Item = (String, Tag);
    type IntoIter = indexmap::map::IntoIter<String, Tag>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Compound {
    type Item = (&'a String, &'a Tag);
    type IntoIter = indexmap::map::Iter<'a, String, Tag>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<K: Into<String>, V: Into<Tag>> FromIterator<(K, V)> for Compound {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Compound(iter.into_iter().map(|(k, v)| (k.into(), v.into())).collect())
    }
}

/// Build a [`Compound`]: `compound! { "a" => 1i32, "b" => "text" }`. Values are anything that converts to a tag.
#[allow(unused_macros)]
macro_rules! compound {
    () => { $crate::convert::nbt::Compound::new() };
    ($($key:expr => $value:expr),+ $(,)?) => {{
        let mut c = $crate::convert::nbt::Compound::new();
        $( c.put($key, $value); )+
        c
    }};
}
#[allow(unused_imports)]
pub(crate) use compound;
