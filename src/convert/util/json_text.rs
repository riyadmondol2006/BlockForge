// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Minecraft text components: JSON text <-> NBT text (Java 1.21.5+), legacy section-sign text and stripped text.
//!
//! Port of Chunker's `JsonTextUtil`. JSON values are `serde_json::Value` (Gson's `JsonElement`). Two obvious slips of
//! the original are fixed: a `show_entity` hover event takes its `name` from `contents.name` (Chunker reads
//! `contents.type`), and a UUID array is read element by element (Chunker never advances its index).

use std::fmt;

use serde_json::{Map, Number, Value};

use super::version::Version;
use crate::convert::nbt::{Compound, Tag, TagType};

/// The colour prefix of legacy text.
pub const CHAT_COLOR: char = '\u{a7}';

/// Colour names and the ARGB value Bedrock stores for them (as the unsigned form of Java's `int`).
pub const COLOR_TO_BEDROCK_HEX: [(&str, u32); 16] = [
    ("black", 0xFF000000),
    ("red", 0xFFB02E26),
    ("green", 0xFF5E7C16),
    ("brown", 0xFF835432),
    ("blue", 0xFF3C44AA),
    ("purple", 0xFF8932B8),
    ("cyan", 0xFF169C9C),
    ("light_gray", 0xFF9D9D97),
    ("gray", 0xFF474F52),
    ("pink", 0xFFF38BAA),
    ("lime", 0xFF80C71F),
    ("yellow", 0xFFFED83D),
    ("light_blue", 0xFF3AB3DA),
    ("magenta", 0xFFC74EBD),
    ("orange", 0xFFF9801D),
    ("white", 0xFFF0F0F0),
];

/// Colour and style names and the character of their legacy `\u{a7}x` code.
pub const COLOR_TO_CHARACTER: [(&str, char); 22] = [
    ("black", '0'),
    ("dark_blue", '1'),
    ("dark_green", '2'),
    ("dark_aqua", '3'),
    ("dark_red", '4'),
    ("dark_purple", '5'),
    ("gold", '6'),
    ("gray", '7'),
    ("dark_gray", '8'),
    ("blue", '9'),
    ("green", 'a'),
    ("aqua", 'b'),
    ("red", 'c'),
    ("light_purple", 'd'),
    ("yellow", 'e'),
    ("white", 'f'),
    ("obfuscated", 'k'),
    ("bold", 'l'),
    ("strikethrough", 'm'),
    ("underline", 'n'),
    ("italic", 'o'),
    ("reset", 'r'),
];

/// The Bedrock ARGB colour of a colour name.
pub fn bedrock_hex_of_color(name: &str) -> Option<u32> {
    COLOR_TO_BEDROCK_HEX.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

/// The colour name of a Bedrock ARGB colour.
pub fn color_of_bedrock_hex(hex: u32) -> Option<&'static str> {
    COLOR_TO_BEDROCK_HEX.iter().find(|(_, v)| *v == hex).map(|(n, _)| *n)
}

/// The legacy code character of a colour or style name.
pub fn character_of_color(name: &str) -> Option<char> {
    COLOR_TO_CHARACTER.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

/// The colour or style name of a legacy code character.
pub fn color_of_character(c: char) -> Option<&'static str> {
    COLOR_TO_CHARACTER.iter().find(|(_, ch)| *ch == c).map(|(n, _)| *n)
}

/// A text component that could not be converted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonTextError(pub String);

impl fmt::Display for JsonTextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for JsonTextError {}

impl From<crate::convert::nbt::NbtError> for JsonTextError {
    fn from(e: crate::convert::nbt::NbtError) -> Self {
        JsonTextError(e.to_string())
    }
}

type Res<T> = Result<T, JsonTextError>;

fn err<T>(msg: impl Into<String>) -> Res<T> {
    Err(JsonTextError(msg.into()))
}

/// `{"text": text}`.
pub fn from_text(text: &str) -> Value {
    let mut m = Map::new();
    m.insert("text".into(), Value::String(text.into()));
    Value::Object(m)
}

/// The empty text component `{"text":""}`.
pub fn empty_text() -> Value {
    from_text("")
}

// Gson's JsonPrimitive getters, which accept strings for numbers and booleans.

fn as_string(v: &Value) -> Res<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Array(a) if a.len() == 1 => as_string(&a[0]),
        other => err(format!("Unable to read {other} as a string")),
    }
}

fn as_bool(v: &Value) -> Res<bool> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::String(s) => Ok(s.eq_ignore_ascii_case("true")),
        Value::Number(n) => Ok(n.to_string().eq_ignore_ascii_case("true")),
        Value::Array(a) if a.len() == 1 => as_bool(&a[0]),
        other => err(format!("Unable to read {other} as a boolean")),
    }
}

fn as_i32(v: &Value) -> Res<i32> {
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i as i32)
            } else {
                Ok(n.as_f64().unwrap_or(0.0) as i32)
            }
        }
        Value::String(s) => s.trim().parse().or_else(|_| err(format!("Unable to read {s} as an int"))),
        Value::Array(a) if a.len() == 1 => as_i32(&a[0]),
        other => err(format!("Unable to read {other} as an int")),
    }
}

fn as_f32(v: &Value) -> Res<f32> {
    match v {
        Value::Number(n) => Ok(n.as_f64().unwrap_or(0.0) as f32),
        Value::String(s) => s.trim().parse().or_else(|_| err(format!("Unable to read {s} as a float"))),
        Value::Array(a) if a.len() == 1 => as_f32(&a[0]),
        other => err(format!("Unable to read {other} as a float")),
    }
}

fn str_member(o: &Map<String, Value>, key: &str) -> Res<Option<String>> {
    o.get(key).map(as_string).transpose()
}

fn bool_member(o: &Map<String, Value>, key: &str) -> Res<Option<bool>> {
    o.get(key).map(as_bool).transpose()
}

fn flag(o: &Map<String, Value>, key: &str) -> Res<bool> {
    Ok(bool_member(o, key)?.unwrap_or(false))
}

fn object_member<'a>(o: &'a Map<String, Value>, key: &str) -> Res<Option<&'a Map<String, Value>>> {
    match o.get(key) {
        None => Ok(None),
        Some(Value::Object(m)) => Ok(Some(m)),
        Some(other) => err(format!("Expected an object for {key}, got {other}")),
    }
}

/// Convert JSON text to legacy `\u{a7}` coded text. `bedrock` drops underline and strikethrough.
pub fn to_legacy(input: &Value, bedrock: bool) -> Res<String> {
    match input {
        Value::Null => Ok(String::new()),
        Value::String(s) => Ok(s.clone()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Array(items) => {
            let mut out = String::new();
            for item in items {
                out.push_str(&to_legacy(item, bedrock)?);
            }
            Ok(out)
        }
        Value::Object(o) => {
            let mut out = String::new();
            // The colour may also be a 1.16 hex colour, which has no legacy code.
            if let Some(color) = str_member(o, "color")?
                && let Some(c) = character_of_color(&color)
            {
                out.push(CHAT_COLOR);
                out.push(c);
            }
            for (key, style, skip_on_bedrock) in [
                ("bold", "bold", false),
                ("italic", "italic", false),
                ("underline", "underline", true),
                ("strikethrough", "strikethrough", true),
                ("obfuscated", "obfuscated", false),
            ] {
                if flag(o, key)? && !(skip_on_bedrock && bedrock) {
                    out.push(CHAT_COLOR);
                    out.push(character_of_color(style).unwrap_or('r'));
                }
            }
            for key in ["text", "translate", "extra"] {
                if let Some(v) = o.get(key) {
                    out.push_str(&to_legacy(v, bedrock)?);
                }
            }
            Ok(out)
        }
    }
}

/// Remove every legacy `\u{a7}x` code (colours 0-9a-f, styles k-o and r) from `s`.
pub fn strip_legacy_formatting(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == CHAT_COLOR
            && let Some(&n) = chars.peek()
            && matches!(n.to_ascii_lowercase(), '0'..='9' | 'a'..='f' | 'k'..='o' | 'r')
        {
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// Convert JSON text to plain text with no formatting. Translatable text is an error.
pub fn to_stripped(input: &Value) -> Res<String> {
    match input {
        Value::Null => Ok(String::new()),
        Value::String(s) => Ok(strip_legacy_formatting(s)),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Array(items) => {
            let mut out = String::new();
            for item in items {
                out.push_str(&to_stripped(item)?);
            }
            Ok(out)
        }
        Value::Object(o) => {
            if o.contains_key("translate") {
                return err(format!("Unable to turn JSON into legacy {input}"));
            }
            let mut out = String::new();
            for key in ["text", "extra"] {
                if let Some(v) = o.get(key) {
                    out.push_str(&to_stripped(v)?);
                }
            }
            Ok(out)
        }
    }
}

/// Parse a JSON text string. `None`, JSON `null` and unparsable input fall back to text components: a bare JSON
/// string becomes `{"text": s}` and anything that is not JSON is taken as literal text.
pub fn from_json(json: Option<&str>) -> Value {
    let Some(s) = json else {
        return empty_text();
    };
    match serde_json::from_str::<Value>(s) {
        Ok(Value::Null) => empty_text(),
        Ok(Value::String(text)) => from_text(&text),
        Ok(v) => v,
        Err(_) => from_text(s),
    }
}

/// Serialize JSON the way Gson does by default: compact, with `< > & = '` and U+2028/9 escaped as `\uXXXX`.
pub fn to_json(value: &Value) -> String {
    let compact = serde_json::to_string(value).unwrap_or_default();
    let mut out = String::with_capacity(compact.len());
    for c in compact.chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '=' => out.push_str("\\u003d"),
            '\'' => out.push_str("\\u0027"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out
}

fn float_value(f: f32) -> Value {
    // Go through the shortest f32 text so 0.1f stays 0.1 and not 0.10000000149011612.
    let v: f64 = f.to_string().parse().unwrap_or(f as f64);
    Number::from_f64(v).map_or(Value::Null, Value::Number)
}

fn add_str(out: &mut Map<String, Value>, from: &Compound, key: &str) -> Res<()> {
    if let Some(v) = from.opt_str(key)? {
        out.insert(key.into(), Value::String(v.into()));
    }
    Ok(())
}

fn add_flag(out: &mut Map<String, Value>, from: &Compound, key: &str) -> Res<()> {
    if let Some(v) = from.opt_i8(key)? {
        out.insert(key.into(), Value::Bool(v == 1));
    }
    Ok(())
}

fn add_component(out: &mut Map<String, Value>, from: &Compound, key: &str) -> Res<()> {
    if let Some(t) = from.get(key) {
        out.insert(key.into(), from_nbt(Some(t))?);
    }
    Ok(())
}

/// Convert an NBT text component (Java 1.21.5+, or a JSON string of older versions) to JSON.
pub fn from_nbt(tag: Option<&Tag>) -> Res<Value> {
    let Some(tag) = tag else {
        return Ok(empty_text());
    };
    match tag {
        Tag::String(s) => Ok(from_json(Some(s))),
        Tag::Compound(c) => {
            compound_from_nbt(c).map_err(|e| JsonTextError(format!("Unable to parse Text component {tag}: {e}")))
        }
        Tag::List(items) => Ok(Value::Array(items.iter().map(|t| from_nbt(Some(t))).collect::<Res<Vec<_>>>()?)),
        other => err(format!("Unknown type for Text component {other}")),
    }
}

fn compound_from_nbt(c: &Compound) -> Res<Value> {
    // Mixed lists wrap non-compound entries as {"": entry}.
    if let Some(inner) = c.get("") {
        return from_nbt(Some(inner));
    }
    let mut out = match c.opt_str("text")? {
        Some(text) => match from_text(text) {
            Value::Object(m) => m,
            _ => Map::new(),
        },
        None if c.contains_key("text") => return err("text is not a string"),
        None => Map::new(),
    };
    add_str(&mut out, c, "color")?;
    match c.get("shadow_color") {
        Some(Tag::Int(v)) => {
            out.insert("shadow_color".into(), Value::Number((*v).into()));
        }
        Some(Tag::List(_)) => {
            let values: Vec<f32> = c.require_list_values("shadow_color")?;
            out.insert("shadow_color".into(), Value::Array(values.into_iter().map(float_value).collect()));
        }
        _ => {}
    }
    for key in ["bold", "italic", "underlined", "strikethrough", "obfuscated"] {
        add_flag(&mut out, c, key)?;
    }
    add_str(&mut out, c, "selector")?;
    add_component(&mut out, c, "separator")?;
    add_str(&mut out, c, "nbt")?;
    add_str(&mut out, c, "source")?;
    add_flag(&mut out, c, "interpret")?;
    add_str(&mut out, c, "translate")?;
    add_component(&mut out, c, "fallback")?;
    add_str(&mut out, c, "keybind")?;
    add_str(&mut out, c, "insertion")?;
    add_str(&mut out, c, "font")?;
    if let Some(score) = c.opt_compound("score")? {
        let mut s = Map::new();
        add_str(&mut s, score, "objective")?;
        add_str(&mut s, score, "name")?;
        out.insert("score".into(), Value::Object(s));
    }
    if let Some(hover) = c.opt_compound("hover_event")? {
        let action = hover.opt_str("action")?.unwrap_or("");
        if !action.is_empty() {
            let mut h = Map::new();
            h.insert("action".into(), Value::String(action.into()));
            match action {
                "show_text" => {
                    if let Some(v) = hover.get("value") {
                        h.insert("contents".into(), from_nbt(Some(v))?);
                    }
                }
                "show_item" => {
                    let mut item = Map::new();
                    add_str(&mut item, hover, "id")?;
                    if let Some(n) = hover.opt_i32("count")? {
                        item.insert("count".into(), Value::Number(n.into()));
                    }
                    if !item.is_empty() {
                        h.insert("contents".into(), Value::Object(item));
                    }
                }
                "show_entity" => {
                    let mut e = Map::new();
                    if let Some(n) = hover.get("name") {
                        e.insert("name".into(), from_nbt(Some(n))?);
                    }
                    if let Some(id) = hover.opt_str("id")? {
                        e.insert("type".into(), Value::String(id.into()));
                    }
                    match hover.get("uuid") {
                        None => {}
                        Some(Tag::String(s)) => {
                            e.insert("id".into(), Value::String(s.clone()));
                        }
                        Some(Tag::IntArray(ids)) => {
                            e.insert("id".into(), Value::Array(ids.iter().map(|&i| Value::Number(i.into())).collect()));
                        }
                        Some(other) => return err(format!("Unknown UUID type {other}")),
                    }
                    if !e.is_empty() {
                        h.insert("contents".into(), Value::Object(e));
                    }
                }
                _ => {}
            }
            out.insert("hoverEvent".into(), Value::Object(h));
        }
    }
    if let Some(click) = c.opt_compound("click_event")? {
        let action = click.opt_str("action")?.unwrap_or("");
        if !action.is_empty() {
            let mut e = Map::new();
            e.insert("action".into(), Value::String(action.into()));
            let value = match action {
                "open_url" => click.opt_str("url")?.map(str::to_owned),
                "open_file" => click.opt_str("path")?.map(str::to_owned),
                "run_command" | "suggest_command" => click.opt_str("command")?.map(str::to_owned),
                "change_page" => click.opt_i32("page")?.map(|p| p.to_string()),
                _ => click.opt_str("value")?.map(str::to_owned),
            };
            if let Some(v) = value {
                e.insert("value".into(), Value::String(v));
            }
            out.insert("clickEvent".into(), Value::Object(e));
        }
    }
    add_component(&mut out, c, "extra")?;
    Ok(Value::Object(out))
}

fn put_flag(out: &mut Compound, o: &Map<String, Value>, key: &str) -> Res<()> {
    if let Some(v) = bool_member(o, key)? {
        out.put(key, Tag::Byte(v as i8));
    }
    Ok(())
}

fn put_str(out: &mut Compound, o: &Map<String, Value>, key: &str) -> Res<()> {
    if let Some(v) = str_member(o, key)? {
        out.put(key, v);
    }
    Ok(())
}

/// Convert a JSON text component to the NBT text component of Java 1.21.5+.
pub fn to_nbt(element: &Value) -> Res<Tag> {
    match element {
        Value::Object(o) => {
            object_to_nbt(o).map_err(|e| JsonTextError(format!("Unable to parse Text JSON {element}: {e}")))
        }
        Value::Array(items) => {
            let mut list = Vec::with_capacity(items.len());
            for child in items {
                list.push(match to_nbt(child)? {
                    c @ Tag::Compound(_) => c,
                    other => {
                        let mut holder = Compound::with_capacity(1);
                        holder.put("", other);
                        Tag::Compound(holder)
                    }
                });
            }
            Ok(Tag::List(list))
        }
        Value::Null => err("Unknown type for Text JSON null"),
        primitive => Ok(Tag::String(as_string(primitive)?)),
    }
}

fn object_to_nbt(o: &Map<String, Value>) -> Res<Tag> {
    let mut out = Compound::with_capacity(1);
    put_str(&mut out, o, "text")?;
    put_str(&mut out, o, "color")?;
    if let Some(shadow) = o.get("shadow_color") {
        match shadow {
            Value::Array(values) => {
                let floats = values.iter().map(as_f32).collect::<Res<Vec<_>>>()?;
                out.put("shadow_color", Tag::list(floats));
            }
            other => {
                out.put("shadow_color", as_i32(other)?);
            }
        }
    }
    for key in ["bold", "italic", "underlined", "strikethrough", "obfuscated"] {
        put_flag(&mut out, o, key)?;
    }
    put_str(&mut out, o, "selector")?;
    if let Some(v) = o.get("separator") {
        out.put("separator", to_nbt(v)?);
    }
    put_str(&mut out, o, "nbt")?;
    put_str(&mut out, o, "source")?;
    put_flag(&mut out, o, "interpret")?;
    put_str(&mut out, o, "translate")?;
    if let Some(v) = o.get("fallback") {
        out.put("fallback", to_nbt(v)?);
    }
    put_str(&mut out, o, "keybind")?;
    put_str(&mut out, o, "insertion")?;
    put_str(&mut out, o, "font")?;
    if let Some(score) = object_member(o, "score")? {
        let mut s = Compound::with_capacity(2);
        put_str(&mut s, score, "objective")?;
        put_str(&mut s, score, "name")?;
        out.put("score", s);
    }
    if let Some(hover) = object_member(o, "hoverEvent")? {
        let action = str_member(hover, "action")?.unwrap_or_default();
        if !action.is_empty() {
            let mut h = Compound::with_capacity(2);
            h.put("action", action.as_str());
            match action.as_str() {
                "show_text" => {
                    if let Some(v) = hover.get("contents") {
                        h.put("value", to_nbt(v)?);
                    }
                    if let Some(v) = hover.get("value") {
                        h.put("value", to_nbt(v)?);
                    }
                }
                "show_item" => {
                    if let Some(contents) = object_member(hover, "contents")? {
                        put_str(&mut h, contents, "id")?;
                        if let Some(count) = contents.get("count") {
                            h.put("count", as_i32(count)?);
                        }
                    }
                }
                "show_entity" => {
                    if let Some(contents) = object_member(hover, "contents")? {
                        if let Some(t) = str_member(contents, "type")? {
                            h.put("id", t);
                        }
                        if let Some(n) = contents.get("name") {
                            h.put("name", to_nbt(n)?);
                        }
                        match contents.get("id") {
                            None => {}
                            Some(Value::Array(ids)) => {
                                let mut out_ids = [0i32; 4];
                                for (slot, id) in out_ids.iter_mut().zip(ids) {
                                    *slot = as_i32(id)?;
                                }
                                h.put("uuid", Tag::IntArray(out_ids.to_vec()));
                            }
                            Some(Value::Object(_) | Value::Null) => {
                                return err(format!("Unknown UUID type {}", contents["id"]));
                            }
                            Some(prim) => {
                                h.put("uuid", as_string(prim)?);
                            }
                        }
                    }
                }
                _ => {}
            }
            out.put("hover_event", h);
        }
    }
    if let Some(click) = object_member(o, "clickEvent")? {
        let action = str_member(click, "action")?.unwrap_or_default();
        if !action.is_empty() {
            let mut e = Compound::with_capacity(2);
            e.put("action", action.as_str());
            if let Some(value) = str_member(click, "value")? {
                match action.as_str() {
                    "open_url" => {
                        e.put("url", value);
                    }
                    "open_file" => {
                        e.put("path", value);
                    }
                    "run_command" | "suggest_command" => {
                        e.put("command", value);
                    }
                    "change_page" => {
                        let page: i32 =
                            value.trim().parse().or_else(|_| err(format!("Invalid page number {value}")))?;
                        e.put("page", page);
                    }
                    _ => {
                        e.put("value", value);
                    }
                }
            }
            out.put("click_event", e);
        }
    }
    if let Some(v) = o.get("extra") {
        out.put("extra", to_nbt(v)?);
    }
    Ok(Tag::Compound(out))
}

/// NBT text on Java 1.21.5 and later, the JSON string before that. `java_version` is the game version.
pub fn to_nbt_for(element: &Value, java_version: Version) -> Res<Tag> {
    if java_version.is_greater_than_or_equal(1, 21, 5) { to_nbt(element) } else { Ok(Tag::String(to_json(element))) }
}

/// A list of text components (a sign's lines, a book's pages) for the Java version, padded with empty text up to
/// `minimum_size`. From 1.21.5 the entries are NBT and a list of mixed types is rebalanced to compounds.
pub fn to_nbt_list(elements: &[Value], minimum_size: usize, java_version: Version) -> Res<Tag> {
    if java_version.is_greater_than_or_equal(1, 21, 5) {
        let mut tags = Vec::with_capacity(elements.len().max(minimum_size));
        let mut first: Option<TagType> = None;
        let mut mixed = false;
        for e in elements {
            let tag = to_nbt(e)?;
            match first {
                None => first = Some(tag.tag_type()),
                Some(t) if t != tag.tag_type() => mixed = true,
                _ => {}
            }
            tags.push(tag);
        }
        let first = first.unwrap_or(TagType::Compound);
        while tags.len() < minimum_size {
            tags.push(Tag::String(to_json(&empty_text())));
        }
        let target = if mixed { TagType::Compound } else { first };
        let mut out = Vec::with_capacity(tags.len());
        for tag in tags {
            if tag.tag_type() != target {
                if tag.tag_type() == TagType::Compound {
                    return err(format!("Unable to convert {tag} to {}", target.name()));
                }
                let mut holder = Compound::with_capacity(1);
                holder.put("", tag);
                out.push(Tag::Compound(holder));
            } else {
                out.push(tag);
            }
        }
        Ok(Tag::List(out))
    } else {
        let mut out: Vec<Tag> = elements.iter().map(|e| Tag::String(to_json(e))).collect();
        while out.len() < minimum_size {
            out.push(Tag::String(to_json(&empty_text())));
        }
        Ok(Tag::List(out))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn from_json_fallbacks() {
        assert_eq!(from_json(None), json!({"text": ""}));
        assert_eq!(from_json(Some("null")), json!({"text": ""}));
        assert_eq!(from_json(Some("\"hi\"")), json!({"text": "hi"}));
        assert_eq!(from_json(Some("not json {")), json!({"text": "not json {"}));
        assert_eq!(from_json(Some("")), json!({"text": ""}));
        assert_eq!(from_json(Some("{\"text\":\"a\",\"bold\":true}")), json!({"text": "a", "bold": true}));
        assert_eq!(from_json(Some("[\"a\",{\"text\":\"b\"}]")), json!(["a", {"text": "b"}]));
    }

    #[test]
    fn gson_style_json() {
        assert_eq!(
            to_json(&json!({"text": "a<b>&c='d'"})),
            "{\"text\":\"a\\u003cb\\u003e\\u0026c\\u003d\\u0027d\\u0027\"}"
        );
        assert_eq!(to_json(&empty_text()), "{\"text\":\"\"}");
    }

    #[test]
    fn legacy_and_stripped() {
        let v =
            json!({"text": "Hello ", "color": "red", "bold": true, "extra": [{"text": "w", "underline": true}, "!"]});
        assert_eq!(to_legacy(&v, false).unwrap(), "\u{a7}c\u{a7}lHello \u{a7}nw!");
        assert_eq!(to_legacy(&v, true).unwrap(), "\u{a7}c\u{a7}lHello w!");
        assert_eq!(to_legacy(&json!({"color": "#ff0000", "text": "x"}), false).unwrap(), "x");
        assert_eq!(to_stripped(&json!({"text": "\u{a7}cA\u{a7}Zb", "extra": ["c"]})).unwrap(), "A\u{a7}Zbc");
        assert!(to_stripped(&json!({"translate": "k"})).is_err());
        assert_eq!(strip_legacy_formatting("\u{a7}a\u{a7}Kx\u{a7}"), "x\u{a7}");
    }

    #[test]
    fn nbt_round_trip() {
        let v = json!({
            "text": "hi", "color": "gold", "bold": true, "italic": false, "insertion": "ins",
            "extra": [{"text": "a"}, "b"],
            "clickEvent": {"action": "open_url", "value": "https://x.y"},
            "hoverEvent": {"action": "show_text", "contents": {"text": "tip"}}
        });
        let tag = to_nbt(&v).unwrap();
        let c = tag.as_compound().unwrap();
        assert_eq!(c.get_str("text"), Some("hi"));
        assert_eq!(c.get_i8("bold"), Some(1));
        assert_eq!(c.get_i8("italic"), Some(0));
        assert_eq!(c.get_compound("click_event").unwrap().get_str("url"), Some("https://x.y"));
        // "extra" had a bare string, which is wrapped in {"": ..}
        let extra = c.get_list("extra").unwrap();
        assert_eq!(extra[1].as_compound().unwrap().get_str(""), Some("b"));
        assert_eq!(
            from_nbt(Some(&tag)).unwrap(),
            json!({
                "text": "hi", "color": "gold", "bold": true, "italic": false, "insertion": "ins",
                "clickEvent": {"action": "open_url", "value": "https://x.y"},
                "hoverEvent": {"action": "show_text", "contents": {"text": "tip"}},
                "extra": [{"text": "a"}, {"text": "b"}]
            })
        );
    }

    #[test]
    fn entity_hover_and_uuid() {
        let v = json!({"text": "", "hoverEvent": {"action": "show_entity",
            "contents": {"type": "minecraft:pig", "id": [1, 2, 3, 4], "name": {"text": "Oink"}}}});
        let tag = to_nbt(&v).unwrap();
        let h = tag.as_compound().unwrap().get_compound("hover_event").unwrap();
        assert_eq!(h.get_str("id"), Some("minecraft:pig"));
        assert_eq!(h.get_int_array("uuid"), Some(&[1, 2, 3, 4][..]));
        assert_eq!(h.get_compound("name").unwrap().get_str("text"), Some("Oink"));
        let back = from_nbt(Some(&tag)).unwrap();
        assert_eq!(back["hoverEvent"]["contents"]["id"], json!([1, 2, 3, 4]));
        assert_eq!(back["hoverEvent"]["contents"]["type"], json!("minecraft:pig"));
    }

    #[test]
    fn versioned_lists() {
        let lines = [from_text("a"), from_text("b")];
        let old = to_nbt_list(&lines, 4, Version::new(1, 20, 4)).unwrap();
        let items = old.as_list().unwrap();
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].as_str(), Some("{\"text\":\"a\"}"));
        assert_eq!(items[3].as_str(), Some("{\"text\":\"\"}"));
        let new = to_nbt_list(&lines, 4, Version::new(1, 21, 5)).unwrap();
        let items = new.as_list().unwrap();
        assert_eq!(items.len(), 4);
        // Padding strings are wrapped so the list stays homogeneous (compounds).
        assert!(items.iter().all(|t| t.as_compound().is_some()));
        assert_eq!(items[3].as_compound().unwrap().get_str(""), Some("{\"text\":\"\"}"));
        assert!(new.validate().is_ok());
        let none = to_nbt_list(&[], 0, Version::new(1, 21, 5)).unwrap();
        assert_eq!(none, Tag::List(vec![]));
        assert_eq!(
            to_nbt_for(&from_text("x"), Version::new(1, 21, 4)).unwrap(),
            Tag::String("{\"text\":\"x\"}".into())
        );
    }

    #[test]
    fn color_tables() {
        assert_eq!(bedrock_hex_of_color("red"), Some(0xFFB02E26));
        assert_eq!(color_of_bedrock_hex(0xFFF0F0F0), Some("white"));
        assert_eq!(character_of_color("gold"), Some('6'));
        assert_eq!(color_of_character('r'), Some("reset"));
    }
}
