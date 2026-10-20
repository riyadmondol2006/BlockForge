// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! The version-neutral string form of a block or item (`mapping.identifier`): a name plus named state values.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::convert::nbt::Tag;

use super::{ModelError, Result};

/// The value of one named state: a boolean, an integer or a string (`StateValue`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StateValue {
    Bool(bool),
    Int(i32),
    Str(Arc<str>),
}

impl StateValue {
    /// A string value.
    pub fn string(value: impl AsRef<str>) -> StateValue {
        StateValue::Str(Arc::from(value.as_ref()))
    }

    /// NBT form: byte 0/1, int or string.
    pub fn to_nbt(&self) -> Tag {
        match self {
            StateValue::Bool(b) => Tag::Byte(*b as i8),
            StateValue::Int(i) => Tag::Int(*i),
            StateValue::Str(s) => Tag::String(s.to_string()),
        }
    }

    /// Decode from NBT: a byte is `true` only when it equals 1; int and string map directly; any other tag is an
    /// error.
    pub fn from_nbt(tag: &Tag) -> Result<StateValue> {
        match tag {
            Tag::Byte(b) => Ok(StateValue::Bool(*b == 1)),
            Tag::Int(i) => Ok(StateValue::Int(*i)),
            Tag::String(s) => Ok(StateValue::string(s)),
            other => Err(ModelError(format!("unable to decode tag value of type {}", tag_name(other)))),
        }
    }

    /// Decode from a JSON primitive (numbers are truncated to 32 bits like Java's `intValue`).
    pub fn from_json(value: &serde_json::Value) -> Result<StateValue> {
        match value {
            serde_json::Value::Bool(b) => Ok(StateValue::Bool(*b)),
            serde_json::Value::String(s) => Ok(StateValue::string(s)),
            serde_json::Value::Number(n) => Ok(StateValue::Int(number_to_i32(n))),
            other => Err(ModelError(format!("unknown type for state value {other}"))),
        }
    }

    /// Encode as a JSON primitive.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            StateValue::Bool(b) => serde_json::Value::Bool(*b),
            StateValue::Int(i) => serde_json::Value::from(*i),
            StateValue::Str(s) => serde_json::Value::String(s.to_string()),
        }
    }

    /// The boolean, if this is one.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            StateValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The integer, if this is one.
    pub fn as_int(&self) -> Option<i32> {
        match self {
            StateValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// The string, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            StateValue::Str(s) => Some(s),
            _ => None,
        }
    }
}

fn tag_name(tag: &Tag) -> &'static str {
    match tag {
        Tag::Byte(_) => "byte",
        Tag::Short(_) => "short",
        Tag::Int(_) => "int",
        Tag::Long(_) => "long",
        Tag::Float(_) => "float",
        Tag::Double(_) => "double",
        Tag::ByteArray(_) => "byte array",
        Tag::String(_) => "string",
        Tag::List(_) => "list",
        Tag::Compound(_) => "compound",
        Tag::IntArray(_) => "int array",
        Tag::LongArray(_) => "long array",
    }
}

fn number_to_i32(n: &serde_json::Number) -> i32 {
    if let Some(i) = n.as_i64() {
        i as i32
    } else if let Some(u) = n.as_u64() {
        u as i32
    } else {
        n.as_f64().unwrap_or(0.0) as i32
    }
}

impl fmt::Display for StateValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateValue::Bool(b) => write!(f, "{b}"),
            StateValue::Int(i) => write!(f, "{i}"),
            StateValue::Str(s) => f.write_str(s),
        }
    }
}

impl From<bool> for StateValue {
    fn from(value: bool) -> Self {
        StateValue::Bool(value)
    }
}

impl From<i32> for StateValue {
    fn from(value: i32) -> Self {
        StateValue::Int(value)
    }
}

impl From<&str> for StateValue {
    fn from(value: &str) -> Self {
        StateValue::string(value)
    }
}

impl Serialize for StateValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            StateValue::Bool(b) => serializer.serialize_bool(*b),
            StateValue::Int(i) => serializer.serialize_i32(*i),
            StateValue::Str(s) => serializer.serialize_str(s),
        }
    }
}

impl<'de> Deserialize<'de> for StateValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = StateValue;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a boolean, a string or a number")
            }

            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<StateValue, E> {
                Ok(StateValue::Bool(v))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<StateValue, E> {
                Ok(StateValue::Int(v as i32))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<StateValue, E> {
                Ok(StateValue::Int(v as i32))
            }

            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<StateValue, E> {
                Ok(StateValue::Int(v as i32))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<StateValue, E> {
                Ok(StateValue::string(v))
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// Serde helper: a list of state values, written as a bare value when it has one element
/// (`serializeStateValues`), read from either form (`deserializeStateValues`).
pub mod one_or_many {
    use super::StateValue;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        One(StateValue),
        Many(Vec<StateValue>),
    }

    pub fn serialize<S: Serializer>(values: &[StateValue], serializer: S) -> Result<S::Ok, S::Error> {
        match values {
            [one] => one.serialize(serializer),
            many => many.serialize(serializer),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<StateValue>, D::Error> {
        Ok(match Repr::deserialize(deserializer)? {
            Repr::One(v) => vec![v],
            Repr::Many(v) => v,
        })
    }
}

/// A name and its named state values: the version-neutral string form of a block or item.
///
/// States are kept in a sorted map, so equality, hashing and the string form are deterministic (Chunker's hash map
/// order was not). A legacy numeric meta value is stored as the state `data` (an integer).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Identifier {
    pub name: Arc<str>,
    pub states: BTreeMap<Arc<str>, StateValue>,
}

impl Identifier {
    /// An identifier without states.
    pub fn new(name: impl AsRef<str>) -> Identifier {
        Identifier { name: Arc::from(name.as_ref()), states: BTreeMap::new() }
    }

    /// An identifier with the given states.
    pub fn with_states<K, I>(name: impl AsRef<str>, states: I) -> Identifier
    where
        K: AsRef<str>,
        I: IntoIterator<Item = (K, StateValue)>,
    {
        Identifier {
            name: Arc::from(name.as_ref()),
            states: states.into_iter().map(|(k, v)| (Arc::from(k.as_ref()), v)).collect(),
        }
    }

    /// An identifier with an optional legacy data value (stored as the state `data`).
    pub fn from_data(name: impl AsRef<str>, data: Option<i32>) -> Identifier {
        let mut id = Identifier::new(name);
        if let Some(data) = data {
            id.states.insert(Arc::from("data"), StateValue::Int(data));
        }
        id
    }

    /// The legacy data value, when the `data` state is an integer.
    pub fn data_value(&self) -> Option<i32> {
        self.states.get("data").and_then(StateValue::as_int)
    }

    /// Whether the name is in the `minecraft` namespace (or has none).
    pub fn is_vanilla(&self) -> bool {
        !self.name.contains(':') || self.name.starts_with("minecraft:")
    }

    /// Look up a state.
    pub fn state(&self, key: &str) -> Option<&StateValue> {
        self.states.get(key)
    }

    /// The states as `[a=b,c=d]` (`[]` when there are none).
    pub fn state_string(&self) -> String {
        if self.states.is_empty() {
            return "[]".to_string();
        }
        let mut out = String::from("[");
        for (i, (k, v)) in self.states.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(k);
            out.push('=');
            out.push_str(&v.to_string());
        }
        out.push(']');
        out
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)?;
        if !self.states.is_empty() {
            f.write_str(&self.state_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nbt_round_trip() {
        assert_eq!(StateValue::from_nbt(&Tag::Byte(1)).unwrap(), StateValue::Bool(true));
        assert_eq!(StateValue::from_nbt(&Tag::Byte(0)).unwrap(), StateValue::Bool(false));
        // Anything but exactly 1 is false.
        assert_eq!(StateValue::from_nbt(&Tag::Byte(2)).unwrap(), StateValue::Bool(false));
        assert_eq!(StateValue::from_nbt(&Tag::Int(7)).unwrap(), StateValue::Int(7));
        assert_eq!(StateValue::from_nbt(&Tag::String("x".into())).unwrap(), StateValue::string("x"));
        assert!(StateValue::from_nbt(&Tag::Long(1)).is_err());
        assert_eq!(StateValue::Bool(true).to_nbt(), Tag::Byte(1));
        assert_eq!(StateValue::Bool(false).to_nbt(), Tag::Byte(0));
        assert_eq!(StateValue::Int(-4).to_nbt(), Tag::Int(-4));
    }

    #[test]
    fn json_forms() {
        let v: StateValue = serde_json::from_str("true").unwrap();
        assert_eq!(v, StateValue::Bool(true));
        let v: StateValue = serde_json::from_str("\"north\"").unwrap();
        assert_eq!(v, StateValue::string("north"));
        let v: StateValue = serde_json::from_str("3").unwrap();
        assert_eq!(v, StateValue::Int(3));
        // Numbers truncate like intValue.
        let v: StateValue = serde_json::from_str("2.9").unwrap();
        assert_eq!(v, StateValue::Int(2));
        assert_eq!(serde_json::to_string(&StateValue::Int(5)).unwrap(), "5");
        assert!(serde_json::from_str::<StateValue>("null").is_err());
        assert_eq!(StateValue::from_json(&serde_json::json!(false)).unwrap(), StateValue::Bool(false));
    }

    #[test]
    fn one_or_many_lists() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W {
            #[serde(with = "one_or_many")]
            v: Vec<StateValue>,
        }
        let one: W = serde_json::from_str(r#"{"v": 1}"#).unwrap();
        assert_eq!(one.v, vec![StateValue::Int(1)]);
        let many: W = serde_json::from_str(r#"{"v": [1, "a"]}"#).unwrap();
        assert_eq!(many.v, vec![StateValue::Int(1), StateValue::string("a")]);
        assert_eq!(serde_json::to_string(&one).unwrap(), r#"{"v":1}"#);
        assert_eq!(serde_json::to_string(&many).unwrap(), r#"{"v":[1,"a"]}"#);
    }

    #[test]
    fn identifier_basics() {
        let id = Identifier::from_data("minecraft:wool", Some(5));
        assert_eq!(id.data_value(), Some(5));
        assert!(id.is_vanilla());
        assert_eq!(Identifier::new("stone").data_value(), None);
        assert!(Identifier::new("stone").is_vanilla());
        assert!(!Identifier::new("mod:thing").is_vanilla());
        let a = Identifier::with_states("x", [("b", StateValue::Int(1)), ("a", StateValue::Bool(true))]);
        let b = Identifier::with_states("x", [("a", StateValue::Bool(true)), ("b", StateValue::Int(1))]);
        assert_eq!(a, b);
        assert_eq!(a.to_string(), "x[a=true,b=1]");
        assert_eq!(Identifier::new("x").to_string(), "x");
        // `data` that is not an integer is not a data value.
        let s = Identifier::with_states("x", [("data", StateValue::string("1"))]);
        assert_eq!(s.data_value(), None);
    }
}
