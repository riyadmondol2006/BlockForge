//! Working out what went wrong, and packing the evidence so it can be shared: the Java edition's
//! `diagnostics` package (`Redactor`, `CrashClassifier`, `SupportBundle`).
//!
//! A support bundle is one zip with the launcher log and the last played instance's logs, with names, tokens,
//! e-mail addresses, ids and IP addresses removed. The person sending it can open the zip first and see what is in it.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use regex::Regex;

/// Only the end of a log is useful, and it keeps the bundle small.
const MAX_LOG_BYTES: u64 = 768 * 1024;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("built-in pattern")
}

// ---------------------------------------------------------------------------------------------------------
// Redactor

struct Patterns {
    email: Regex,
    uuid: Regex,
    jwt: Regex,
    bearer: Regex,
    ipv4: Regex,
    argument: Regex,
    field: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        email: re(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"),
        uuid: re(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b"),
        jwt: re(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b"),
        bearer: re(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{8,}"),
        ipv4: re(r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b"),
        // launch arguments and JSON/config fields that carry credentials or identity
        argument: re(r"(?i)(--(?:accessToken|session|uuid|username|xuid|clientId)\s+)\S+"),
        field: re(r#"(?i)("?(?:access_?token|refresh_?token|session_?token|id_?token|client_?secret|password|token)"?\s*[:=]\s*"?)[^\s",}]+"#),
    })
}

/// Take anything personal or secret out of `text`. `extra_secrets` are values that must disappear wherever they
/// occur, such as the player's name (values shorter than 3 characters are ignored, as they would match everything).
pub fn scrub(text: &str, extra_secrets: &[String]) -> String {
    scrub_with(text, extra_secrets, home_dir().as_deref(), user_name().as_deref())
}

fn home_dir() -> Option<String> {
    std::env::var("HOME").ok().or_else(|| std::env::var("USERPROFILE").ok()).filter(|h| !h.is_empty())
}

fn user_name() -> Option<String> {
    std::env::var("USER").ok().or_else(|| std::env::var("USERNAME").ok()).filter(|h| !h.is_empty())
}

pub fn scrub_with(text: &str, extra_secrets: &[String], home: Option<&str>, user: Option<&str>) -> String {
    if text.is_empty() {
        return String::new();
    }
    let p = patterns();
    let mut result = text.to_string();
    for secret in extra_secrets {
        if secret.chars().count() >= 3 {
            result = result.replace(secret.as_str(), "<hidden>");
        }
    }
    result = replace_home(&result, home, user);
    result = p.argument.replace_all(&result, "${1}<hidden>").into_owned();
    result = p.field.replace_all(&result, "${1}<hidden>").into_owned();
    result = p.bearer.replace_all(&result, "Bearer <token>").into_owned();
    result = p.jwt.replace_all(&result, "<token>").into_owned();
    result = p.email.replace_all(&result, "<email>").into_owned();
    result = p.uuid.replace_all(&result, "<uuid>").into_owned();
    redact_addresses(&result)
}

/// Replace the home folder, and failing that the login name, wherever it appears in a path.
fn replace_home(text: &str, home: Option<&str>, user: Option<&str>) -> String {
    let mut result = text.to_string();
    if let Some(home) = home.filter(|h| h.len() > 3) {
        result = result.replace(home, "~").replace(&home.replace('\\', "/"), "~");
    }
    if let Some(user) = user.filter(|u| u.chars().count() >= 3) {
        // only inside paths, so an ordinary word that equals the user name isn't touched
        let pattern = format!(r#"(?i)([/\\](?:home|Users)[/\\]){}([/\\\s"']|$)"#, regex::escape(user));
        if let Ok(r) = Regex::new(&pattern) {
            result = r.replace_all(&result, "${1}<user>${2}").into_owned();
        }
    }
    result
}

/// Hide IPv4 addresses but leave loopback and version-like numbers (such as `mod-1.2.3.4`) alone.
fn redact_addresses(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for m in patterns().ipv4.find_iter(text) {
        out.push_str(&text[last..m.start()]);
        let before = text[..m.start()].chars().next_back().unwrap_or(' ');
        let looks_like_version = before.is_alphabetic() || matches!(before, '-' | '_' | '+');
        let ip = m.as_str();
        let loopback = ip.starts_with("127.") || ip == "0.0.0.0";
        out.push_str(if looks_like_version || loopback { ip } else { "<ip>" });
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

// ---------------------------------------------------------------------------------------------------------
// CrashClassifier

/// What most likely went wrong, in plain words. Every diagnosis has a stable code that docs and support can use.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnosis {
    pub code: &'static str,
    pub title: &'static str,
    pub explanation: String,
    pub fix: String,
    /// The line of the log that led here.
    pub evidence: String,
}

impl std::fmt::Display for Diagnosis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.title)
    }
}

fn evidence(log: &str, pattern: &Regex) -> String {
    let Some(m) = pattern.find(log) else { return String::new() };
    let start = log[..m.start()].rfind('\n').map_or(0, |i| i + 1);
    let end = log[m.end()..].find('\n').map_or(log.len(), |i| m.end() + i);
    let line = log[start..end].trim();
    if line.chars().count() > 300 {
        format!("{}\u{2026}", line.chars().take(300).collect::<String>())
    } else {
        line.to_string()
    }
}

fn has(log: &str, pattern: &str) -> bool {
    Regex::new(&format!("(?i){pattern}")).map(|r| r.is_match(log)).unwrap_or(false)
}

/// Read a game log or crash report and say what probably broke. Each check looks for a specific signature, so a
/// diagnosis says "likely", never "certainly", and the first one listed is the most specific.
pub fn classify(log: &str) -> Vec<Diagnosis> {
    let mut found = Vec::new();
    if log.is_empty() {
        return found;
    }
    wrong_java(log, &mut found);
    bad_jvm_argument(log, &mut found);
    out_of_memory(log, &mut found);
    missing_dependency(log, &mut found);
    duplicate_mods(log, &mut found);
    mixin(log, &mut found);
    corrupt_file(log, &mut found);
    disk_full(log, &mut found);
    graphics(log, &mut found);
    natives(log, &mut found);
    permissions(log, &mut found);
    found
}

fn wrong_java(log: &str, out: &mut Vec<Diagnosis>) {
    let p =
        re(r"(?i)UnsupportedClassVersionError|compiled by a more recent version of the Java Runtime|requires Java \d+");
    if !p.is_match(log) {
        return;
    }
    let line = evidence(log, &p);
    let class_version = re(r"(?i)(?:class file version|major version|version) (\d{2})\.\d");
    let mut needed = "a newer version of Java".to_string();
    let haystack = if line.is_empty() { log } else { line.as_str() };
    if let Some(c) = class_version.captures(haystack) {
        if let Ok(class_file) = c[1].parse::<i32>() {
            if class_file >= 52 {
                needed = format!("Java {} or newer", class_file - 44);
            }
        }
    }
    out.push(Diagnosis {
        code: "MC-JAVA-VERSION",
        title: "Minecraft is using the wrong version of Java",
        explanation: format!("Something in this instance was built for {needed}, but an older Java is running it."),
        fix: format!(
            "In the instance settings turn on \"Use Java provided by Minecraft\", or choose a Java that is {needed}."
        ),
        evidence: line,
    });
}

fn bad_jvm_argument(log: &str, out: &mut Vec<Diagnosis>) {
    let p = re(
        r"(?i)Unrecognized VM option|Unrecognized option|Could not create the Java Virtual Machine|Invalid maximum heap size|Improperly specified VM option",
    );
    if !p.is_match(log) {
        return;
    }
    out.push(Diagnosis {
        code: "MC-JVM-ARGUMENT",
        title: "A Java argument isn't accepted",
        explanation: "Java refused to start because one of the extra arguments isn't valid for the Java that's being used.".into(),
        fix: "Open Settings and clear the Java arguments, or remove the option named in the log. Options for old Java versions often don't work on new ones.".into(),
        evidence: evidence(log, &p),
    });
}

fn out_of_memory(log: &str, out: &mut Vec<Diagnosis>) {
    let p = re(
        r"(?i)java\.lang\.OutOfMemoryError|insufficient memory for the Java Runtime|Could not reserve enough space for (?:\d+KB )?(?:object )?heap",
    );
    if !p.is_match(log) {
        return;
    }
    let reserve = has(log, "Could not reserve enough space|insufficient memory for the Java Runtime");
    out.push(Diagnosis {
        code: "MC-OUT-OF-MEMORY",
        title: "Minecraft ran out of memory",
        explanation: if reserve {
            "Java couldn't get as much memory as it was asked for, so the game never started.".into()
        } else {
            "The game used all the memory it was allowed to.".into()
        },
        fix: if reserve {
            "Lower the memory in Settings, or close other programs. A 32-bit Java can't use more than about 1.5 GB."
                .into()
        } else {
            "Raise the memory in Settings. Large modpacks usually need 6 GB or more.".into()
        },
        evidence: evidence(log, &p),
    });
}

fn missing_dependency(log: &str, out: &mut Vec<Diagnosis>) {
    let p = re(
        r"(?i)Missing or unsupported mandatory dependencies|Unmet dependency listing|Could not find required mod|requires (?:any )?(?:version|mod)|Mod .* requires|MissingModsException|incompatible mod set",
    );
    if !p.is_match(log) {
        return;
    }
    let mod_names = re(r#"(?i)(?:requires|depends on|missing)[^\n]{0,80}?(?:mod )?['"`]?([a-z0-9_.-]{3,40})['"`]?"#);
    let mut names: Vec<String> = Vec::new();
    for c in mod_names.captures_iter(log) {
        if names.len() >= 5 {
            break;
        }
        let name = c[1].to_string();
        let lower = name.to_ascii_lowercase();
        if lower != "version" && lower != "any" && lower != "the" && !names.contains(&name) {
            names.push(name);
        }
    }
    out.push(Diagnosis {
        code: "MC-MISSING-DEPENDENCY",
        title: "A mod is missing something it needs",
        explanation: format!(
            "A mod needs another mod, or a different version of one, that isn't installed.{}",
            if names.is_empty() { String::new() } else { format!(" Mentioned in the log: {}.", names.join(", ")) }
        ),
        fix: "Install the mod named in the log (and the right loader and Minecraft version for it), or remove the mod that asked for it.".into(),
        evidence: evidence(log, &p),
    });
}

fn duplicate_mods(log: &str, out: &mut Vec<Diagnosis>) {
    let p = re(r"(?i)duplicate mods?|DuplicateModsFoundException|found duplicate");
    if !p.is_match(log) {
        return;
    }
    out.push(Diagnosis {
        code: "MC-DUPLICATE-MOD",
        title: "The same mod is installed twice",
        explanation: "Two files in the mods folder are the same mod, usually two versions of it.".into(),
        fix: "Open the instance's mods folder and delete the older copy.".into(),
        evidence: evidence(log, &p),
    });
}

fn mixin(log: &str, out: &mut Vec<Diagnosis>) {
    let p = re(
        r"(?i)MixinApplyError|InvalidMixinException|Mixin apply failed|Mixin transformation of|MixinTransformerError",
    );
    if !p.is_match(log) {
        return;
    }
    out.push(Diagnosis {
        code: "MC-MIXIN-CONFLICT",
        title: "Two mods are fighting over the same part of the game",
        explanation: "A mod's changes to Minecraft couldn't be applied, which usually means it clashes with another mod or was made for a different version.".into(),
        fix: "Look for the mod named next to \"Mixin\" in the log. Update it, or remove it and any similar mod, then try again.".into(),
        evidence: evidence(log, &p),
    });
}

fn corrupt_file(log: &str, out: &mut Vec<Diagnosis>) {
    let p = re(
        r"(?i)Invalid or corrupt jarfile|zip END header not found|ZipException|error in opening zip file|Unexpected end of ZLIB",
    );
    if !p.is_match(log) {
        return;
    }
    out.push(Diagnosis {
        code: "MC-CORRUPT-FILE",
        title: "A downloaded file is damaged",
        explanation: "A jar file couldn't be opened, which normally means it didn't finish downloading.".into(),
        fix: "Delete the mod named in the log and add it again. If it is a library, use Tools > Clear downloads."
            .into(),
        evidence: evidence(log, &p),
    });
}

fn disk_full(log: &str, out: &mut Vec<Diagnosis>) {
    let p = re(r"(?i)No space left on device|not enough space on the disk|There is not enough space");
    if !p.is_match(log) {
        return;
    }
    out.push(Diagnosis {
        code: "SYS-DISK-FULL",
        title: "The disk is full",
        explanation: "There's no room left to write files.".into(),
        fix: "Free up space on the drive BlockForge uses, then try again.".into(),
        evidence: evidence(log, &p),
    });
}

fn graphics(log: &str, out: &mut Vec<Diagnosis>) {
    let p = re(
        r"(?i)GLFW error|Pixel format not accepted|does not appear to support OpenGL|GL_INVALID|Failed to create window|OpenGL 3\.\d? is required",
    );
    if !p.is_match(log) {
        return;
    }
    out.push(Diagnosis {
        code: "MC-GRAPHICS",
        title: "Minecraft couldn't start its graphics",
        explanation: "The graphics driver didn't give Minecraft a window it can draw in.".into(),
        fix: "Update your graphics driver. On a laptop, make sure Java runs on the dedicated GPU. Turn off overlays and screen recorders.".into(),
        evidence: evidence(log, &p),
    });
}

fn natives(log: &str, out: &mut Vec<Diagnosis>) {
    let p = re(r"(?i)UnsatisfiedLinkError|Failed to locate library|Could not load native|lwjgl.*(?:not found|failed)");
    if !p.is_match(log) || has(log, "GLFW error") {
        return;
    }
    out.push(Diagnosis {
        code: "MC-NATIVES",
        title: "A native library failed to load",
        explanation: "Minecraft couldn't load one of its system libraries (for example LWJGL or OpenAL).".into(),
        fix: "Use Tools > Delete libraries and start again so they're downloaded fresh. On ARM or Apple Silicon, make sure the instance runs an ARM Java.".into(),
        evidence: evidence(log, &p),
    });
}

fn permissions(log: &str, out: &mut Vec<Diagnosis>) {
    let p = re(r"(?i)AccessDeniedException|Permission denied|Access is denied");
    if !p.is_match(log) {
        return;
    }
    out.push(Diagnosis {
        code: "SYS-PERMISSION",
        title: "BlockForge isn't allowed to use a file",
        explanation:
            "A file or folder couldn't be opened because of its permissions, or because another program has it open."
                .into(),
        fix: "Close other programs using the instance folder, and make sure it is inside a folder you own.".into(),
        evidence: evidence(log, &p),
    });
}

// ---------------------------------------------------------------------------------------------------------
// SupportBundle

/// What goes into a bundle. Everything is optional.
#[derive(Default, Clone, Debug)]
pub struct BundleInput {
    /// The launcher's own log.
    pub launcher_log: Option<PathBuf>,
    /// The instance's folder, for its game log, crash reports and mod list.
    pub instance_root: Option<PathBuf>,
    pub instance_name: Option<String>,
    /// Lines of facts about the launcher and system, such as versions and the chosen Java.
    pub facts: Vec<String>,
    /// Values that must never appear in the bundle, like the player's name.
    pub secrets: Vec<String>,
}

/// The last part of a text file, or an empty string if it can't be read.
pub fn tail(file: &Path) -> String {
    tail_limited(file, MAX_LOG_BYTES)
}

fn tail_limited(file: &Path, max: u64) -> String {
    let Ok(mut f) = File::open(file) else { return String::new() };
    let Ok(meta) = f.metadata() else { return String::new() };
    if !meta.is_file() {
        return String::new();
    }
    let length = meta.len();
    let start = length.saturating_sub(max);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut bytes = Vec::with_capacity((length - start) as usize);
    if f.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        let rest = text.find('\n').map_or(text.as_str(), |i| &text[i + 1..]);
        format!("[earlier lines left out]\n{rest}")
    } else {
        text
    }
}

fn newest_crash_report(root: &Path) -> Option<PathBuf> {
    std::fs::read_dir(root.join("crash-reports"))
        .ok()?
        .flatten()
        .filter(|e| e.path().is_file())
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

fn mod_list(root: &Path) -> String {
    let mods = root.join("mods");
    if !mods.is_dir() {
        return "No mods folder\n".into();
    }
    let Ok(rd) = std::fs::read_dir(&mods) else { return "Couldn't read the mods folder\n".into() };
    let mut lines: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            match e.metadata() {
                Ok(m) => format!("{name}  ({} bytes)", m.len()),
                Err(_) => name,
            }
        })
        .collect();
    lines.sort();
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn summary(input: &BundleInput, diagnoses: &[Diagnosis], crash_file: Option<&Path>) -> String {
    let mut s = String::new();
    s.push_str("BlockForge support bundle\n");
    s.push_str("Personal details (player name, account IDs, tokens, e-mail, IP addresses, your user name) have been removed. Open the files to check.\n\n");
    if let Some(name) = &input.instance_name {
        s.push_str(&format!("Instance: {name}\n"));
    }
    for f in &input.facts {
        s.push_str(f);
        s.push('\n');
    }
    if let Some(c) = crash_file.and_then(|c| c.file_name()) {
        s.push_str(&format!("Crash report: {}\n", c.to_string_lossy()));
    }
    s.push_str("\nLikely causes\n");
    if diagnoses.is_empty() {
        s.push_str("  Nothing recognised in the logs.\n");
    }
    for d in diagnoses {
        s.push_str(&format!("  [{}] {}\n    {}\n    Try: {}\n", d.code, d.title, d.explanation, d.fix));
        if !d.evidence.is_empty() {
            s.push_str(&format!("    Log: {}\n", d.evidence));
        }
    }
    s
}

/// Write the bundle to `zip_path` and return the diagnoses found in the logs.
pub fn create_bundle(zip_path: &Path, input: &BundleInput) -> Result<Vec<Diagnosis>> {
    let launcher_log = input.launcher_log.as_deref().map(tail).unwrap_or_default();
    let mut game_log = String::new();
    let mut crash_report = String::new();
    let mut crash_file = None;
    if let Some(root) = &input.instance_root {
        game_log = tail(&root.join("logs").join("latest.log"));
        crash_file = newest_crash_report(root);
        crash_report = crash_file.as_deref().map(tail).unwrap_or_default();
    }
    let diagnoses = classify(&format!("{crash_report}\n{game_log}"));

    if let Some(parent) = zip_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let file = File::create(zip_path).with_context(|| format!("creating {}", zip_path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut add = |name: &str, content: &str| -> Result<()> {
        zip.start_file(name, options)?;
        zip.write_all(scrub(content, &input.secrets).as_bytes())?;
        Ok(())
    };
    add("summary.txt", &summary(input, &diagnoses, crash_file.as_deref()))?;
    add("launcher.log", &launcher_log)?;
    if let Some(root) = &input.instance_root {
        add("latest.log", &game_log)?;
        if crash_file.is_some() {
            add("crash-report.txt", &crash_report)?;
        }
        add("instance.json", &tail(&root.join("instance.json")))?;
        add("mods.txt", &mod_list(root))?;
    }
    zip.finish()?;
    Ok(diagnoses)
}

/// The names of the files in a bundle, for tests and for showing what is inside.
pub fn bundle_contents(zip_path: &Path) -> Result<BTreeSet<String>> {
    let mut a = zip::ZipArchive::new(File::open(zip_path)?)?;
    Ok((0..a.len()).filter_map(|i| a.by_index(i).ok().and_then(|f| f.name().map(|n| n.to_string()).ok())).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: Option<&str> = Some("/home/steve");
    const USER: Option<&str> = Some("steve");

    fn s(text: &str) -> String {
        scrub_with(text, &[], HOME, USER)
    }

    #[test]
    fn tokens_ids_and_addresses_disappear() {
        let out = s(
            "java --accessToken abc123def --uuid 069a79f4-44e9-4726-a5be-fca90e38aaf5 --username Notch --version 1.21",
        );
        assert_eq!(out, "java --accessToken <hidden> --uuid <hidden> --username <hidden> --version 1.21");
        assert_eq!(s(r#"{"refresh_token":"M.R3_secret","name":"x"}"#), r#"{"refresh_token":"<hidden>","name":"x"}"#);
        assert_eq!(s("Authorization: Bearer abcdefgh12345678"), "Authorization: Bearer <token>");
        assert_eq!(s("jwt eyJhbGciOiJI.eyJzdWIiOiIx.SflKxwRJSMeK ok"), "jwt <token> ok");
        assert_eq!(s("mail me at steve@example.com now"), "mail me at <email> now");
        assert_eq!(s("id 069a79f4-44e9-4726-a5be-fca90e38aaf5."), "id <uuid>.");
        assert_eq!(s("connecting to 203.0.113.7:25565"), "connecting to <ip>:25565");
    }

    #[test]
    fn loopback_and_versions_are_left_alone() {
        assert_eq!(s("server 127.0.0.1 and 0.0.0.0"), "server 127.0.0.1 and 0.0.0.0");
        assert_eq!(s("lib-1.2.3.4.jar and mod_5.6.7.8"), "lib-1.2.3.4.jar and mod_5.6.7.8");
    }

    #[test]
    fn home_folder_and_user_name_are_hidden_in_paths_only() {
        assert_eq!(s("/home/steve/.local/share/blockforge/x"), "~/.local/share/blockforge/x");
        assert_eq!(scrub_with("C:\\Users\\steve\\AppData", &[], Some("C:\\Users\\steve"), Some("steve")), "~\\AppData");
        assert_eq!(scrub_with("/Users/steve/Library", &[], None, Some("steve")), "/Users/<user>/Library");
        assert_eq!(scrub_with("steve joined the game", &[], None, Some("steve")), "steve joined the game");
    }

    #[test]
    fn extra_secrets_are_removed_everywhere() {
        let secrets = vec!["Notch".to_string(), "ab".to_string()];
        assert_eq!(
            scrub_with("Notch said hi to Notch; ab stays", &secrets, None, None),
            "<hidden> said hi to <hidden>; ab stays"
        );
    }

    #[test]
    fn recognises_the_common_crashes() {
        let java = classify(
            "Exception in thread \"main\" java.lang.UnsupportedClassVersionError: net/minecraft/Foo has been compiled by a more recent version (class file version 65.0)",
        );
        assert_eq!(java[0].code, "MC-JAVA-VERSION");
        assert!(java[0].explanation.contains("Java 21 or newer"), "{}", java[0].explanation);

        let oom = classify("java.lang.OutOfMemoryError: Java heap space");
        assert_eq!(oom[0].code, "MC-OUT-OF-MEMORY");
        assert!(oom[0].fix.starts_with("Raise the memory"));
        let reserve = classify(
            "Error occurred during initialization of VM\nCould not reserve enough space for 8388608KB object heap",
        );
        assert!(reserve.iter().any(|d| d.code == "MC-OUT-OF-MEMORY" && d.fix.starts_with("Lower the memory")));

        let dep = classify("Mod sodium requires mod 'fabric-api' version 0.100 or later");
        assert_eq!(dep[0].code, "MC-MISSING-DEPENDENCY");

        assert!(classify("DuplicateModsFoundException: found duplicate").iter().any(|d| d.code == "MC-DUPLICATE-MOD"));
        assert!(classify("MixinApplyError: Mixin [x] from phase failed").iter().any(|d| d.code == "MC-MIXIN-CONFLICT"));
        assert!(
            classify("java.util.zip.ZipException: zip END header not found")
                .iter()
                .any(|d| d.code == "MC-CORRUPT-FILE")
        );
        assert!(classify("java.io.IOException: No space left on device").iter().any(|d| d.code == "SYS-DISK-FULL"));
        assert!(
            classify("GLFW error 65543: WGL: The driver does not appear to support OpenGL")
                .iter()
                .any(|d| d.code == "MC-GRAPHICS")
        );
        assert!(classify("java.lang.UnsatisfiedLinkError: liblwjgl.so").iter().any(|d| d.code == "MC-NATIVES"));
        assert!(
            classify("GLFW error and UnsatisfiedLinkError").iter().all(|d| d.code != "MC-NATIVES"),
            "a graphics failure is not a missing library"
        );
        assert!(classify("java.nio.file.AccessDeniedException: x").iter().any(|d| d.code == "SYS-PERMISSION"));
        assert!(classify("[Render thread/INFO]: all fine").is_empty());
        assert!(classify("").is_empty());
    }

    #[test]
    fn evidence_is_the_line_that_matched() {
        let d = classify("line one\nSomething failed: java.lang.OutOfMemoryError: heap\nline three");
        assert_eq!(d[0].evidence, "Something failed: java.lang.OutOfMemoryError: heap");
    }

    #[test]
    fn tail_keeps_the_end_of_a_big_file() {
        let dir = std::env::temp_dir().join(format!("blockforge-diag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("big.log");
        let text: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&f, &text).unwrap();
        let t = tail_limited(&f, 100);
        assert!(t.starts_with("[earlier lines left out]\nline "), "{t}");
        assert!(t.ends_with("line 999\n"));
        assert_eq!(tail_limited(&f, 1_000_000), text);
        assert_eq!(tail_limited(&dir.join("missing.log"), 100), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bundle_has_the_expected_files_and_nothing_private() {
        let dir = std::env::temp_dir().join(format!("blockforge-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let inst = dir.join("inst");
        std::fs::create_dir_all(inst.join("logs")).unwrap();
        std::fs::create_dir_all(inst.join("mods")).unwrap();
        std::fs::create_dir_all(inst.join("crash-reports")).unwrap();
        std::fs::write(
            inst.join("logs/latest.log"),
            "Setting user: Notch\njava.lang.OutOfMemoryError: Java heap space\n",
        )
        .unwrap();
        std::fs::write(inst.join("crash-reports/crash-1.txt"), "crash for Notch").unwrap();
        std::fs::write(inst.join("mods/a.jar"), "12345").unwrap();
        std::fs::write(inst.join("instance.json"), "{\"accessToken\":\"secret\"}").unwrap();
        let log = dir.join("blockforge.log");
        std::fs::write(&log, "started as Notch").unwrap();

        let mut input = BundleInput {
            launcher_log: Some(log),
            instance_root: Some(inst),
            instance_name: Some("Test Pack".into()),
            ..Default::default()
        };
        input.facts.push("Launcher: BlockForge 0.1.0".into());
        input.secrets.push("Notch".into());
        let zip_path = dir.join("out").join("bundle.zip");
        let found = create_bundle(&zip_path, &input).unwrap();
        assert_eq!(found[0].code, "MC-OUT-OF-MEMORY");

        let names = bundle_contents(&zip_path).unwrap();
        for n in ["summary.txt", "launcher.log", "latest.log", "crash-report.txt", "instance.json", "mods.txt"] {
            assert!(names.contains(n), "missing {n}: {names:?}");
        }
        let mut a = zip::ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
        let mut all = String::new();
        for i in 0..a.len() {
            a.by_index(i).unwrap().read_to_string(&mut all).unwrap();
        }
        assert!(!all.contains("Notch"), "player name leaked");
        assert!(!all.contains("secret"), "token leaked");
        assert!(
            all.contains("a.jar  (5 bytes)")
                && all.contains("Launcher: BlockForge 0.1.0")
                && all.contains("MC-OUT-OF-MEMORY")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
