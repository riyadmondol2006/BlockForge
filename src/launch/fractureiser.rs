//! Looks inside mods for the code signatures of the Fractureiser malware before a game starts (the `scanModsOnLaunch`
//! setting). A port of the Java edition's check, which uses the open source "nekodetector" by MCRcortex: it reads every
//! class in a jar and compares the instructions of each method with three known patterns of the first stage of the
//! malware. A jar that was found clean is remembered by its checksum, so it is read once.
//!
//! This reads class files without running anything.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha1::{Digest, Sha1};

use crate::paths::Dirs;

/// One real instruction of a method, as far as the patterns care.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Insn {
    /// `new`, `anewarray`, `checkcast`, `instanceof`, with the class name.
    Type { opcode: u8, desc: String },
    /// An `invoke...` instruction.
    Method { opcode: u8, owner: String, name: String, desc: String },
    /// `bipush`, `sipush`, `newarray`.
    Int { opcode: u8, operand: i32 },
    /// Everything else.
    Other { opcode: u8 },
}

impl Insn {
    fn opcode(&self) -> u8 {
        match self {
            Insn::Type { opcode, .. }
            | Insn::Method { opcode, .. }
            | Insn::Int { opcode, .. }
            | Insn::Other { opcode } => *opcode,
        }
    }
}

const NEW: u8 = 0xbb;
const INVOKEVIRTUAL: u8 = 0xb6;
const INVOKESPECIAL: u8 = 0xb7;
const INVOKESTATIC: u8 = 0xb8;
const BIPUSH: u8 = 0x10;
const BASTORE: u8 = 0x54;
const DUP: u8 = 0x59;

fn ty(opcode: u8, desc: &str) -> Insn {
    Insn::Type { opcode, desc: desc.into() }
}
fn call(opcode: u8, owner: &str, name: &str, desc: &str) -> Insn {
    Insn::Method { opcode, owner: owner.into(), name: name.into(), desc: desc.into() }
}
fn plain(opcode: u8) -> Insn {
    Insn::Other { opcode }
}
fn int(operand: i32) -> Insn {
    Insn::Int { opcode: BIPUSH, operand }
}

fn sig1() -> Vec<Insn> {
    vec![
        ty(NEW, "java/lang/String"),
        call(INVOKESPECIAL, "java/lang/String", "<init>", "([B)V"),
        ty(NEW, "java/lang/String"),
        call(INVOKESPECIAL, "java/lang/String", "<init>", "([B)V"),
        call(INVOKESTATIC, "java/lang/Class", "forName", "(Ljava/lang/String;)Ljava/lang/Class;"),
        call(INVOKEVIRTUAL, "java/lang/Class", "getConstructor", "([Ljava/lang/Class;)Ljava/lang/reflect/Constructor;"),
        call(INVOKESPECIAL, "java/lang/String", "<init>", "([B)V"),
        call(INVOKESPECIAL, "java/lang/String", "<init>", "([B)V"),
        call(INVOKESPECIAL, "java/lang/String", "<init>", "([B)V"),
        call(INVOKESPECIAL, "java/net/URL", "<init>", "(Ljava/lang/String;Ljava/lang/String;ILjava/lang/String;)V"),
        call(INVOKEVIRTUAL, "java/lang/reflect/Constructor", "newInstance", "([Ljava/lang/Object;)Ljava/lang/Object;"),
        call(
            INVOKESTATIC,
            "java/lang/Class",
            "forName",
            "(Ljava/lang/String;ZLjava/lang/ClassLoader;)Ljava/lang/Class;",
        ),
        call(INVOKESPECIAL, "java/lang/String", "<init>", "([B)V"),
        call(
            INVOKEVIRTUAL,
            "java/lang/Class",
            "getMethod",
            "(Ljava/lang/String;[Ljava/lang/Class;)Ljava/lang/reflect/Method;",
        ),
        call(
            INVOKEVIRTUAL,
            "java/lang/reflect/Method",
            "invoke",
            "(Ljava/lang/Object;[Ljava/lang/Object;)Ljava/lang/Object;",
        ),
    ]
}

fn sig2() -> Vec<Insn> {
    vec![
        call(INVOKESTATIC, "java/lang/Runtime", "getRuntime", "()Ljava/lang/Runtime;"),
        call(INVOKESTATIC, "java/util/Base64", "getDecoder", "()Ljava/util/Base64$Decoder;"),
        call(INVOKEVIRTUAL, "java/lang/String", "concat", "(Ljava/lang/String;)Ljava/lang/String;"),
        call(INVOKEVIRTUAL, "java/util/Base64$Decoder", "decode", "(Ljava/lang/String;)[B"),
        call(INVOKESPECIAL, "java/lang/String", "<init>", "([B)V"),
        call(INVOKEVIRTUAL, "java/io/File", "getPath", "()Ljava/lang/String;"),
        call(INVOKEVIRTUAL, "java/lang/Runtime", "exec", "([Ljava/lang/String;)Ljava/lang/Process;"),
    ]
}

/// The malware's address, written into a byte array one character at a time (`85.217.144.130`).
fn sig3() -> Vec<Insn> {
    let digits: [i32; 14] = [56, 53, 46, 50, 49, 55, 46, 49, 52, 52, 46, 49, 51, 48];
    let mut sig = Vec::new();
    for (i, d) in digits.iter().enumerate() {
        if i > 0 {
            sig.push(plain(DUP));
            // the index: iconst_1..iconst_5, then bipush
            if (1..=5).contains(&i) {
                sig.push(plain(0x03 + i as u8));
            } else {
                sig.push(int(i as i32));
            }
        }
        sig.push(int(*d));
        sig.push(plain(BASTORE));
    }
    // the list in the original has no trailing dup, only the stores
    sig
}

/// Whether two instructions with the same opcode are the same instruction (Java's `same`).
fn same(actual: &Insn, wanted: &Insn) -> bool {
    match (actual, wanted) {
        (Insn::Type { desc: a, .. }, Insn::Type { desc: b, .. }) => a == b,
        (Insn::Method { owner: a1, name: a2, desc: a3, .. }, Insn::Method { owner: b1, name: b2, desc: b3, .. }) => {
            a1 == b1 && a2 == b2 && a3 == b3
        }
        (Insn::Other { .. }, _) | (_, Insn::Other { .. }) => true,
        _ => false,
    }
}

/// Method 1, a hard detect: the instructions of the first stage in order.
fn matches_sig1(code: &[Insn]) -> bool {
    let sig = sig1();
    let mut j = 0;
    let mut matched = true;
    let mut i = 0;
    while i < code.len() && j < sig.len() {
        if code[i].opcode() == sig[j].opcode() {
            if !same(&code[i], &sig[j]) {
                matched = false;
                break;
            }
            j += 1;
        }
        i += 1;
    }
    matched && j == sig.len()
}

/// Method 2, a near hard detect: the pattern that starts the dropped program, with other instructions allowed in between.
fn matches_sig2(code: &[Insn]) -> bool {
    let sig = sig2();
    'outer: for q in 0..code.len() {
        let mut j = 0;
        let mut i = q;
        while i < code.len() && j < sig.len() {
            if code[i].opcode() == sig[j].opcode() {
                let ok = same(&code[i], &sig[j]);
                j += 1;
                if !ok {
                    continue 'outer;
                }
            }
            i += 1;
        }
        if j == sig.len() {
            return true;
        }
    }
    false
}

/// Method 3, a likely match: the address as a byte array.
fn matches_sig3(code: &[Insn]) -> bool {
    let sig = sig3();
    let mut matched = false;
    let mut pos = 0;
    for insn in code {
        if pos == sig.len() {
            break;
        }
        if insn.opcode() == sig[pos].opcode() {
            match (&sig[pos], insn) {
                (Insn::Int { operand: want, .. }, Insn::Int { operand: got, .. }) => {
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

/// If one method's instructions match any of the three patterns.
pub fn method_is_suspicious(code: &[Insn]) -> bool {
    matches_sig1(code) || matches_sig2(code) || matches_sig3(code)
}

// ---- reading class files -------------------------------------------------------------------------------------

#[derive(Debug)]
enum Const {
    Utf8(String),
    Class(u16),
    NameAndType(u16, u16),
    Ref(u16, u16),
    Other,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn u1(&mut self) -> Option<u8> {
        let v = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(v)
    }
    fn u2(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes([self.u1()?, self.u1()?]))
    }
    fn u4(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes([self.u1()?, self.u1()?, self.u1()?, self.u1()?]))
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.data.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(s)
    }
}

/// The instructions of every method of a class. None when the bytes are not a class file this can read.
pub fn methods_of(class: &[u8]) -> Option<Vec<Vec<Insn>>> {
    let mut r = Reader { data: class, pos: 0 };
    if r.u4()? != 0xCAFE_BABE {
        return None;
    }
    r.u2()?;
    r.u2()?;
    let count = r.u2()? as usize;
    let mut pool: Vec<Const> = Vec::with_capacity(count);
    pool.push(Const::Other);
    while pool.len() < count {
        let tag = r.u1()?;
        match tag {
            1 => {
                let len = r.u2()? as usize;
                pool.push(Const::Utf8(modified_utf8(r.take(len)?)));
            }
            3 | 4 => {
                r.take(4)?;
                pool.push(Const::Other);
            }
            5 | 6 => {
                r.take(8)?;
                pool.push(Const::Other);
                pool.push(Const::Other);
            }
            7 => pool.push(Const::Class(r.u2()?)),
            8 | 16 | 19 | 20 => {
                r.u2()?;
                pool.push(Const::Other);
            }
            9..=11 => {
                let a = r.u2()?;
                let b = r.u2()?;
                pool.push(Const::Ref(a, b));
            }
            12 => {
                let a = r.u2()?;
                let b = r.u2()?;
                pool.push(Const::NameAndType(a, b));
            }
            15 => {
                r.take(3)?;
                pool.push(Const::Other);
            }
            17 | 18 => {
                r.take(4)?;
                pool.push(Const::Other);
            }
            _ => return None,
        }
    }
    r.take(6)?; // access, this class, super class
    let interfaces = r.u2()? as usize;
    r.take(interfaces * 2)?;
    // fields
    let fields = r.u2()? as usize;
    for _ in 0..fields {
        r.take(6)?;
        skip_attributes(&mut r)?;
    }
    let methods = r.u2()? as usize;
    let mut out = Vec::with_capacity(methods);
    for _ in 0..methods {
        r.take(6)?;
        let attrs = r.u2()? as usize;
        let mut code_insns: Vec<Insn> = Vec::new();
        for _ in 0..attrs {
            let name_index = r.u2()? as usize;
            let len = r.u4()? as usize;
            let body = r.take(len)?;
            if matches!(pool.get(name_index), Some(Const::Utf8(n)) if n == "Code") {
                code_insns = decode_code(body, &pool)?;
            }
        }
        out.push(code_insns);
    }
    Some(out)
}

fn skip_attributes(r: &mut Reader) -> Option<()> {
    let n = r.u2()? as usize;
    for _ in 0..n {
        r.u2()?;
        let len = r.u4()? as usize;
        r.take(len)?;
    }
    Some(())
}

/// Java's "modified UTF-8" is plain UTF-8 for everything a class or method name has.
fn modified_utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

fn utf8(pool: &[Const], i: u16) -> Option<&str> {
    match pool.get(i as usize)? {
        Const::Utf8(s) => Some(s),
        _ => None,
    }
}

fn class_name(pool: &[Const], i: u16) -> Option<&str> {
    match pool.get(i as usize)? {
        Const::Class(n) => utf8(pool, *n),
        _ => None,
    }
}

fn decode_code(attr: &[u8], pool: &[Const]) -> Option<Vec<Insn>> {
    let mut r = Reader { data: attr, pos: 0 };
    r.u2()?; // max stack
    r.u2()?; // max locals
    let code_len = r.u4()? as usize;
    let code = r.take(code_len)?;
    let mut insns = Vec::new();
    let mut i = 0usize;
    while i < code.len() {
        let op = code[i];
        let at = |n: usize| code.get(i + n).copied();
        let u2_at = |n: usize| Some(u16::from_be_bytes([code.get(i + n).copied()?, code.get(i + n + 1).copied()?]));
        let (insn, len) = match op {
            BIPUSH => (Insn::Int { opcode: op, operand: at(1)? as i8 as i32 }, 2),
            0x11 => (Insn::Int { opcode: op, operand: u2_at(1)? as i16 as i32 }, 3),
            0xbc => (Insn::Int { opcode: op, operand: at(1)? as i32 }, 2),
            NEW | 0xbd | 0xc0 | 0xc1 => (Insn::Type { opcode: op, desc: class_name(pool, u2_at(1)?)?.to_string() }, 3),
            INVOKEVIRTUAL | INVOKESPECIAL | INVOKESTATIC | 0xb9 => {
                let (owner, name, desc) = method_ref(pool, u2_at(1)?)?;
                (Insn::Method { opcode: op, owner, name, desc }, if op == 0xb9 { 5 } else { 3 })
            }
            0x12 | 0x15..=0x19 | 0x36..=0x3a | 0xa9 => (plain(op), 2),
            0x13 | 0x14 | 0x84 | 0x99..=0xa8 | 0xb2..=0xb5 | 0xc6 | 0xc7 => (plain(op), 3),
            0xc5 => (plain(op), 4),
            0xba | 0xc8 | 0xc9 => (plain(op), 5),
            0xc4 => {
                // wide: iinc takes two more bytes than the other loads and stores
                let inner = at(1)?;
                (plain(inner), if inner == 0x84 { 6 } else { 4 })
            }
            0xaa => {
                let start = (i + 4) & !3;
                let base = start - i;
                let low = i32::from_be_bytes([at(base + 4)?, at(base + 5)?, at(base + 6)?, at(base + 7)?]);
                let high = i32::from_be_bytes([at(base + 8)?, at(base + 9)?, at(base + 10)?, at(base + 11)?]);
                let n = (high.checked_sub(low)?.checked_add(1)?).max(0) as usize;
                (plain(op), base + 12 + n * 4)
            }
            0xab => {
                let start = (i + 4) & !3;
                let base = start - i;
                let pairs =
                    i32::from_be_bytes([at(base + 4)?, at(base + 5)?, at(base + 6)?, at(base + 7)?]).max(0) as usize;
                (plain(op), base + 8 + pairs * 8)
            }
            _ => (plain(op), 1),
        };
        insns.push(insn);
        i += len;
    }
    Some(insns)
}

fn method_ref(pool: &[Const], index: u16) -> Option<(String, String, String)> {
    let Const::Ref(class, nat) = pool.get(index as usize)? else { return None };
    let Const::NameAndType(name, desc) = pool.get(*nat as usize)? else { return None };
    Some((class_name(pool, *class)?.to_string(), utf8(pool, *name)?.to_string(), utf8(pool, *desc)?.to_string()))
}

/// If a class has a method that matches one of the patterns. A class that cannot be read counts as clean (the
/// detector the Java edition uses does the same).
pub fn class_is_suspicious(class: &[u8]) -> bool {
    methods_of(class).is_some_and(|methods| methods.iter().any(|m| method_is_suspicious(m)))
}

/// If any class in a jar or zip matches. Unreadable entries are skipped.
pub fn jar_is_suspicious(path: &Path) -> std::io::Result<bool> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?).map_err(std::io::Error::other)?;
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else { continue };
        if !entry.name().is_ok_and(|n| n.ends_with(".class")) {
            continue;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        if entry.read_to_end(&mut bytes).is_err() {
            continue;
        }
        if class_is_suspicious(&bytes) {
            return Ok(true);
        }
    }
    Ok(false)
}

// ---- scanning a game's files -----------------------------------------------------------------------------------

/// The jars, zips and litemods a game loads: mods, datapacks, resource packs, shader packs and jar mods.
pub fn files_to_scan(instance_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for folder in ["mods", "datapacks", "resourcepacks", "shaderpacks", "jarmods"] {
        let Ok(rd) = std::fs::read_dir(instance_root.join(folder)) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_file() && (name.ends_with(".jar") || name.ends_with(".zip") || name.ends_with(".litemod")) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn scanned_hashes_file(dirs: &Dirs) -> PathBuf {
    dirs.cache.join("fracturiser_scanned_hashes.json")
}

/// Scans the files and returns the ones that look infected. Files found clean before (the same SHA-1) are skipped, and
/// new clean ones are remembered in `cache/fracturiser_scanned_hashes.json`, the Java edition's own list.
pub fn scan(dirs: &Dirs, files: &[PathBuf]) -> Vec<PathBuf> {
    if files.is_empty() {
        return Vec::new();
    }
    let file = scanned_hashes_file(dirs);
    let mut known: HashSet<String> = std::fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .collect();
    let before = known.len();
    let found = std::sync::Mutex::new(Vec::new());
    let learned = std::sync::Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = files.len().min(4);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let Some(path) = files.get(i) else { break };
                    let Ok(bytes) = std::fs::read(path) else { continue };
                    let hash = hex::encode(Sha1::digest(&bytes));
                    if known.contains(&hash) {
                        continue;
                    }
                    drop(bytes);
                    match jar_is_suspicious(path) {
                        Ok(true) => found.lock().unwrap_or_else(|e| e.into_inner()).push(path.clone()),
                        Ok(false) => learned.lock().unwrap_or_else(|e| e.into_inner()).push(hash),
                        Err(_) => {}
                    }
                }
            });
        }
    });
    known.extend(learned.into_inner().unwrap_or_default());
    // only write the list when something new was scanned
    if known.len() != before {
        let mut list: Vec<&String> = known.iter().collect();
        list.sort();
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&file, serde_json::to_vec(&list).unwrap_or_default());
    }
    let mut found = found.into_inner().unwrap_or_default();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip_bytes() -> Vec<Insn> {
        // new byte[14] { '8', '5', ... } as javac compiles it
        let digits = [56, 53, 46, 50, 49, 55, 46, 49, 52, 52, 46, 49, 51, 48];
        let mut code = vec![int(14), Insn::Int { opcode: 0xbc, operand: 8 }];
        for (i, d) in digits.iter().enumerate() {
            code.push(plain(DUP));
            code.push(if i <= 5 { plain(0x03 + i as u8) } else { int(i as i32) });
            code.push(int(*d));
            code.push(plain(BASTORE));
        }
        code.push(plain(0xb0)); // areturn
        code
    }

    #[test]
    fn the_address_pattern_is_found() {
        let code = ip_bytes();
        assert!(matches_sig3(&code));
        assert!(method_is_suspicious(&code));
        // another address is not
        let mut other = code.clone();
        for insn in other.iter_mut() {
            if *insn == int(56) {
                *insn = int(57);
            }
        }
        assert!(!method_is_suspicious(&other));
        assert!(!method_is_suspicious(&[]));
    }

    #[test]
    fn the_reflection_patterns_are_found() {
        let mut code = vec![plain(0x2a)];
        code.extend(sig2());
        assert!(matches_sig2(&code));
        // with other instructions in between
        let mut spread = Vec::new();
        for i in sig2() {
            spread.push(plain(0x00));
            spread.push(i);
        }
        assert!(matches_sig2(&spread));
        // an unrelated Runtime.exec is not enough
        assert!(!matches_sig2(&[
            call(INVOKESTATIC, "java/lang/Runtime", "getRuntime", "()Ljava/lang/Runtime;"),
            call(INVOKEVIRTUAL, "java/lang/Runtime", "exec", "([Ljava/lang/String;)Ljava/lang/Process;")
        ]));
        assert!(matches_sig1(&sig1()));
        let mut broken = sig1();
        broken[4] = call(INVOKESTATIC, "java/lang/Class", "forName", "(Ljava/lang/String;)Ljava/lang/Object;");
        assert!(!matches_sig1(&broken));
    }

    /// `javac` output for a class whose method builds the address byte array (a harmless class, only the pattern).
    const IP_CLASS: &str = include_str!("testdata/Ip.class.hex");
    const CLEAN_CLASS: &str = include_str!("testdata/Clean.class.hex");

    fn from_hex(text: &str) -> Vec<u8> {
        let clean: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        hex::decode(clean).unwrap()
    }

    #[test]
    fn real_class_files_are_read() {
        let ip = from_hex(IP_CLASS);
        assert!(methods_of(&ip).is_some());
        assert!(class_is_suspicious(&ip));
        let clean = from_hex(CLEAN_CLASS);
        let methods = methods_of(&clean).unwrap();
        assert!(!methods.is_empty());
        assert!(!class_is_suspicious(&clean));
        // the Java edition's own wrapper classes read fine and are clean
        let mut zip =
            zip::ZipArchive::new(std::io::Cursor::new(include_bytes!("../../assets/legacy-launch.jar").as_slice()))
                .unwrap();
        let mut bytes = Vec::new();
        zip.by_name("com/blockforge/launcher/mclauncher/legacy/LegacyMCLauncher.class")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert!(methods_of(&bytes).unwrap().iter().any(|m| m.len() > 10));
        assert!(!class_is_suspicious(&bytes));
        assert!(methods_of(b"not a class").is_none());
    }

    #[test]
    fn scanning_a_folder_remembers_clean_files() {
        let dir = std::env::temp_dir().join(format!("bf-neko-{}", std::process::id()));
        let dirs = Dirs::at(&dir);
        let mods = dir.join("instance/mods");
        std::fs::create_dir_all(&mods).unwrap();
        let write_jar = |name: &str, class: &[u8]| {
            use std::io::Write;
            let mut z = zip::ZipWriter::new(std::fs::File::create(mods.join(name)).unwrap());
            z.start_file("a/B.class", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(class).unwrap();
            z.finish().unwrap();
        };
        write_jar("good.jar", &from_hex(CLEAN_CLASS));
        write_jar("bad.jar", &from_hex(IP_CLASS));
        std::fs::write(mods.join("notes.txt"), "x").unwrap();
        let files = files_to_scan(&dir.join("instance"));
        assert_eq!(files.len(), 2);
        let found = scan(&dirs, &files);
        assert_eq!(found, vec![mods.join("bad.jar")]);
        let saved: Vec<String> = serde_json::from_slice(&std::fs::read(scanned_hashes_file(&dirs)).unwrap()).unwrap();
        assert_eq!(saved.len(), 1);
        // the second time the clean one is not read again, the bad one is still reported
        assert_eq!(scan(&dirs, &files), vec![mods.join("bad.jar")]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
