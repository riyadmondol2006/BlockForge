// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The tag tree: [`Tag`] and its type ids.

use super::compound::Compound;
use super::error::NbtError;

/// The 13 NBT tag types with the ids they have on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum TagType {
    End = 0,
    Byte = 1,
    Short = 2,
    Int = 3,
    Long = 4,
    Float = 5,
    Double = 6,
    ByteArray = 7,
    String = 8,
    List = 9,
    Compound = 10,
    IntArray = 11,
    LongArray = 12,
}

impl TagType {
    /// All types in id order.
    pub const ALL: [TagType; 13] = [
        TagType::End,
        TagType::Byte,
        TagType::Short,
        TagType::Int,
        TagType::Long,
        TagType::Float,
        TagType::Double,
        TagType::ByteArray,
        TagType::String,
        TagType::List,
        TagType::Compound,
        TagType::IntArray,
        TagType::LongArray,
    ];

    /// The wire id.
    pub fn id(self) -> u8 {
        self as u8
    }

    /// The type for a wire id, `None` above 12.
    pub fn from_id(id: u8) -> Option<TagType> {
        TagType::ALL.get(id as usize).copied()
    }

    /// A lower-case name for messages.
    pub fn name(self) -> &'static str {
        match self {
            TagType::End => "end",
            TagType::Byte => "byte",
            TagType::Short => "short",
            TagType::Int => "int",
            TagType::Long => "long",
            TagType::Float => "float",
            TagType::Double => "double",
            TagType::ByteArray => "byte array",
            TagType::String => "string",
            TagType::List => "list",
            TagType::Compound => "compound",
            TagType::IntArray => "int array",
            TagType::LongArray => "long array",
        }
    }
}

/// One NBT tag.
///
/// A list is homogeneous: every element has the type of the first one (an empty list is written with the END element
/// type, which both editions read back as an empty list of any type). [`Tag::try_list`] checks this when building;
/// the writer reports [`NbtError::MixedList`] otherwise.
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

macro_rules! as_prim {
    ($(#[$m:meta])* $name:ident, $var:ident, $t:ty) => {
        $(#[$m])*
        pub fn $name(&self) -> Option<$t> {
            match self {
                Tag::$var(v) => Some(*v),
                _ => None,
            }
        }
    };
}

macro_rules! as_ref {
    ($(#[$m:meta])* $name:ident, $name_mut:ident, $var:ident, $t:ty) => {
        $(#[$m])*
        pub fn $name(&self) -> Option<&$t> {
            match self {
                Tag::$var(v) => Some(v),
                _ => None,
            }
        }
        /// The same, mutable.
        pub fn $name_mut(&mut self) -> Option<&mut $t> {
            match self {
                Tag::$var(v) => Some(v),
                _ => None,
            }
        }
    };
}

impl Tag {
    /// The type of this tag.
    pub fn tag_type(&self) -> TagType {
        match self {
            Tag::Byte(_) => TagType::Byte,
            Tag::Short(_) => TagType::Short,
            Tag::Int(_) => TagType::Int,
            Tag::Long(_) => TagType::Long,
            Tag::Float(_) => TagType::Float,
            Tag::Double(_) => TagType::Double,
            Tag::ByteArray(_) => TagType::ByteArray,
            Tag::String(_) => TagType::String,
            Tag::List(_) => TagType::List,
            Tag::Compound(_) => TagType::Compound,
            Tag::IntArray(_) => TagType::IntArray,
            Tag::LongArray(_) => TagType::LongArray,
        }
    }

    /// The wire id of this tag's type.
    pub fn type_id(&self) -> u8 {
        self.tag_type().id()
    }

    as_prim!(
        /// The value of a byte tag.
        as_i8, Byte, i8
    );
    as_prim!(
        /// The value of a short tag.
        as_i16, Short, i16
    );
    as_prim!(
        /// The value of an int tag.
        as_i32, Int, i32
    );
    as_prim!(
        /// The value of a long tag.
        as_i64, Long, i64
    );
    as_prim!(
        /// The value of a float tag.
        as_f32, Float, f32
    );
    as_prim!(
        /// The value of a double tag.
        as_f64, Double, f64
    );

    as_ref!(
        /// The contents of a string tag.
        as_string, as_string_mut, String, String
    );
    as_ref!(
        /// The contents of a byte array tag.
        as_byte_array, as_byte_array_mut, ByteArray, Vec<i8>
    );
    as_ref!(
        /// The contents of an int array tag.
        as_int_array, as_int_array_mut, IntArray, Vec<i32>
    );
    as_ref!(
        /// The contents of a long array tag.
        as_long_array, as_long_array_mut, LongArray, Vec<i64>
    );
    as_ref!(
        /// The elements of a list tag.
        as_list, as_list_mut, List, Vec<Tag>
    );
    as_ref!(
        /// The compound inside a compound tag.
        as_compound, as_compound_mut, Compound, Compound
    );

    /// The contents of a string tag as `&str`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }

    /// Any integral tag (byte, short, int, long) widened to `i64`.
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Tag::Byte(v) => Some(*v as i64),
            Tag::Short(v) => Some(*v as i64),
            Tag::Int(v) => Some(*v as i64),
            Tag::Long(v) => Some(*v),
            _ => None,
        }
    }

    /// Any numeric tag as `f64`.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Tag::Float(v) => Some(*v as f64),
            Tag::Double(v) => Some(*v),
            other => other.as_integer().map(|v| v as f64),
        }
    }

    /// A byte tag read as a boolean (non-zero is true), as Minecraft stores them.
    pub fn as_bool(&self) -> Option<bool> {
        self.as_i8().map(|v| v != 0)
    }

    /// An empty list.
    pub fn empty_list() -> Tag {
        Tag::List(Vec::new())
    }

    /// A list of the given values, each converted to a tag. The elements are not checked, see [`Tag::try_list`].
    pub fn list<T: Into<Tag>>(items: impl IntoIterator<Item = T>) -> Tag {
        Tag::List(items.into_iter().map(Into::into).collect())
    }

    /// A list that is checked to have a single element type.
    pub fn try_list(items: Vec<Tag>) -> Result<Tag, NbtError> {
        check_homogeneous(&items)?;
        Ok(Tag::List(items))
    }

    /// The element type of a list tag (END when it is empty), `None` for other tags.
    pub fn list_type(&self) -> Option<TagType> {
        match self {
            Tag::List(items) => Some(items.first().map_or(TagType::End, Tag::tag_type)),
            _ => None,
        }
    }

    /// Whether this tag and everything inside it can be written (lists are homogeneous).
    pub fn validate(&self) -> Result<(), NbtError> {
        match self {
            Tag::List(items) => {
                check_homogeneous(items)?;
                items.iter().try_for_each(Tag::validate)
            }
            Tag::Compound(c) => c.0.values().try_for_each(Tag::validate),
            _ => Ok(()),
        }
    }
}

/// Fail when the list elements have different types.
pub(crate) fn check_homogeneous(items: &[Tag]) -> Result<(), NbtError> {
    if let Some(first) = items.first() {
        let expected = first.tag_type();
        if let Some(bad) = items.iter().find(|t| t.tag_type() != expected) {
            return Err(NbtError::MixedList { expected, found: bad.tag_type() });
        }
    }
    Ok(())
}

macro_rules! from_tag {
    ($($t:ty => $var:ident),* $(,)?) => {$(
        impl From<$t> for Tag {
            fn from(v: $t) -> Tag {
                Tag::$var(v.into())
            }
        }
    )*};
}

from_tag!(i8 => Byte, i16 => Short, i32 => Int, i64 => Long, f32 => Float, f64 => Double,
    String => String, &str => String, &String => String, Vec<i8> => ByteArray, Vec<i32> => IntArray,
    Vec<i64> => LongArray, Compound => Compound, Vec<Tag> => List);

impl From<bool> for Tag {
    /// Booleans are stored as a byte, 1 or 0.
    fn from(v: bool) -> Tag {
        Tag::Byte(v as i8)
    }
}

impl From<u8> for Tag {
    /// A byte, reinterpreted as signed.
    fn from(v: u8) -> Tag {
        Tag::Byte(v as i8)
    }
}

impl From<Vec<u8>> for Tag {
    /// A byte array from unsigned bytes.
    fn from(v: Vec<u8>) -> Tag {
        Tag::ByteArray(v.into_iter().map(|b| b as i8).collect())
    }
}

impl From<&[i8]> for Tag {
    fn from(v: &[i8]) -> Tag {
        Tag::ByteArray(v.to_vec())
    }
}

impl From<&[i32]> for Tag {
    fn from(v: &[i32]) -> Tag {
        Tag::IntArray(v.to_vec())
    }
}

impl From<&[i64]> for Tag {
    fn from(v: &[i64]) -> Tag {
        Tag::LongArray(v.to_vec())
    }
}

/// A value that can be pulled out of a [`Tag`] of one fixed type; used for list helpers.
pub trait FromTag: Sized {
    /// The tag type this value comes from.
    const TYPE: TagType;
    /// The value, `None` when the tag has another type.
    fn from_tag(tag: &Tag) -> Option<Self>;
}

macro_rules! from_tag_impl {
    ($($t:ty => $ty:ident, |$tag:ident| $e:expr),* $(,)?) => {$(
        impl FromTag for $t {
            const TYPE: TagType = TagType::$ty;
            fn from_tag($tag: &Tag) -> Option<Self> {
                $e
            }
        }
    )*};
}

from_tag_impl!(
    i8 => Byte, |t| t.as_i8(),
    i16 => Short, |t| t.as_i16(),
    i32 => Int, |t| t.as_i32(),
    i64 => Long, |t| t.as_i64(),
    f32 => Float, |t| t.as_f32(),
    f64 => Double, |t| t.as_f64(),
    String => String, |t| t.as_str().map(str::to_owned),
    Compound => Compound, |t| t.as_compound().cloned(),
    Vec<i32> => IntArray, |t| t.as_int_array().cloned(),
    Vec<i64> => LongArray, |t| t.as_long_array().cloned(),
    Vec<i8> => ByteArray, |t| t.as_byte_array().cloned(),
);
