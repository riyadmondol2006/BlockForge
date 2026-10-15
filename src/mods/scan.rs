//! The scan run before the game starts (`scanModsOnLaunch`): every mod and pack is checked for the code of the
//! Fracturiser malware that spread through Minecraft mod sites in 2023. This is a port of the signatures in
//! MCRcortex's nekodetector, which the Java edition uses: the same three checks on the same bytecode patterns.
//!
//! A jar that has been found clean is remembered by its sha1 (and, for unchanged files, by size and modified
//! time), so launching does not read every jar again.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Map, Value};

use super::ModSettings;
use super::identify::sha1_file;
use crate::config::write_json_atomic;
use crate::instances::Instance;
use crate::paths::Dirs;

/// What a scan found, for the launcher to show before starting the game.
#[derive(Clone, Debug, Default)]
pub struct ScanReport {
    /// Files whose code was read this time.
    pub scanned: usize,
    /// Files skipped because they were already known to be clean.
    pub known_clean: usize,
    /// Files that contain the malware's code.
    pub infected: Vec<PathBuf>,
    /// Files that could not be read at all (damaged or not a zip). These are not counted as infected.
    pub unreadable: Vec<PathBuf>,
    /// `false` when scanning is switched off in the settings.
    pub enabled: bool,
}

impl ScanReport {
    /// Whether the game may start.
    pub fn is_clean(&self) -> bool {
        self.infected.is_empty()
    }

    /// A message for the player when something was found, or `None`.
    pub fn warning(&self) -> Option<String> {
        if self.infected.is_empty() {
            return None;
        }
        let names: Vec<String> = self.infected.iter().map(|p| p.display().to_string()).collect();
        Some(format!(
            "Malicious code (Fractureiser) was found in {} of your mods, so the game was not started. Delete these files and change your passwords:\n{}",
            self.infected.len(),
            names.join("\n")
        ))
    }
}

/// The files looked at on launch: jars, zips and litemods in `mods`, `datapacks`, `resourcepacks`, `shaderpacks` and
/// `jarmods`. Not folders, and not `disabledmods`, since those do not run.
pub fn paths_to_scan(instance: &Instance) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for folder in ["mods", "datapacks", "resourcepacks", "shaderpacks", "jarmods"] {
        let Ok(rd) = std::fs::read_dir(instance.root.join(folder)) else { continue };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file() && p.file_name().is_some_and(|n| super::files::is_accepted_mod_file(&n.to_string_lossy()))
            })
            .collect();
        files.sort();
        out.extend(files);
    }
    out
}

/// Scans an instance before launch. Does nothing (and says so) when `scanModsOnLaunch` is off.
pub fn scan_instance(dirs: &Dirs, instance: &Instance, settings: &ModSettings) -> ScanReport {
    if !settings.scan_mods_on_launch {
        return ScanReport { enabled: false, ..Default::default() };
    }
    let mut report = scan_paths(dirs, &paths_to_scan(instance));
    report.enabled = true;
    report
}

/// Scans these files, four at a time.
pub fn scan_paths(dirs: &Dirs, paths: &[PathBuf]) -> ScanReport {
    let mut report = ScanReport { enabled: true, ..Default::default() };
    // nothing to scan (no mods), so nothing to load, start or write
    if paths.is_empty() {
        return report;
    }
    let hashes_file = dirs.cache.join("fracturiser_scanned_hashes.json");
    let prints_file = dirs.cache.join("file-fingerprints.json");
    let known: Mutex<HashSet<String>> = Mutex::new(load_hashes(&hashes_file).into_iter().collect());
    let known_before = known.lock().map(|k| k.len()).unwrap_or(0);
    let prints = Mutex::new(Fingerprints::load(&prints_file));
    let found = Mutex::new(Vec::<PathBuf>::new());
    let unreadable = Mutex::new(Vec::<PathBuf>::new());
    let (scanned, skipped) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let next = AtomicUsize::new(0);

    std::thread::scope(|s| {
        for _ in 0..paths.len().min(4) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(path) = paths.get(i) else { return };
                    // a jar that hasn't changed since it was last found clean doesn't need to be read again
                    if prints.lock().map(|p| p.get(path, "fractureiser").is_some()).unwrap_or(false) {
                        skipped.fetch_add(1, Ordering::SeqCst);
                        continue;
                    }
                    let Ok(hash) = sha1_file(path) else {
                        unreadable.lock().unwrap_or_else(|e| e.into_inner()).push(path.clone());
                        continue;
                    };
                    if known.lock().map(|k| k.contains(&hash)).unwrap_or(false) {
                        if let Ok(mut p) = prints.lock() {
                            p.put(path, "fractureiser", "clean");
                        }
                        skipped.fetch_add(1, Ordering::SeqCst);
                        continue;
                    }
                    match scan_jar(path) {
                        Ok(true) => found.lock().unwrap_or_else(|e| e.into_inner()).push(path.clone()),
                        Ok(false) => {
                            scanned.fetch_add(1, Ordering::SeqCst);
                            if let Ok(mut k) = known.lock() {
                                k.insert(hash);
                            }
                            if let Ok(mut p) = prints.lock() {
                                p.put(path, "fractureiser", "clean");
                            }
                        }
                        Err(_) => unreadable.lock().unwrap_or_else(|e| e.into_inner()).push(path.clone()),
                    }
                }
            });
        }
    });

    report.scanned = scanned.load(Ordering::SeqCst);
    report.known_clean = skipped.load(Ordering::SeqCst);
    report.infected = found.into_inner().unwrap_or_default();
    report.infected.sort();
    report.unreadable = unreadable.into_inner().unwrap_or_default();
    // only write the list when something new was scanned
    if let Ok(k) = known.lock() {
        if k.len() != known_before {
            let mut list: Vec<&String> = k.iter().collect();
            list.sort();
            let _ = write_json_atomic(&hashes_file, &list);
        }
    }
    if let Ok(p) = prints.lock() {
        p.save(&prints_file);
    }
    report
}

/// Scans one jar: true when any class in it matches a signature. A file that is not a zip is an error.
pub fn scan_jar(path: &Path) -> Result<bool, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else { continue };
        let Ok(name) = entry.name().map(|n| n.to_string()) else { continue };
        if !name.ends_with(".class") {
            continue;
        }
        let mut bytes = Vec::with_capacity(entry.size().min(16 << 20) as usize);
        if entry.by_ref().take(64 << 20).read_to_end(&mut bytes).is_err() {
            continue;
        }
        if scan_class(&bytes) {
            return Ok(true);
        }
    }
    Ok(false)
}

// ---------------------------------------------------------------------------------------------------------
// Remembering clean files

fn load_hashes(path: &Path) -> Vec<String> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Facts about files together with the size and modified time they had when the fact was learned (the Java
/// edition's `file-fingerprints.json`: `"kind|/absolute/path": "size|modified-millis|value"`).
struct Fingerprints {
    entries: Map<String, Value>,
    dirty: bool,
}

impl Fingerprints {
    fn load(path: &Path) -> Fingerprints {
        let entries = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Map<String, Value>>(&b).ok())
            .unwrap_or_default();
        Fingerprints { entries, dirty: false }
    }

    fn key(file: &Path, kind: &str) -> String {
        let abs = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
        format!("{kind}|{}", abs.display())
    }

    fn stamp(file: &Path) -> Option<String> {
        let meta = std::fs::metadata(file).ok()?;
        if !meta.is_file() {
            return None;
        }
        let millis = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
        Some(format!("{}|{millis}|", meta.len()))
    }

    fn get(&self, file: &Path, kind: &str) -> Option<String> {
        let entry = self.entries.get(&Self::key(file, kind))?.as_str()?;
        let stamp = Self::stamp(file)?;
        entry.strip_prefix(stamp.as_str()).map(str::to_string)
    }

    fn put(&mut self, file: &Path, kind: &str, value: &str) {
        let Some(stamp) = Self::stamp(file) else { return };
        let new = format!("{stamp}{value}");
        let key = Self::key(file, kind);
        if self.entries.get(&key).and_then(Value::as_str) != Some(new.as_str()) {
            self.entries.insert(key, Value::String(new));
            self.dirty = true;
        }
    }

    fn save(&self, path: &Path) {
        if self.dirty {
            let _ = write_json_atomic(path, &self.entries);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// Reading Java class files

const NEW: u8 = 0xbb;
const INVOKEVIRTUAL: u8 = 0xb6;
const INVOKESPECIAL: u8 = 0xb7;
const INVOKESTATIC: u8 = 0xb8;
const BIPUSH: u8 = 0x10;
const BASTORE: u8 = 0x54;
const DUP: u8 = 0x59;
const ICONST_1: u8 = 0x04;

/// One instruction with the operands the signatures look at.
#[derive(Clone, Debug)]
enum Insn {
    /// `new`: the class name.
    Type(u8, String),
    /// A method call: owner, name and descriptor.
    Method(u8, String, String, String),
    /// `bipush`: its number.
    Int(u8, i32),
    /// Anything else.
    Plain(u8),
}

impl Insn {
    fn opcode(&self) -> u8 {
        match self {
            Insn::Type(o, _) | Insn::Method(o, ..) | Insn::Int(o, _) | Insn::Plain(o) => *o,
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn u1(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }
    fn u2(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes([self.u1()?, self.u1()?]))
    }
    fn u4(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes([self.u1()?, self.u1()?, self.u1()?, self.u1()?]))
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.pos = self.pos.checked_add(n).filter(|p| *p <= self.data.len())?;
        Some(())
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|p| *p <= self.data.len())?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Some(s)
    }
}

#[derive(Clone)]
enum Const {
    None,
    Utf8(String),
    Class(u16),
    NameAndType(u16, u16),
    Ref(u16, u16),
    Other,
}

struct Pool(Vec<Const>);

impl Pool {
    fn utf8(&self, i: u16) -> Option<&str> {
        match self.0.get(i as usize)? {
            Const::Utf8(s) => Some(s),
            _ => None,
        }
    }
    fn class(&self, i: u16) -> Option<&str> {
        match self.0.get(i as usize)? {
            Const::Class(n) => self.utf8(*n),
            _ => None,
        }
    }
    fn method(&self, i: u16) -> Option<(String, String, String)> {
        let Const::Ref(class, nat) = self.0.get(i as usize)? else { return None };
        let Const::NameAndType(name, desc) = self.0.get(*nat as usize)? else { return None };
        Some((self.class(*class)?.to_string(), self.utf8(*name)?.to_string(), self.utf8(*desc)?.to_string()))
    }
}

fn read_pool(r: &mut Reader) -> Option<Pool> {
    let count = r.u2()? as usize;
    let mut pool = vec![Const::None; count];
    let mut i = 1;
    while i < count {
        let tag = r.u1()?;
        pool[i] = match tag {
            1 => {
                let len = r.u2()? as usize;
                Const::Utf8(modified_utf8(r.bytes(len)?))
            }
            3 | 4 => {
                r.skip(4)?;
                Const::Other
            }
            5 | 6 => {
                r.skip(8)?;
                i += 1; // longs and doubles take two slots
                Const::Other
            }
            7 => Const::Class(r.u2()?),
            8 | 16 | 19 | 20 => {
                r.skip(2)?;
                Const::Other
            }
            9 | 10 | 11 => Const::Ref(r.u2()?, r.u2()?),
            12 => Const::NameAndType(r.u2()?, r.u2()?),
            15 => {
                r.skip(3)?;
                Const::Other
            }
            17 | 18 => {
                r.skip(4)?;
                Const::Other
            }
            _ => return None,
        };
        i += 1;
    }
    Some(Pool(pool))
}

/// Class files use a variant of UTF-8; for names and descriptors a lossy read is exact.
fn modified_utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The code of every method in a class file, as instructions. `None` when the file is not a readable class.
fn read_methods(data: &[u8]) -> Option<Vec<Vec<Insn>>> {
    let mut r = Reader { data, pos: 0 };
    if r.u4()? != 0xCAFE_BABE {
        return None;
    }
    r.skip(4)?; // minor and major version
    let pool = read_pool(&mut r)?;
    r.skip(6)?; // access, this, super
    let interfaces = r.u2()? as usize;
    r.skip(interfaces * 2)?;
    for _ in 0..r.u2()? {
        // fields
        r.skip(6)?;
        skip_attributes(&mut r)?;
    }
    let methods = r.u2()?;
    let mut out = Vec::new();
    for _ in 0..methods {
        r.skip(6)?; // access, name, descriptor
        let attributes = r.u2()?;
        let mut insns = Vec::new();
        for _ in 0..attributes {
            let name = r.u2()?;
            let len = r.u4()? as usize;
            let body = r.bytes(len)?;
            if pool.utf8(name) == Some("Code") {
                insns = read_code(body, &pool)?;
            }
        }
        out.push(insns);
    }
    Some(out)
}

fn skip_attributes(r: &mut Reader) -> Option<()> {
    for _ in 0..r.u2()? {
        r.skip(2)?;
        let len = r.u4()? as usize;
        r.skip(len)?;
    }
    Some(())
}

fn read_code(body: &[u8], pool: &Pool) -> Option<Vec<Insn>> {
    let mut r = Reader { data: body, pos: 4 }; // max stack and max locals
    let len = r.u4()? as usize;
    let code = r.bytes(len)?;
    let mut insns = Vec::new();
    let mut pc = 0usize;
    while pc < code.len() {
        let op = code[pc];
        let at = |n: usize| code.get(pc + n).copied();
        let u2 = |n: usize| Some(u16::from_be_bytes([at(n)?, at(n + 1)?]));
        let (insn, size) = match op {
            NEW => (Insn::Type(op, pool.class(u2(1)?)?.to_string()), 3),
            INVOKEVIRTUAL | INVOKESPECIAL | INVOKESTATIC => {
                let (owner, name, desc) = pool.method(u2(1)?)?;
                (Insn::Method(op, owner, name, desc), 3)
            }
            BIPUSH => (Insn::Int(op, at(1)? as i8 as i32), 2),
            0x11 | 0x13 | 0x14 | 0x84 | 0x99..=0xa8 | 0xb2..=0xb5 | 0xbd | 0xc0 | 0xc1 | 0xc6 | 0xc7 => {
                (Insn::Plain(op), 3)
            }
            0x12 | 0x15..=0x19 | 0x36..=0x3a | 0xa9 | 0xbc => (Insn::Plain(op), 2),
            0xb9 | 0xba | 0xc8 | 0xc9 => (Insn::Plain(op), 5),
            0xc5 => (Insn::Plain(op), 4),
            0xc4 => (Insn::Plain(op), if at(1)? == 0x84 { 6 } else { 4 }),
            0xaa => {
                let pad = (4 - (pc + 1) % 4) % 4;
                let base = pc + 1 + pad;
                let be = |off: usize| -> Option<i32> {
                    Some(i32::from_be_bytes([
                        *code.get(base + off)?,
                        *code.get(base + off + 1)?,
                        *code.get(base + off + 2)?,
                        *code.get(base + off + 3)?,
                    ]))
                };
                let (low, high) = (be(4)?, be(8)?);
                let n = usize::try_from(high.checked_sub(low)?.checked_add(1)?).ok()?;
                (Insn::Plain(op), 1 + pad + 12 + n.checked_mul(4)?)
            }
            0xab => {
                let pad = (4 - (pc + 1) % 4) % 4;
                let base = pc + 1 + pad;
                let be = |off: usize| -> Option<i32> {
                    Some(i32::from_be_bytes([
                        *code.get(base + off)?,
                        *code.get(base + off + 1)?,
                        *code.get(base + off + 2)?,
                        *code.get(base + off + 3)?,
                    ]))
                };
                let pairs = usize::try_from(be(4)?).ok()?;
                (Insn::Plain(op), 1 + pad + 8 + pairs.checked_mul(8)?)
            }
            0x00..=0x0f | 0x1a..=0x35 | 0x3b..=0x83 | 0x85..=0x98 | 0xac..=0xb1 | 0xbe | 0xbf | 0xc2 | 0xc3 => {
                (Insn::Plain(op), 1)
            }
            _ => return None,
        };
        insns.push(insn);
        pc = pc.checked_add(size)?;
    }
    Some(insns)
}

// ---------------------------------------------------------------------------------------------------------
// The signatures

fn m(op: u8, owner: &str, name: &str, desc: &str) -> Insn {
    Insn::Method(op, owner.into(), name.into(), desc.into())
}

/// Signature 1 is a hard detection: the loader that builds a URL class loader from byte arrays.
fn sig1() -> Vec<Insn> {
    let s_init = || m(INVOKESPECIAL, "java/lang/String", "<init>", "([B)V");
    let new_string = || Insn::Type(NEW, "java/lang/String".into());
    vec![
        new_string(),
        s_init(),
        new_string(),
        s_init(),
        m(INVOKESTATIC, "java/lang/Class", "forName", "(Ljava/lang/String;)Ljava/lang/Class;"),
        m(INVOKEVIRTUAL, "java/lang/Class", "getConstructor", "([Ljava/lang/Class;)Ljava/lang/reflect/Constructor;"),
        s_init(),
        s_init(),
        s_init(),
        m(INVOKESPECIAL, "java/net/URL", "<init>", "(Ljava/lang/String;Ljava/lang/String;ILjava/lang/String;)V"),
        m(INVOKEVIRTUAL, "java/lang/reflect/Constructor", "newInstance", "([Ljava/lang/Object;)Ljava/lang/Object;"),
        m(INVOKESTATIC, "java/lang/Class", "forName", "(Ljava/lang/String;ZLjava/lang/ClassLoader;)Ljava/lang/Class;"),
        s_init(),
        m(
            INVOKEVIRTUAL,
            "java/lang/Class",
            "getMethod",
            "(Ljava/lang/String;[Ljava/lang/Class;)Ljava/lang/reflect/Method;",
        ),
        m(
            INVOKEVIRTUAL,
            "java/lang/reflect/Method",
            "invoke",
            "(Ljava/lang/Object;[Ljava/lang/Object;)Ljava/lang/Object;",
        ),
    ]
}

/// Signature 2 is a near-certain detection: a base64 decoded command handed to `Runtime.exec`.
fn sig2() -> Vec<Insn> {
    vec![
        m(INVOKESTATIC, "java/lang/Runtime", "getRuntime", "()Ljava/lang/Runtime;"),
        m(INVOKESTATIC, "java/util/Base64", "getDecoder", "()Ljava/util/Base64$Decoder;"),
        m(INVOKEVIRTUAL, "java/lang/String", "concat", "(Ljava/lang/String;)Ljava/lang/String;"),
        m(INVOKEVIRTUAL, "java/util/Base64$Decoder", "decode", "(Ljava/lang/String;)[B"),
        m(INVOKESPECIAL, "java/lang/String", "<init>", "([B)V"),
        m(INVOKEVIRTUAL, "java/io/File", "getPath", "()Ljava/lang/String;"),
        m(INVOKEVIRTUAL, "java/lang/Runtime", "exec", "([Ljava/lang/String;)Ljava/lang/Process;"),
    ]
}

/// Signature 3 is a likely detection: the malware's server address written out one byte at a time.
fn sig3() -> Vec<Insn> {
    // the address "85.217.144.130" as it is stored in a byte array, one `array[i] = byte` after another
    let digits: [i32; 14] = [56, 53, 46, 50, 49, 55, 46, 49, 52, 52, 46, 49, 51, 48];
    let mut v = Vec::new();
    for (i, d) in digits.iter().enumerate() {
        if i > 0 {
            v.push(Insn::Plain(DUP));
            match i {
                1..=5 => v.push(Insn::Plain(ICONST_1 + (i as u8 - 1))),
                _ => v.push(Insn::Int(BIPUSH, i as i32)),
            }
        }
        v.push(Insn::Int(BIPUSH, *d));
        if i + 1 < digits.len() {
            v.push(Insn::Plain(BASTORE));
        }
    }
    v
}

/// The Java edition's `same`: for the instruction kinds the signatures use, are two instructions the same?
fn same(a: &Insn, b: &Insn) -> bool {
    match (a, b) {
        (Insn::Type(_, x), Insn::Type(_, y)) => x == y,
        (Insn::Method(_, o1, n1, d1), Insn::Method(_, o2, n2, d2)) => o1 == o2 && n1 == n2 && d1 == d2,
        (Insn::Plain(_), _) | (Insn::Int(..), _) => true,
        _ => false,
    }
}

/// Whether a class file contains one of the malware's signatures. A class that cannot be read counts as clean.
pub fn scan_class(data: &[u8]) -> bool {
    let Some(methods) = read_methods(data) else { return false };
    let (s1, s2, s3) = (sig1(), sig2(), sig3());
    methods.iter().any(|insns| match_sig1(insns, &s1) || match_sig2(insns, &s2) || match_sig3(insns, &s3))
}

/// A hard detect: the instructions with the signature's opcodes must be the signature, in order, all of them.
fn match_sig1(insns: &[Insn], sig: &[Insn]) -> bool {
    let mut matched = true;
    let mut j = 0;
    for insn in insns {
        if j >= sig.len() {
            break;
        }
        if insn.opcode() == sig[j].opcode() {
            let ok = same(insn, &sig[j]);
            j += 1;
            if !ok {
                matched = false;
                break;
            }
        }
    }
    if j != sig.len() {
        matched = false;
    }
    matched
}

/// A near-hard detect: the signature's calls appear in order somewhere in the method. Only starts at calls of
/// `Runtime.getRuntime`'s opcode matter, since any other start would reach the same place.
fn match_sig2(insns: &[Insn], sig: &[Insn]) -> bool {
    'outer: for q in 0..insns.len() {
        if insns[q].opcode() != sig[0].opcode() {
            continue;
        }
        let mut j = 0;
        for insn in &insns[q..] {
            if j >= sig.len() {
                break;
            }
            if insn.opcode() != sig[j].opcode() {
                continue;
            }
            let ok = same(insn, &sig[j]);
            j += 1;
            if !ok {
                continue 'outer;
            }
        }
        if j == sig.len() {
            return true;
        }
    }
    false
}

/// A likely match: the byte array holding the address. This keeps the original's way of counting, including that a
/// number that doesn't match leaves the position where it was and that the method's last instruction decides
/// whether a partly matched run still counts.
fn match_sig3(insns: &[Insn], sig: &[Insn]) -> bool {
    let (mut matched, mut pos) = (false, 0usize);
    for insn in insns {
        if pos == sig.len() {
            break;
        }
        if insn.opcode() == sig[pos].opcode() {
            match (&sig[pos], insn) {
                (Insn::Int(_, want), Insn::Int(_, got)) => {
                    if want == got {
                        matched = true;
                        pos += 1;
                    }
                }
                _ => {
                    matched = true;
                    pos += 1;
                }
            }
        } else {
            matched = false;
            pos = 0;
        }
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Builds a class file with one method whose code is given as instructions, to test the signatures against.
    struct ClassBuilder {
        pool: Vec<Vec<u8>>,
    }

    impl ClassBuilder {
        fn new() -> Self {
            ClassBuilder { pool: Vec::new() }
        }
        fn add(&mut self, bytes: Vec<u8>) -> u16 {
            self.pool.push(bytes);
            self.pool.len() as u16
        }
        fn utf8(&mut self, s: &str) -> u16 {
            let mut b = vec![1];
            b.extend((s.len() as u16).to_be_bytes());
            b.extend(s.as_bytes());
            self.add(b)
        }
        fn class(&mut self, name: &str) -> u16 {
            let n = self.utf8(name);
            let mut b = vec![7];
            b.extend(n.to_be_bytes());
            self.add(b)
        }
        fn method(&mut self, owner: &str, name: &str, desc: &str) -> u16 {
            let c = self.class(owner);
            let (n, d) = (self.utf8(name), self.utf8(desc));
            let mut nat = vec![12];
            nat.extend(n.to_be_bytes());
            nat.extend(d.to_be_bytes());
            let nat = self.add(nat);
            let mut b = vec![10];
            b.extend(c.to_be_bytes());
            b.extend(nat.to_be_bytes());
            self.add(b)
        }

        /// Turns instructions into bytecode, adding what they refer to to the pool.
        fn code(&mut self, insns: &[Insn]) -> Vec<u8> {
            let mut code = Vec::new();
            for i in insns {
                match i {
                    Insn::Type(op, name) => {
                        let c = self.class(name);
                        code.push(*op);
                        code.extend(c.to_be_bytes());
                    }
                    Insn::Method(op, o, n, d) => {
                        let mi = self.method(o, n, d);
                        code.push(*op);
                        code.extend(mi.to_be_bytes());
                    }
                    Insn::Int(op, v) => {
                        code.push(*op);
                        code.push(*v as i8 as u8);
                    }
                    Insn::Plain(op) => code.push(*op),
                }
            }
            code.push(0xb1); // return
            code
        }

        fn build(mut self, insns: &[Insn]) -> Vec<u8> {
            let code = self.code(insns);
            let code_name = self.utf8("Code");
            let (mname, mdesc) = (self.utf8("run"), self.utf8("()V"));
            let this = self.class("Evil");
            let mut out = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 52];
            out.extend((self.pool.len() as u16 + 1).to_be_bytes());
            for p in &self.pool {
                out.extend(p);
            }
            out.extend([0, 0x21]); // access
            out.extend(this.to_be_bytes());
            out.extend([0, 0]); // super
            out.extend([0, 0]); // interfaces
            out.extend([0, 0]); // fields
            out.extend([0, 1]); // methods
            out.extend([0, 1]);
            out.extend(mname.to_be_bytes());
            out.extend(mdesc.to_be_bytes());
            out.extend([0, 1]); // attributes
            out.extend(code_name.to_be_bytes());
            let mut body = vec![0, 4, 0, 4];
            body.extend((code.len() as u32).to_be_bytes());
            body.extend(&code);
            body.extend([0, 0, 0, 0]); // exception table, attributes
            out.extend((body.len() as u32).to_be_bytes());
            out.extend(body);
            out.extend([0, 0]); // class attributes
            out
        }
    }

    fn class_with(insns: &[Insn]) -> Vec<u8> {
        ClassBuilder::new().build(insns)
    }

    #[test]
    fn signature_one_is_a_hard_detect() {
        let infected = class_with(&sig1());
        assert!(scan_class(&infected));
        // an unrelated call in the middle of the signature, using the same opcode, breaks the match
        let mut broken = sig1();
        broken[4] = m(INVOKESTATIC, "java/lang/Class", "forName", "(Ljava/lang/String;)Ljava/lang/Object;");
        assert!(!scan_class(&class_with(&broken)));
        // only part of it is not enough
        assert!(!scan_class(&class_with(&sig1()[..10])));
    }

    #[test]
    fn signature_two_finds_a_base64_command_run() {
        let mut insns = vec![Insn::Plain(0x00), m(INVOKESTATIC, "a/B", "c", "()V")];
        insns.extend(sig2());
        insns.push(Insn::Plain(0x57));
        assert!(scan_class(&class_with(&insns)));
        // the same calls with the exec missing
        let mut shorter = sig2();
        shorter.pop();
        assert!(!scan_class(&class_with(&shorter)));
    }

    #[test]
    fn signature_three_finds_the_address_bytes() {
        let mut insns = vec![Insn::Plain(0x00)];
        insns.extend(sig3());
        assert!(scan_class(&class_with(&insns)));
        // a different address is not the malware
        let mut other = sig3();
        other[0] = Insn::Int(BIPUSH, 57);
        let mut insns = vec![Insn::Plain(0x00)];
        insns.extend(other);
        assert!(!scan_class(&class_with(&insns)));
    }

    #[test]
    fn ordinary_and_broken_classes_are_clean() {
        assert!(!scan_class(&class_with(&[Insn::Plain(0x2a), m(INVOKESPECIAL, "java/lang/Object", "<init>", "()V")])));
        assert!(!scan_class(&[]));
        assert!(!scan_class(b"not a class file at all"));
        let mut cut = class_with(&sig1());
        cut.truncate(cut.len() / 2);
        assert!(!scan_class(&cut));
    }

    #[test]
    fn switch_instructions_are_stepped_over() {
        // tableswitch with padding, then the signature after it
        let mut b = ClassBuilder::new();
        let mut code = vec![0x03, 0xaa]; // iconst_0, tableswitch at pc 1: padding to pc 4
        code.extend([0, 0]);
        code.extend(0i32.to_be_bytes()); // default
        code.extend(0i32.to_be_bytes()); // low
        code.extend(1i32.to_be_bytes()); // high
        code.extend(0i32.to_be_bytes());
        code.extend(0i32.to_be_bytes());
        let sig = b.code(&sig1());
        code.extend(&sig[..sig.len() - 1]);
        code.push(0xb1);
        // rebuild the class by hand around this code
        let code_name = b.utf8("Code");
        let (mname, mdesc) = (b.utf8("run"), b.utf8("()V"));
        let this = b.class("Evil");
        let mut out = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 52];
        out.extend((b.pool.len() as u16 + 1).to_be_bytes());
        for p in &b.pool {
            out.extend(p);
        }
        out.extend([0, 0x21]);
        out.extend(this.to_be_bytes());
        out.extend([0, 0, 0, 0, 0, 0, 0, 1, 0, 1]);
        out.extend(mname.to_be_bytes());
        out.extend(mdesc.to_be_bytes());
        out.extend([0, 1]);
        out.extend(code_name.to_be_bytes());
        let mut body = vec![0, 4, 0, 4];
        body.extend((code.len() as u32).to_be_bytes());
        body.extend(&code);
        body.extend([0, 0, 0, 0]);
        out.extend((body.len() as u32).to_be_bytes());
        out.extend(body);
        out.extend([0, 0]);
        assert!(scan_class(&out));
    }

    fn jar_with(dir: &Path, name: &str, class: Option<&[u8]>) -> PathBuf {
        let path = dir.join(name);
        let mut z = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        z.start_file("fabric.mod.json", opts).unwrap();
        z.write_all(b"{}").unwrap();
        if let Some(c) = class {
            z.start_file("a/Evil.class", opts).unwrap();
            z.write_all(c).unwrap();
        }
        z.finish().unwrap();
        path
    }

    #[test]
    fn a_scan_reports_infected_jars_and_remembers_clean_ones() {
        let base = std::env::temp_dir().join(format!("blockforge-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dirs = Dirs::at(&base);
        std::fs::create_dir_all(&dirs.cache).unwrap();
        let mods = base.join("inst/mods");
        std::fs::create_dir_all(&mods).unwrap();
        let clean = jar_with(&mods, "clean.jar", Some(&class_with(&[Insn::Plain(0x2a)])));
        let evil = jar_with(&mods, "evil.jar", Some(&class_with(&sig1())));
        std::fs::write(mods.join("broken.jar"), b"nope").unwrap();

        let first = scan_paths(&dirs, &[clean.clone(), evil.clone(), mods.join("broken.jar")]);
        assert_eq!(first.infected, vec![evil.clone()]);
        assert_eq!(first.scanned, 1);
        assert_eq!(first.unreadable, vec![mods.join("broken.jar")]);
        assert!(!first.is_clean());
        assert!(first.warning().unwrap().contains("evil.jar"));
        // the clean jar's hash is saved, in the Java edition's format
        let saved: Vec<String> =
            serde_json::from_slice(&std::fs::read(dirs.cache.join("fracturiser_scanned_hashes.json")).unwrap())
                .unwrap();
        assert_eq!(saved, vec![sha1_file(&clean).unwrap()]);
        let prints: Map<String, Value> =
            serde_json::from_slice(&std::fs::read(dirs.cache.join("file-fingerprints.json")).unwrap()).unwrap();
        assert!(prints.keys().any(|k| k.starts_with("fractureiser|") && k.ends_with("clean.jar")));

        let second = scan_paths(&dirs, std::slice::from_ref(&clean));
        assert_eq!((second.scanned, second.known_clean), (0, 1));
        assert!(second.is_clean() && second.warning().is_none());

        // an empty list scans and writes nothing
        assert_eq!(scan_paths(&Dirs::at(base.join("other")), &[]).scanned, 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn scanning_can_be_switched_off() {
        let mut inst = super::super::files::tests_support::temp_instance("scanoff");
        let dirs = Dirs::at(inst.root.join("data"));
        let mut s = ModSettings::default();
        s.scan_mods_on_launch = false;
        let r = scan_instance(&dirs, &inst, &s);
        assert!(!r.enabled && r.is_clean());
        inst.file.launcher.name = "x".into();
        let _ = std::fs::remove_dir_all(&inst.root);
    }
}
