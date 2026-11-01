//! Offline, transactional writes to Wine's text registry files (`system.reg`, `user.reg`).
//!
//! Ported from BedrockOnLinux `bol/wine_registry.py` (MIT), with credit. Starting `reg.exe` through Wine only to
//! write a handful of values wakes up a whole Wine session (explorer, the graphics driver); BlockForge always
//! stops the prefix first, so it edits the files Wine will read the next time instead. That is only safe while
//! nothing owns the hives, so every write checks that the prefix is idle and holds a lock.
//!
//! The file format is Wine's own text format. Only what BlockForge needs is written: strings, DWORDs and
//! deletions. Anything else in a hive is carried through byte for byte, and a file that does not look like a hive
//! (or looks cut off) is refused instead of rewritten. The values written can be secrets (the Microsoft refresh
//! token lives in `system.reg`), so nothing here logs or returns a value.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::{PrefixError, require_prefix_idle};

/// What to do with one value.
#[derive(Clone, PartialEq, Eq)]
pub enum RegValue {
    Sz(String),
    Dword(u32),
    Delete,
}

// A value is not printed: the machine hive holds a refresh token.
impl std::fmt::Debug for RegValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegValue::Sz(_) => f.write_str("Sz(..)"),
            RegValue::Dword(v) => write!(f, "Dword({v})"),
            RegValue::Delete => f.write_str("Delete"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegChange {
    /// The key below the hive's root, with single backslashes: `Software\Microsoft\GameInput`.
    pub key: String,
    pub name: String,
    pub value: RegValue,
}

impl RegChange {
    pub fn sz(key: &str, name: &str, value: &str) -> Self {
        Self { key: key.into(), name: name.into(), value: RegValue::Sz(value.into()) }
    }
    pub fn dword(key: &str, name: &str, value: u32) -> Self {
        Self { key: key.into(), name: name.into(), value: RegValue::Dword(value) }
    }
    pub fn delete(key: &str, name: &str) -> Self {
        Self { key: key.into(), name: name.into(), value: RegValue::Delete }
    }
}

// ---------------------------------------------------------------- text escaping

/// Escape text for a Wine registry file. Control characters and anything outside ASCII become `\xNNNN` (UTF-16
/// units, always four digits so a following hex digit cannot be swallowed); that spelling is read by every Wine
/// that reads these files, and it keeps the file ASCII however odd the value is.
fn escape(text: &str, in_key: bool) -> Result<Vec<u8>, PrefixError> {
    if text.contains('\0') {
        return Err(PrefixError::Invalid("registry text cannot hold a NUL".into()));
    }
    let mut out = Vec::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '\\' => out.extend_from_slice(b"\\\\"),
            '"' => out.extend_from_slice(b"\\\""),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            '[' | ']' if in_key => {
                out.push(b'\\');
                out.push(ch as u8);
            }
            c if (c as u32) >= 0x20 && (c as u32) < 0x7f => out.push(c as u8),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.extend_from_slice(format!("\\x{unit:04x}").as_bytes());
                }
            }
        }
    }
    Ok(out)
}

/// Read Wine's escapes back (and raw UTF-8) into the text they stand for, folded to lower case: Wine matches
/// key and value names without regard to case, so `Software\Microsoft` and `SOFTWARE\MICROSOFT` are one key.
fn normalize(raw: &[u8]) -> String {
    let mut units: Vec<u16> = Vec::with_capacity(raw.len());
    let mut plain: Vec<u8> = Vec::new();
    let flush = |plain: &mut Vec<u8>, units: &mut Vec<u16>| {
        if !plain.is_empty() {
            units.extend(String::from_utf8_lossy(plain).encode_utf16());
            plain.clear();
        }
    };
    let mut i = 0;
    while i < raw.len() {
        let b = raw[i];
        if b != b'\\' || i + 1 >= raw.len() {
            plain.push(b);
            i += 1;
            continue;
        }
        flush(&mut plain, &mut units);
        let e = raw[i + 1];
        i += 2;
        match e {
            b'a' => units.push(7),
            b'b' => units.push(8),
            b'e' => units.push(0x1b),
            b'f' => units.push(0x0c),
            b'n' => units.push(b'\n' as u16),
            b'r' => units.push(b'\r' as u16),
            b't' => units.push(b'\t' as u16),
            b'v' => units.push(0x0b),
            b'x' => {
                let mut v: u32 = 0;
                let mut digits = 0;
                while digits < 4 && i < raw.len() {
                    match (raw[i] as char).to_digit(16) {
                        Some(d) => {
                            v = v * 16 + d;
                            i += 1;
                            digits += 1;
                        }
                        None => break,
                    }
                }
                if digits == 0 {
                    units.push(b'x' as u16);
                } else {
                    units.push(v as u16);
                }
            }
            other => units.push(other as u16),
        }
    }
    flush(&mut plain, &mut units);
    String::from_utf16_lossy(&units).to_lowercase()
}

fn normalize_text(text: &str) -> String {
    text.to_lowercase()
}

// ---------------------------------------------------------------- the hive, split once into sections

/// Where each section of a file starts and ends, found in a single pass so that the handful of values PLAY
/// writes never rescan a 100,000-line `system.reg` once per value.
struct Hive<'a> {
    src: &'a [u8],
    /// Everything before the first section.
    preamble_end: usize,
    sections: Vec<Section>,
    /// Normalized key -> indexes in `sections`, in file order (Wine merges duplicates, the last one wins).
    by_key: HashMap<String, Vec<usize>>,
}

struct Section {
    start: usize,
    end: usize,
    /// The edited text of the section, once a change touches it.
    edited: Option<Vec<u8>>,
}

fn is_continued(line: &[u8]) -> bool {
    let mut end = line.len();
    while end > 0 && (line[end - 1] == b'\n' || line[end - 1] == b'\r') {
        end -= 1;
    }
    end > 0 && line[end - 1] == b'\\'
}

/// The key of a `[key] 1234` header line, or None when the line is not a well-formed header.
fn parse_header(line: &[u8]) -> Option<String> {
    debug_assert!(line.first() == Some(&b'['));
    let mut i = 1;
    let mut closed = None;
    while i < line.len() {
        match line[i] {
            b'\\' => i += 2,
            b']' => {
                closed = Some(i);
                break;
            }
            _ => i += 1,
        }
    }
    let close = closed?;
    if close == 1 {
        return None;
    }
    let rest = &line[close + 1..];
    let rest = std::str::from_utf8(rest).ok()?.trim();
    if !(rest.is_empty() || rest.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    Some(normalize(&line[1..close]))
}

impl<'a> Hive<'a> {
    fn parse(src: &'a [u8]) -> Result<Self, PrefixError> {
        const HEADER: &[u8] = b"WINE REGISTRY Version ";
        if !src.starts_with(HEADER) {
            return Err(PrefixError::Malformed("this is not a Wine registry file".into()));
        }
        let first_end = src
            .iter()
            .position(|&b| b == b'\n')
            .ok_or_else(|| PrefixError::Malformed("the Wine registry file is cut off after its first line".into()))?;
        let version = String::from_utf8_lossy(&src[HEADER.len()..first_end]).trim().to_string();
        if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit()) {
            return Err(PrefixError::Malformed("the Wine registry file has no version number".into()));
        }
        if src.contains(&0) {
            return Err(PrefixError::Malformed("the Wine registry file holds binary data".into()));
        }
        if src.last() != Some(&b'\n') {
            return Err(PrefixError::Malformed("the Wine registry file ends in the middle of a line".into()));
        }
        let mut hive = Hive { src, preamble_end: src.len(), sections: Vec::new(), by_key: HashMap::new() };
        let mut continued = false;
        let mut offset = 0usize;
        let mut line_no = 0usize;
        for line in src.split_inclusive(|&b| b == b'\n') {
            line_no += 1;
            let at = offset;
            offset += line.len();
            if continued {
                // A continuation line is indented; anything else means the value was cut off.
                if !matches!(line.first(), Some(b' ' | b'\t')) {
                    return Err(PrefixError::Malformed(format!(
                        "the Wine registry file has a value cut off before line {line_no}"
                    )));
                }
                continued = is_continued(line);
                continue;
            }
            if line.first() == Some(&b'[') {
                let key = parse_header(line).ok_or_else(|| {
                    PrefixError::Malformed(format!(
                        "the Wine registry file has a damaged section header on line {line_no}"
                    ))
                })?;
                if let Some(prev) = hive.sections.last_mut() {
                    prev.end = at;
                } else {
                    hive.preamble_end = at;
                }
                hive.by_key.entry(key).or_default().push(hive.sections.len());
                hive.sections.push(Section { start: at, end: src.len(), edited: None });
                continue;
            }
            continued = is_continued(line);
        }
        if continued {
            return Err(PrefixError::Malformed("the Wine registry file ends in the middle of a value".into()));
        }
        Ok(hive)
    }

    fn section_text(&self, i: usize) -> &[u8] {
        let s = &self.sections[i];
        s.edited.as_deref().unwrap_or(&self.src[s.start..s.end])
    }

    fn render(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.src.len() + 256);
        out.extend_from_slice(&self.src[..self.preamble_end]);
        for i in 0..self.sections.len() {
            out.extend_from_slice(self.section_text(i));
        }
        out
    }

    fn apply(&mut self, change: &RegChange, now: i64) -> Result<(), PrefixError> {
        let key_norm = normalize_text(&change.key);
        let name_norm = normalize_text(&change.name);
        let line = value_line(change)?;
        let delete = change.value == RegValue::Delete;

        let indexes = self.by_key.get(&key_norm).cloned().unwrap_or_default();
        if indexes.is_empty() {
            if delete {
                return Ok(());
            }
            self.append_section(change, &line, now)?;
            return Ok(());
        }
        // Wine merges sections of one key and the last value wins, so a value is changed wherever it appears and
        // a missing one is added to the last section: the result reads the same however the file is split.
        let mut found_any = false;
        for &i in &indexes {
            let text = self.section_text(i).to_vec();
            let (new_text, found) = edit_section(&text, &name_norm, if delete { None } else { Some(&line) });
            found_any |= found;
            if found && new_text != text {
                self.sections[i].edited = Some(new_text);
            }
        }
        if !found_any && !delete {
            let last = *indexes.last().expect("not empty");
            let mut text = self.section_text(last).to_vec();
            // After the last line with anything on it, so the blank lines that separate this section from the next
            // stay where they are and nothing already in the section is reordered.
            let mut insert = 0;
            let mut pos = 0;
            for l in text.split_inclusive(|&b| b == b'\n') {
                pos += l.len();
                if !l.iter().all(|b| b.is_ascii_whitespace()) {
                    insert = pos;
                }
            }
            let mut inserted = line.clone();
            inserted.push(b'\n');
            text.splice(insert..insert, inserted);
            self.sections[last].edited = Some(text);
        }
        Ok(())
    }

    fn append_section(&mut self, change: &RegChange, line: &[u8], now: i64) -> Result<(), PrefixError> {
        let tail: &[u8] = match self.sections.len() {
            0 => &self.src[..self.preamble_end],
            n => self.section_text(n - 1),
        };
        let last_line_blank =
            tail.rsplit(|&b| b == b'\n').nth(1).is_none_or(|l| l.iter().all(|b| b.is_ascii_whitespace()));
        let mut text = Vec::new();
        if !last_line_blank {
            text.push(b'\n');
        }
        // A Unix timestamp after the key and a Windows FILETIME comment: what wineserver itself writes.
        let filetime = ((now + 11_644_473_600) as u128) * 10_000_000;
        text.push(b'[');
        text.extend_from_slice(&escape(&change.key, true)?);
        text.extend_from_slice(format!("] {now}\n#time={filetime:x}\n").as_bytes());
        text.extend_from_slice(line);
        text.extend_from_slice(b"\n\n");
        // Later changes in the same run find the new section like any other.
        let idx = self.sections.len();
        self.sections.push(Section { start: self.src.len(), end: self.src.len(), edited: Some(text) });
        self.by_key.entry(normalize_text(&change.key)).or_default().push(idx);
        Ok(())
    }
}

/// The `"name"=value` line for a change (without its newline); empty for a deletion.
fn value_line(change: &RegChange) -> Result<Vec<u8>, PrefixError> {
    let mut line = Vec::new();
    match &change.value {
        RegValue::Delete => return Ok(line),
        RegValue::Sz(text) => {
            line.push(b'"');
            line.extend_from_slice(&escape(&change.name, false)?);
            line.extend_from_slice(b"\"=\"");
            line.extend_from_slice(&escape(text, false)?);
            line.push(b'"');
        }
        RegValue::Dword(v) => {
            line.push(b'"');
            line.extend_from_slice(&escape(&change.name, false)?);
            line.extend_from_slice(format!("\"=dword:{v:08x}").as_bytes());
        }
    }
    Ok(line)
}

/// The name of the value a line starts, normalized; None for any other line.
fn value_name(line: &[u8]) -> Option<String> {
    match line.first()? {
        b'@' if line.get(1) == Some(&b'=') => Some(String::new()),
        b'"' => {
            let mut i = 1;
            while i < line.len() {
                match line[i] {
                    b'\\' => i += 2,
                    b'"' => {
                        return (line.get(i + 1) == Some(&b'=')).then(|| normalize(&line[1..i]));
                    }
                    _ => i += 1,
                }
            }
            None
        }
        _ => None,
    }
}

/// Replace (`new` = Some) or remove (`new` = None) every value called `name` in one section's text. Returns the
/// new text and whether the value was there.
fn edit_section(text: &[u8], name: &str, new: Option<&[u8]>) -> (Vec<u8>, bool) {
    let mut out = Vec::with_capacity(text.len() + 64);
    let mut found = false;
    let mut replaced = false;
    let mut skipping = false;
    let mut first = true;
    let mut continued = false;
    for line in text.split_inclusive(|&b| b == b'\n') {
        if skipping {
            skipping = is_continued(line);
            continue;
        }
        if continued {
            continued = is_continued(line);
            out.extend_from_slice(line);
            continue;
        }
        if !first && value_name(line).is_some_and(|n| n == name) {
            found = true;
            skipping = is_continued(line);
            if let Some(new) = new {
                if !replaced {
                    replaced = true;
                    // Keep the file's own line ending for the line.
                    out.extend_from_slice(new);
                    if line.ends_with(b"\r\n") {
                        out.extend_from_slice(b"\r\n");
                    } else {
                        out.push(b'\n');
                    }
                }
            }
            continue;
        }
        first = false;
        continued = is_continued(line);
        out.extend_from_slice(line);
    }
    (out, found)
}

/// Render `changes` into a hive's bytes. Refuses a file that is not a Wine registry.
pub fn render(original: &[u8], changes: &[RegChange], now: i64) -> Result<Vec<u8>, PrefixError> {
    let mut hive = Hive::parse(original)?;
    for change in changes {
        hive.apply(change, now)?;
    }
    Ok(hive.render())
}

/// `HKLM\System\Select\Current` of a system hive.
///
/// Wine keeps `CurrentControlSet` as a symbolic link, and its text-hive loader creates paths without following
/// it, so a section written under the literal `System\CurrentControlSet` would hang unreachable below the link.
/// Offline writers must name the concrete `ControlSetNNN` instead.
fn current_control_set(original: &[u8]) -> Result<u32, PrefixError> {
    let hive = Hive::parse(original)?;
    let sections = hive.by_key.get("system\\select").cloned().unwrap_or_default();
    // The last value wins, as Wine merges duplicates in file order.
    let mut current = None;
    for i in sections {
        for line in hive.section_text(i).split(|&b| b == b'\n') {
            if value_name(line).as_deref() == Some("current") {
                let text = String::from_utf8_lossy(line);
                if let Some(hex) = text.trim().split("=dword:").nth(1) {
                    if let Ok(v) = u32::from_str_radix(hex.trim(), 16) {
                        current = Some(v);
                    }
                }
            }
        }
    }
    match current {
        Some(v) if v > 0 => Ok(v),
        _ => Err(PrefixError::Malformed("the Wine registry has no valid System\\Select\\Current".into())),
    }
}

fn resolve_control_set(original: &[u8], changes: &[RegChange]) -> Result<Vec<RegChange>, PrefixError> {
    let alias = |c: &RegChange| {
        let mut parts = c.key.split('\\');
        parts.next().is_some_and(|p| p.eq_ignore_ascii_case("system"))
            && parts.next().is_some_and(|p| p.eq_ignore_ascii_case("currentcontrolset"))
    };
    if !changes.iter().any(alias) {
        return Ok(changes.to_vec());
    }
    let current = current_control_set(original)?;
    Ok(changes
        .iter()
        .map(|c| {
            if !alias(c) {
                return c.clone();
            }
            let mut parts: Vec<&str> = c.key.split('\\').collect();
            let set = format!("ControlSet{current:03}");
            parts[0] = "System";
            parts[1] = &set;
            RegChange { key: parts.join("\\"), name: c.name.clone(), value: c.value.clone() }
        })
        .collect())
}

// ---------------------------------------------------------------- writing the files back

/// The one flock that serializes BlockForge's own offline edits of a prefix.
/// Held, never read: the lock lives as long as the open file.
pub(crate) struct OfflineLock {
    _file: File,
}

impl OfflineLock {
    pub(crate) fn take(prefix: &Path) -> Result<Self, PrefixError> {
        let path = prefix.join(".blockforge-offline-registry.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| PrefixError::io(format!("open {}", path.display()), e))?;
        file.lock().map_err(|e| PrefixError::io("lock the prefix for editing", e))?;
        Ok(Self { _file: file })
    }
}

struct Pending {
    path: PathBuf,
    original: Vec<u8>,
    rendered: Vec<u8>,
}

fn stage(path: &Path, payload: &[u8]) -> io::Result<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let staged = dir.join(format!(".{name}.bf-{}", uuid::Uuid::new_v4().simple()));
    let result = (|| {
        let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&staged)?;
        f.write_all(payload)?;
        f.sync_all()?;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o600))
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&staged);
        return Err(e);
    }
    Ok(staged)
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Stage every file, then swap them in; if a swap fails, put the earlier ones back. `rename` is injectable so a
/// test can fail the second swap.
fn commit(
    prefix: &Path,
    files: &[Pending],
    rename: &mut dyn FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<(), PrefixError> {
    let mut staged: Vec<(usize, PathBuf)> = Vec::new();
    let cleanup = |staged: &[(usize, PathBuf)]| {
        for (_, p) in staged {
            let _ = fs::remove_file(p);
        }
    };
    for (i, f) in files.iter().enumerate() {
        match stage(&f.path, &f.rendered) {
            Ok(p) => staged.push((i, p)),
            Err(e) => {
                cleanup(&staged);
                return Err(PrefixError::io(format!("write {}", f.path.display()), e));
            }
        }
    }
    let mut replaced: Vec<usize> = Vec::new();
    let mut failure = None;
    for (i, p) in &staged {
        match rename(p, &files[*i].path) {
            Ok(()) => replaced.push(*i),
            Err(e) => {
                failure = Some((*i, e));
                break;
            }
        }
    }
    if failure.is_none() {
        if let Err(e) = sync_dir(prefix) {
            failure = Some((usize::MAX, e));
        }
    }
    if let Some((at, e)) = failure {
        // Put every file already swapped back as it was: the whole edit applies or none of it does.
        let mut restore_failed = false;
        for &i in replaced.iter().rev() {
            let back = stage(&files[i].path, &files[i].original).and_then(|p| fs::rename(&p, &files[i].path));
            restore_failed |= back.is_err();
        }
        cleanup(&staged);
        let what = if at == usize::MAX {
            "make the registry change durable".to_string()
        } else {
            format!("replace {}", files[at].path.display())
        };
        let mut err = PrefixError::io(what, e);
        if restore_failed {
            err = PrefixError::Unsafe(format!(
                "{err}; putting the earlier registry file back also failed, repair the prefix"
            ));
        }
        return Err(err);
    }
    cleanup(&staged);
    Ok(())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Apply registry values to a stopped prefix without starting Wine. Both files are rendered and staged before
/// either live file is replaced; a failure after the first swap restores it. The files are forced to mode 0600
/// because the machine hive can hold the Microsoft refresh token. Returns whether anything changed.
pub fn update_prefix_registry(prefix: &Path, machine: &[RegChange], user: &[RegChange]) -> Result<bool, PrefixError> {
    update_with(prefix, machine, user, now_secs(), &mut |from, to| fs::rename(from, to))
}

pub(super) fn update_with(
    prefix: &Path,
    machine: &[RegChange],
    user: &[RegChange],
    now: i64,
    rename: &mut dyn FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<bool, PrefixError> {
    let requested: Vec<(PathBuf, &[RegChange])> =
        [(prefix.join("system.reg"), machine), (prefix.join("user.reg"), user)]
            .into_iter()
            .filter(|(_, changes)| !changes.is_empty())
            .collect();
    if requested.is_empty() {
        return Ok(false);
    }
    let _lock = OfflineLock::take(prefix)?;
    // The lock keeps BlockForge's writers apart; this stops a live wineserver from later overwriting the edit with
    // its in-memory hive.
    require_prefix_idle(prefix, "edit the Wine registry")?;

    let mut pending = Vec::new();
    for (path, changes) in &requested {
        let meta = fs::symlink_metadata(path).map_err(|e| PrefixError::io(format!("read {}", path.display()), e))?;
        if !meta.file_type().is_file() {
            return Err(PrefixError::Unsafe(format!("{} is not a plain file", path.display())));
        }
        let original = fs::read(path).map_err(|e| PrefixError::io(format!("read {}", path.display()), e))?;
        let changes = if path.file_name().is_some_and(|n| n == "system.reg") {
            resolve_control_set(&original, changes)?
        } else {
            changes.to_vec()
        };
        let rendered = render(&original, &changes, now)?;
        pending.push(Pending { path: path.clone(), original, rendered });
    }
    // Tighten permissions of older prefixes even when every value is already right.
    for p in &pending {
        let _ = fs::set_permissions(&p.path, fs::Permissions::from_mode(0o600));
    }
    let changed: Vec<Pending> = pending.into_iter().filter(|p| p.original != p.rendered).collect();
    if changed.is_empty() {
        return Ok(false);
    }
    commit(prefix, &changed, rename)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scratch;

    const NOW: i64 = 1_700_000_000;

    fn hive(extra: &str) -> Vec<u8> {
        format!("WINE REGISTRY Version 2\n;; All keys relative to \\\\Machine\n\n#arch=win64\n\n{extra}").into_bytes()
    }

    fn text(b: &[u8]) -> String {
        String::from_utf8(b.to_vec()).unwrap()
    }

    fn run(original: &[u8], changes: &[RegChange]) -> String {
        text(&render(original, changes, NOW).unwrap())
    }

    #[test]
    fn adds_replaces_and_deletes_values_and_keys() {
        let original = hive(
            "[Software\\\\Wine\\\\WineGDK] 1600000000\n#time=1d1\n\"RefreshToken\"=\"old\"\n\"Other\"=dword:00000001\n\n[Software\\\\Zed] 1\n\"a\"=\"b\"\n",
        );
        let out = run(
            &original,
            &[
                RegChange::sz("Software\\Wine\\WineGDK", "RefreshToken", "new"),
                RegChange::dword("Software\\Wine\\WineGDK", "Added", 8),
                RegChange::delete("Software\\Zed", "a"),
                RegChange::dword("Software\\Microsoft\\GamingServices", "IgnoreVersionMismatch", 1),
            ],
        );
        assert!(out.contains("\"RefreshToken\"=\"new\"\n"));
        assert!(!out.contains("\"old\""));
        assert!(out.contains("\"Added\"=dword:00000008\n"));
        assert!(out.contains("\"Other\"=dword:00000001\n"), "values we do not own stay");
        assert!(!out.contains("\"a\"=\"b\""));
        assert!(out.contains("[Software\\\\Microsoft\\\\GamingServices] 1700000000\n#time=1d"));
        assert!(out.contains("\"IgnoreVersionMismatch\"=dword:00000001\n"));
        // The new value goes in before the blank line that separates the sections.
        assert!(out.contains("\"Added\"=dword:00000008\n\n[Software\\\\Zed]"));
    }

    #[test]
    fn escapes_unicode_quotes_backslashes_and_controls() {
        let value = "C:\\Program Files\\\"Q\"\n\tü😀\u{1}";
        let out = run(&hive(""), &[RegChange::sz("Software\\Ünï\\Sub", "na\"me", value)]);
        assert!(out.contains("[Software\\\\\\x00dcn\\x00ef\\\\Sub] 1700000000"), "{out}");
        assert!(out.contains(r#""na\"me"="C:\\Program Files\\\"Q\"\n\t\x00fc\xd83d\xde00\x0001""#), "{out}");
        assert!(out.is_ascii());
        // And what was written is found again, whichever spelling the file uses for it.
        let again = run(out.as_bytes(), &[RegChange::sz("Software\\Ünï\\Sub", "na\"me", "x")]);
        assert_eq!(again.matches("na\\\"me").count(), 1);
        assert!(again.contains("=\"x\""));
        let raw = hive("[Software\\\\Ünï\\\\Sub] 1\n\"v\"=\"1\"\n");
        let out = run(&raw, &[RegChange::sz("Software\\Ünï\\Sub", "v", "2")]);
        assert_eq!(out.matches("[Software").count(), 1, "a raw UTF-8 key is the same key");
        assert!(out.contains("\"v\"=\"2\""));
    }

    #[test]
    fn keys_and_names_match_without_regard_to_case() {
        let original = hive("[SOFTWARE\\\\Microsoft\\\\GameInput] 1\n\"redistdir\"=\"old\"\n");
        let out = run(&original, &[RegChange::sz("Software\\Microsoft\\GameInput", "RedistDir", "new")]);
        assert_eq!(out.matches("[").count(), 1 + out.matches("[Software\\\\Wine").count(), "{out}");
        assert!(out.contains("\"RedistDir\"=\"new\"\n") && !out.contains("redistdir"));
    }

    #[test]
    fn a_change_is_idempotent() {
        let changes = [
            RegChange::sz("Software\\A\\B", "x", "y"),
            RegChange::dword("Software\\A\\B", "n", 3),
            RegChange::delete("Software\\Gone", "g"),
        ];
        let once = render(&hive(""), &changes, NOW).unwrap();
        let twice = render(&once, &changes, NOW + 99).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn duplicate_sections_all_agree_afterwards() {
        let original = hive(
            "[Software\\\\Dup] 1\n\"v\"=\"first\"\n\n[Software\\\\Mid] 1\n\"m\"=\"1\"\n\n[Software\\\\Dup] 2\n\"v\"=\"second\"\n\n",
        );
        let out = run(&original, &[RegChange::sz("Software\\Dup", "v", "ours")]);
        assert_eq!(out.matches("\"v\"=\"ours\"").count(), 2, "{out}");
        assert!(!out.contains("first") && !out.contains("second"));
        let out = run(&original, &[RegChange::sz("Software\\Dup", "fresh", "1")]);
        // A missing value goes into the section Wine would let win: the last one.
        let last = out.rfind("[Software\\\\Dup]").unwrap();
        assert!(out[last..].contains("\"fresh\"=\"1\""));
        assert!(!out[..last].contains("fresh"));
        let out = run(&original, &[RegChange::delete("Software\\Dup", "v")]);
        assert!(!out.contains("\"v\"="));
        assert!(out.contains("\"m\"=\"1\""));
    }

    #[test]
    fn a_continued_hex_value_is_replaced_whole() {
        let original = hive("[K] 1\n\"b\"=hex:01,02,03,\\\n  04,05,06\n\"keep\"=\"1\"\n");
        let out = run(&original, &[RegChange::sz("K", "b", "text")]);
        assert!(out.contains("\"b\"=\"text\"\n\"keep\"=\"1\"\n"), "{out}");
        assert!(!out.contains("04,05"));
        let out = run(&original, &[RegChange::delete("K", "b")]);
        assert!(!out.contains("04,05") && out.contains("\"keep\"=\"1\""));
    }

    #[test]
    fn current_control_set_is_resolved_to_the_concrete_set() {
        let original = hive("[System\\\\Select] 1\n\"Current\"=dword:00000002\n\n");
        let resolved =
            resolve_control_set(&original, &[RegChange::dword("System\\CurrentControlSet\\Services\\X", "Start", 3)])
                .unwrap();
        assert_eq!(resolved[0].key, "System\\ControlSet002\\Services\\X");
        let none = hive("[Software\\\\X] 1\n");
        assert!(
            resolve_control_set(&none, &[RegChange::dword("System\\CurrentControlSet\\Services\\X", "Start", 3)])
                .is_err()
        );
        let zero = hive("[System\\\\Select] 1\n\"Current\"=dword:00000000\n");
        assert!(resolve_control_set(&zero, &[RegChange::dword("system\\currentcontrolset\\x", "Start", 3)]).is_err());
    }

    #[test]
    fn malformed_hives_are_refused() {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty", b"".to_vec()),
            ("not a hive", b"hello\nworld\n".to_vec()),
            ("no version", b"WINE REGISTRY Version \n".to_vec()),
            ("no newline", b"WINE REGISTRY Version 2".to_vec()),
            ("cut mid-line", hive("[K] 1\n\"a\"=\"b")),
            ("cut mid-value", hive("[K] 1\n\"a\"=hex:01,\\\n")),
            ("damaged header", hive("[K 1\n\"a\"=\"b\"\n")),
            ("empty key", hive("[] 1\n")),
            ("junk after header", hive("[K] soon\n")),
            ("binary junk", {
                let mut v = hive("[K] 1\n");
                v.push(0);
                v.push(b'\n');
                v
            }),
            ("header inside a value", hive("[K] 1\n\"a\"=hex:01,\\\n[L] 2\n")),
        ];
        for (what, bytes) in cases {
            assert!(render(&bytes, &[RegChange::sz("K", "a", "b")], NOW).is_err(), "{what} was not refused");
        }
    }

    #[test]
    fn a_nul_or_empty_key_in_a_change_is_refused() {
        assert!(render(&hive(""), &[RegChange::sz("K", "a", "b\0c")], NOW).is_err());
        assert!(render(&hive(""), &[RegChange::sz("K\0", "a", "b")], NOW).is_err());
    }

    #[test]
    fn crlf_files_keep_their_line_endings() {
        let original = b"WINE REGISTRY Version 2\r\n\r\n[K] 1\r\n\"a\"=\"b\"\r\n\r\n".to_vec();
        let out = text(&render(&original, &[RegChange::sz("K", "a", "c")], NOW).unwrap());
        assert!(out.contains("\"a\"=\"c\"\r\n"), "{out:?}");
    }

    #[test]
    fn writes_both_files_atomically_with_private_permissions() {
        let pfx = scratch("reg-write");
        fs::write(pfx.join("system.reg"), hive("[Software\\\\X] 1\n\"a\"=\"1\"\n")).unwrap();
        fs::write(pfx.join("user.reg"), hive("")).unwrap();
        fs::set_permissions(pfx.join("system.reg"), fs::Permissions::from_mode(0o644)).unwrap();
        let changed = update_prefix_registry(
            &pfx,
            &[RegChange::sz("Software\\X", "a", "2")],
            &[RegChange::sz("Environment", "E", "0")],
        )
        .unwrap();
        assert!(changed);
        assert!(fs::read_to_string(pfx.join("system.reg")).unwrap().contains("\"a\"=\"2\""));
        assert!(fs::read_to_string(pfx.join("user.reg")).unwrap().contains("[Environment]"));
        for f in ["system.reg", "user.reg"] {
            assert_eq!(fs::metadata(pfx.join(f)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let again = update_prefix_registry(&pfx, &[RegChange::sz("Software\\X", "a", "2")], &[]).unwrap();
        assert!(!again, "nothing changed the second time");
        let leftovers: Vec<_> = fs::read_dir(&pfx)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".bf-"))
            .collect();
        assert!(leftovers.is_empty());
        fs::remove_dir_all(&pfx).ok();
    }

    #[test]
    fn a_failure_after_the_first_swap_puts_the_first_file_back() {
        let pfx = scratch("reg-interrupt");
        let system = hive("[Software\\\\X] 1\n\"a\"=\"1\"\n");
        let user = hive("");
        fs::write(pfx.join("system.reg"), &system).unwrap();
        fs::write(pfx.join("user.reg"), &user).unwrap();
        let mut calls = 0;
        let err = update_with(
            &pfx,
            &[RegChange::sz("Software\\X", "a", "2")],
            &[RegChange::sz("Environment", "E", "0")],
            NOW,
            &mut |from, to| {
                calls += 1;
                if calls == 2 { Err(io::Error::other("disk gone")) } else { fs::rename(from, to) }
            },
        );
        assert!(err.is_err());
        assert_eq!(fs::read(pfx.join("system.reg")).unwrap(), system, "the first swap was undone");
        assert_eq!(fs::read(pfx.join("user.reg")).unwrap(), user);
        let leftovers: Vec<_> = fs::read_dir(&pfx)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".bf-"))
            .collect();
        assert!(leftovers.is_empty(), "no staged file is left behind");
        fs::remove_dir_all(&pfx).ok();
    }

    #[test]
    fn a_stale_staged_file_from_a_crash_does_not_matter() {
        let pfx = scratch("reg-stale");
        fs::write(pfx.join("system.reg"), hive("")).unwrap();
        fs::write(pfx.join(".system.reg.bf-deadbeef"), b"half a hive").unwrap();
        assert!(update_prefix_registry(&pfx, &[RegChange::dword("Software\\X", "a", 1)], &[]).unwrap());
        assert!(fs::read_to_string(pfx.join("system.reg")).unwrap().contains("dword:00000001"));
        fs::remove_dir_all(&pfx).ok();
    }

    #[test]
    fn a_malformed_hive_is_left_exactly_as_it_was() {
        let pfx = scratch("reg-bad");
        fs::write(pfx.join("system.reg"), b"WINE REGISTRY Version 2\n[K] 1\n\"a\"=\"b").unwrap();
        fs::write(pfx.join("user.reg"), hive("")).unwrap();
        let before = fs::read(pfx.join("system.reg")).unwrap();
        assert!(
            update_prefix_registry(&pfx, &[RegChange::sz("K", "a", "c")], &[RegChange::sz("E", "x", "y")]).is_err()
        );
        assert_eq!(fs::read(pfx.join("system.reg")).unwrap(), before);
        assert!(
            !fs::read_to_string(pfx.join("user.reg")).unwrap().contains("[E]"),
            "the other hive was not touched either"
        );
        fs::remove_dir_all(&pfx).ok();
    }

    #[test]
    fn a_symlinked_hive_is_refused() {
        let pfx = scratch("reg-link");
        let elsewhere = scratch("reg-link-target");
        fs::write(elsewhere.join("real.reg"), hive("")).unwrap();
        std::os::unix::fs::symlink(elsewhere.join("real.reg"), pfx.join("system.reg")).unwrap();
        assert!(update_prefix_registry(&pfx, &[RegChange::dword("K", "a", 1)], &[]).is_err());
        assert!(!fs::read_to_string(elsewhere.join("real.reg")).unwrap().contains("[K]"));
        fs::remove_dir_all(&pfx).ok();
        fs::remove_dir_all(&elsewhere).ok();
    }

    #[test]
    fn a_live_prefix_is_refused_before_anything_is_read() {
        let pfx = scratch("reg-live");
        fs::write(pfx.join("system.reg"), hive("")).unwrap();
        let mut child = std::process::Command::new("sleep").arg("30").env("WINEPREFIX", &pfx).spawn().unwrap();
        let result = update_prefix_registry(&pfx, &[RegChange::dword("K", "a", 1)], &[]);
        child.kill().ok();
        child.wait().ok();
        let err = result.unwrap_err();
        assert!(matches!(err, PrefixError::Busy(_)), "{err}");
        assert_eq!(fs::read(pfx.join("system.reg")).unwrap(), hive(""));
        fs::remove_dir_all(&pfx).ok();
    }

    #[test]
    fn a_large_hive_is_edited_quickly() {
        let mut big = String::new();
        for i in 0..4000 {
            big.push_str(&format!("[Software\\\\K{i}] 1\n"));
            for j in 0..25 {
                big.push_str(&format!("\"v{j}\"=\"x\"\n"));
            }
            big.push('\n');
        }
        let started = std::time::Instant::now();
        let out =
            render(&hive(&big), &[RegChange::sz("Software\\K3999", "v3", "y"), RegChange::dword("New", "a", 1)], NOW)
                .unwrap();
        assert!(started.elapsed().as_millis() < 500);
        assert!(text(&out).contains("\"v3\"=\"y\""));
    }
}
