//! Low-end mode: lowers the game's graphics settings (`options.txt`) on a slow computer, and puts them back when it is
//! switched off.
//!
//! * It only ever lowers: a value the player already set lower stays, and only the keys this Minecraft version knows
//!   are written (the table in [`targets`], checked against each version's options class).
//! * It is undone exactly: `blockforge-lowend.json` in the instance folder keeps, for each key, the value it had and
//!   the value written. Switched off, a key goes back only while it still has the written value; a key the player
//!   changed since is theirs and stays as it is. A key the player changed while the mode was on is never touched again.
//! * Every other line of `options.txt` is kept byte for byte, with its own line ending.
//! * It never stops a start: a read-only file (players lock it on purpose so packs don't reset it), a file another
//!   program holds, or any other problem is written to the log and the game starts with the file as it is.
//!
//! `auto` (the default) turns it on when [`looks_weak`] says the computer is slow; `on` and `off` force it. Each
//! instance can have its own `lowEndMode`, which wins over the launcher's.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Settings;
use crate::instances::Instance;
use crate::launch::memory::{self, Gpu};

/// The record of what Low-end mode changed, in the instance folder.
pub const BACKUP_FILE: &str = "blockforge-lowend.json";

/// The three choices of the setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// On when the computer looks slow.
    Auto,
    On,
    Off,
}

impl Mode {
    /// From the setting's text; anything unknown is automatic.
    pub fn parse(text: &str) -> Mode {
        match text.trim().to_ascii_lowercase().as_str() {
            "on" | "true" => Mode::On,
            "off" | "false" => Mode::Off,
            _ => Mode::Auto,
        }
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::On => "on",
            Mode::Off => "off",
        }
    }

    /// The word shown in the window and the log.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Auto => "Automatic",
            Mode::On => "On",
            Mode::Off => "Off",
        }
    }

    /// Whether the settings are lowered, for a computer that does or doesn't look slow.
    pub fn active(self, weak: bool) -> bool {
        match self {
            Mode::Auto => weak,
            Mode::On => true,
            Mode::Off => false,
        }
    }
}

/// The mode for an instance: its own `lowEndMode` if it has one, otherwise the launcher-wide setting.
pub fn mode(settings: &Settings, instance: &Instance) -> Mode {
    instance_override(instance).unwrap_or_else(|| Mode::parse(&settings.low_end_mode))
}

/// The instance's own choice, if it has one.
pub fn instance_override(instance: &Instance) -> Option<Mode> {
    instance.file.launcher.extras.get("lowEndMode").and_then(|v| v.as_str()).map(Mode::parse)
}

// ---- is this a slow computer?

/// What matters for telling a slow computer. Unknown numbers are 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Computer {
    /// Logical processors (threads).
    pub cores: usize,
    pub total_mb: i64,
    pub gpu: Gpu,
    /// The game's files are on a spinning hard disk.
    pub spinning_disk: bool,
}

/// Memory at or under this (MB) counts as "6 GB or less": a 6 GB computer reports a little under 6 GB.
pub const SMALL_MEMORY_MB: i64 = 6656;
/// "8 GB or less", the same way.
pub const MODEST_MEMORY_MB: i64 = 8704;
/// "4 processor threads or fewer".
pub const FEW_CORES: usize = 4;
/// Integrated graphics with this many threads or fewer count as slow too.
pub const IGPU_CORES: usize = 6;

/// Whether the computer looks too slow for Minecraft's usual settings, and the reason in plain words. One rule, the
/// first that fits:
///
/// 1. 4 processor threads or fewer: the game's main thread, chunk building and Java's own threads compete for them;
/// 2. 6 GB of memory or less: the system, the launcher and a game with a bigger view don't fit together;
/// 3. integrated graphics only, with 8 GB of memory or less or 6 threads or fewer: the graphics share the memory and
///    the power budget with the processor, which is what a budget laptop has;
/// 4. a spinning hard disk with 8 GB of memory or less: chunks and textures load slowly and the game stutters while
///    the system swaps.
pub fn looks_weak(c: &Computer) -> Option<String> {
    if c.cores > 0 && c.cores <= FEW_CORES {
        return Some(format!("{} processor threads", c.cores));
    }
    let gb = |mb: i64| format!("{:.0} GB", mb as f64 / 1024.0);
    if c.total_mb > 0 && c.total_mb <= SMALL_MEMORY_MB {
        return Some(format!("{} of memory", gb(c.total_mb)));
    }
    if c.gpu == Gpu::Integrated
        && ((c.total_mb > 0 && c.total_mb <= MODEST_MEMORY_MB) || (c.cores > 0 && c.cores <= IGPU_CORES))
    {
        return Some("built-in graphics".to_string());
    }
    if c.spinning_disk && c.total_mb > 0 && c.total_mb <= MODEST_MEMORY_MB {
        return Some(format!("a hard disk and {} of memory", gb(c.total_mb)));
    }
    None
}

/// This computer, with the graphics card from the cached profile (looked up the first time, which can take a moment:
/// call it off the window's thread).
pub fn this_computer(dirs: &crate::paths::Dirs, settings: &Settings) -> Computer {
    let profile = memory::PcProfile::get(dirs);
    Computer {
        cores: profile.cores,
        total_mb: profile.total_mb.max(0),
        gpu: profile.gpu,
        spinning_disk: crate::disk::is_spinning(settings, &dirs.base),
    }
}

// ---- options.txt, line by line

/// `options.txt` as lines, each with its own ending, so whatever isn't changed is written back byte for byte.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OptionsFile {
    lines: Vec<Vec<u8>>,
}

impl OptionsFile {
    pub fn parse(bytes: &[u8]) -> OptionsFile {
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, b) in bytes.iter().enumerate() {
            if *b == b'\n' {
                lines.push(bytes[start..=i].to_vec());
                start = i + 1;
            }
        }
        if start < bytes.len() {
            lines.push(bytes[start..].to_vec());
        }
        OptionsFile { lines }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.lines.concat()
    }

    /// A line without its ending.
    fn body(line: &[u8]) -> &[u8] {
        let mut end = line.len();
        if end > 0 && line[end - 1] == b'\n' {
            end -= 1;
        }
        if end > 0 && line[end - 1] == b'\r' {
            end -= 1;
        }
        &line[..end]
    }

    fn find(&self, key: &str) -> Option<usize> {
        let mut want = key.as_bytes().to_vec();
        want.push(b':');
        // the game reads the file top to bottom, so the last line with the key is the one that counts
        self.lines.iter().rposition(|l| Self::body(l).starts_with(&want))
    }

    /// The value of a key, as written (quotes and all).
    pub fn get(&self, key: &str) -> Option<String> {
        let line = &self.lines[self.find(key)?];
        Some(String::from_utf8_lossy(&Self::body(line)[key.len() + 1..]).to_string())
    }

    /// The line ending the file mostly uses.
    fn ending(&self) -> &'static [u8] {
        let crlf = self.lines.iter().filter(|l| l.ends_with(b"\r\n")).count();
        if crlf * 2 > self.lines.len() { b"\r\n" } else { b"\n" }
    }

    /// Changes the value of a key, keeping the line's ending; a missing key is added at the end.
    pub fn set(&mut self, key: &str, value: &str) {
        match self.find(key) {
            Some(i) => {
                let line = &self.lines[i];
                let ending = line[Self::body(line).len()..].to_vec();
                let mut new = format!("{key}:{value}").into_bytes();
                new.extend(ending);
                self.lines[i] = new;
            }
            None => {
                let ending = self.ending();
                if let Some(last) = self.lines.last_mut() {
                    if !last.ends_with(b"\n") {
                        last.extend_from_slice(ending);
                    }
                }
                let mut new = format!("{key}:{value}").into_bytes();
                new.extend_from_slice(ending);
                self.lines.push(new);
            }
        }
    }

    /// Takes a key's line out.
    pub fn remove(&mut self, key: &str) {
        if let Some(i) = self.find(key) {
            self.lines.remove(i);
        }
    }
}

// ---- what is lowered, and how far

/// How a key is lowered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A number that should be at most this (chunks, levels, a scale).
    AtMost(f64),
    /// A number where higher is lighter, which should be at least this (particles: 0 all, 1 decreased, 2 minimal).
    AtLeast(f64),
    /// A switch that should be off: `false`, or `0` where the version keeps it as a number.
    Off,
    /// Clouds: `true` (fancy), `fast` or `false`; lowered to `false`.
    Clouds,
    /// The graphics preset of 1.21.11 and newer: Fast, Fancy and Fabulous become Custom, so the game keeps the lowered
    /// values instead of setting its preset's again at the next start.
    Preset,
    /// The frame cap while nobody plays (1.21.2 and newer): `minimized` becomes `afk`.
    Idle,
}

/// One setting Low-end mode looks at, with the game's own default for when the file doesn't have it, written the way
/// this version writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub key: &'static str,
    pub kind: Kind,
    pub default: String,
    /// What the graphics preset sets it to (Fast, Fancy, Fabulous), on versions with presets: while a preset other
    /// than Custom is chosen, that is the value the game uses, whatever the file says.
    pub preset: Option<[&'static str; 3]>,
}

/// If quotes go around text values: from 1.19 the game writes them as JSON.
fn quotes_text(minor: u32) -> bool {
    minor >= 19
}

/// What the Fast, Fancy and Fabulous presets set (`GraphicsPreset.apply`, 26.3), in the file's own form.
const PRESETS: [(&str, [&str; 3]); 17] = [
    ("renderDistance", ["8", "16", "32"]),
    ("simulationDistance", ["6", "12", "12"]),
    ("particles", ["1", "0", "0"]),
    ("renderClouds", ["\"fast\"", "\"true\"", "\"true\""]),
    ("entityShadows", ["false", "true", "true"]),
    ("mipmapLevels", ["2", "4", "4"]),
    ("ao", ["false", "true", "true"]),
    ("biomeBlendRadius", ["1", "2", "2"]),
    ("entityDistanceScaling", ["0.75", "1.0", "1.25"]),
    ("prioritizeChunkUpdates", ["0", "1", "1"]),
    ("menuBackgroundBlurriness", ["2", "5", "5"]),
    ("cloudRange", ["32", "64", "128"]),
    ("cutoutLeaves", ["false", "true", "true"]),
    ("improvedTransparency", ["false", "false", "true"]),
    ("weatherRadius", ["5", "10", "10"]),
    ("maxAnisotropyBit", ["1", "1", "2"]),
    ("textureFiltering", ["0", "1", "2"]),
];

/// The keys to lower for a Minecraft version (by its "1.x" number, 100 for 26.x) and whether it has graphics presets
/// (1.21.11 and newer). Checked against the options classes of 1.8.9, 1.12.2, 1.20.1, 1.21.1, 1.21.11, 26.1 and 26.3.
/// Empty before 1.7, whose settings are too different. The values are the lab's low-end preset (A4): what costs the
/// most processor time per frame goes first (Fast graphics -28 %, clouds off -20 %, biome blend 0 and no mipmaps
/// -15-20 %).
pub fn targets(minor: u32, presets: bool) -> Vec<Target> {
    let mut t = Vec::new();
    if minor < 7 {
        return t;
    }
    let q = |s: &str| if quotes_text(minor) || presets { format!("\"{s}\"") } else { s.to_string() };
    let mut put = |key: &'static str, kind: Kind, default: String| {
        let preset = if presets { PRESETS.iter().find(|(k, _)| *k == key).map(|(_, v)| *v) } else { None };
        t.push(Target { key, kind, default, preset })
    };
    if presets {
        put("graphicsPreset", Kind::Preset, q("fancy"));
    }
    // 60 frames a second at most: a slow computer rarely gets there, and the cap keeps the game from taking the
    // processor from the chunk work in light scenes (the first-start choice for slow computers uses it too)
    if minor >= 8 {
        put("maxFps", Kind::AtMost(60.0), "120".into());
    }
    put("renderDistance", Kind::AtMost(6.0), "12".into());
    if minor >= 18 {
        put("simulationDistance", Kind::AtMost(5.0), "12".into());
    }
    put("particles", Kind::AtLeast(2.0), "0".into());
    if !presets {
        if minor >= 16 {
            put("graphicsMode", Kind::AtMost(0.0), "1".into());
        } else {
            put("fancyGraphics", Kind::Off, "true".into());
        }
    }
    if minor >= 8 {
        put("renderClouds", Kind::Clouds, q("true"));
        put("entityShadows", Kind::Off, "true".into());
        put("mipmapLevels", Kind::AtMost(0.0), "4".into());
    } else {
        put("clouds", Kind::Off, "true".into());
    }
    put("ao", Kind::Off, if minor >= 19 { "true".into() } else { "2".into() });
    if minor >= 14 {
        put("biomeBlendRadius", Kind::AtMost(0.0), "2".into());
    }
    if minor >= 16 {
        put("entityDistanceScaling", Kind::AtMost(0.75), "1.0".into());
    }
    if minor >= 19 {
        // chunk updates on the worker threads only, never waited for on the frame
        put("prioritizeChunkUpdates", Kind::AtMost(0.0), "0".into());
    }
    if minor >= 21 {
        // the blur behind menus is a full-screen effect; nothing in the world changes
        put("menuBackgroundBlurriness", Kind::AtMost(0.0), "5".into());
    }
    if presets {
        // the rest of what Fast sets, and the frame cap while nobody plays
        put("cloudRange", Kind::AtMost(32.0), "128".into());
        put("cutoutLeaves", Kind::Off, "true".into());
        put("improvedTransparency", Kind::Off, "false".into());
        put("weatherRadius", Kind::AtMost(5.0), "10".into());
        put("maxAnisotropyBit", Kind::AtMost(1.0), "2".into());
        put("textureFiltering", Kind::AtMost(0.0), "1".into());
        put("inactivityFpsLimit", Kind::Idle, q("afk"));
    }
    t
}

fn unquote(v: &str) -> &str {
    let v = v.trim();
    v.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(v)
}

/// The same text with the quotes `like` has.
fn quoted_like(like: &str, value: &str) -> String {
    if like.trim().starts_with('"') { format!("\"{value}\"") } else { value.to_string() }
}

/// A number written the way `like` writes it: with a decimal point when it has one.
fn number_like(like: &str, value: f64) -> String {
    if unquote(like).contains('.') {
        let s = format!("{value}");
        if s.contains('.') { s } else { format!("{s}.0") }
    } else {
        format!("{}", value.round() as i64)
    }
}

/// The lowered value for a key whose value is now `current`, in the same form; None when it is low enough already or
/// can't be read (then it is left alone).
pub fn lowered(kind: Kind, current: &str) -> Option<String> {
    let plain = unquote(current);
    match kind {
        Kind::AtMost(limit) => {
            let n: f64 = plain.parse().ok()?;
            (n > limit).then(|| number_like(current, limit))
        }
        Kind::AtLeast(limit) => {
            let n: f64 = plain.parse().ok()?;
            (n < limit).then(|| number_like(current, limit))
        }
        Kind::Off => match plain {
            "true" => Some(quoted_like(current, "false")),
            "false" | "0" => None,
            n => {
                let n: i64 = n.parse().ok()?;
                (n > 0).then(|| quoted_like(current, "0"))
            }
        },
        Kind::Clouds => match plain {
            "true" | "fast" | "fancy" | "1" | "2" => Some(quoted_like(current, "false")),
            _ => None,
        },
        Kind::Preset => match plain {
            "fast" | "fancy" | "fabulous" => Some(quoted_like(current, "custom")),
            _ => None,
        },
        Kind::Idle => (plain == "minimized").then(|| quoted_like(current, "afk")),
    }
}

/// If two values are the same setting (quotes, spaces and `8` against `8.0` don't matter).
pub fn same_value(a: &str, b: &str) -> bool {
    let (a, b) = (unquote(a), unquote(b));
    if a == b {
        return true;
    }
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => (x - y).abs() < 1e-9,
        _ => false,
    }
}

// ---- the record of what was changed

/// One key Low-end mode changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// The value before; None when the file didn't have the key.
    pub original: Option<String>,
    /// The value written.
    pub applied: String,
}

/// `blockforge-lowend.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Backup {
    pub changed: BTreeMap<String, Change>,
    /// Keys the player changed while the mode was on: never touched again.
    pub players: BTreeSet<String>,
}

impl Backup {
    pub fn load(game_directory: &Path) -> Backup {
        std::fs::read(game_directory.join(BACKUP_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.players.is_empty()
    }
}

/// Lowers the settings in `file`, minding what an earlier start changed. Returns the keys changed now, as (key, old
/// value, new value).
pub fn lower(file: &mut OptionsFile, backup: &mut Backup, targets: &[Target]) -> Vec<(String, Option<String>, String)> {
    // what the player changed since the last start is theirs
    let keys: Vec<String> = backup.changed.keys().cloned().collect();
    for key in keys {
        let still = file.get(&key).is_some_and(|v| same_value(&v, &backup.changed[&key].applied));
        if !still {
            backup.changed.remove(&key);
            backup.players.insert(key);
        }
    }
    // with a preset other than Custom the game uses the preset's values, not the file's: those are what is lowered,
    // and the preset becomes Custom. A preset the player chose while the mode was on is theirs: then the settings it
    // sets are left alone (the game would set them again anyway).
    let preset_target = targets.iter().find(|t| t.kind == Kind::Preset);
    let chosen = preset_target.map(|t| file.get(t.key).unwrap_or_else(|| t.default.clone()));
    let preset = match chosen.as_deref().map(unquote) {
        Some("fast") => Some(0),
        Some("fancy") => Some(1),
        Some("fabulous") => Some(2),
        _ => None,
    };
    let preset_is_players = preset_target.is_some_and(|t| backup.players.contains(t.key));
    let mut done = Vec::new();
    for t in targets {
        if backup.players.contains(t.key) {
            continue;
        }
        let current = file.get(t.key);
        let new = match (t.preset, preset) {
            (Some(_), Some(_)) if preset_is_players => continue,
            // the preset's value, lowered where it is above the limit, so Custom looks like the preset otherwise
            (Some(values), Some(i)) => lowered(t.kind, values[i]).unwrap_or_else(|| values[i].to_string()),
            _ => match lowered(t.kind, current.as_deref().unwrap_or(&t.default)) {
                Some(new) => new,
                None => continue,
            },
        };
        if current.as_deref().is_some_and(|c| c == new) {
            continue;
        }
        file.set(t.key, &new);
        // lowered before (a newer BlockForge may lower a little further): the value to put back stays the player's
        let original = match backup.changed.get(t.key) {
            Some(earlier) => earlier.original.clone(),
            None => current.clone(),
        };
        backup.changed.insert(t.key.to_string(), Change { original, applied: new.clone() });
        done.push((t.key.to_string(), current, new));
    }
    done
}

/// Puts back what Low-end mode changed, where the value is still the one it wrote. Returns how many keys went back.
pub fn restore(file: &mut OptionsFile, backup: &Backup) -> usize {
    let mut n = 0;
    for (key, change) in &backup.changed {
        if !file.get(key).is_some_and(|v| same_value(&v, &change.applied)) {
            continue;
        }
        match &change.original {
            Some(v) => file.set(key, v),
            None => file.remove(key),
        }
        n += 1;
    }
    n
}

// ---- games of one instance running at the same time

static IN_USE: Mutex<Option<HashMap<PathBuf, usize>>> = Mutex::new(None);

/// While a game of an instance is being started or runs. Another game of the same instance doesn't change the
/// settings file meanwhile: the running game writes its own settings back when it closes.
#[derive(Debug)]
pub struct InUse(PathBuf);

impl InUse {
    /// Marks the instance as in use; the second value says whether another game of it already was.
    pub fn claim(game_directory: &Path) -> (InUse, bool) {
        let mut guard = IN_USE.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        let count = map.entry(game_directory.to_path_buf()).or_insert(0);
        *count += 1;
        (InUse(game_directory.to_path_buf()), *count > 1)
    }
}

impl Drop for InUse {
    fn drop(&mut self) {
        let mut guard = IN_USE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = guard.as_mut() {
            if let Some(count) = map.get_mut(&self.0) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    map.remove(&self.0);
                }
            }
        }
    }
}

// ---- putting it together for one start

/// How the log line starts when settings were lowered at this start (the window then says so too).
pub const LOWERED_NOTE: &str = "Low-end mode: lowered";

/// What happened to the settings file at a start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing to do (the mode is off and nothing was changed before).
    Nothing,
    /// On, and these keys were lowered now (key, old value, new value).
    Lowered(Vec<(String, Option<String>, String)>),
    /// On, and everything is low already.
    AlreadyLow,
    /// Off, and this many keys went back.
    Restored(usize),
    /// The file was left as it is, for this reason.
    Skipped(String),
}

impl Outcome {
    /// The line for the game's log, if any.
    pub fn note(&self) -> Option<String> {
        match self {
            Outcome::Nothing => None,
            Outcome::Lowered(keys) => {
                let shown: Vec<String> = keys
                    .iter()
                    .map(|(k, old, new)| {
                        format!("{k} {} -> {}", old.as_deref().map(unquote).unwrap_or("default"), unquote(new))
                    })
                    .collect();
                Some(format!("{LOWERED_NOTE} {} settings ({})", keys.len(), shown.join(", ")))
            }
            Outcome::AlreadyLow => Some("Low-end mode: on, the settings are low already".to_string()),
            Outcome::Restored(n) => Some(format!("Low-end mode is off: put back {n} settings it had lowered")),
            Outcome::Skipped(why) => Some(format!("Low-end mode: {why}")),
        }
    }
}

/// Lowers (`on`) or puts back (`!on`) the settings of the game in `game_directory`. `minor` is the version's "1.x"
/// number ([`memory::minor_version`]) and `presets` whether it has graphics presets. Never fails: a problem is the
/// reason of an [`Outcome::Skipped`].
pub fn apply(game_directory: &Path, minor: u32, presets: bool, on: bool) -> Outcome {
    match apply_inner(game_directory, minor, presets, on) {
        Ok(o) => o,
        Err(e) => Outcome::Skipped(format!("couldn't change the settings file, it is left as it is ({e:#})")),
    }
}

fn apply_inner(game_directory: &Path, minor: u32, presets: bool, on: bool) -> Result<Outcome> {
    let path = game_directory.join("options.txt");
    let backup_path = game_directory.join(BACKUP_FILE);
    let mut backup = Backup::load(game_directory);
    if !on && backup.is_empty() {
        return Ok(Outcome::Nothing);
    }
    if !path.exists() {
        // No settings file yet. Writing one here would have to guess the format and data version of this version's
        // file (a 1.14+ file without its data version is "upgraded" by the game and loses settings), so it is left
        // to the game: it writes a complete file, and the mode applies from the next start. On a slow computer the
        // first start already gets light settings from the first-start choice (`gameoptions`), which writes its file
        // before this runs.
        return Ok(if on {
            Outcome::Skipped("there is no settings file yet; it applies from the next start".to_string())
        } else {
            Outcome::Nothing
        });
    }
    let meta = std::fs::metadata(&path)?;
    if meta.permissions().readonly() {
        return Ok(Outcome::Skipped("the settings file is read-only, so it is left as it is".to_string()));
    }
    let bytes = std::fs::read(&path).context("reading options.txt")?;
    let mut file = OptionsFile::parse(&bytes);
    let before = backup.clone();
    let outcome = if on {
        let list = targets(minor, presets);
        let done = lower(&mut file, &mut backup, &list);
        if done.is_empty() { Outcome::AlreadyLow } else { Outcome::Lowered(done) }
    } else {
        let n = restore(&mut file, &backup);
        backup = Backup::default();
        Outcome::Restored(n)
    };
    let new_bytes = file.to_bytes();
    // the record first: if the settings can't be written after it, it is put back as it was
    if backup != before {
        if backup.is_empty() {
            let _ = std::fs::remove_file(&backup_path);
        } else {
            crate::config::write_json_atomic(&backup_path, &backup).context("saving blockforge-lowend.json")?;
        }
    }
    if new_bytes != bytes {
        if let Err(e) = write_replacing(&path, &new_bytes) {
            if before.is_empty() {
                let _ = std::fs::remove_file(&backup_path);
            } else {
                let _ = crate::config::write_json_atomic(&backup_path, &before);
            }
            return Err(e);
        }
    }
    Ok(outcome)
}

/// Writes through a file beside it and a rename, so the game never reads half a file.
fn write_replacing(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("txt.blockforge-{}", std::process::id()));
    std::fs::write(&temp, bytes).with_context(|| format!("writing {}", temp.display()))?;
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(e).context("replacing options.txt (is the game or another program using it?)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bf-lowend-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn only_a_start_that_lowered_something_is_announced() {
        let lowered = Outcome::Lowered(vec![("renderDistance".into(), Some("12".into()), "6".into())]);
        assert!(lowered.note().unwrap().starts_with(LOWERED_NOTE));
        for quiet in [Outcome::AlreadyLow, Outcome::Restored(3), Outcome::Skipped("read-only".into())] {
            assert!(!quiet.note().unwrap().starts_with(LOWERED_NOTE), "{quiet:?}");
        }
        assert_eq!(Outcome::Nothing.note(), None);
    }

    fn c(cores: usize, gb: f64, gpu: Gpu, spinning_disk: bool) -> Computer {
        Computer { cores, total_mb: (gb * 1024.0) as i64 - 200, gpu, spinning_disk }
    }

    #[test]
    fn slow_computers_are_told_by_one_rule() {
        assert_eq!(looks_weak(&c(4, 16.0, Gpu::Dedicated, false)).as_deref(), Some("4 processor threads"));
        assert_eq!(looks_weak(&c(8, 6.0, Gpu::Dedicated, false)).as_deref(), Some("6 GB of memory"));
        assert_eq!(looks_weak(&c(8, 8.0, Gpu::Integrated, false)).as_deref(), Some("built-in graphics"));
        assert_eq!(looks_weak(&c(6, 16.0, Gpu::Integrated, false)).as_deref(), Some("built-in graphics"));
        assert!(looks_weak(&c(8, 8.0, Gpu::Dedicated, true)).unwrap().starts_with("a hard disk"));
        // a usual gaming or office computer is not slow
        assert_eq!(looks_weak(&c(8, 16.0, Gpu::Dedicated, false)), None);
        assert_eq!(looks_weak(&c(12, 16.0, Gpu::Integrated, false)), None);
        assert_eq!(looks_weak(&c(8, 8.0, Gpu::Unknown, false)), None);
        assert_eq!(looks_weak(&c(16, 32.0, Gpu::Dedicated, true)), None);
        // nothing known: not slow
        assert_eq!(looks_weak(&Computer { cores: 0, total_mb: 0, gpu: Gpu::Unknown, spinning_disk: false }), None);
    }

    #[test]
    fn modes() {
        assert_eq!(Mode::parse("ON"), Mode::On);
        assert_eq!(Mode::parse("off"), Mode::Off);
        assert_eq!(Mode::parse(""), Mode::Auto);
        assert!(Mode::Auto.active(true) && !Mode::Auto.active(false));
        assert!(Mode::On.active(false) && !Mode::Off.active(true));
        let mut s = Settings::default();
        let mut file = crate::instances::InstanceFile::default();
        file.launcher = serde_json::from_str(r#"{"name":"A"}"#).unwrap();
        let mut i = Instance { folder: "A".into(), mod_count: 0, root: "/x".into(), file };
        assert_eq!(mode(&s, &i), Mode::Auto);
        s.low_end_mode = "off".into();
        assert_eq!(mode(&s, &i), Mode::Off);
        i.file.launcher.extras.insert("lowEndMode".into(), "on".into());
        assert_eq!(mode(&s, &i), Mode::On);
    }

    #[test]
    fn lines_keep_their_bytes_and_endings() {
        let text =
            b"version:3955\r\nrenderDistance:12\r\nkey_key.jump:key.keyboard.space\r\nlastServer:\xff\xfe\r\nao:true";
        let mut f = OptionsFile::parse(text);
        assert_eq!(f.to_bytes(), text.to_vec());
        assert_eq!(f.get("renderDistance").as_deref(), Some("12"));
        assert_eq!(f.get("render"), None);
        f.set("renderDistance", "8");
        f.set("mipmapLevels", "2");
        f.remove("ao");
        let out = f.to_bytes();
        assert_eq!(
            out,
            b"version:3955\r\nrenderDistance:8\r\nkey_key.jump:key.keyboard.space\r\nlastServer:\xff\xfe\r\nmipmapLevels:2\r\n"
                .to_vec()
        );
        // a key twice: the last one is what the game uses
        let twice = OptionsFile::parse(b"a:1\na:2\n");
        assert_eq!(twice.get("a").as_deref(), Some("2"));
    }

    #[test]
    fn values_are_only_ever_lowered_in_their_own_form() {
        assert_eq!(lowered(Kind::AtMost(8.0), "12").as_deref(), Some("8"));
        assert_eq!(lowered(Kind::AtMost(8.0), "6"), None);
        assert_eq!(lowered(Kind::AtMost(0.75), "1.0").as_deref(), Some("0.75"));
        assert_eq!(lowered(Kind::AtMost(1.0), "2.0").as_deref(), Some("1.0"));
        assert_eq!(lowered(Kind::AtMost(0.75), "0.5"), None);
        assert_eq!(lowered(Kind::AtLeast(2.0), "0").as_deref(), Some("2"));
        assert_eq!(lowered(Kind::AtLeast(1.0), "2"), None, "minimal particles stay minimal");
        assert_eq!(lowered(Kind::Off, "true").as_deref(), Some("false"));
        assert_eq!(lowered(Kind::Off, "2").as_deref(), Some("0"), "1.12's smooth lighting is a number");
        assert_eq!(lowered(Kind::Off, "false"), None);
        assert_eq!(lowered(Kind::Clouds, "\"true\"").as_deref(), Some("\"false\""));
        assert_eq!(lowered(Kind::Clouds, "fast").as_deref(), Some("false"));
        assert_eq!(lowered(Kind::Clouds, "false"), None);
        assert_eq!(lowered(Kind::Preset, "\"fabulous\"").as_deref(), Some("\"custom\""));
        assert_eq!(lowered(Kind::Preset, "\"fast\"").as_deref(), Some("\"custom\""));
        assert_eq!(lowered(Kind::Preset, "\"custom\""), None);
        assert_eq!(lowered(Kind::Idle, "\"minimized\"").as_deref(), Some("\"afk\""));
        assert_eq!(lowered(Kind::Idle, "\"afk\""), None);
        assert_eq!(lowered(Kind::AtMost(8.0), "lots"), None, "unreadable values are left alone");
        assert!(same_value("\"8\"", "8.0") && !same_value("8", "9"));
    }

    fn keys(t: &[Target]) -> Vec<&str> {
        t.iter().map(|t| t.key).collect()
    }

    #[test]
    fn each_version_gets_only_its_own_keys() {
        // 1.8.9 and 1.12.2: fancyGraphics, a numeric ao, no simulation distance, biome blend or entity distance
        let old = targets(8, false);
        assert_eq!(
            keys(&old),
            [
                "maxFps",
                "renderDistance",
                "particles",
                "fancyGraphics",
                "renderClouds",
                "entityShadows",
                "mipmapLevels",
                "ao"
            ]
        );
        assert_eq!(old.iter().find(|t| t.key == "ao").unwrap().default, "2");
        assert_eq!(old.iter().find(|t| t.key == "renderClouds").unwrap().default, "true");
        assert!(old.iter().all(|t| t.preset.is_none()));
        assert_eq!(keys(&targets(12, false)), keys(&old));
        // 1.20.1 and 1.21.1
        let k = keys(&targets(21, false)).join(" ");
        assert_eq!(
            k,
            "maxFps renderDistance simulationDistance particles graphicsMode renderClouds entityShadows mipmapLevels ao \
             biomeBlendRadius entityDistanceScaling prioritizeChunkUpdates menuBackgroundBlurriness"
        );
        assert_eq!(targets(21, false).iter().find(|t| t.key == "renderClouds").unwrap().default, "\"true\"");
        // 1.21.11 and 26.x: the preset first, the keys only those versions have, no graphicsMode
        let new = targets(100, true);
        let k = keys(&new);
        assert_eq!(k[0], "graphicsPreset");
        for key in ["cutoutLeaves", "weatherRadius", "textureFiltering", "cloudRange", "inactivityFpsLimit"] {
            assert!(k.contains(&key), "{key}");
        }
        assert!(!k.contains(&"graphicsMode") && !k.contains(&"fancyGraphics"));
        assert_eq!(new.iter().find(|t| t.key == "renderDistance").unwrap().preset, Some(["8", "16", "32"]));
        assert!(targets(6, false).is_empty());
        assert!(keys(&targets(7, false)).contains(&"clouds"));
        assert!(!keys(&targets(7, false)).contains(&"maxFps"));
    }

    const V1211: &str = "version:3955\nao:true\nbiomeBlendRadius:2\nentityShadows:true\ngraphicsMode:1\nmipmapLevels:4\nparticles:0\nrenderClouds:\"true\"\nrenderDistance:14\nsimulationDistance:12\nkey_key.attack:key.mouse.left\n";

    #[test]
    fn lowering_and_putting_back_leave_the_players_changes() {
        let mut f = OptionsFile::parse(V1211.as_bytes());
        let mut b = Backup::default();
        let done = lower(&mut f, &mut b, &targets(21, false));
        assert_eq!(done.len(), 12);
        assert_eq!(f.get("renderDistance").as_deref(), Some("6"));
        assert_eq!(f.get("particles").as_deref(), Some("2"));
        assert_eq!(f.get("mipmapLevels").as_deref(), Some("0"));
        assert_eq!(f.get("biomeBlendRadius").as_deref(), Some("0"));
        assert_eq!(f.get("maxFps").as_deref(), Some("60"));
        assert_eq!(f.get("menuBackgroundBlurriness").as_deref(), Some("0"));
        // prioritizeChunkUpdates is 0 by default: nothing to add
        assert_eq!(f.get("prioritizeChunkUpdates"), None);
        assert_eq!(f.get("renderClouds").as_deref(), Some("\"false\""));
        // a key the file didn't have is added
        assert_eq!(f.get("entityDistanceScaling").as_deref(), Some("0.75"));
        assert_eq!(b.changed["entityDistanceScaling"].original, None);
        // the next start has nothing to do
        assert!(lower(&mut f, &mut b, &targets(21, false)).is_empty());
        // the player turns the view back up while the mode is on: theirs from now on
        f.set("renderDistance", "12");
        assert!(lower(&mut f, &mut b, &targets(21, false)).is_empty());
        assert_eq!(f.get("renderDistance").as_deref(), Some("12"));
        assert!(b.players.contains("renderDistance"));
        // the player changes particles, then switches the mode off: everything else goes back
        f.set("particles", "1");
        let n = restore(&mut f, &b);
        assert_eq!(n, 10);
        assert_eq!(f.get("particles").as_deref(), Some("1"));
        assert_eq!(f.get("renderDistance").as_deref(), Some("12"));
        assert_eq!(f.get("simulationDistance").as_deref(), Some("12"));
        assert_eq!(f.get("entityDistanceScaling"), None, "a key that was added is taken out again");
        let expect = V1211.replace("particles:0", "particles:1").replace("renderDistance:14", "renderDistance:12");
        assert_eq!(String::from_utf8(f.to_bytes()).unwrap(), expect);
    }

    #[test]
    fn a_stricter_limit_lowers_further_and_still_puts_back_the_players_value() {
        let mut f = OptionsFile::parse(b"simulationDistance:6\n");
        let mut b = Backup::default();
        b.changed.insert("simulationDistance".into(), Change { original: Some("12".into()), applied: "6".into() });
        let done = lower(&mut f, &mut b, &targets(21, false));
        assert!(
            done.iter().any(|(k, old, new)| k == "simulationDistance" && old.as_deref() == Some("6") && new == "5")
        );
        assert_eq!(b.changed["simulationDistance"], Change { original: Some("12".into()), applied: "5".into() });
        restore(&mut f, &b);
        assert_eq!(f.get("simulationDistance").as_deref(), Some("12"));
    }

    #[test]
    fn the_whole_round_trip_on_disk() {
        let dir = temp("disk");
        let original = V1211.replace('\n', "\r\n");
        std::fs::write(dir.join("options.txt"), &original).unwrap();
        let o = apply(&dir, 21, false, true);
        let Outcome::Lowered(keys) = &o else { panic!("{o:?}") };
        assert_eq!(keys.len(), 12);
        assert!(o.note().unwrap().starts_with("Low-end mode: lowered 12 settings"));
        let lowered_text = std::fs::read_to_string(dir.join("options.txt")).unwrap();
        assert!(
            lowered_text.contains("renderDistance:6\r\n") && lowered_text.ends_with("menuBackgroundBlurriness:0\r\n")
        );
        assert!(dir.join(BACKUP_FILE).exists());
        assert_eq!(apply(&dir, 21, false, true), Outcome::AlreadyLow);
        assert_eq!(apply(&dir, 21, false, false), Outcome::Restored(12));
        assert_eq!(std::fs::read_to_string(dir.join("options.txt")).unwrap(), original);
        assert!(!dir.join(BACKUP_FILE).exists());
        assert_eq!(apply(&dir, 21, false, false), Outcome::Nothing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_preset_becomes_custom_with_its_values_lowered_and_comes_back() {
        let dir = temp("preset");
        let original = "version:5023\ngraphicsPreset:\"fancy\"\nrenderDistance:16\n";
        std::fs::write(dir.join("options.txt"), original).unwrap();
        let o = apply(&dir, 100, true, true);
        assert!(matches!(&o, Outcome::Lowered(k) if k.len() == 19), "{o:?}");
        let text = std::fs::read_to_string(dir.join("options.txt")).unwrap();
        assert_eq!(
            text,
            "version:5023\ngraphicsPreset:\"custom\"\nrenderDistance:6\nmaxFps:60\nsimulationDistance:5\nparticles:2\n\
             renderClouds:\"false\"\nentityShadows:false\nmipmapLevels:0\nao:false\nbiomeBlendRadius:0\n\
             entityDistanceScaling:0.75\nprioritizeChunkUpdates:0\nmenuBackgroundBlurriness:0\ncloudRange:32\n\
             cutoutLeaves:false\nimprovedTransparency:false\nweatherRadius:5\nmaxAnisotropyBit:1\ntextureFiltering:0\n"
        );
        // the game keeps Custom and writes the same values back: nothing more to do
        assert_eq!(apply(&dir, 100, true, true), Outcome::AlreadyLow);
        assert_eq!(apply(&dir, 100, true, false), Outcome::Restored(19));
        assert_eq!(std::fs::read_to_string(dir.join("options.txt")).unwrap(), original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_preset_the_player_picks_while_the_mode_is_on_is_theirs() {
        let mut f = OptionsFile::parse(b"graphicsPreset:\"fast\"\n");
        let mut b = Backup::default();
        let list = targets(100, true);
        assert!(!lower(&mut f, &mut b, &list).is_empty());
        assert_eq!(f.get("graphicsPreset").as_deref(), Some("\"custom\""));
        // Fast's own value where it is low enough already
        assert_eq!(f.get("maxAnisotropyBit").as_deref(), Some("1"));
        // in the game the player picks Fancy, which sets its values again
        f.set("graphicsPreset", "\"fancy\"");
        for (k, v) in [("renderDistance", "16"), ("mipmapLevels", "4"), ("ao", "true")] {
            f.set(k, v);
        }
        assert!(lower(&mut f, &mut b, &list).is_empty());
        assert_eq!(f.get("graphicsPreset").as_deref(), Some("\"fancy\""));
        assert_eq!(f.get("renderDistance").as_deref(), Some("16"));
        assert!(b.players.contains("graphicsPreset") && b.players.contains("renderDistance"));
    }

    #[test]
    fn old_versions_as_the_game_writes_them() {
        let dir = temp("old");
        std::fs::write(
            dir.join("options.txt"),
            "invertYMouse:false\nrenderDistance:12\nfancyGraphics:true\nao:2\nrenderClouds:true\nparticles:0\nmipmapLevels:4\nentityShadows:true\n",
        )
        .unwrap();
        assert!(matches!(apply(&dir, 8, false, true), Outcome::Lowered(k) if k.len() == 8));
        let text = std::fs::read_to_string(dir.join("options.txt")).unwrap();
        assert_eq!(
            text,
            "invertYMouse:false\nrenderDistance:6\nfancyGraphics:false\nao:0\nrenderClouds:false\nparticles:2\nmipmapLevels:0\nentityShadows:false\nmaxFps:60\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_and_locked_files_never_stop_a_start() {
        let dir = temp("locked");
        assert!(matches!(apply(&dir, 21, false, true), Outcome::Skipped(_)));
        assert_eq!(apply(&dir, 21, false, false), Outcome::Nothing);
        assert!(!dir.join("options.txt").exists(), "no file is made up");
        std::fs::write(dir.join("options.txt"), V1211).unwrap();
        let mut p = std::fs::metadata(dir.join("options.txt")).unwrap().permissions();
        p.set_readonly(true);
        std::fs::set_permissions(dir.join("options.txt"), p.clone()).unwrap();
        let o = apply(&dir, 21, false, true);
        assert!(matches!(&o, Outcome::Skipped(why) if why.contains("read-only")), "{o:?}");
        assert_eq!(std::fs::read_to_string(dir.join("options.txt")).unwrap(), V1211);
        assert!(!dir.join(BACKUP_FILE).exists());
        #[allow(clippy::permissions_set_readonly_false)]
        p.set_readonly(false);
        std::fs::set_permissions(dir.join("options.txt"), p).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_game_of_the_same_instance_is_noticed() {
        let dir = Path::new("/nonexistent/lowend-in-use");
        let (first, others) = InUse::claim(dir);
        assert!(!others);
        let (second, others) = InUse::claim(dir);
        assert!(others);
        drop(second);
        drop(first);
        let (_again, others) = InUse::claim(dir);
        assert!(!others);
    }
}
