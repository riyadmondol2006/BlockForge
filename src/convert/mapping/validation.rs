// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Identifier coverage tests, a port of Chunker's `*IdentifierValidationTests`.
//!
//! Chunker ships the vanilla block, item and block-state lists of every Java and Bedrock version it supports under
//! `cli/data` (`java/<v>/{blocks,items}.json`, `bedrock/<v>/{block_states,item_names}.json`). These tests push every one
//! of those blocks and items through our tables (`edition -> pivot -> edition`) and the other way round, and report
//! what does not map. Set `CHUNKER_DATA` to the data folder; without it the tests look in `<repo>/../Chunker/cli/data`
//! and `~/Downloads/ABDM/Compressed/Chunker/cli/data` and skip (with a note) when it is missing.
//!
//! Run: `cargo test --lib convert::mapping::validation -- --nocapture`. The output carries one `@@V` line per
//! (area, version, check) and one `@@F` line per failing id, which `docs/porting/COVERAGE.md` was generated from.
//!
//! Failures fail the test unless the id is on an allow-list: Chunker's own (`UNSUPPORTED_*`, `LOSSY_*`,
//! `removeLossyStates`) are ported verbatim, and gaps we know about are in [`known_gap`] with a reason.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::engine::PivotBlock;
use super::engine::PivotInfo;
use super::{StateMap, StateValue, Version};

#[path = "validation_blocks.rs"]
mod blocks;
#[path = "validation_cross.rs"]
mod cross;
#[path = "validation_tables.rs"]
mod id_tables;
#[path = "validation_items.rs"]
mod items;
#[path = "validation_known.rs"]
mod known;
#[path = "validation_lossy.rs"]
mod lossy;
#[path = "validation_data.rs"]
mod vdata;

pub(crate) use known::known_gap;

/// Where Chunker's `cli/data` lives, if anywhere.
pub(crate) fn data_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(p) = std::env::var("CHUNKER_DATA") {
        candidates.push(PathBuf::from(p));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../Chunker/cli/data"));
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join("Downloads/ABDM/Compressed/Chunker/cli/data"));
    }
    candidates.into_iter().find(|p| p.join("java").is_dir() && p.join("bedrock").is_dir())
}

/// The data dir or a printed skip note.
pub(crate) fn data_or_skip(test: &str) -> Option<PathBuf> {
    let dir = data_dir();
    if dir.is_none() {
        println!("skipping {test}: Chunker data not found (set CHUNKER_DATA to the cli/data folder)");
    }
    dir
}

/// Results of one check on one version.
#[derive(Clone, Debug, Default)]
pub(crate) struct Tally {
    pub checked: usize,
    /// Expected non-passes: Chunker's allow-lists (unsupported, lossy) and explained version gaps.
    pub allowed: usize,
    pub failed: usize,
    /// Failing key (usually a block or item id) -> (count, first message).
    pub fails: BTreeMap<String, (usize, String)>,
    /// Allowed key -> count.
    pub allows: BTreeMap<String, usize>,
}

impl Tally {
    pub fn pass(&mut self) {
        self.checked += 1;
    }

    pub fn allow(&mut self, key: &str) {
        self.checked += 1;
        self.allowed += 1;
        *self.allows.entry(key.to_string()).or_default() += 1;
    }

    pub fn fail(&mut self, key: &str, msg: impl FnOnce() -> String) {
        self.checked += 1;
        self.failed += 1;
        let e = self.fails.entry(key.to_string()).or_insert_with(|| (0, String::new()));
        if e.0 == 0 {
            e.1 = msg();
        }
        e.0 += 1;
    }
}

/// All checks of one version of one area (`blocks`, `items`, `tables`, `cross`).
pub(crate) struct VerReport {
    /// Distinct vanilla ids (blocks or items) the version's data holds.
    pub ids: usize,
    pub area: &'static str,
    pub edition: &'static str,
    pub version: String,
    pub checks: Vec<(&'static str, Tally)>,
}

/// Run `f` over `items` on all cores, keeping the order.
pub(crate) fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(items.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = f(item);
                    out.lock().expect("no panic while holding the lock")[i] = Some(r);
                }
            });
        }
    });
    out.into_inner().expect("no panic while holding the lock").into_iter().flatten().collect()
}

/// A pivot block's states must all belong to its type and hold valid values.
pub(crate) fn pivot_valid(info: &PivotInfo, p: &PivotBlock) -> Result<(), String> {
    let Some(states) = info.types.get(&p.ty) else { return Err(format!("{} is not a vanilla pivot type", p.ty)) };
    for (k, v) in &p.states {
        if !states.contains(k) {
            return Err(format!("state {k} is not a state of {}", p.ty));
        }
        match info.states.get(k) {
            Some(si) if si.values.contains(v) => {}
            _ => return Err(format!("{v} is not a value of state {k} ({})", p.ty)),
        }
    }
    Ok(())
}

/// Every state combination of a pivot type as `(state, value)` lists (the full product; the biggest type has far
/// fewer than 20k, 46k across all 1292 types).
pub(crate) fn pivot_combos(info: &PivotInfo, ty: &str) -> Vec<Vec<(std::sync::Arc<str>, std::sync::Arc<str>)>> {
    let Some(states) = info.types.get(ty) else { return Vec::new() };
    let mut out: Vec<Vec<(std::sync::Arc<str>, std::sync::Arc<str>)>> = vec![Vec::new()];
    for s in states {
        let Some(si) = info.states.get(s) else { continue };
        let mut next = Vec::with_capacity(out.len() * si.values.len());
        for base in &out {
            for v in &si.values {
                let mut c = base.clone();
                c.push((s.clone(), v.clone()));
                next.push(c);
            }
        }
        out = next;
    }
    out
}

/// `id[k=v,...]` for messages.
pub(crate) fn show(id: &str, m: &StateMap) -> String {
    let s: Vec<String> = m.iter().map(|(k, v)| format!("{k}={}", v.short())).collect();
    format!("{id}[{}]", s.join(","))
}

impl StateValue {
    /// Compact text: `true`, `3`, `north`.
    pub(crate) fn short(&self) -> String {
        match self {
            StateValue::Bool(b) => b.to_string(),
            StateValue::Int(i) => i.to_string(),
            StateValue::Str(s) => s.to_string(),
        }
    }
}

/// Parse a data folder name like `1.21.100.0` (Chunker keeps the first three parts).
pub(crate) fn parse_dir_version(name: &str) -> Option<Version> {
    let mut parts = name.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    Some(Version::new(major, minor, patch))
}

/// Output of all reports: `@@V` lines, `@@F` lines and a summary. Returns the unexpected failures.
pub(crate) fn print_reports(reports: &[VerReport]) -> Vec<String> {
    // check key -> id -> (versions, example)
    type Agg = BTreeMap<(String, String, String), (Vec<String>, usize, String)>;
    let mut fails: Agg = BTreeMap::new();
    let mut allows: BTreeMap<(String, String, String), (usize, BTreeSet<String>)> = BTreeMap::new();
    for r in reports {
        for (check, t) in &r.checks {
            println!(
                "@@V\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                r.area, r.edition, r.version, r.ids, check, t.checked, t.allowed, t.failed
            );
            for (id, (n, msg)) in &t.fails {
                let e = fails.entry((format!("{}/{}", r.area, check), r.edition.to_string(), id.clone())).or_default();
                e.0.push(r.version.clone());
                e.1 += n;
                if e.2.is_empty() {
                    e.2 = msg.clone();
                }
            }
            for (id, n) in &t.allows {
                let e = allows.entry((format!("{}/{}", r.area, check), r.edition.to_string(), id.clone())).or_default();
                e.0 += n;
                e.1.insert(r.version.clone());
            }
        }
    }
    let mut unexpected = Vec::new();
    for ((check, edition, id), (versions, n, msg)) in &fails {
        let first = versions.first().cloned().unwrap_or_default();
        let last = versions.last().cloned().unwrap_or_default();
        let known = known_gap(check, edition, id);
        println!(
            "@@F\t{check}\t{edition}\t{id}\t{}\t{first}\t{last}\t{n}\t{}\t{}",
            versions.len(),
            known.unwrap_or("-"),
            msg.replace('\t', " ")
        );
        if known.is_none() {
            unexpected.push(format!("{check} {edition} {id} ({} versions, {first}..{last}): {msg}", versions.len()));
        }
    }
    for ((check, edition, id), (n, versions)) in &allows {
        println!("@@A\t{check}\t{edition}\t{id}\t{}\t{n}", versions.len());
    }
    unexpected
}

/// Print the unexpected failures and panic when there are any.
pub(crate) fn finish(area: &str, unexpected: Vec<String>) {
    if !unexpected.is_empty() {
        let shown: Vec<&String> = unexpected.iter().take(40).collect();
        panic!(
            "{area}: {} unexpected failures not on an allow-list or in known_gap (first {}):\n{}",
            unexpected.len(),
            shown.len(),
            shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
        );
    }
}

#[test]
fn java_blocks() {
    let Some(dir) = data_or_skip("java_blocks") else { return };
    let reports = blocks::run_java(&dir);
    finish("java blocks", print_reports(&reports));
}

#[test]
fn bedrock_blocks() {
    let Some(dir) = data_or_skip("bedrock_blocks") else { return };
    let reports = blocks::run_bedrock(&dir);
    finish("bedrock blocks", print_reports(&reports));
}

#[test]
fn java_and_bedrock_items() {
    let Some(dir) = data_or_skip("items") else { return };
    let mut reports = items::run_bedrock(&dir);
    reports.extend(items::run_java(&dir));
    finish("items", print_reports(&reports));
}

#[test]
fn cross_edition_blocks() {
    let Some(dir) = data_or_skip("cross_edition_blocks") else { return };
    let reports = cross::run(&dir);
    finish("cross edition", print_reports(&reports));
}

#[test]
fn id_tables() {
    let Some(dir) = data_or_skip("id_tables") else { return };
    let reports = id_tables::run(&dir);
    finish("id tables", print_reports(&reports));
}
