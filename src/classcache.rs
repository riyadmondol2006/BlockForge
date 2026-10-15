//! The game start cache: Java's class-data sharing for the game, as the Java edition's `ClassDataCache` does it.
//!
//! An archive of the classes a game's start loads (Java itself and the libraries), already parsed and checked, is
//! mapped straight into memory on the next start instead of reading and verifying thousands of classes again.
//! A start without an archive also writes down which classes it loaded; once the game has closed, a background
//! Java builds the archive from that list. Every later start of the same game with the same Java and the same
//! libraries uses it. When any of that changes the archive no longer matches: it is thrown away and made again.
//!
//! Safe by design: Java only uses an archive that matches exactly and passes its checksum, and otherwise starts
//! normally. Only Java 17 and newer. Fabric's class list can't always be archived; that is remembered (status
//! `failed`), so it isn't tried at every start.
//!
//! Files are kept in `cache/class-cache/` in the Java edition's names and format, so both editions share them:
//! `<Instance>-<javaMajor>.jsa` (the archive) and `.json` (what it was made for).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths::Dirs;

/// The oldest Java this is used with (1.18 and newer run on 17 or newer).
pub const MINIMUM_JAVA: u32 = 17;
/// A list shorter than this is from a start that didn't get far (a crash): record again next time.
const MINIMUM_CLASSES: usize = 2000;
/// Building the archive takes 20-50 s; give up well after that.
const DUMP_TIMEOUT: Duration = Duration::from_secs(6 * 60);

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
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct State {
    pub fingerprint: String,
    /// `recording`, `ready` or `failed`.
    pub status: String,
    pub java: String,
    pub java_major: u32,
    /// The archive's size when it was made: a different size means it was cut short or replaced.
    pub archive_bytes: i64,
    pub options: Vec<String>,
}

/// An archive waiting to be built once its game closes.
static PENDING: Mutex<Option<HashMap<String, State>>> = Mutex::new(None);

// ---- pure decisions ----------------------------------------------------------------------------------------

/// Whether the person's own Java arguments already decide about class-data sharing, in which case they are left to it.
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

/// What to do for a start. `archive_bytes` is the archive file's size, or -1 when it isn't there.
pub fn decide(
    enabled: bool,
    java_major: u32,
    user_arguments: &[String],
    state: Option<&State>,
    fingerprint: &str,
    archive_bytes: i64,
) -> Action {
    if !enabled || java_major < MINIMUM_JAVA || user_controls_sharing(user_arguments) {
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
            format!("-XX:SharedArchiveFile={}", absolute(archive).display()),
            // checks the archive's checksum first (about 0.1 s): Java only compares the header, and a file damaged
            // in the middle made the game crash on start in testing; with the check it is skipped
            "-XX:+VerifySharedSpaces".into(),
            // a stale or broken archive is skipped with a message; it is harmless, so keep it out of the game's log
            "-Xlog:cds=off".into(),
        ],
        Action::Record => vec![format!("-XX:DumpLoadedClassList={}", absolute(class_list).display())],
        Action::None => Vec::new(),
    }
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// The options from the game's Java command line that the archive depends on and that building it needs: the
/// class path and the module options. Taken from the options before the main class.
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

/// A fingerprint of everything the archive depends on: the first 16 bytes of a SHA-256, as hex.
pub fn fingerprint(java_executable: &str, java_version: &str, archive_options: &[String]) -> String {
    let mut text = format!("{java_executable}\n{java_version}\n");
    for o in archive_options {
        text.push_str(o);
        text.push('\n');
    }
    hex::encode(&Sha256::digest(text.as_bytes())[..16])
}

/// The recorded class list, ready for building the archive: the lines that describe lambdas and method handles
/// are left out, as building them runs code (and fails on some libraries).
pub fn usable_class_list(lines: &[String]) -> Vec<String> {
    lines.iter().filter(|l| !l.starts_with('@')).cloned().collect()
}

pub fn count_classes(lines: &[String]) -> usize {
    lines.iter().filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('@')).count()
}

/// Java's argument file format: one argument per line, quoted, with backslashes and quotes escaped.
pub fn to_argument_file(arguments: &[String]) -> String {
    arguments.iter().map(|a| format!("\"{}\"\n", a.replace('\\', "\\\\").replace('"', "\\\""))).collect()
}

// ---- files ---------------------------------------------------------------------------------------------------

/// Where the archives live: the launcher's cache folder, so they never end up in instance exports or backups.
pub fn folder(dirs: &Dirs) -> PathBuf {
    dirs.cache.join("class-cache")
}

fn base_name(instance_safe_name: &str, java_major: u32) -> String {
    let safe: String =
        instance_safe_name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')).collect();
    format!("{}-{java_major}", if safe.is_empty() { "instance" } else { &safe })
}

/// The archive for one instance on one Java version.
pub fn archive_for(dirs: &Dirs, instance_safe_name: &str, java_major: u32) -> PathBuf {
    folder(dirs).join(format!("{}.jsa", base_name(instance_safe_name, java_major)))
}

fn read_state(file: &Path) -> Option<State> {
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

fn write_state(file: &Path, state: &State) {
    let result = file
        .parent()
        .map(std::fs::create_dir_all)
        .transpose()
        .and_then(|_| std::fs::write(file, serde_json::to_vec(state).unwrap_or_default()));
    if let Err(e) = result {
        crate::logs::debug(format!("Couldn't save the class cache state: {e}"));
    }
}

fn pending() -> std::sync::MutexGuard<'static, Option<HashMap<String, State>>> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner())
}

// ---- at launch and after -----------------------------------------------------------------------------------

/// Add the class cache options to a game's command line.
///
/// * `arguments`: the whole command line, changed in place
/// * `java_index`: where the java program is in it; `main_class_index`: where the main class is
/// * `java_home`: the Java's folder, to notice a Java update
///
/// Returns how many arguments were added (before the main class).
pub fn apply(
    dirs: &Dirs,
    arguments: &mut Vec<String>,
    java_index: usize,
    main_class_index: usize,
    enabled: bool,
    java_major: u32,
    java_home: Option<&Path>,
    user_arguments: &[String],
    instance_safe_name: &str,
) -> usize {
    if !enabled || java_major < MINIMUM_JAVA || main_class_index <= java_index || main_class_index > arguments.len() {
        return 0;
    }
    let result = (|| -> std::io::Result<usize> {
        let options = archive_options(&arguments[java_index + 1..main_class_index]);
        let java_executable = arguments[java_index].clone();
        // Java checks each jar's size and time too, so a re-downloaded library makes the archive stale
        let fp = fingerprint(
            &java_executable,
            &format!("{}{}", java_version(java_home), class_path_stamp(&options)),
            &options,
        );

        let base = base_name(instance_safe_name, java_major);
        let archive = folder(dirs).join(format!("{base}.jsa"));
        let class_list = folder(dirs).join(format!("{base}.classlist"));
        let state_file = folder(dirs).join(format!("{base}.json"));
        let state = read_state(&state_file);

        let archive_bytes = std::fs::metadata(&archive).map(|m| m.len() as i64).unwrap_or(-1);
        let action = decide(enabled, java_major, user_arguments, state.as_ref(), &fp, archive_bytes);
        if action == Action::None {
            return Ok(0);
        }
        std::fs::create_dir_all(folder(dirs))?;
        if action == Action::Record {
            // anything there belongs to an older Java or other libraries
            quietly_delete(&archive);
            quietly_delete(&class_list);
            let recording = State {
                fingerprint: fp,
                status: "recording".into(),
                java: java_executable,
                java_major,
                archive_bytes: 0,
                options,
            };
            write_state(&state_file, &recording);
            pending().get_or_insert_with(HashMap::new).insert(instance_safe_name.to_string(), recording);
            crate::logs::info(
                "Class cache: this start writes down the classes it loads; the cache is made after the game closes, for quicker starts next time",
            );
        } else {
            crate::logs::info(format!(
                "Class cache: using {}",
                archive.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
            ));
        }
        let added = flags(action, &archive, &class_list);
        let n = added.len();
        arguments.splice(java_index + 1..java_index + 1, added);
        Ok(n)
    })();
    match result {
        Ok(n) => n,
        Err(e) => {
            crate::logs::debug(format!("Class cache skipped: {e}"));
            0
        }
    }
}

/// The size and time of every jar on the class path.
pub fn class_path_stamp(archive_options: &[String]) -> String {
    let mut out = String::new();
    for pair in archive_options.windows(2) {
        if matches!(pair[0].as_str(), "-cp" | "-classpath" | "--class-path") {
            for entry in std::env::split_paths(&pair[1]) {
                match std::fs::metadata(&entry) {
                    Ok(m) if m.is_file() => {
                        let ms = m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map_or(0, |d| d.as_millis());
                        out.push_str(&format!("{}@{ms};", m.len()));
                    }
                    Ok(_) => {}
                    Err(_) => out.push_str("?;"),
                }
            }
        }
    }
    out
}

/// The Java's exact version from its `release` file, so an update makes the archive stale.
pub fn java_version(java_home: Option<&Path>) -> String {
    let Some(home) = java_home else { return String::new() };
    let release = home.join("release");
    let Ok(text) = std::fs::read_to_string(&release) else { return String::new() };
    for line in text.lines() {
        if line.starts_with("JAVA_RUNTIME_VERSION=") || line.starts_with("JAVA_VERSION=") {
            let ms = std::fs::metadata(&release)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis());
            return format!("{}{ms}", line.trim());
        }
    }
    String::new()
}

/// The game has closed: if its classes were written down, build its archive in the background, at low priority.
pub fn game_exited(dirs: &Dirs, instance_safe_name: &str) {
    let Some(state) = pending().as_mut().and_then(|m| m.remove(instance_safe_name)) else { return };
    let (dirs, name) = (dirs.clone(), instance_safe_name.to_string());
    let _ = std::thread::Builder::new().name("class-cache-builder".into()).spawn(move || build(&dirs, &name, state));
}

fn build(dirs: &Dirs, instance_safe_name: &str, mut state: State) {
    let base = base_name(instance_safe_name, state.java_major);
    let dir = folder(dirs);
    let class_list = dir.join(format!("{base}.classlist"));
    let usable_list = dir.join(format!("{base}.classlist.txt"));
    let archive = dir.join(format!("{base}.jsa"));
    let building = dir.join(format!("{base}.jsa.part"));
    let state_file = dir.join(format!("{base}.json"));
    let argument_file = dir.join(format!("{base}.args"));
    let output = dir.join(format!("{base}.log"));

    let outcome = (|| -> std::io::Result<()> {
        if !class_list.is_file() {
            return Ok(());
        }
        let lines: Vec<String> = std::fs::read_to_string(&class_list)?.lines().map(str::to_string).collect();
        if count_classes(&lines) < MINIMUM_CLASSES {
            // the game didn't get far; try again on the next start
            return Ok(());
        }
        std::fs::write(&usable_list, usable_class_list(&lines).join("\n") + "\n")?;

        let mut options = vec![
            "-Xshare:dump".to_string(),
            format!("-XX:SharedClassListFile={}", absolute(&usable_list).display()),
            format!("-XX:SharedArchiveFile={}", absolute(&building).display()),
        ];
        options.extend(state.options.iter().cloned());
        // the class path can be longer than Windows allows on a command line, so it all goes in a file
        std::fs::write(&argument_file, to_argument_file(&options))?;

        let mut command: Vec<String> = Vec::new();
        if cfg!(unix) && crate::launch::executable_in_path("nice") {
            command.extend(["nice".to_string(), "-n".to_string(), "15".to_string()]);
        }
        command.push(state.java.clone());
        command.push(format!("@{}", absolute(&argument_file).display()));

        let started = std::time::Instant::now();
        let log = std::fs::File::create(&output)?;
        let mut child = crate::platform::command(&command[0])
            .args(&command[1..])
            .stdout(log.try_clone()?)
            .stderr(log)
            .stdin(std::process::Stdio::null())
            .spawn()?;
        let deadline = started + DUMP_TIMEOUT;
        let status = loop {
            match child.try_wait()? {
                Some(s) => break Some(s),
                None if std::time::Instant::now() > deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                None => std::thread::sleep(Duration::from_millis(300)),
            }
        };
        let ok =
            status.is_some_and(|s| s.success()) && std::fs::metadata(&building).map(|m| m.len() > 0).unwrap_or(false);
        if ok {
            quietly_delete(&archive);
            std::fs::rename(&building, &archive)?;
            state.status = "ready".into();
            state.archive_bytes = std::fs::metadata(&archive)?.len() as i64;
            crate::logs::info(format!(
                "Class cache for {instance_safe_name} made in {} s ({} MB)",
                started.elapsed().as_secs(),
                state.archive_bytes / (1024 * 1024)
            ));
        } else {
            state.status = "failed".into();
            crate::logs::info(format!(
                "Class cache for {instance_safe_name} couldn't be made; the game starts as usual"
            ));
        }
        write_state(&state_file, &state);
        Ok(())
    })();
    if let Err(e) = outcome {
        crate::logs::debug(format!("Class cache build failed: {e}"));
    }
    for f in [&class_list, &usable_list, &building, &argument_file, &output] {
        quietly_delete(f);
    }
}

/// Java makes its archives read-only, which Windows won't delete as they are.
fn quietly_delete(file: &Path) {
    if let Ok(meta) = std::fs::metadata(file) {
        let mut perms = meta.permissions();
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(file, perms);
        }
        let _ = std::fs::remove_file(file);
    }
}

// ---- clearing ------------------------------------------------------------------------------------------------

fn is_ours(name: &str) -> bool {
    let n = name.to_lowercase();
    [".jsa", ".jsa.part", ".classlist", ".classlist.txt", ".json", ".args", ".log"].iter().any(|e| n.ends_with(e))
}

/// "Name-21.jsa" belongs to "Name-", but "Name-2-21.jsa" (another instance) doesn't.
fn is_version_suffix(name: &str, prefix: &str) -> bool {
    let rest = &name[prefix.len()..];
    match rest.find('.') {
        Some(dot) if dot > 0 => rest[..dot].chars().all(|c| c.is_ascii_digit()),
        _ => false,
    }
}

fn delete_matching(folder: &Path, prefix: Option<&str>) -> u64 {
    let Ok(rd) = std::fs::read_dir(folder) else { return 0 };
    let mut freed = 0;
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

/// Delete the archives of one instance (all Java versions).
pub fn delete_for(dirs: &Dirs, instance_safe_name: &str) -> u64 {
    let prefix = base_name(instance_safe_name, 0);
    delete_matching(&folder(dirs), Some(&prefix[..prefix.len() - 1]))
}

/// Delete every archive; they are made again after the next start of each game. Returns the bytes freed.
pub fn clear(dirs: &Dirs) -> u64 {
    delete_matching(&folder(dirs), None)
}

/// How much room the archives take, in bytes.
pub fn size(dirs: &Dirs) -> u64 {
    std::fs::read_dir(folder(dirs))
        .map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn state(fp: &str, status: &str, bytes: i64) -> State {
        State { fingerprint: fp.into(), status: status.into(), archive_bytes: bytes, ..Default::default() }
    }

    #[test]
    fn the_players_own_sharing_options_are_left_alone() {
        assert!(user_controls_sharing(&args(&["-Xmx2G", "-Xshare:off"])));
        assert!(user_controls_sharing(&args(&["-XX:+AutoCreateSharedArchive"])));
        assert!(user_controls_sharing(&args(&["-XX:AOTCache=x.aot"])));
        assert!(!user_controls_sharing(&args(&["-Xmx2G", "-XX:+UseG1GC"])));
        assert_eq!(decide(true, 21, &args(&["-Xshare:auto"]), None, "f", -1), Action::None);
    }

    #[test]
    fn decisions_follow_the_java_edition() {
        let ready = state("fp", "ready", 1000);
        assert_eq!(decide(false, 21, &[], None, "fp", -1), Action::None, "turned off");
        assert_eq!(decide(true, 16, &[], None, "fp", -1), Action::None, "java 16 is too old");
        assert_eq!(decide(true, 21, &[], None, "fp", -1), Action::Record, "no archive yet");
        assert_eq!(decide(true, 21, &[], Some(&ready), "fp", 1000), Action::Use);
        assert_eq!(decide(true, 21, &[], Some(&ready), "fp", 999), Action::Record, "the file was cut short");
        assert_eq!(decide(true, 21, &[], Some(&ready), "fp", -1), Action::Record, "the file is gone");
        assert_eq!(decide(true, 21, &[], Some(&ready), "other", 1000), Action::Record, "libraries or Java changed");
        assert_eq!(decide(true, 21, &[], Some(&state("fp", "failed", 0)), "fp", -1), Action::None, "not tried again");
        assert_eq!(
            decide(true, 21, &[], Some(&state("old", "failed", 0)), "fp", -1),
            Action::Record,
            "failed for something else"
        );
        assert_eq!(
            decide(true, 21, &[], Some(&state("fp", "recording", 0)), "fp", -1),
            Action::Record,
            "the last start didn't finish"
        );
        assert_eq!(
            decide(true, 21, &[], Some(&state("fp", "ready", 0)), "fp", 5),
            Action::Use,
            "a size of 0 means unknown"
        );
    }

    #[test]
    fn flags_for_each_action() {
        let f = flags(Action::Use, Path::new("/c/A-21.jsa"), Path::new("/c/A-21.classlist"));
        assert_eq!(f, ["-XX:SharedArchiveFile=/c/A-21.jsa", "-XX:+VerifySharedSpaces", "-Xlog:cds=off"]);
        assert_eq!(
            flags(Action::Record, Path::new("/c/A-21.jsa"), Path::new("/c/A-21.classlist")),
            ["-XX:DumpLoadedClassList=/c/A-21.classlist"]
        );
        assert!(flags(Action::None, Path::new("a"), Path::new("b")).is_empty());
    }

    #[test]
    fn only_the_options_the_archive_depends_on_are_kept() {
        let jvm = args(&[
            "-Xmx4G",
            "-XX:+UseG1GC",
            "--enable-native-access=ALL-UNNAMED",
            "--add-exports",
            "java.base/jdk.internal.misc=ALL-UNNAMED",
            "-cp",
            "a.jar:b.jar",
            "-Dfoo=bar",
            "--add-opens=java.base/java.lang=ALL-UNNAMED",
        ]);
        assert_eq!(
            archive_options(&jvm),
            [
                "--enable-native-access=ALL-UNNAMED",
                "--add-exports",
                "java.base/jdk.internal.misc=ALL-UNNAMED",
                "-cp",
                "a.jar:b.jar",
                "--add-opens=java.base/java.lang=ALL-UNNAMED"
            ]
        );
        assert!(archive_options(&args(&["-cp"])).is_empty(), "a flag with no value is dropped");
    }

    #[test]
    fn fingerprints_match_the_java_editions() {
        // SHA-256 of "/j/bin/java\n21.0.1\n-cp\na.jar\n", first 16 bytes as hex: computed with `sha256sum`
        let fp = fingerprint("/j/bin/java", "21.0.1", &args(&["-cp", "a.jar"]));
        assert_eq!(fp.len(), 32);
        assert_eq!(fp, hex::encode(&Sha256::digest(b"/j/bin/java\n21.0.1\n-cp\na.jar\n")[..16]));
        assert_ne!(fp, fingerprint("/j/bin/java", "21.0.2", &args(&["-cp", "a.jar"])));
    }

    #[test]
    fn class_lists_drop_the_dynamic_lines() {
        let lines =
            args(&["# comment", "java/lang/Object id: 0", "@lambda-proxy Foo bar", "", "java/lang/String id: 1"]);
        assert_eq!(count_classes(&lines), 2);
        assert_eq!(usable_class_list(&lines), ["# comment", "java/lang/Object id: 0", "", "java/lang/String id: 1"]);
    }

    #[test]
    fn argument_files_are_quoted() {
        assert_eq!(
            to_argument_file(&args(&["-cp", "C:\\a b\\x.jar", "say \"hi\""])),
            "\"-cp\"\n\"C:\\\\a b\\\\x.jar\"\n\"say \\\"hi\\\"\"\n"
        );
    }

    #[test]
    fn names_of_files() {
        assert_eq!(base_name("My Pack!", 21), "MyPack-21");
        assert_eq!(base_name("", 17), "instance-17");
        assert_eq!(base_name("a.b_c-d", 25), "a.b_c-d-25");
    }

    #[test]
    fn state_files_are_the_java_editions_json() {
        let java_written = r#"{"fingerprint":"17f6","status":"failed","java":"/j/bin/java","javaMajor":25,"archiveBytes":0,"options":["-cp","a.jar"]}"#;
        let s: State = serde_json::from_str(java_written).unwrap();
        assert_eq!((s.status.as_str(), s.java_major, s.options.len()), ("failed", 25, 2));
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["javaMajor"], 25);
        assert_eq!(v["archiveBytes"], 0);
    }

    fn scratch(tag: &str) -> Dirs {
        let base = std::env::temp_dir().join(format!("blockforge-cds-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        Dirs::at(base)
    }

    #[test]
    fn the_first_start_records_and_a_ready_archive_is_used() {
        let dirs = scratch("apply");
        let mut cmd = args(&[
            "/j/bin/java",
            "-Xmx4G",
            "-cp",
            "/nonexistent/a.jar",
            "net.minecraft.client.main.Main",
            "--username",
            "x",
        ]);
        let added = apply(&dirs, &mut cmd, 0, 4, true, 21, None, &[], "MyPack");
        assert_eq!(added, 1);
        assert!(cmd[1].starts_with("-XX:DumpLoadedClassList="), "{cmd:?}");
        assert_eq!(cmd[5], "net.minecraft.client.main.Main", "everything after the flags moved by one");
        assert!(pending().as_ref().is_some_and(|m| m.contains_key("MyPack")));
        let saved = read_state(&folder(&dirs).join("MyPack-21.json")).unwrap();
        assert_eq!(saved.status, "recording");
        assert_eq!(saved.java_major, 21);

        // pretend the archive was built: the next start with the same command uses it
        let mut ready = saved.clone();
        ready.status = "ready".into();
        let archive = archive_for(&dirs, "MyPack", 21);
        std::fs::write(&archive, vec![0u8; 64]).unwrap();
        ready.archive_bytes = 64;
        write_state(&folder(&dirs).join("MyPack-21.json"), &ready);
        let mut again = args(&["/j/bin/java", "-Xmx4G", "-cp", "/nonexistent/a.jar", "net.minecraft.client.main.Main"]);
        assert_eq!(apply(&dirs, &mut again, 0, 4, true, 21, None, &[], "MyPack"), 3);
        assert!(again[1].starts_with("-XX:SharedArchiveFile="));
        assert_eq!(again[2], "-XX:+VerifySharedSpaces");

        // another Java, off, or an old Java: untouched
        let mut other = args(&["/j/bin/java", "-cp", "x", "Main"]);
        assert_eq!(apply(&dirs, &mut other, 0, 3, false, 21, None, &[], "MyPack"), 0);
        assert_eq!(apply(&dirs, &mut other, 0, 3, true, 8, None, &[], "MyPack"), 0);
        assert_eq!(other, args(&["/j/bin/java", "-cp", "x", "Main"]));
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn clearing_removes_only_the_cache_files() {
        let dirs = scratch("clear");
        let f = folder(&dirs);
        std::fs::create_dir_all(&f).unwrap();
        for name in ["A-21.jsa", "A-21.json", "A-17.jsa", "A-2-21.jsa", "B-21.jsa", "notes.txt"] {
            std::fs::write(f.join(name), vec![0u8; 100]).unwrap();
        }
        assert_eq!(size(&dirs), 600);
        assert_eq!(delete_for(&dirs, "A"), 300, "A-21 (two files) and A-17 go, A-2-21 is another instance");
        assert!(f.join("A-2-21.jsa").exists() && f.join("B-21.jsa").exists());
        assert_eq!(clear(&dirs), 200);
        assert!(f.join("notes.txt").exists(), "files that aren't the cache's are never deleted");
        assert!(!f.join("B-21.jsa").exists());
        assert_eq!(clear(&scratch("empty")), 0);
        let _ = std::fs::remove_dir_all(&dirs.base);
    }
}
