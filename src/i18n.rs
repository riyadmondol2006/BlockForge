//! Translations, the way the Java edition does them: the text in the code is the English original (the "msgid"), a
//! gettext `.po` file per language says what to show instead, and `{0}`, `{1}` in a text are filled in from the
//! arguments (Java's `MessageFormat`). A text nobody has translated is shown as written.
//!
//! The Java edition looks for `assets/lang/<language>-<COUNTRY>.po` (`de-DE.po`), offers English plus every language
//! it finds a file for, and stores the language by its English display name ("English", "German (Germany)") in
//! `BlockForge.json`. This module does the same, so a settings file written by either edition selects the same
//! language in both. Files are looked for in two places:
//!
//! * `assets/lang/` in the source tree, embedded into the program when it is built (see `build.rs`)
//! * `lang/` inside the data folder, read when the program starts, so a translation can be tried or shared
//!   without rebuilding
//!
//! The Java edition currently ships no translation files at all (they were taken out of its repository), so the
//! language list is just English until some are added to either place.
//!
//! This module also has the date and instance-title formats from the general settings, since both are shown
//! through the same "what does the player see" rules.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use chrono::{Datelike, NaiveDateTime, Timelike};

use crate::config::Settings;

pub mod template;

// ---------------------------------------------------------------------------------------------------------------
// Languages

/// One language the launcher knows the name of. Not all of them have a translation file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocaleInfo {
    pub language: &'static str,
    pub country: &'static str,
    /// What Java's `Locale.getDisplayName()` says in English; this is the text saved in the settings.
    pub display_name: &'static str,
    /// Needs a font with its letters that the launcher's own font does not have (Java: `localesWithoutFont`).
    pub needs_system_font: bool,
    /// Same, for the tab titles (Java: `localesWithoutTabFont`).
    pub needs_system_tab_font: bool,
}

const fn locale(
    language: &'static str,
    country: &'static str,
    display_name: &'static str,
    font: bool,
    tab_font: bool,
) -> LocaleInfo {
    LocaleInfo { language, country, display_name, needs_system_font: font, needs_system_tab_font: tab_font }
}

/// The languages the launcher is prepared for, in the order the Java edition lists them.
pub const LOCALES: &[LocaleInfo] = &[
    locale("en", "", "English", false, false),
    locale("af", "ZA", "Afrikaans (South Africa)", false, false),
    locale("ar", "SA", "Arabic (Saudi Arabia)", true, true),
    locale("ca", "ES", "Catalan (Spain)", false, false),
    locale("zh", "CN", "Chinese (China)", true, true),
    locale("zh", "TW", "Chinese (Taiwan)", true, true),
    locale("cs", "CZ", "Czech (Czechia)", false, false),
    locale("da", "DK", "Danish (Denmark)", false, false),
    locale("nl", "NL", "Dutch (Netherlands)", false, false),
    locale("fi", "FI", "Finnish (Finland)", false, false),
    locale("fr", "FR", "French (France)", false, false),
    locale("de", "DE", "German (Germany)", false, false),
    locale("el", "GR", "Greek (Greece)", false, true),
    locale("he", "IL", "Hebrew (Israel)", true, true),
    locale("hu", "HU", "Hungarian (Hungary)", false, false),
    locale("it", "IT", "Italian (Italy)", false, false),
    locale("ja", "JP", "Japanese (Japan)", true, true),
    locale("ko", "KR", "Korean (South Korea)", true, true),
    locale("no", "NO", "Norwegian (Norway)", false, false),
    locale("pl", "PL", "Polish (Poland)", false, false),
    locale("pt", "PT", "Portuguese (Portugal)", false, false),
    locale("pt", "BR", "Portuguese (Brazil)", false, false),
    locale("ro", "RO", "Romanian (Romania)", false, false),
    locale("ru", "RU", "Russian (Russia)", false, false),
    // "SP" is not a country code, so Java prints it as it is
    locale("sr", "SP", "Serbian (SP)", false, false),
    locale("es", "ES", "Spanish (Spain)", false, false),
    locale("sv", "SE", "Swedish (Sweden)", false, false),
    locale("tr", "TR", "Turkish (T\u{fc}rkiye)", false, false),
    locale("uk", "UA", "Ukrainian (Ukraine)", false, false),
];

/// The language that needs no file.
pub const DEFAULT_LANGUAGE: &str = "English";

impl LocaleInfo {
    /// `de-DE`, the name of the translation file without its `.po`. English has none.
    pub fn tag(&self) -> String {
        if self.country.is_empty() { self.language.to_string() } else { format!("{}-{}", self.language, self.country) }
    }
}

/// Finds a language by the text saved in the settings, or by a tag such as `de-DE` or `de_DE`.
pub fn find_locale(name: &str) -> Option<&'static LocaleInfo> {
    let name = name.trim();
    LOCALES.iter().find(|l| l.display_name == name).or_else(|| {
        let tag = name.replace('_', "-");
        LOCALES.iter().find(|l| l.tag().eq_ignore_ascii_case(&tag))
    })
}

// ---------------------------------------------------------------------------------------------------------------
// The .po catalog

/// What one language says. Built from a `.po` file.
#[derive(Default, Debug)]
pub struct Catalog {
    entries: HashMap<(String, String), Entry>,
    /// plural id -> (context, singular id), for the lookups that only know the plural text
    by_plural: HashMap<(String, String), String>,
    plural: Option<PluralRule>,
}

#[derive(Default, Debug, Clone)]
struct Entry {
    plural_id: Option<String>,
    strings: Vec<String>,
}

impl Catalog {
    /// Reads the text of a `.po` file. Lines it cannot make sense of are skipped, so one bad entry never costs
    /// the whole language.
    pub fn parse(text: &str) -> Catalog {
        #[derive(PartialEq, Clone, Copy)]
        enum Field {
            None,
            Ctx,
            Id,
            IdPlural,
            Str(usize),
        }
        let mut cat = Catalog::default();
        let mut ctx = String::new();
        let mut id = String::new();
        let mut id_plural: Option<String> = None;
        let mut strings: Vec<String> = Vec::new();
        let mut field = Field::None;
        let mut have_entry = false;

        fn flush(
            cat: &mut Catalog,
            ctx: &mut String,
            id: &mut String,
            id_plural: &mut Option<String>,
            strings: &mut Vec<String>,
            have: &mut bool,
        ) {
            if *have {
                if id.is_empty() && ctx.is_empty() {
                    // the header: its Plural-Forms line says how this language counts
                    if let Some(h) = strings.first() {
                        for line in h.lines() {
                            if let Some(rest) = line.strip_prefix("Plural-Forms:") {
                                cat.plural = PluralRule::parse(rest);
                            }
                        }
                    }
                } else if strings.iter().any(|s| !s.is_empty()) {
                    let entry = Entry { plural_id: id_plural.clone(), strings: std::mem::take(strings) };
                    if let Some(p) = &entry.plural_id {
                        cat.by_plural.insert((ctx.clone(), p.clone()), id.clone());
                    }
                    cat.entries.insert((ctx.clone(), id.clone()), entry);
                }
            }
            ctx.clear();
            id.clear();
            *id_plural = None;
            strings.clear();
            *have = false;
        }

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() {
                flush(&mut cat, &mut ctx, &mut id, &mut id_plural, &mut strings, &mut have_entry);
                field = Field::None;
                continue;
            }
            // comments, flags, references and obsolete entries (#~) carry nothing we need
            if line.starts_with('#') {
                continue;
            }
            let (keyword, rest) = match line.find([' ', '\t']) {
                Some(i) if !line.starts_with('"') => (&line[..i], line[i..].trim_start()),
                _ => ("", line),
            };
            if keyword.is_empty() && line.starts_with('"') {
                // a continuation of the previous field
                let piece = unquote(line);
                match field {
                    Field::Ctx => ctx.push_str(&piece),
                    Field::Id => id.push_str(&piece),
                    Field::IdPlural => {
                        if let Some(p) = id_plural.as_mut() {
                            p.push_str(&piece)
                        }
                    }
                    Field::Str(i) => {
                        if let Some(s) = strings.get_mut(i) {
                            s.push_str(&piece)
                        }
                    }
                    Field::None => {}
                }
                continue;
            }
            match keyword {
                "msgctxt" => {
                    if have_entry && field != Field::Ctx && (!id.is_empty() || !strings.is_empty()) {
                        flush(&mut cat, &mut ctx, &mut id, &mut id_plural, &mut strings, &mut have_entry);
                    }
                    ctx = unquote(rest);
                    field = Field::Ctx;
                    have_entry = true;
                }
                "msgid" => {
                    if have_entry && !strings.is_empty() {
                        flush(&mut cat, &mut ctx, &mut id, &mut id_plural, &mut strings, &mut have_entry);
                    }
                    id = unquote(rest);
                    field = Field::Id;
                    have_entry = true;
                }
                "msgid_plural" => {
                    id_plural = Some(unquote(rest));
                    field = Field::IdPlural;
                }
                k if k == "msgstr" || k.starts_with("msgstr[") => {
                    let index = k
                        .strip_prefix("msgstr[")
                        .and_then(|r| r.strip_suffix(']'))
                        .and_then(|n| n.parse::<usize>().ok())
                        .unwrap_or(0);
                    if strings.len() <= index {
                        strings.resize(index + 1, String::new());
                    }
                    strings[index] = unquote(rest);
                    field = Field::Str(index);
                }
                _ => {}
            }
        }
        flush(&mut cat, &mut ctx, &mut id, &mut id_plural, &mut strings, &mut have_entry);
        cat
    }

    /// How many texts have a translation.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn singular(&self, ctx: &str, id: &str) -> Option<&str> {
        self.entries
            .get(&(ctx.to_string(), id.to_string()))
            .and_then(|e| e.strings.first())
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }

    fn plural_form(&self, ctx: &str, singular: &str, plural: &str, n: u64) -> Option<&str> {
        let key = self.entries.get(&(ctx.to_string(), singular.to_string())).or_else(|| {
            let sing = self.by_plural.get(&(ctx.to_string(), plural.to_string()))?;
            self.entries.get(&(ctx.to_string(), sing.clone()))
        })?;
        let nplurals = self.plural.as_ref().map(|p| p.nplurals).unwrap_or(2).max(1);
        let index = match &self.plural {
            Some(rule) => rule.index(n),
            None => (n != 1) as usize,
        };
        let index = index.min(nplurals - 1).min(key.strings.len().saturating_sub(1));
        key.strings.get(index).map(String::as_str).filter(|s| !s.is_empty())
    }
}

/// The text between a line's first and last double quote, with the `.po` escapes undone.
fn unquote(s: &str) -> String {
    let s = s.trim();
    let inner = s.strip_prefix('"').unwrap_or(s);
    let inner = inner.strip_suffix('"').unwrap_or(inner);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('a') => out.push('\u{7}'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('v') => out.push('\u{b}'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Plural rules: "nplurals=2; plural=(n != 1);"

#[derive(Debug, Clone)]
struct PluralRule {
    nplurals: usize,
    expr: Expr,
}

#[derive(Debug, Clone)]
enum Expr {
    N,
    Num(i64),
    Not(Box<Expr>),
    Bin(Box<Expr>, Op, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl PluralRule {
    fn parse(header_value: &str) -> Option<PluralRule> {
        let mut nplurals = 2usize;
        let mut expr_text = None;
        for part in header_value.split(';') {
            let part = part.trim();
            if let Some(v) = part.strip_prefix("nplurals=") {
                nplurals = v.trim().parse().unwrap_or(2);
            } else if let Some(v) = part.strip_prefix("plural=") {
                expr_text = Some(v.trim().to_string());
            }
        }
        let tokens = tokenize(&expr_text?)?;
        let mut p = Parser { tokens, pos: 0 };
        let expr = p.ternary()?;
        if p.pos != p.tokens.len() {
            return None;
        }
        Some(PluralRule { nplurals, expr })
    }

    fn index(&self, n: u64) -> usize {
        eval(&self.expr, n as i64).max(0) as usize
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    N,
    Num(i64),
    Op(Op),
    Not,
    Question,
    Colon,
    Open,
    Close,
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        let two: String = b[i..(i + 2).min(b.len())].iter().collect();
        match c {
            c if c.is_whitespace() => {
                i += 1;
                continue;
            }
            'n' => out.push(Tok::N),
            '0'..='9' => {
                let start = i;
                while i + 1 < b.len() && b[i + 1].is_ascii_digit() {
                    i += 1;
                }
                out.push(Tok::Num(b[start..=i].iter().collect::<String>().parse().ok()?));
            }
            '(' => out.push(Tok::Open),
            ')' => out.push(Tok::Close),
            '?' => out.push(Tok::Question),
            ':' => out.push(Tok::Colon),
            _ => {
                let (op, len) = match two.as_str() {
                    "||" => (Tok::Op(Op::Or), 2),
                    "&&" => (Tok::Op(Op::And), 2),
                    "==" => (Tok::Op(Op::Eq), 2),
                    "!=" => (Tok::Op(Op::Ne), 2),
                    "<=" => (Tok::Op(Op::Le), 2),
                    ">=" => (Tok::Op(Op::Ge), 2),
                    _ => match c {
                        '<' => (Tok::Op(Op::Lt), 1),
                        '>' => (Tok::Op(Op::Gt), 1),
                        '+' => (Tok::Op(Op::Add), 1),
                        '-' => (Tok::Op(Op::Sub), 1),
                        '*' => (Tok::Op(Op::Mul), 1),
                        '/' => (Tok::Op(Op::Div), 1),
                        '%' => (Tok::Op(Op::Rem), 1),
                        '!' => (Tok::Not, 1),
                        _ => return None,
                    },
                };
                out.push(op);
                i += len;
                continue;
            }
        }
        i += 1;
    }
    Some(out)
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn ternary(&mut self) -> Option<Expr> {
        let cond = self.binary(0)?;
        if self.peek() == Some(&Tok::Question) {
            self.pos += 1;
            let a = self.ternary()?;
            if self.peek() != Some(&Tok::Colon) {
                return None;
            }
            self.pos += 1;
            let b = self.ternary()?;
            return Some(Expr::Cond(Box::new(cond), Box::new(a), Box::new(b)));
        }
        Some(cond)
    }

    fn level(op: Op) -> u8 {
        match op {
            Op::Or => 0,
            Op::And => 1,
            Op::Eq | Op::Ne => 2,
            Op::Lt | Op::Gt | Op::Le | Op::Ge => 3,
            Op::Add | Op::Sub => 4,
            Op::Mul | Op::Div | Op::Rem => 5,
        }
    }

    fn binary(&mut self, min: u8) -> Option<Expr> {
        let mut left = self.unary()?;
        while let Some(Tok::Op(op)) = self.peek().cloned() {
            let lvl = Self::level(op);
            if lvl < min {
                break;
            }
            self.pos += 1;
            let right = self.binary(lvl + 1)?;
            left = Expr::Bin(Box::new(left), op, Box::new(right));
        }
        Some(left)
    }

    fn unary(&mut self) -> Option<Expr> {
        match self.peek()?.clone() {
            Tok::Not => {
                self.pos += 1;
                Some(Expr::Not(Box::new(self.unary()?)))
            }
            Tok::N => {
                self.pos += 1;
                Some(Expr::N)
            }
            Tok::Num(v) => {
                self.pos += 1;
                Some(Expr::Num(v))
            }
            Tok::Open => {
                self.pos += 1;
                let e = self.ternary()?;
                if self.peek() != Some(&Tok::Close) {
                    return None;
                }
                self.pos += 1;
                Some(e)
            }
            _ => None,
        }
    }
}

fn eval(e: &Expr, n: i64) -> i64 {
    match e {
        Expr::N => n,
        Expr::Num(v) => *v,
        Expr::Not(x) => (eval(x, n) == 0) as i64,
        Expr::Cond(c, a, b) => {
            if eval(c, n) != 0 {
                eval(a, n)
            } else {
                eval(b, n)
            }
        }
        Expr::Bin(a, op, b) => {
            let (x, y) = (eval(a, n), eval(b, n));
            match op {
                Op::Or => (x != 0 || y != 0) as i64,
                Op::And => (x != 0 && y != 0) as i64,
                Op::Eq => (x == y) as i64,
                Op::Ne => (x != y) as i64,
                Op::Lt => (x < y) as i64,
                Op::Gt => (x > y) as i64,
                Op::Le => (x <= y) as i64,
                Op::Ge => (x >= y) as i64,
                Op::Add => x.wrapping_add(y),
                Op::Sub => x.wrapping_sub(y),
                Op::Mul => x.wrapping_mul(y),
                Op::Div => x.checked_div(y).unwrap_or(0),
                Op::Rem => x.checked_rem(y).unwrap_or(0),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Arguments: Java's MessageFormat, as far as the launcher's texts use it

/// A value that fills a `{0}` in a text. Whole numbers get thousands separators like Java's `MessageFormat`.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Text(String),
    Int(i64),
    Float(f64),
}

impl From<&str> for Arg {
    fn from(v: &str) -> Arg {
        Arg::Text(v.to_string())
    }
}
impl From<&String> for Arg {
    fn from(v: &String) -> Arg {
        Arg::Text(v.clone())
    }
}
impl From<String> for Arg {
    fn from(v: String) -> Arg {
        Arg::Text(v)
    }
}
impl From<Cow<'_, str>> for Arg {
    fn from(v: Cow<'_, str>) -> Arg {
        Arg::Text(v.into_owned())
    }
}
macro_rules! arg_from_int {
    ($($t:ty),*) => {$(impl From<$t> for Arg { fn from(v: $t) -> Arg { Arg::Int(v as i64) } })*};
}
arg_from_int!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);
impl From<f64> for Arg {
    fn from(v: f64) -> Arg {
        Arg::Float(v)
    }
}
impl From<f32> for Arg {
    fn from(v: f32) -> Arg {
        Arg::Float(v as f64)
    }
}

/// Separators a language uses in numbers: (thousands, decimal).
fn number_separators(language: &str) -> (&'static str, &'static str) {
    match language {
        "de" | "es" | "it" | "nl" | "pt" | "da" | "el" | "ca" | "tr" | "ro" | "id" => (".", ","),
        "fr" => ("\u{202f}", ","),
        "ru" | "uk" | "pl" | "cs" | "fi" | "sv" | "no" | "hu" | "sr" | "af" => ("\u{a0}", ","),
        _ => (",", "."),
    }
}

fn group_digits(digits: &str, sep: &str) -> String {
    let n = digits.len();
    let mut out = String::with_capacity(n + n / 3 * sep.len());
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (n - i) % 3 == 0 {
            out.push_str(sep);
        }
        out.push(c);
    }
    out
}

/// `1234567` as "1,234,567" (or the way the language writes it).
pub fn format_number(value: i64, language: &str) -> String {
    let (sep, _) = number_separators(language);
    let digits = value.unsigned_abs().to_string();
    let grouped = group_digits(&digits, sep);
    if value < 0 { format!("-{grouped}") } else { grouped }
}

fn format_float(value: f64, language: &str) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let (group, decimal) = number_separators(language);
    // NumberFormat shows at most three decimals and drops trailing zeros
    let rounded = format!("{:.3}", value.abs());
    let (int_part, frac) = rounded.split_once('.').unwrap_or((&rounded, ""));
    let frac = frac.trim_end_matches('0');
    let mut out = group_digits(int_part, group);
    if !frac.is_empty() {
        out.push_str(decimal);
        out.push_str(frac);
    }
    if value < 0.0 && out.chars().any(|c| c.is_ascii_digit() && c != '0') {
        out.insert(0, '-');
    }
    out
}

/// Fills `{0}`, `{1}` ... in a text. `{0,number}` and `{0,number,integer}` format as numbers; a brace with no
/// matching partner, or an index with no argument, is left as it was written (Java would throw; showing the text
/// is kinder in a window).
pub fn format_message(pattern: &str, args: &[Arg], language: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '{' {
            out.push(c);
            i += 1;
            continue;
        }
        let Some(close) = chars[i + 1..].iter().position(|&x| x == '}').map(|p| p + i + 1) else {
            out.extend(&chars[i..]);
            break;
        };
        let inner: String = chars[i + 1..close].iter().collect();
        let mut parts = inner.splitn(3, ',');
        let index = parts.next().unwrap_or("").trim().parse::<usize>().ok();
        let kind = parts.next().map(|s| s.trim().to_ascii_lowercase());
        let style = parts.next().map(|s| s.trim().to_ascii_lowercase());
        match index.and_then(|n| args.get(n)) {
            Some(arg) => {
                let integer_only = style.as_deref() == Some("integer");
                let shown = match (arg, kind.as_deref()) {
                    (Arg::Text(t), _) => t.clone(),
                    (Arg::Int(v), _) => format_number(*v, language),
                    (Arg::Float(v), _) if integer_only => format_number(v.round() as i64, language),
                    (Arg::Float(v), _) => format_float(*v, language),
                };
                out.push_str(&shown);
            }
            None => out.extend(&chars[i..=close]),
        }
        i = close + 1;
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// The selected language

struct State {
    /// The text saved in the settings: "English", "German (Germany)"
    selected: String,
    tag: String,
    language: String,
    catalog: Option<Arc<Catalog>>,
    /// Languages with a translation file, as display names
    available: Vec<&'static LocaleInfo>,
    external_dir: Option<std::path::PathBuf>,
}

fn state() -> &'static RwLock<State> {
    static STATE: OnceLock<RwLock<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        RwLock::new(State {
            selected: DEFAULT_LANGUAGE.to_string(),
            tag: "en".to_string(),
            language: "en".to_string(),
            catalog: None,
            available: embedded_available(),
            external_dir: None,
        })
    })
}

/// Tests that change or depend on the language take this, since the language is shared by the whole program.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Bumped every time the language changes, so a window can notice and redraw its texts.
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

// The translation files compiled in; build.rs writes this list from assets/lang
include!(concat!(env!("OUT_DIR"), "/embedded_languages.rs"));

fn embedded_available() -> Vec<&'static LocaleInfo> {
    LOCALES.iter().filter(|l| !l.country.is_empty() && embedded_po(&l.tag()).is_some()).collect()
}

fn embedded_po(tag: &str) -> Option<&'static [u8]> {
    EMBEDDED_LANGUAGES.iter().find(|(t, _)| *t == tag).map(|(_, bytes)| *bytes)
}

fn external_po(dir: &Path, tag: &str) -> Option<Vec<u8>> {
    std::fs::read(dir.join(format!("{tag}.po"))).ok()
}

/// Looks for translation files (Java's `Language.init`): the ones compiled in, and any `<tag>.po` in `external_dir`.
/// Call once at start, before setting the language.
pub fn init(external_dir: Option<&Path>) {
    let mut st = state().write().unwrap_or_else(|e| e.into_inner());
    let mut found = embedded_available();
    if let Some(dir) = external_dir {
        for l in LOCALES.iter().filter(|l| !l.country.is_empty()) {
            if !found.contains(&l) && dir.join(format!("{}.po", l.tag())).is_file() {
                found.push(l);
            }
        }
        st.external_dir = Some(dir.to_path_buf());
    }
    // keep the Java edition's order
    found.sort_by_key(|l| LOCALES.iter().position(|x| x == *l));
    st.available = found;
}

/// The languages to offer in a list: English, then every language with a translation file. These are the texts
/// that `set_language` and the settings file use.
pub fn languages() -> Vec<String> {
    let st = state().read().unwrap_or_else(|e| e.into_inner());
    let mut out = vec![DEFAULT_LANGUAGE.to_string()];
    out.extend(st.available.iter().map(|l| l.display_name.to_string()));
    out
}

/// If this name is a language the launcher can switch to.
pub fn is_language(name: &str) -> bool {
    name == DEFAULT_LANGUAGE
        || state().read().unwrap_or_else(|e| e.into_inner()).available.iter().any(|l| l.display_name == name)
}

/// The language in use, as saved in the settings.
pub fn selected() -> String {
    state().read().unwrap_or_else(|e| e.into_inner()).selected.clone()
}

/// The language in use as a locale (for fonts and number formats).
pub fn selected_locale() -> &'static LocaleInfo {
    let name = selected();
    find_locale(&name).unwrap_or(&LOCALES[0])
}

/// Switches the language (Java's `Language.setLanguage`). An unknown name, or a language whose file cannot be read,
/// falls back to English. Returns the language that is now in use.
pub fn set_language(name: &str) -> String {
    {
        let st = state().read().unwrap_or_else(|e| e.into_inner());
        if st.selected == name {
            return st.selected.clone();
        }
    }
    let mut st = state().write().unwrap_or_else(|e| e.into_inner());
    let info = st.available.iter().copied().find(|l| l.display_name == name);
    let (selected, tag, language, catalog) = match info {
        Some(info) => {
            let tag = info.tag();
            let bytes = embedded_po(&tag)
                .map(|b| b.to_vec())
                .or_else(|| st.external_dir.as_deref().and_then(|d| external_po(d, &tag)));
            match bytes {
                Some(b) => (
                    info.display_name.to_string(),
                    tag,
                    info.language.to_string(),
                    Some(Arc::new(Catalog::parse(&String::from_utf8_lossy(&b)))),
                ),
                None => (DEFAULT_LANGUAGE.to_string(), "en".into(), "en".into(), None),
            }
        }
        None => (DEFAULT_LANGUAGE.to_string(), "en".into(), "en".into(), None),
    };
    st.selected = selected.clone();
    st.tag = tag;
    st.language = language;
    st.catalog = catalog;
    GENERATION.fetch_add(1, Ordering::Relaxed);
    selected
}

/// Uses a catalog directly (for tests and for a translation tool that previews a file).
pub fn set_catalog(display_name: &str, catalog: Option<Catalog>) {
    let mut st = state().write().unwrap_or_else(|e| e.into_inner());
    let info = find_locale(display_name);
    st.selected = info.map(|l| l.display_name).unwrap_or(DEFAULT_LANGUAGE).to_string();
    st.language = info.map(|l| l.language).unwrap_or("en").to_string();
    st.tag = info.map(|l| l.tag()).unwrap_or_else(|| "en".into());
    st.catalog = catalog.map(Arc::new);
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

fn snapshot() -> (Option<Arc<Catalog>>, String) {
    let st = state().read().unwrap_or_else(|e| e.into_inner());
    (st.catalog.clone(), st.language.clone())
}

/// A text in the selected language, or as written when there is no translation.
pub fn tr(text: &str) -> String {
    let (cat, _) = snapshot();
    match cat.as_deref().and_then(|c| c.singular("", text)) {
        Some(t) => t.to_string(),
        None => text.to_string(),
    }
}

/// `tr` with `{0}`, `{1}` ... filled in from `args`.
pub fn tr_args(text: &str, args: &[Arg]) -> String {
    let (cat, lang) = snapshot();
    let base = cat.as_deref().and_then(|c| c.singular("", text)).unwrap_or(text);
    format_message(base, args, &lang)
}

/// A text told apart from others with the same English by a context ("gettext's `pgettext`").
pub fn trc(context: &str, text: &str) -> String {
    let (cat, _) = snapshot();
    cat.as_deref().and_then(|c| c.singular(context, text)).unwrap_or(text).to_string()
}

/// The singular or plural form for `n` (`tr_n("{0} mod", "{0} mods", 3, &[3.into()])`). With no translation it is
/// the English rule: one is singular, everything else plural.
pub fn tr_n(singular: &str, plural: &str, n: u64, args: &[Arg]) -> String {
    trnc("", singular, plural, n, args)
}

/// `tr_n` with a context.
pub fn trnc(context: &str, singular: &str, plural: &str, n: u64, args: &[Arg]) -> String {
    let (cat, lang) = snapshot();
    let base = cat.as_deref().and_then(|c| c.plural_form(context, singular, plural, n)).unwrap_or(if n == 1 {
        singular
    } else {
        plural
    });
    format_message(base, args, &lang)
}

/// `tr!("Text {0}", value)`: the translated text with the values filled in.
#[macro_export]
macro_rules! tr {
    ($text:expr) => {
        $crate::i18n::tr($text)
    };
    ($text:expr, $($arg:expr),+ $(,)?) => {
        $crate::i18n::tr_args($text, &[$($crate::i18n::Arg::from($arg)),+])
    };
}

// ---------------------------------------------------------------------------------------------------------------
// Settings: language, date format, instance title format

/// The date formats the settings offer (`Constants.DATE_FORMATS`); the first is the default.
pub const DATE_FORMATS: [&str; 7] =
    ["dd/MM/yyyy", "MM/dd/yyyy", "yyyy/MM/dd", "dd MMMM yyyy", "dd-MM-yyyy", "MM-dd-yyyy", "yyyy-MM-dd"];

/// The instance title formats (`Constants.INSTANCE_TITLE_FORMATS`), written for Java's `String.format`: `%1$s` is
/// the instance name, `%2$s` the pack name, `%3$s` the pack version and `%4$s` the Minecraft version.
pub const INSTANCE_TITLE_FORMATS: [&str; 4] = ["%1$s (%2$s %3$s)", "%1$s", "%1$s (%4$s)", "%1$s (%3$s)"];

/// What the settings file says about the language, falling back to English.
pub fn language_setting(settings: &Settings) -> String {
    settings.extras.get("language").and_then(|v| v.as_str()).unwrap_or(DEFAULT_LANGUAGE).to_string()
}

/// Date format from the settings, or the default when it is not one of the offered ones (Java's `validateDateFormat`).
pub fn date_format_setting(settings: &Settings) -> String {
    let v = settings.extras.get("dateFormat").and_then(|v| v.as_str()).unwrap_or(DATE_FORMATS[0]);
    if DATE_FORMATS.contains(&v) { v.to_string() } else { DATE_FORMATS[0].to_string() }
}

/// Title format from the settings. The old `showPackNameAndVersion` yes/no setting still wins if a very old
/// settings file has it (Java's `validateInstanceTitleFormat`).
pub fn title_format_setting(settings: &Settings) -> String {
    if let Some(show) = settings.extras.get("showPackNameAndVersion").and_then(|v| v.as_bool()) {
        return INSTANCE_TITLE_FORMATS[if show { 0 } else { 1 }].to_string();
    }
    let v = settings.extras.get("instanceTitleFormat").and_then(|v| v.as_str()).unwrap_or(INSTANCE_TITLE_FORMATS[0]);
    if INSTANCE_TITLE_FORMATS.contains(&v) { v.to_string() } else { INSTANCE_TITLE_FORMATS[0].to_string() }
}

/// Saves a language choice into the settings and switches to it. The caller saves the settings.
pub fn choose_language(settings: &mut Settings, name: &str) {
    let now = set_language(name);
    settings.extras.insert("language".into(), serde_json::Value::String(now));
}

/// Saves a date format choice (ignored when it is not one of `DATE_FORMATS`). The caller saves the settings.
pub fn choose_date_format(settings: &mut Settings, format: &str) {
    if DATE_FORMATS.contains(&format) {
        settings.extras.insert("dateFormat".into(), serde_json::Value::String(format.to_string()));
    }
}

/// Saves an instance title format choice (ignored when it is not one of `INSTANCE_TITLE_FORMATS`).
pub fn choose_title_format(settings: &mut Settings, format: &str) {
    if INSTANCE_TITLE_FORMATS.contains(&format) {
        settings.extras.insert("instanceTitleFormat".into(), serde_json::Value::String(format.to_string()));
        settings.extras.remove("showPackNameAndVersion");
    }
}

/// Starts the translations from the settings and the data folder. One call at start does it all.
pub fn start(settings: &Settings, dirs: &crate::paths::Dirs) {
    init(Some(&dirs.base.join("lang")));
    set_language(&language_setting(settings));
}

// ---------------------------------------------------------------------------------------------------------------
// Dates, the way Java's SimpleDateFormat and Joda write them

const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS_EN: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// Month names for the languages the launcher has them for; None means use English.
fn months_for(language: &str) -> Option<[&'static str; 12]> {
    Some(match language {
        "de" => [
            "Januar",
            "Februar",
            "M\u{e4}rz",
            "April",
            "Mai",
            "Juni",
            "Juli",
            "August",
            "September",
            "Oktober",
            "November",
            "Dezember",
        ],
        "fr" => [
            "janvier",
            "f\u{e9}vrier",
            "mars",
            "avril",
            "mai",
            "juin",
            "juillet",
            "ao\u{fb}t",
            "septembre",
            "octobre",
            "novembre",
            "d\u{e9}cembre",
        ],
        "es" => [
            "enero",
            "febrero",
            "marzo",
            "abril",
            "mayo",
            "junio",
            "julio",
            "agosto",
            "septiembre",
            "octubre",
            "noviembre",
            "diciembre",
        ],
        "it" => [
            "gennaio",
            "febbraio",
            "marzo",
            "aprile",
            "maggio",
            "giugno",
            "luglio",
            "agosto",
            "settembre",
            "ottobre",
            "novembre",
            "dicembre",
        ],
        "pt" => [
            "janeiro",
            "fevereiro",
            "mar\u{e7}o",
            "abril",
            "maio",
            "junho",
            "julho",
            "agosto",
            "setembro",
            "outubro",
            "novembro",
            "dezembro",
        ],
        "nl" => [
            "januari",
            "februari",
            "maart",
            "april",
            "mei",
            "juni",
            "juli",
            "augustus",
            "september",
            "oktober",
            "november",
            "december",
        ],
        "sv" => [
            "januari",
            "februari",
            "mars",
            "april",
            "maj",
            "juni",
            "juli",
            "augusti",
            "september",
            "oktober",
            "november",
            "december",
        ],
        "da" | "no" => [
            "januar",
            "februar",
            "mars",
            "april",
            "mai",
            "juni",
            "juli",
            "august",
            "september",
            "oktober",
            "november",
            "desember",
        ],
        "fi" => [
            "tammikuuta",
            "helmikuuta",
            "maaliskuuta",
            "huhtikuuta",
            "toukokuuta",
            "kes\u{e4}kuuta",
            "hein\u{e4}kuuta",
            "elokuuta",
            "syyskuuta",
            "lokakuuta",
            "marraskuuta",
            "joulukuuta",
        ],
        "pl" => [
            "stycznia",
            "lutego",
            "marca",
            "kwietnia",
            "maja",
            "czerwca",
            "lipca",
            "sierpnia",
            "wrze\u{15b}nia",
            "pa\u{17a}dziernika",
            "listopada",
            "grudnia",
        ],
        "cs" => [
            "ledna",
            "\u{fa}nora",
            "b\u{159}ezna",
            "dubna",
            "kv\u{11b}tna",
            "\u{10d}ervna",
            "\u{10d}ervence",
            "srpna",
            "z\u{e1}\u{159}\u{ed}",
            "\u{159}\u{ed}jna",
            "listopadu",
            "prosince",
        ],
        "ro" => [
            "ianuarie",
            "februarie",
            "martie",
            "aprilie",
            "mai",
            "iunie",
            "iulie",
            "august",
            "septembrie",
            "octombrie",
            "noiembrie",
            "decembrie",
        ],
        "ca" => [
            "de gener",
            "de febrer",
            "de mar\u{e7}",
            "d\u{2019}abril",
            "de maig",
            "de juny",
            "de juliol",
            "d\u{2019}agost",
            "de setembre",
            "d\u{2019}octubre",
            "de novembre",
            "de desembre",
        ],
        "tr" => [
            "Ocak",
            "\u{15e}ubat",
            "Mart",
            "Nisan",
            "May\u{131}s",
            "Haziran",
            "Temmuz",
            "A\u{11f}ustos",
            "Eyl\u{fc}l",
            "Ekim",
            "Kas\u{131}m",
            "Aral\u{131}k",
        ],
        "hu" => [
            "janu\u{e1}r",
            "febru\u{e1}r",
            "m\u{e1}rcius",
            "\u{e1}prilis",
            "m\u{e1}jus",
            "j\u{fa}nius",
            "j\u{fa}lius",
            "augusztus",
            "szeptember",
            "okt\u{f3}ber",
            "november",
            "december",
        ],
        "ru" => [
            "\u{44f}\u{43d}\u{432}\u{430}\u{440}\u{44f}",
            "\u{444}\u{435}\u{432}\u{440}\u{430}\u{43b}\u{44f}",
            "\u{43c}\u{430}\u{440}\u{442}\u{430}",
            "\u{430}\u{43f}\u{440}\u{435}\u{43b}\u{44f}",
            "\u{43c}\u{430}\u{44f}",
            "\u{438}\u{44e}\u{43d}\u{44f}",
            "\u{438}\u{44e}\u{43b}\u{44f}",
            "\u{430}\u{432}\u{433}\u{443}\u{441}\u{442}\u{430}",
            "\u{441}\u{435}\u{43d}\u{442}\u{44f}\u{431}\u{440}\u{44f}",
            "\u{43e}\u{43a}\u{442}\u{44f}\u{431}\u{440}\u{44f}",
            "\u{43d}\u{43e}\u{44f}\u{431}\u{440}\u{44f}",
            "\u{434}\u{435}\u{43a}\u{430}\u{431}\u{440}\u{44f}",
        ],
        _ => return None,
    })
}

/// The language code Java would take from the operating system (`LC_ALL`, `LC_TIME`, `LANG`), "en" if none.
/// Java writes month names in the system's language, not in the language picked in the launcher.
pub fn system_language() -> String {
    for var in ["LC_ALL", "LC_TIME", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            let code: String =
                v.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_lowercase();
            if !code.is_empty() && code != "c" && code != "posix" {
                return code;
            }
        }
    }
    "en".to_string()
}

/// Writes a date in a Java pattern such as `dd/MM/yyyy` or `dd MMMM yyyy HH:mm:ss a`, with month and day names
/// in `language` (English when the launcher has no names for it). Quoted text (`'at'`) is kept as written.
pub fn format_date_in(pattern: &str, when: &NaiveDateTime, language: &str) -> String {
    let months = months_for(language).unwrap_or(MONTHS_EN);
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // 'text' is literal, '' is a single quote
            if chars.get(i + 1) == Some(&'\'') {
                out.push('\'');
                i += 2;
                continue;
            }
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        if !c.is_ascii_alphabetic() {
            out.push(c);
            i += 1;
            continue;
        }
        let mut count = 1;
        while chars.get(i + count) == Some(&c) {
            count += 1;
        }
        i += count;
        let pad = |v: i64| format!("{:0width$}", v, width = count);
        match c {
            'y' | 'Y' | 'u' => {
                let year = when.year() as i64;
                if count == 2 {
                    out.push_str(&format!("{:02}", year.rem_euclid(100)));
                } else {
                    out.push_str(&pad(year));
                }
            }
            'M' | 'L' => match count {
                1 | 2 => out.push_str(&pad(when.month() as i64)),
                3 => out.push_str(&short_name(months[when.month0() as usize])),
                _ => out.push_str(months[when.month0() as usize]),
            },
            'd' => out.push_str(&pad(when.day() as i64)),
            'H' => out.push_str(&pad(when.hour() as i64)),
            'h' => out.push_str(&pad(match when.hour() % 12 {
                0 => 12,
                h => h as i64,
            })),
            'm' => out.push_str(&pad(when.minute() as i64)),
            's' => out.push_str(&pad(when.second() as i64)),
            'S' => out.push_str(
                &format!("{:0width$}", when.nanosecond() / 1_000_000, width = count.max(3))
                    .chars()
                    .take(count)
                    .collect::<String>(),
            ),
            'a' => out.push_str(if when.hour() < 12 { "AM" } else { "PM" }),
            'E' => {
                let name = DAYS_EN[when.weekday().num_days_from_monday() as usize];
                out.push_str(if count >= 4 { name } else { &name[..3] });
            }
            other => out.extend(std::iter::repeat_n(other, count)),
        }
    }
    out
}

fn short_name(month: &str) -> String {
    month.chars().take(3).collect()
}

/// A date in the pattern chosen in the settings, with month names in the system's language like Java.
pub fn format_date(pattern: &str, when: &NaiveDateTime) -> String {
    format_date_in(pattern, when, &system_language())
}

/// The release time Mojang's lists give (`2024-06-13T08:32:38+00:00`) written in the chosen pattern, as the version
/// list of the create page shows it. None when the text is not a date.
pub fn format_iso_date(pattern: &str, iso: &str) -> Option<String> {
    let when = chrono::DateTime::parse_from_rfc3339(iso)
        .map(|d| d.with_timezone(&chrono::Local).naive_local())
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(iso.get(..19)?, "%Y-%m-%dT%H:%M:%S").ok())?;
    Some(format_date(pattern, &when))
}

/// The stamp in front of the launcher's log lines: the date format and then `HH:mm:ss a` (Java's `Timestamper`).
pub fn log_timestamp(pattern: &str, when: &NaiveDateTime) -> String {
    format_date(&format!("{pattern} HH:mm:ss a"), when)
}

/// What each date format would show, for the settings list. Like the Java settings page it uses the 31st of
/// December of this year, which tells day-first and month-first formats apart.
pub fn date_format_examples() -> Vec<(String, String)> {
    let year = chrono::Local::now().year();
    let sample = chrono::NaiveDate::from_ymd_opt(year, 12, 31).and_then(|d| d.and_hms_opt(0, 0, 0)).unwrap_or_default();
    DATE_FORMATS.iter().map(|f| (f.to_string(), format_date(f, &sample))).collect()
}

/// An instance's title as the chosen format writes it. Java's `String.format(format, name, pack, packVersion,
/// minecraftVersion)`: `%1$s` ... `%4$s` pick an argument, `%s` takes them in order, `%%` is a percent sign.
pub fn instance_title(format: &str, name: &str, pack: &str, pack_version: &str, minecraft_version: &str) -> String {
    let args = [name, pack, pack_version, minecraft_version];
    let chars: Vec<char> = format.chars().collect();
    let mut out = String::new();
    let mut next = 0usize;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        // %%
        if chars.get(i + 1) == Some(&'%') {
            out.push('%');
            i += 2;
            continue;
        }
        // %<index>$s or %s
        let mut j = i + 1;
        let digits_start = j;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        let explicit = if j > digits_start && chars.get(j) == Some(&'$') {
            let n: usize = chars[digits_start..j].iter().collect::<String>().parse().unwrap_or(0);
            j += 1;
            Some(n)
        } else {
            j = digits_start;
            None
        };
        if chars.get(j) == Some(&'s') {
            let index = match explicit {
                Some(n) => n.saturating_sub(1),
                None => {
                    next += 1;
                    next - 1
                }
            };
            out.push_str(args.get(index).copied().unwrap_or("null"));
            i = j + 1;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_PO: &str = r#"# German
msgid ""
msgstr ""
"Content-Type: text/plain; charset=UTF-8\n"
"Plural-Forms: nplurals=2; plural=(n != 1);\n"

#: src/Foo.java:12
msgid "Play"
msgstr "Spielen"

msgid "Downloading {0}"
msgstr "{0} wird heruntergeladen"

msgctxt "menu"
msgid "Open"
msgstr "Öffnen"

msgid "Open"
msgstr "Offen"

#, fuzzy
msgid "Delete \"{0}\"?"
msgstr "\"{0}\" wirklich löschen?"

msgid "A long text that is "
"split over lines"
msgstr "Ein langer Text, "
"der über Zeilen geht\n"

msgid "{0} mod"
msgid_plural "{0} mods"
msgstr[0] "{0} Mod"
msgstr[1] "{0} Mods"

msgid "Not translated yet"
msgstr ""

#~ msgid "Gone"
#~ msgstr "Weg"
"#;

    /// The language is global, so tests that use it run one at a time (other modules' tests that check English
    /// texts take the same lock).
    use super::TEST_LOCK as LOCK;

    fn with_german<T>(f: impl FnOnce() -> T) -> T {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_catalog("German (Germany)", Some(Catalog::parse(SAMPLE_PO)));
        let out = f();
        set_catalog(DEFAULT_LANGUAGE, None);
        out
    }

    #[test]
    fn parses_entries_contexts_and_continuations() {
        let c = Catalog::parse(SAMPLE_PO);
        assert_eq!(c.singular("", "Play"), Some("Spielen"));
        assert_eq!(c.singular("menu", "Open"), Some("Öffnen"));
        assert_eq!(c.singular("", "Open"), Some("Offen"));
        assert_eq!(c.singular("", "Delete \"{0}\"?"), Some("\"{0}\" wirklich löschen?"));
        assert_eq!(
            c.singular("", "A long text that is split over lines"),
            Some("Ein langer Text, der über Zeilen geht\n")
        );
        // untranslated and obsolete entries do not count
        assert_eq!(c.singular("", "Not translated yet"), None);
        assert_eq!(c.singular("", "Gone"), None);
        assert_eq!(c.len(), 7);
    }

    #[test]
    fn translates_and_falls_back_to_english() {
        with_german(|| {
            assert_eq!(tr("Play"), "Spielen");
            assert_eq!(tr("Stop"), "Stop");
            assert_eq!(trc("menu", "Open"), "Öffnen");
            assert_eq!(tr_args("Downloading {0}", &[Arg::from("Fabric")]), "Fabric wird heruntergeladen");
            assert_eq!(tr!("Downloading {0}", "Quilt"), "Quilt wird heruntergeladen");
            assert_eq!(tr!("Not translated yet"), "Not translated yet");
        });
    }

    #[test]
    fn plurals_follow_the_header_rule() {
        with_german(|| {
            assert_eq!(tr_n("{0} mod", "{0} mods", 1, &[1.into()]), "1 Mod");
            assert_eq!(tr_n("{0} mod", "{0} mods", 5, &[5.into()]), "5 Mods");
            assert_eq!(tr_n("{0} mod", "{0} mods", 1200, &[1200.into()]), "1.200 Mods");
        });
        // English, no catalog
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(tr_n("{0} mod", "{0} mods", 1, &[1.into()]), "1 mod");
        assert_eq!(tr_n("{0} mod", "{0} mods", 0, &[0.into()]), "0 mods");
    }

    /// Reads a real file when `BF_PO_FILE` names one: `BF_PO_FILE=de-DE.po cargo test --lib reads_a_real -- --ignored`.
    #[test]
    #[ignore]
    fn reads_a_real_po_file() {
        let Some(path) = std::env::var_os("BF_PO_FILE") else { return };
        let c = Catalog::parse(&std::fs::read_to_string(path).unwrap());
        eprintln!("{} translated texts, plural rule {:?}", c.len(), c.plural.as_ref().map(|p| p.nplurals));
        assert!(c.len() > 100);
        assert!(c.plural.is_some());
    }

    #[test]
    fn plural_expressions() {
        let slavic = PluralRule::parse(
            " nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);",
        )
        .unwrap();
        assert_eq!(slavic.nplurals, 3);
        assert_eq!([1, 2, 5, 11, 21, 22, 25, 111].map(|n| slavic.index(n)), [0, 1, 2, 2, 0, 1, 2, 2]);
        let none = PluralRule::parse("nplurals=1; plural=0;").unwrap();
        assert_eq!(none.index(7), 0);
        let french = PluralRule::parse("nplurals=2; plural=(n > 1);").unwrap();
        assert_eq!([0, 1, 2].map(|n| french.index(n)), [0, 0, 1]);
        assert!(PluralRule::parse("nplurals=2; plural=(n ! 1);").is_none());
    }

    #[test]
    fn message_format_like_java() {
        let en = "en";
        assert_eq!(format_message("{0} of {1}", &["a".into(), 2.into()], en), "a of 2");
        assert_eq!(format_message("Memory: {0} MB", &[4096.into()], en), "Memory: 4,096 MB");
        assert_eq!(format_message("Memory: {0} MB", &[4096.into()], "de"), "Memory: 4.096 MB");
        assert_eq!(format_message("{0}", &[(-1234567).into()], en), "-1,234,567");
        assert_eq!(format_message("{0}", &[2.5.into()], en), "2.5");
        assert_eq!(format_message("{0}", &[2.0.into()], en), "2");
        assert_eq!(format_message("{0,number,integer}", &[2.6.into()], en), "3");
        // apostrophes stay (the Java edition doubles them before formatting), unknown or open braces stay
        assert_eq!(format_message("Couldn't load {0}", &["x".into()], en), "Couldn't load x");
        assert_eq!(format_message("{1} and {0}", &["a".into()], en), "{1} and a");
        assert_eq!(format_message("open {0", &["a".into()], en), "open {0");
        // the same argument twice
        assert_eq!(format_message("{0}/{0}", &["z".into()], en), "z/z");
    }

    #[test]
    fn language_names_match_the_java_edition() {
        assert_eq!(find_locale("English").unwrap().language, "en");
        assert_eq!(find_locale("German (Germany)").unwrap().tag(), "de-DE");
        assert_eq!(find_locale("pt_BR").unwrap().display_name, "Portuguese (Brazil)");
        assert!(find_locale("Klingon").is_none());
        assert_eq!(LOCALES.len(), 29);
        assert!(LOCALES.iter().filter(|l| l.needs_system_font).count() == 6);
        assert!(LOCALES.iter().filter(|l| l.needs_system_tab_font).count() == 7);
        // nothing is shipped, so the list is English alone until files are added
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let list = languages();
        assert_eq!(list[0], "English");
        assert_eq!(set_language("Klingon"), "English");
        assert!(!is_language("German (Germany)") || list.len() > 1);
    }

    #[test]
    fn dates_are_written_like_java() {
        let when = chrono::NaiveDate::from_ymd_opt(2026, 12, 31).unwrap().and_hms_opt(15, 4, 9).unwrap();
        let en = |p: &str| format_date_in(p, &when, "en");
        assert_eq!(en("dd/MM/yyyy"), "31/12/2026");
        assert_eq!(en("MM/dd/yyyy"), "12/31/2026");
        assert_eq!(en("yyyy/MM/dd"), "2026/12/31");
        assert_eq!(en("dd MMMM yyyy"), "31 December 2026");
        assert_eq!(en("dd-MM-yyyy"), "31-12-2026");
        assert_eq!(en("MM-dd-yyyy"), "12-31-2026");
        assert_eq!(en("yyyy-MM-dd"), "2026-12-31");
        assert_eq!(en("dd/MM/yyyy HH:mm:ss a"), "31/12/2026 15:04:09 PM");
        assert_eq!(en("d MMM yy, h:mm a 'at' EEE"), "31 Dec 26, 3:04 PM at Thu");
        assert_eq!(format_date_in("dd MMMM yyyy", &when, "de"), "31 Dezember 2026");
        assert_eq!(format_date_in("dd MMMM yyyy", &when, "xx"), "31 December 2026");
        assert_eq!(date_format_examples().len(), 7);
        assert_eq!(log_timestamp("yyyy-MM-dd", &when).len(), "2026-12-31 15:04:09 PM".len());
    }

    #[test]
    fn mojang_release_times_are_formatted() {
        let out = format_iso_date("yyyy-MM-dd", "2024-06-13T12:00:00+00:00").unwrap();
        assert!(out == "2024-06-13" || out == "2024-06-14" || out == "2024-06-12", "{out}");
        assert!(format_iso_date("yyyy", "not a date").is_none());
    }

    #[test]
    fn instance_titles() {
        let t = |f: &str| instance_title(f, "My World", "Vanilla", "1.2", "1.21.1");
        assert_eq!(t(INSTANCE_TITLE_FORMATS[0]), "My World (Vanilla 1.2)");
        assert_eq!(t(INSTANCE_TITLE_FORMATS[1]), "My World");
        assert_eq!(t(INSTANCE_TITLE_FORMATS[2]), "My World (1.21.1)");
        assert_eq!(t(INSTANCE_TITLE_FORMATS[3]), "My World (1.2)");
        assert_eq!(t("%s - %s 100%%"), "My World - Vanilla 100%");
    }

    #[test]
    fn settings_helpers_validate_like_java() {
        let mut s = Settings::default();
        assert_eq!(language_setting(&s), "English");
        assert_eq!(date_format_setting(&s), "dd/MM/yyyy");
        assert_eq!(title_format_setting(&s), "%1$s (%2$s %3$s)");
        s.extras.insert("dateFormat".into(), "dd.MM.yy".into());
        s.extras.insert("instanceTitleFormat".into(), "weird".into());
        assert_eq!(date_format_setting(&s), "dd/MM/yyyy");
        assert_eq!(title_format_setting(&s), "%1$s (%2$s %3$s)");
        choose_date_format(&mut s, "yyyy-MM-dd");
        choose_title_format(&mut s, "%1$s");
        assert_eq!(date_format_setting(&s), "yyyy-MM-dd");
        assert_eq!(title_format_setting(&s), "%1$s");
        choose_date_format(&mut s, "nonsense");
        assert_eq!(date_format_setting(&s), "yyyy-MM-dd");
        // the old yes/no setting wins until a title format is chosen
        s.extras.insert("showPackNameAndVersion".into(), false.into());
        assert_eq!(title_format_setting(&s), "%1$s");
        s.extras.insert("showPackNameAndVersion".into(), true.into());
        assert_eq!(title_format_setting(&s), "%1$s (%2$s %3$s)");
    }
}
