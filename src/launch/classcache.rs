//! Java's class-data sharing for the game: an archive of the classes a game's start loads (Java itself and the
//! libraries), already parsed and checked, which Java maps straight into memory on the next start instead of reading
//! and verifying thousands of classes again. Starts about 8% (0.5-0.8 s) quicker. Minecraft's own classes can't be
//! shared, as its jar is signed.
//!
//! How: a start without an archive also writes down which classes it loaded (no measurable cost); once the game has
//! closed, a background Java builds the archive from that list. Every later start of the same game with the same Java
//! and the same libraries uses it. When any of that changes (a Java or library update, another loader version), the
//! archive no longer matches: it is thrown away and made again the same way.
//!
//! Safe by design: Java only uses an archive that matches exactly and passes its checksum, and otherwise starts
//! normally. Only Java 17 and newer; never Java 8-16. Fabric's class list can't be archived (Java rejects it); that is
//! remembered, so it isn't tried at every start. The files are the Java edition's (`cache/class-cache/<name>-<java>.*`),
//! so both editions share them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::launch::jvmargs;
use crate::paths::Dirs;

/// The oldest Java this is used with (1.18 and newer run on 17 or newer).
pub const MINIMUM_JAVA: u32 = 17;
/// A list shorter than this is from a start that didn't get far (a crash): record again next time.
pub const MINIMUM_CLASSES: usize = 2000;
/// Building the archive takes 20-50 s; give up well after that.
const DUMP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(6 * 60);

/// What to do for one start.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Nothing: turned off, an old Java, the person's own sharing options, or the archive couldn't be made.
    None,
    /// Use the archive.
    Use,
    /// Write down the classes, so the archive can be made after the game closes.
    Record,
}

/// What is kept next to an archive.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct State {
    pub fingerprint: String,
    /// "recording", "ready" or "failed".
    pub status: String,
    pub java: String,
    pub java_major: u32,
    /// The archive's size when it was made: a different size means it was cut short or replaced.
    pub archive_bytes: i64,
    pub options: Vec<String>,
}

/// If the person's own Java arguments already decide about class-data sharing, in which case they are left to it.
pub fn user_controls_sharing(user_arguments: &[String]) -> bool {
    user_arguments.iter().any(|a| {
        let a = a.trim();
        a.starts_with("-Xshare")
            || a.starts_with("-XX:SharedArchiveFile")
            || a.starts_with("-XX:ArchiveClassesAtExit")
            || a.contains("AutoCreateSharedArchive")
            || a.starts_with("-XX:SharedClassListFile")
            || a.starts_with("-XX:DumpLoadedClassList")
            || a.starts_with("-XX:+DumpSharedSpaces")
            || a.starts_with("-XX:AOTCache")
            || a.starts_with("-XX:AOTMode")
            || a.starts_with("-XX:AOTConfiguration")
    })
}

/// What to do for a start.
///
/// * `java_major` the game's Java version (None when unknown)
/// * `state` what was saved for this game and Java
/// * `fingerprint` the fingerprint of this start (Java, libraries and module options)
/// * `archive_bytes` the size of the archive file, or -1 when it isn't there
pub fn decide(
    enabled: bool,
    java_major: Option<u32>,
    user_arguments: &[String],
    state: Option<&State>,
    fingerprint: &str,
    archive_bytes: i64,
) -> Action {
    if !enabled || java_major.is_none_or(|m| m < MINIMUM_JAVA) || user_controls_sharing(user_arguments) {
        return Action::None;
    }
    if let Some(state) = state.filter(|s| s.fingerprint == fingerprint) {
        if state.status == "ready"
            && archive_bytes > 0
            && (state.archive_bytes <= 0 || state.archive_bytes == archive_bytes)
        {
            return Action::Use;
        }
        if state.status == "failed" {
            // this combination can't be archived (some mod loaders); don't try at every start
            return Action::None;
        }
    }
    Action::Record
}

/// The Java options for an action.
pub fn flags(action: Action, archive: &Path, class_list: &Path) -> Vec<String> {
    match action {
        Action::Use => vec![
            format!("-XX:SharedArchiveFile={}", archive.display()),
            // checks the archive's checksum first (about 0.1 s): Java only compares the header, and a file damaged in
            // the middle (a bad disk, a crash while copying) made the game crash on start in testing
            "-XX:+VerifySharedSpaces".to_string(),
            // a stale or broken archive is skipped with a message; it is harmless, so keep it out of the game's log
            "-Xlog:cds=off".to_string(),
        ],
        Action::Record => vec![format!("-XX:DumpLoadedClassList={}", class_list.display())],
        Action::None => Vec::new(),
    }
}

/// The options from the game's Java command line that the archive depends on and that building it needs: the class
/// path and the module options. Taken from the options before the main class.
pub fn archive_options(java_options: &[String]) -> Vec<String> {
    let mut options = Vec::new();
    let mut i = 0;
    while i < java_options.len() {
        let a = &java_options[i];
        let takes_value = matches!(
            a.as_str(),
            "-cp"
                | "-classpath"
                | "--class-path"
                | "-p"
                | "--module-path"
                | "--add-modules"
                | "--add-exports"
                | "--add-opens"
                | "--add-reads"
                | "--enable-native-access"
        );
        if takes_value && i + 1 < java_options.len() {
            options.push(a.clone());
            options.push(java_options[i + 1].clone());
            i += 1;
        } else if a.starts_with("--add-")
            || a.starts_with("--enable-native-access=")
            || a.starts_with("--module-path=")
            || a.starts_with("--class-path=")
        {
            options.push(a.clone());
        }
        i += 1;
    }
    options
}

/// The first Java whose archive has to know about the agents' module (measured on 25; 17 and 21 don't mind).
pub const AGENT_MODULE_JAVA: u32 = 25;

/// A game started with a Java agent (`-javaagent`) runs with the `java.instrument` module added. From Java 25 an
/// archive made without it logs an error and leaves out its saved module graph, so the archive of such a game is made
/// with `--add-modules java.instrument`. Being part of the archive's options, it is also part of its fingerprint: a
/// game started with and without agents gets its own archive.
pub fn add_agent_module(archive_options: &mut Vec<String>, java_options: &[String], java_major: u32) {
    let agents = java_options.iter().any(|a| a.starts_with("-javaagent:"));
    let there = archive_options.windows(2).any(|w| w[0] == "--add-modules" && w[1] == "java.instrument")
        || archive_options.iter().any(|a| a == "--add-modules=java.instrument");
    if agents && java_major >= AGENT_MODULE_JAVA && !there {
        archive_options.push("--add-modules".to_string());
        archive_options.push("java.instrument".to_string());
    }
}

/// A fingerprint of everything the archive depends on: the first 16 bytes of a SHA-256, in hex.
pub fn fingerprint(java_executable: &str, java_version: &str, archive_options: &[String]) -> String {
    let mut text = format!("{java_executable}\n{java_version}\n");
    for o in archive_options {
        text.push_str(o);
        text.push('\n');
    }
    sha256(text.as_bytes()).iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// The recorded class list, ready for building the archive: the lines that describe lambdas and method handles are
/// left out, as building them runs code (and fails on some libraries).
pub fn usable_class_list(lines: &[String]) -> Vec<String> {
    lines.iter().filter(|l| !l.starts_with('@')).cloned().collect()
}

pub fn count_classes(lines: &[String]) -> usize {
    lines.iter().filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('@')).count()
}

// ---- files ---------------------------------------------------------------------------------------------------

/// Where the archives live: the launcher's cache folder, so they never end up in instance exports or backups.
pub fn folder(dirs: &Dirs) -> PathBuf {
    dirs.cache.join("class-cache")
}

pub fn base_name(instance_safe_name: &str, java_major: u32) -> String {
    let safe: String =
        instance_safe_name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')).collect();
    format!("{}-{}", if safe.is_empty() { "instance" } else { &safe }, java_major)
}

/// The archive for one instance on one Java version.
pub fn archive_for(dirs: &Dirs, instance_safe_name: &str, java_major: u32) -> PathBuf {
    folder(dirs).join(format!("{}.jsa", base_name(instance_safe_name, java_major)))
}

fn read_state(file: &Path) -> Option<State> {
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

fn write_state(file: &Path, state: &State) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, serde_json::to_vec(state).unwrap_or_default());
}

/// A game whose classes are being written down; the archive is built once it closes.
#[derive(Clone, Debug)]
pub struct Pending {
    pub instance_safe_name: String,
    pub state: State,
    pub folder: PathBuf,
}

/// What was decided for a start, for the log.
pub struct Applied {
    pub action: Action,
    pub message: Option<String>,
    pub pending: Option<Pending>,
}

/// Adds the class cache options to the front of a game's Java options (the ones before the main class).
///
/// * `java_options` the options; changed in place
/// * `java_executable` the Java that runs the game
/// * `java_home` its folder, for its exact version
pub fn apply(
    dirs: &Dirs,
    java_options: &mut Vec<String>,
    java_executable: &str,
    java_home: Option<&Path>,
    enabled: bool,
    java_major: Option<u32>,
    user_arguments: &[String],
    instance_safe_name: &str,
) -> Applied {
    let none = Applied { action: Action::None, message: None, pending: None };
    let Some(major) = java_major.filter(|m| enabled && *m >= MINIMUM_JAVA) else { return none };
    let mut options = archive_options(java_options);
    add_agent_module(&mut options, java_options, major);
    // Java checks each jar's size and time too, so a re-downloaded library makes the archive stale
    let print =
        fingerprint(java_executable, &format!("{}{}", java_version(java_home), class_path_stamp(&options)), &options);
    let base = base_name(instance_safe_name, major);
    let folder = folder(dirs);
    let archive = folder.join(format!("{base}.jsa"));
    let class_list = folder.join(format!("{base}.classlist"));
    let state_file = folder.join(format!("{base}.json"));
    let state = read_state(&state_file);
    let archive_bytes = std::fs::metadata(&archive).map(|m| m.len() as i64).unwrap_or(-1);
    let action = decide(enabled, Some(major), user_arguments, state.as_ref(), &print, archive_bytes);
    if action == Action::None {
        return none;
    }
    if std::fs::create_dir_all(&folder).is_err() {
        return none;
    }
    // Two games of the same instance can run at once. Only the first writes down the classes: a second one would
    // overwrite the list the first is still writing.
    if action == Action::Record && !claim_recording(&base) {
        return none;
    }
    let mut pending = None;
    let message = if action == Action::Record {
        // anything there belongs to an older Java or other libraries
        quietly_delete(&archive);
        quietly_delete(&class_list);
        let recording = State {
            fingerprint: print,
            status: "recording".into(),
            java: java_executable.to_string(),
            java_major: major,
            archive_bytes: 0,
            options,
        };
        write_state(&state_file, &recording);
        pending = Some(Pending {
            instance_safe_name: instance_safe_name.to_string(),
            state: recording,
            folder: folder.clone(),
        });
        "Class cache: this start writes down the classes it loads; the cache is made after the game closes, for quicker starts next time".to_string()
    } else {
        format!("Class cache: using {}", archive.file_name().map(|n| n.to_string_lossy()).unwrap_or_default())
    };
    let new_flags = flags(action, &archive, &class_list);
    java_options.splice(0..0, new_flags);
    Applied { action, message: Some(message), pending }
}

/// The size and time of every jar on the class path.
fn class_path_stamp(archive_options: &[String]) -> String {
    let mut out = String::new();
    for pair in archive_options.windows(2) {
        if matches!(pair[0].as_str(), "-cp" | "-classpath" | "--class-path") {
            for entry in pair[1].split(crate::platform::path_separator()) {
                match std::fs::metadata(entry) {
                    Ok(m) if m.is_file() => {
                        let ms = m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_millis())
                            .unwrap_or(0);
                        out.push_str(&format!("{}@{};", m.len(), ms));
                    }
                    Ok(_) => {}
                    Err(_) => out.push_str("?;"),
                }
            }
        }
    }
    out
}

/// The Java's exact version from its release file, so an update makes the archive stale.
fn java_version(java_home: Option<&Path>) -> String {
    let Some(home) = java_home else { return String::new() };
    let release = home.join("release");
    let Ok(text) = std::fs::read_to_string(&release) else { return String::new() };
    for line in text.lines() {
        if line.starts_with("JAVA_RUNTIME_VERSION=") || line.starts_with("JAVA_VERSION=") {
            let ms = std::fs::metadata(&release)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or(0);
            return format!("{}{ms}", line.trim());
        }
    }
    String::new()
}

/// The game has closed: if its classes were written down, build its archive in the background, at low priority.
pub fn game_exited(pending: Option<Pending>) {
    let Some(pending) = pending else { return };
    release_recording(&base_name(&pending.instance_safe_name, pending.state.java_major));
    if let Ok(handle) = std::thread::Builder::new().name("class-cache-builder".into()).spawn(move || {
        build(&pending);
    }) {
        BUILDERS.lock().unwrap_or_else(|e| e.into_inner()).push(handle);
    }
}

/// The archives whose class list a running game is writing, and since when (an entry that old is from a game that
/// never got to report back).
static RECORDING: std::sync::Mutex<Vec<(String, std::time::Instant)>> = std::sync::Mutex::new(Vec::new());

/// Take the right to write the class list of this archive; false if a game is doing that already.
fn claim_recording(base: &str) -> bool {
    let mut list = RECORDING.lock().unwrap_or_else(|e| e.into_inner());
    list.retain(|(_, since)| since.elapsed() < std::time::Duration::from_secs(6 * 3600));
    if list.iter().any(|(b, _)| b == base) {
        return false;
    }
    list.push((base.to_string(), std::time::Instant::now()));
    true
}

fn release_recording(base: &str) {
    RECORDING.lock().unwrap_or_else(|e| e.into_inner()).retain(|(b, _)| b != base);
}

/// Archives being built in the background. A program that is about to exit (the command line) waits for them with
/// [`wait_for_builders`]; a window that stays open does not need to.
static BUILDERS: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>> = std::sync::Mutex::new(Vec::new());

/// How many archives are being built right now.
pub fn builders_running() -> usize {
    BUILDERS.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|h| !h.is_finished()).count()
}

/// Waits until the archives being built are done (20-50 s each).
pub fn wait_for_builders() {
    let handles: Vec<_> = std::mem::take(&mut *BUILDERS.lock().unwrap_or_else(|e| e.into_inner()));
    for h in handles {
        let _ = h.join();
    }
}

/// Builds the archive now (blocks for 20-50 s). Returns whether there is a ready archive afterwards.
pub fn build(pending: &Pending) -> bool {
    let base = base_name(&pending.instance_safe_name, pending.state.java_major);
    let folder = &pending.folder;
    let class_list = folder.join(format!("{base}.classlist"));
    let usable_list = folder.join(format!("{base}.classlist.txt"));
    let archive = folder.join(format!("{base}.jsa"));
    let building = folder.join(format!("{base}.jsa.part"));
    let state_file = folder.join(format!("{base}.json"));
    let argument_file = folder.join(format!("{base}.args"));
    let output = folder.join(format!("{base}.log"));

    let result = (|| -> Option<bool> {
        let text = std::fs::read_to_string(&class_list).ok()?;
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        if count_classes(&lines) < MINIMUM_CLASSES {
            // the game didn't get far; try again on the next start
            return None;
        }
        std::fs::write(&usable_list, usable_class_list(&lines).join("\n")).ok()?;
        let mut options = vec![
            "-Xshare:dump".to_string(),
            format!("-XX:SharedClassListFile={}", usable_list.display()),
            format!("-XX:SharedArchiveFile={}", building.display()),
        ];
        options.extend(pending.state.options.iter().cloned());
        // the class path can be longer than Windows allows on a command line, so it all goes in a file
        std::fs::write(&argument_file, jvmargs::to_argument_file(&options)).ok()?;

        let mut command = if cfg!(unix) && crate::launch::executable_in_path("nice") {
            let mut c = crate::platform::command("nice");
            c.args(["-n", "15"]).arg(&pending.state.java);
            c
        } else {
            crate::platform::command(&pending.state.java)
        };
        command.arg(format!("@{}", argument_file.display()));
        // Java lists every class it skips; that output isn't needed, but must go somewhere
        let log = std::fs::File::create(&output).ok()?;
        command.stdout(log.try_clone().ok()?).stderr(log).stdin(std::process::Stdio::null());
        let mut child = command.spawn().ok()?;
        let started = std::time::Instant::now();
        let finished = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status.success()),
                Ok(None) if started.elapsed() > DUMP_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(250)),
                Err(_) => break None,
            }
        };
        let ok = finished == Some(true) && std::fs::metadata(&building).map(|m| m.len() > 0).unwrap_or(false);
        let mut state = pending.state.clone();
        if ok {
            quietly_delete(&archive);
            std::fs::rename(&building, &archive).ok()?;
            state.status = "ready".into();
            state.archive_bytes = std::fs::metadata(&archive).map(|m| m.len() as i64).unwrap_or(0);
        } else {
            state.status = "failed".into();
        }
        write_state(&state_file, &state);
        Some(ok)
    })();

    for f in [&class_list, &usable_list, &building, &argument_file, &output] {
        quietly_delete(f);
    }
    result.unwrap_or(false)
}

fn quietly_delete(file: &Path) {
    // Java makes its archives read-only, which Windows won't delete as they are
    if let Ok(meta) = std::fs::metadata(file) {
        let mut perms = meta.permissions();
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(file, perms);
        }
    }
    let _ = std::fs::remove_file(file);
}

// ---- clearing ------------------------------------------------------------------------------------------------

fn is_ours(name: &str) -> bool {
    let n = name.to_lowercase();
    [".jsa", ".jsa.part", ".classlist", ".classlist.txt", ".json", ".args", ".log"].iter().any(|e| n.ends_with(e))
}

/// "Name-21.jsa" belongs to "Name-", but "Name-2-21.jsa" (another instance) doesn't.
fn is_version_suffix(name: &str, prefix: &str) -> bool {
    let rest = &name[prefix.len()..];
    rest.find('.').is_some_and(|dot| dot > 0 && rest[..dot].bytes().all(|b| b.is_ascii_digit()))
}

fn delete_matching(folder: &Path, prefix: Option<&str>) -> u64 {
    let mut freed = 0;
    let Ok(rd) = std::fs::read_dir(folder) else { return 0 };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !is_ours(&name) {
            continue;
        }
        if prefix.is_none_or(|p| name.starts_with(p) && is_version_suffix(&name, p)) {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            quietly_delete(&e.path());
            if !e.path().exists() {
                freed += size;
            }
        }
    }
    freed
}

/// Deletes the archives of one instance (all Java versions).
pub fn delete_for(dirs: &Dirs, instance_safe_name: &str) -> u64 {
    let with_zero = base_name(instance_safe_name, 0);
    let prefix = &with_zero[..with_zero.len() - 1];
    delete_matching(&folder(dirs), Some(prefix))
}

/// Deletes every archive; they are made again after the next start of each game. Returns the bytes freed.
pub fn clear(dirs: &Dirs) -> u64 {
    delete_matching(&folder(dirs), None)
}

/// How much room the archives take, in bytes.
pub fn size(dirs: &Dirs) -> u64 {
    std::fs::read_dir(folder(dirs))
        .map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
        .unwrap_or(0)
}

// ---- SHA-256 (small, so no new dependency for one fingerprint) ----------------------------------------------

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_game_records_the_class_list() {
        assert!(claim_recording("test-instance-17"));
        assert!(!claim_recording("test-instance-17"));
        assert!(claim_recording("test-other-17"));
        release_recording("test-instance-17");
        assert!(claim_recording("test-instance-17"));
        release_recording("test-instance-17");
        release_recording("test-other-17");
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn sha256_matches_the_standard_vectors() {
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let long = vec![b'a'; 1000];
        assert_eq!(hex(&sha256(&long)), "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3");
    }

    #[test]
    fn decisions() {
        let s = |status: &str, bytes: i64| State {
            fingerprint: "fp".into(),
            status: status.into(),
            archive_bytes: bytes,
            ..Default::default()
        };
        let none: Vec<String> = Vec::new();
        assert_eq!(decide(false, Some(21), &none, None, "fp", -1), Action::None);
        assert_eq!(decide(true, Some(8), &none, None, "fp", -1), Action::None);
        assert_eq!(decide(true, None, &none, None, "fp", -1), Action::None);
        assert_eq!(decide(true, Some(21), &["-Xshare:off".into()], None, "fp", -1), Action::None);
        assert_eq!(decide(true, Some(21), &none, None, "fp", -1), Action::Record);
        assert_eq!(decide(true, Some(21), &none, Some(&s("ready", 100)), "fp", 100), Action::Use);
        // the archive was cut short or replaced
        assert_eq!(decide(true, Some(21), &none, Some(&s("ready", 100)), "fp", 50), Action::Record);
        assert_eq!(decide(true, Some(21), &none, Some(&s("ready", 100)), "other", 100), Action::Record);
        assert_eq!(decide(true, Some(21), &none, Some(&s("ready", 100)), "fp", -1), Action::Record);
        assert_eq!(decide(true, Some(21), &none, Some(&s("failed", 0)), "fp", -1), Action::None);
        assert_eq!(decide(true, Some(21), &none, Some(&s("failed", 0)), "changed", -1), Action::Record);
        assert_eq!(decide(true, Some(25), &none, Some(&s("recording", 0)), "fp", -1), Action::Record);
    }

    #[test]
    fn user_sharing_options_are_left_alone() {
        for a in [
            "-Xshare:auto",
            "-XX:SharedArchiveFile=x",
            "-XX:+AutoCreateSharedArchive",
            "-XX:ArchiveClassesAtExit=x",
            "-XX:AOTCache=x",
        ] {
            assert!(user_controls_sharing(&[a.to_string()]), "{a}");
        }
        assert!(!user_controls_sharing(&["-XX:+UseG1GC".to_string(), "-Xmx2G".to_string()]));
    }

    #[test]
    fn java_25_archives_know_about_the_agents() {
        let game = vec!["-javaagent:/c/blockforge-boost.jar=quiet".to_string(), "-cp".into(), "a.jar".into()];
        let mut options = archive_options(&game);
        add_agent_module(&mut options, &game, 25);
        assert_eq!(options, ["-cp", "a.jar", "--add-modules", "java.instrument"]);
        // once is enough
        add_agent_module(&mut options, &game, 25);
        assert_eq!(options.len(), 4);
        // older Javas, and games without agents, are left as they were
        let mut old = archive_options(&game);
        add_agent_module(&mut old, &game, 21);
        assert_eq!(old, ["-cp", "a.jar"]);
        let plain = vec!["-cp".to_string(), "a.jar".into()];
        let mut none = archive_options(&plain);
        add_agent_module(&mut none, &plain, 25);
        assert_eq!(none, ["-cp", "a.jar"]);
        // with and without agents are two archives
        assert_ne!(fingerprint("java", "25", &options), fingerprint("java", "25", &none));
    }

    #[test]
    fn archive_options_keep_the_class_and_module_path() {
        let opts: Vec<String> = [
            "-Xmx2G",
            "-cp",
            "a:b",
            "--add-opens",
            "java.base/java.lang=ALL-UNNAMED",
            "--add-modules=x",
            "-Dfoo=bar",
            "-p",
            "/m",
            "--enable-native-access=ALL-UNNAMED",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            archive_options(&opts),
            vec![
                "-cp",
                "a:b",
                "--add-opens",
                "java.base/java.lang=ALL-UNNAMED",
                "--add-modules=x",
                "-p",
                "/m",
                "--enable-native-access=ALL-UNNAMED"
            ]
        );
    }

    #[test]
    fn class_lists() {
        let lines: Vec<String> =
            ["java/lang/Object id: 0", "@lambda-form-invoker foo", "# comment", "", "java/lang/String id: 1"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(count_classes(&lines), 2);
        assert_eq!(usable_class_list(&lines).len(), 4);
    }

    #[test]
    fn names_and_files() {
        assert_eq!(base_name("My World!", 21), "MyWorld-21");
        assert_eq!(base_name("", 17), "instance-17");
        assert!(is_version_suffix("Name-21.jsa", "Name-"));
        assert!(!is_version_suffix("Name-2-21.jsa", "Name-"));
        let dir = std::env::temp_dir().join(format!("bf-cds-{}", std::process::id()));
        let dirs = Dirs::at(&dir);
        let f = folder(&dirs);
        std::fs::create_dir_all(&f).unwrap();
        for n in ["Pack-21.jsa", "Pack-21.json", "Pack-17.jsa", "Pack-2-21.jsa", "notes.txt"] {
            std::fs::write(f.join(n), b"1234").unwrap();
        }
        assert_eq!(size(&dirs), 20);
        assert_eq!(delete_for(&dirs, "Pack"), 12);
        assert!(f.join("Pack-2-21.jsa").exists() && f.join("notes.txt").exists());
        assert_eq!(clear(&dirs), 4);
        assert!(f.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_start_with_no_archive_records_and_a_ready_one_is_used() {
        let dir = std::env::temp_dir().join(format!("bf-cds2-{}", std::process::id()));
        let dirs = Dirs::at(&dir);
        let mut opts = vec!["-Xmx2G".to_string(), "-cp".into(), "/nonexistent/a.jar".into()];
        let applied = apply(&dirs, &mut opts, "/usr/bin/java", None, true, Some(21), &[], "My Pack");
        assert_eq!(applied.action, Action::Record);
        assert!(opts[0].starts_with("-XX:DumpLoadedClassList="));
        let pending = applied.pending.unwrap();
        assert_eq!(pending.state.status, "recording");
        // the game crashed early: no class list, so nothing is built and the next start records again
        assert!(!build(&pending));
        // pretend the archive was built
        let mut state = pending.state.clone();
        state.status = "ready".into();
        state.archive_bytes = 4;
        write_state(&folder(&dirs).join("MyPack-21.json"), &state);
        std::fs::write(archive_for(&dirs, "MyPack", 21), b"1234").unwrap();
        let mut opts2 = vec!["-Xmx2G".to_string(), "-cp".into(), "/nonexistent/a.jar".into()];
        let again = apply(&dirs, &mut opts2, "/usr/bin/java", None, true, Some(21), &[], "MyPack");
        assert_eq!(again.action, Action::Use);
        assert!(
            opts2[0].starts_with("-XX:SharedArchiveFile=") && opts2.contains(&"-XX:+VerifySharedSpaces".to_string())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
