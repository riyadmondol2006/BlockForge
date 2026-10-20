// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Write-only SNBT (Chunker's `toSNBT`), used for log messages and `Display`. There is no SNBT parser: Chunker has
//! none either.

use std::fmt::{self, Write};

use super::compound::Compound;
use super::tag::Tag;

macro_rules! java_float_fn {
    ($name:ident, $t:ty) => {
        /// Java's `toString` shape: `1.0`, `1.5E10`, `1.0E-5`, `NaN`, `Infinity`.
        fn $name(v: $t) -> String {
            if v.is_nan() {
                return "NaN".into();
            }
            if v.is_infinite() {
                return if v < 0.0 { "-Infinity" } else { "Infinity" }.into();
            }
            if v == 0.0 {
                return if v.is_sign_negative() { "-0.0" } else { "0.0" }.into();
            }
            if (1e-3..1e7).contains(&v.abs()) {
                let s = format!("{v}");
                if s.contains('.') { s } else { format!("{s}.0") }
            } else {
                let s = format!("{v:e}");
                let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
                let m = if m.contains('.') { m.to_owned() } else { format!("{m}.0") };
                format!("{m}E{e}")
            }
        }
    };
}

java_float_fn!(double_text, f64);
java_float_fn!(float_text, f32);

fn escape_string(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            c if c <= '\u{1f}' || c == '\u{7f}' => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
}

fn write_compound(c: &Compound, out: &mut String) {
    out.push('{');
    for (i, (key, tag)) in c.0.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        // Keys are only quoted when they contain a quote that had to be escaped.
        if key.contains('"') {
            out.push('"');
            out.push_str(&key.replace('"', "\\\""));
            out.push('"');
        } else {
            out.push_str(key);
        }
        out.push(':');
        write_tag(tag, out);
    }
    out.push('}');
}

fn write_tag(tag: &Tag, out: &mut String) {
    match tag {
        Tag::Byte(v) => {
            let _ = write!(out, "{v}b");
        }
        Tag::Short(v) => {
            let _ = write!(out, "{v}s");
        }
        Tag::Int(v) => {
            let _ = write!(out, "{v}");
        }
        Tag::Long(v) => {
            let _ = write!(out, "{v}l");
        }
        Tag::Float(v) => {
            out.push_str(&float_text(*v));
            out.push('f');
        }
        Tag::Double(v) => out.push_str(&double_text(*v)),
        Tag::String(s) => {
            out.push('"');
            escape_string(s, out);
            out.push('"');
        }
        Tag::ByteArray(a) => write_array(out, "[B;", a.iter().map(|v| format!("{v}b"))),
        Tag::IntArray(a) => write_array(out, "[I;", a.iter().map(|v| v.to_string())),
        Tag::LongArray(a) => write_array(out, "[L;", a.iter().map(|v| format!("{v}l"))),
        Tag::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_tag(item, out);
            }
            out.push(']');
        }
        Tag::Compound(c) => write_compound(c, out),
    }
}

fn write_array(out: &mut String, open: &str, items: impl Iterator<Item = String>) {
    out.push_str(open);
    for (i, item) in items.enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&item);
    }
    out.push(']');
}

impl Tag {
    /// The SNBT text of this tag, in the form Chunker prints.
    pub fn to_snbt(&self) -> String {
        let mut out = String::new();
        write_tag(self, &mut out);
        out
    }
}

impl Compound {
    /// The SNBT text of this compound.
    pub fn to_snbt(&self) -> String {
        let mut out = String::new();
        write_compound(self, &mut out);
        out
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_snbt())
    }
}

impl fmt::Display for Compound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_snbt())
    }
}
