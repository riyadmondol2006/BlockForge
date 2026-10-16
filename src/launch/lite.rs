//! The smoothness helper ("BlockForge Lite"): a second small Java agent (`packaging/lite`) that the game is started
//! with, next to the fast-start one (see `boost`). Like that one it isn't a mod, so it works with vanilla and every mod
//! loader (Fabric, Quilt, Forge, NeoForge). It makes small changes inside the running game that make it smoother; it
//! writes what it did to a report file, and it is told which mods are installed so it can leave alone what a
//! performance mod already does.
//!
//! It is on by default. A player can switch it off for all games (the `lite` setting) or for one instance (the `lite`
//! entry of the instance's launcher settings, which wins). Single changes can be switched off by their id with the
//! `liteOff` setting (`ID;ID`). When a game stops soon after it started with a helper on, [`may_be_the_cause`] says
//! so; two such starts in a row switch this helper off for that instance by themselves ([`record_start`]).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::config::Settings;
use crate::instances::Instance;
use crate::launch::boost;
use crate::paths::Dirs;

/// The built agent. It is part of the program, so there is nothing to install or to go missing.
static AGENT_JAR: &[u8] = include_bytes!("../../packaging/lite/blockforge-lite.jar");

/// The agent's file name, also used to tell whether a player already starts the game with it.
pub const JAR_NAME: &str = "blockforge-lite.jar";

/// Every line the agent prints starts with this, so it can be found in the game's log.
pub const MARKER: &str = "[BlockForge Lite]";

/// After this many early failures in a row with a helper on, this helper is switched off for the instance.
pub const FAILURES_BEFORE_OFF: u32 = 2;

/// Whether the helper is on for this instance: the instance's own `lite` entry, otherwise the launcher-wide setting.
pub fn enabled(settings: &Settings, instance: &Instance) -> bool {
    instance_override(instance).unwrap_or(settings.lite)
}

/// The instance's own switch, if it has one (as opposed to following the launcher-wide setting).
pub fn instance_override(instance: &Instance) -> Option<bool> {
    instance.file.launcher.extras.get("lite").and_then(|v| v.as_bool())
}

/// Where the agent file is kept.
pub fn jar_path(dirs: &Dirs) -> PathBuf {
    dirs.cache.join(JAR_NAME)
}

/// Writes the agent file if it isn't there or is another version, and returns where it is.
pub fn ensure_jar(dirs: &Dirs) -> Result<PathBuf> {
    let path = jar_path(dirs);
    if std::fs::read(&path).ok().as_deref() != Some(AGENT_JAR) {
        std::fs::create_dir_all(&dirs.cache).with_context(|| format!("making {}", dirs.cache.display()))?;
        // write beside it and rename, so a game that is starting never sees half a file
        let temp = path.with_extension(format!("jar.{}", std::process::id()));
        std::fs::write(&temp, AGENT_JAR)?;
        std::fs::rename(&temp, &path)?;
    }
    Ok(path)
}

/// The name the files of one instance are kept under: its name in plain letters and digits, else its folder's.
pub fn instance_key(instance: &Instance) -> String {
    let plain = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>();
    [plain(instance.name()), plain(&instance.folder)]
        .into_iter()
        .find(|k| !k.is_empty())
        .unwrap_or_else(|| "instance".to_string())
}

/// The folder the helper's files are kept in (reports, mod lists, failure counts).
pub fn folder(dirs: &Dirs) -> PathBuf {
    dirs.cache.join("lite")
}

/// The file the agent writes what it did to, one per instance.
pub fn report_path(dirs: &Dirs, key: &str) -> PathBuf {
    folder(dirs).join(format!("{key}.report"))
}

/// The file with the names of the instance's mods, next to the report.
pub fn mods_path(dirs: &Dirs, key: &str) -> PathBuf {
    folder(dirs).join(format!("{key}.mods"))
}

fn failures_path(dirs: &Dirs, key: &str) -> PathBuf {
    folder(dirs).join(format!("{key}.failures"))
}

/// The file names of the mods the game will load: jars and zips directly in `mods` (a `.disabled` one isn't loaded),
/// sorted.
pub fn enabled_mods(game_directory: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(game_directory.join("mods")) else { return Vec::new() };
    let mut names: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| {
            let lower = n.to_lowercase();
            lower.ends_with(".jar") || lower.ends_with(".zip")
        })
        .collect();
    names.sort();
    names
}

/// The ids a mod jar gives itself: from `fabric.mod.json` (`id` and `provides`), `quilt.mod.json`, and the first
/// `modId` of Forge's and NeoForge's `mods.toml`; `optifine` for OptiFine, which has no mod file of its own.
pub fn mod_ids(jar: &Path) -> Vec<String> {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(jar) else { return Vec::new() };
    let Ok(mut zip) = zip::ZipArchive::new(file) else { return Vec::new() };
    let mut ids: Vec<String> = Vec::new();
    let read = |zip: &mut zip::ZipArchive<std::fs::File>, name: &str| -> Option<String> {
        let mut entry = zip.by_name(name).ok()?;
        // a mod's description is small; anything big isn't one
        if entry.size() > 1 << 20 {
            return None;
        }
        let mut text = String::new();
        entry.read_to_string(&mut text).ok()?;
        Some(text)
    };
    let json = |t: String| serde_json::from_str::<serde_json::Value>(&t).ok();
    if let Some(v) = read(&mut zip, "fabric.mod.json").and_then(json) {
        ids.extend(v.get("id").and_then(|i| i.as_str()).map(str::to_string));
        if let Some(list) = v.get("provides").and_then(|p| p.as_array()) {
            ids.extend(list.iter().filter_map(|p| p.as_str()).map(str::to_string));
        }
    }
    if let Some(v) = read(&mut zip, "quilt.mod.json").and_then(json) {
        ids.extend(v.pointer("/quilt_loader/id").and_then(|i| i.as_str()).map(str::to_string));
    }
    for toml in ["META-INF/mods.toml", "META-INF/neoforge.mods.toml"] {
        if let Some(t) = read(&mut zip, toml) {
            ids.extend(first_mod_id(&t));
        }
    }
    let optifine = |n: &str| n.starts_with("optifine/OptiFineClassTransformer") || n.starts_with("net/optifine/");
    if zip.file_names().any(|n| n.is_ok_and(|n| optifine(&n))) {
        ids.push("optifine".to_string());
    }
    ids.retain(|i| !i.trim().is_empty());
    ids
}

/// The first `modId = "..."` of a `mods.toml` (later ones belong to the dependencies).
fn first_mod_id(toml: &str) -> Option<String> {
    toml.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("modId")?.trim_start().strip_prefix('=')?.trim();
        let id = rest.trim_start_matches(['"', '\'']).split(['"', '\'']).next().unwrap_or("").trim();
        (!id.is_empty()).then(|| id.to_string())
    })
}

/// What the agent is told is installed: each enabled mod's file name and the ids it gives itself, one per line.
pub fn mod_list(game_directory: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in enabled_mods(game_directory) {
        let ids = mod_ids(&game_directory.join("mods").join(&name));
        out.push(name);
        for id in ids {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

/// The ids in the `liteOff` setting (`ID;ID`, spaces and commas also separate), as the agent wants them.
pub fn off_list(setting: &str) -> String {
    setting.split([';', ',', ' ']).map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(";")
}

/// The change that is never switched on by the launcher, even at the balanced level: it can lag a frame behind a fast
/// camera turn and its gain was only measured with software rendering.
const NEVER_BY_DEFAULT: &str = "hidden-entities";

/// The agent's options: `quiet`, where to write the report, the file with the mod names and the switched-off
/// changes. With `balanced` (Low-end mode is on) it also asks for level 2: the changes that may alter something small
/// a player could notice (a frame limiter that waits less, sounds that start without waiting for the sound thread,
/// muted music that isn't decoded). The agent splits the options at commas, so a path with one is left out (the
/// agent then does without it).
pub fn options(report: &Path, mods: Option<&Path>, off: &str, balanced: bool) -> String {
    let mut parts = vec!["quiet".to_string()];
    if balanced {
        parts.push("level=2".to_string());
    }
    let usable = |p: &Path| !p.to_string_lossy().contains(',');
    if usable(report) {
        parts.push(format!("report={}", report.display()));
    }
    if let Some(m) = mods.filter(|m| usable(m)) {
        parts.push(format!("mods={}", m.display()));
    }
    let mut off = off_list(off);
    if balanced && !off.split(';').any(|id| id == NEVER_BY_DEFAULT) {
        if !off.is_empty() {
            off.push(';');
        }
        off.push_str(NEVER_BY_DEFAULT);
    }
    if !off.is_empty() {
        parts.push(format!("off={off}"));
    }
    parts.join(",")
}

/// The Java option that loads the agent.
pub fn java_argument(jar: &Path, options: &str) -> String {
    format!("-javaagent:{}={}", jar.display(), options)
}

/// If the Java options already start this agent: the player put it in their own Java arguments (any copy of the file
/// counts), or it was added before.
pub fn already_there(java_options: &[String]) -> bool {
    java_options.iter().any(|a| {
        a.strip_prefix("-javaagent:").is_some_and(|rest| {
            let path = rest.split('=').next().unwrap_or(rest);
            path.replace('\\', "/").rsplit('/').next().is_some_and(|f| f.eq_ignore_ascii_case(JAR_NAME))
        })
    })
}

/// Where the agent goes among the Java options: right after the fast-start one when that is there (agents start in
/// the order they are given), otherwise first.
pub fn insert_position(java_options: &[String]) -> usize {
    java_options
        .iter()
        .position(|a| a.starts_with("-javaagent:") && a.contains("blockforge-boost"))
        .map(|i| i + 1)
        .unwrap_or(0)
}

/// What setting the helper up for one start came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Added {
    /// The Java option to add, at this place among the options.
    Argument(usize, String),
    /// The player's own Java arguments already start it.
    PlayersOwn,
    /// The agent's path can't be given to Java (it has a `=` or `,`); the start goes on without it.
    UnusablePath(PathBuf),
}

/// If Java can load an agent from this path: it refuses to start when the jar's path has a `=` in it (measured on
/// Java 8, 21 and 25), and a `,` would be read as the start of the agent's options.
pub fn usable_jar_path(jar: &Path) -> bool {
    let text = jar.to_string_lossy();
    !text.contains('=') && !text.contains(',')
}

/// Prepares the helper for one start: writes the agent and the list of mods. `java_options` are the options so far.
/// Fails only when the agent itself couldn't be written (the start then goes on without it).
pub fn prepare(
    dirs: &Dirs,
    instance: &Instance,
    off_setting: &str,
    balanced: bool,
    java_options: &[String],
) -> Result<(Added, Vec<String>)> {
    let mut notes = Vec::new();
    if already_there(java_options) {
        return Ok((Added::PlayersOwn, notes));
    }
    if !usable_jar_path(&jar_path(dirs)) {
        return Ok((Added::UnusablePath(jar_path(dirs)), notes));
    }
    let jar = ensure_jar(dirs)?;
    let key = instance_key(instance);
    let report = report_path(dirs, &key);
    let mods_file = mods_path(dirs, &key);
    let mods = mod_list(&instance.root);
    let mut text = mods.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    let written = std::fs::create_dir_all(folder(dirs)).and_then(|_| std::fs::write(&mods_file, text));
    if let Err(e) = &written {
        notes.push(format!("Warning: Smoothness helper: couldn't write the list of mods ({e}); it goes without"));
    }
    let options = options(&report, written.is_ok().then_some(mods_file.as_path()), off_setting, balanced);
    Ok((Added::Argument(insert_position(java_options), java_argument(&jar, &options)), notes))
}

/// The names Minecraft's classes and methods have while the game runs, which the helper's changes are found by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeNames {
    /// Mojang's own names: 26.1 and newer on every loader, NeoForge from 1.20.5.
    Mojang,
    /// Fabric's and Quilt's intermediary names (`class_310`, `method_1234`), up to 1.21.11.
    Intermediary,
    /// Forge's SRG names (`m_91383_`, `func_71411_J`), and NeoForge before 1.20.5.
    Srg,
    /// Vanilla up to 1.21.11: short obfuscated names (`fgo`, `a`).
    Obfuscated,
}

/// Which names a game runs with, from its Minecraft version and mod loader (as `Instance::loader` names it).
///
/// The helper is written in Mojang names. Where the game runs with other names, the launcher is to pass it a small
/// file that says what each name is at run time (Mojang's mappings joined with intermediary or SRG, all on disk or one
/// download away). That file's form is the agent's to define; until it is, nothing is passed and the helper only
/// changes games that run with Mojang names, leaving the others alone.
pub fn runtime_names(minecraft_version: &str, loader: Option<&str>) -> RuntimeNames {
    let minor = crate::launch::memory::minor_version(Some(minecraft_version));
    if minor >= 100 {
        return RuntimeNames::Mojang;
    }
    let patch: u32 = minecraft_version
        .split('.')
        .nth(2)
        .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    match loader.map(str::to_ascii_lowercase).as_deref() {
        Some("fabric") | Some("quilt") => RuntimeNames::Intermediary,
        Some("neoforge") if minor > 20 || (minor == 20 && patch >= 5) => RuntimeNames::Mojang,
        Some("forge") | Some("neoforge") => RuntimeNames::Srg,
        _ => RuntimeNames::Obfuscated,
    }
}

/// If a game that has ended could have been stopped by one of the helpers (fast start, smoothness). `code` is the
/// exit code (None when the game was killed from outside, which is the player's doing), `ran_for` how long it ran,
/// `log` its output lines.
///
/// True when the JVM complained about an agent, when an agent printed a problem and the game then failed, or when the
/// game failed soon after it started (the helpers are only one of the possible reasons, so the window words it as
/// "try turning them off").
pub fn may_be_the_cause(fast_start: bool, lite: bool, code: Option<i32>, ran_for: Duration, log: &[String]) -> bool {
    if !fast_start && !lite {
        return false;
    }
    if boost::may_be_the_cause(true, code, ran_for, log) {
        return true;
    }
    let failed = code.is_some_and(|c| c != 0);
    lite && failed && log.iter().any(|l| l.contains(MARKER) && l.contains("problem"))
}

/// Remembers how a start with a helper on went: an early failure adds one to the instance's count, anything else
/// clears it. Returns the count now.
pub fn record_start(dirs: &Dirs, key: &str, early_failure: bool) -> u32 {
    let path = failures_path(dirs, key);
    if !early_failure {
        let _ = std::fs::remove_file(&path);
        return 0;
    }
    let count = std::fs::read_to_string(&path).ok().and_then(|t| t.trim().parse::<u32>().ok()).unwrap_or(0) + 1;
    let _ = std::fs::create_dir_all(folder(dirs)).and_then(|_| std::fs::write(&path, count.to_string()));
    count
}

/// If the helper should switch itself off for an instance after this many early failures in a row.
pub fn should_turn_off(failures_in_a_row: u32) -> bool {
    failures_in_a_row >= FAILURES_BEFORE_OFF
}

/// One change the helper reported on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub id: String,
    pub state: PatchState,
    /// The agent's own words on what happened (why it was skipped, what it did), if any.
    pub detail: String,
    /// What the change is, in plain words (the report's 5th column), if the agent gave it.
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchState {
    On,
    Off,
    Problem,
    Other,
}

impl PatchState {
    fn parse(word: &str) -> PatchState {
        match word.trim().to_ascii_lowercase().as_str() {
            "on" | "applied" | "active" | "ok" | "enabled" | "patched" | "true" => PatchState::On,
            "off" | "skipped" | "skip" | "disabled" | "inactive" | "false" | "not-applied" | "unused" => {
                PatchState::Off
            }
            "problem" | "failed" | "fail" | "error" => PatchState::Problem,
            _ => PatchState::Other,
        }
    }

    /// The word shown in the window.
    pub fn label(self) -> &'static str {
        match self {
            PatchState::On => "Applied",
            PatchState::Off => "Not used",
            PatchState::Problem => "Problem",
            PatchState::Other => "Unknown",
        }
    }
}

/// Reads a report file the agent wrote: one change per line, `id`, `applied`/`skipped`/`failed`, the class and the reason,
/// separated by tabs, `#` starting a comment. Lenient on purpose, as the agent is built on its own: `id state [what
/// happened]`
/// (separated by tabs, spaces, `=` or `:`; `#` starts a comment), or JSON (`{"patches":[{"id":..,"status":..}]}` or a
/// map of id to state).
pub fn parse_report(text: &str) -> Vec<Patch> {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return parse_json_report(trimmed);
    }
    let mut out = Vec::new();
    for raw in trimmed.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let mut parts = line.splitn(2, ['\t', '=', ':', ' ']);
        let id = parts.next().unwrap_or("").trim();
        let rest = parts.next().unwrap_or("").trim_start_matches([' ', '\t', '=', ':']);
        if id.is_empty() {
            continue;
        }
        // the agent's own form: id, applied/skipped/failed, class, reason, separated by tabs
        let columns: Vec<&str> = raw.trim_start().trim_end_matches(['\r', ' ']).split('\t').map(str::trim).collect();
        if columns.len() >= 3 {
            // id, state, class, reason, description; with three columns the third is the only text there is
            let detail = if columns.len() >= 4 { columns[3] } else { columns[2] };
            let description = columns.get(4..).map(|d| d.join(" ")).unwrap_or_default();
            out.push(Patch {
                id: id.to_string(),
                state: PatchState::parse(columns[1]),
                detail: detail.to_string(),
                description: description.trim().to_string(),
            });
            continue;
        }
        let mut words = rest.splitn(2, ['\t', ' ']);
        let state = words.next().unwrap_or("").trim_end_matches([':', ',', ';']);
        let detail = words.next().unwrap_or("").trim().trim_start_matches(['-', ':']).trim();
        out.push(Patch {
            id: id.to_string(),
            state: PatchState::parse(state),
            detail: detail.to_string(),
            description: String::new(),
        });
    }
    out
}

fn parse_json_report(text: &str) -> Vec<Patch> {
    use serde_json::Value;
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let entry = |o: &serde_json::Map<String, Value>| -> Option<Patch> {
        let id = o.get("id").or_else(|| o.get("name"))?.as_str()?.to_string();
        let state = ["status", "state", "applied", "on"].iter().find_map(|k| o.get(*k)).map(|s| match s {
            Value::Bool(b) => b.to_string(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        });
        let detail = ["detail", "reason", "note", "message"].iter().find_map(|k| o.get(*k)?.as_str()).unwrap_or("");
        Some(Patch {
            id,
            state: PatchState::parse(state.as_deref().unwrap_or("")),
            detail: detail.to_string(),
            description: o.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
        })
    };
    let list = match &v {
        Value::Array(a) => Some(a),
        Value::Object(o) => o.get("patches").or_else(|| o.get("changes")).and_then(Value::as_array),
        _ => None,
    };
    if let Some(list) = list {
        return list.iter().filter_map(Value::as_object).filter_map(entry).collect();
    }
    match &v {
        Value::Object(o) => o
            .iter()
            .map(|(id, s)| {
                let state = match s {
                    Value::Bool(b) => b.to_string(),
                    Value::String(s) => s.clone(),
                    Value::Object(inner) => {
                        inner.get("status").or_else(|| inner.get("state")).and_then(Value::as_str).unwrap_or("").into()
                    }
                    _ => String::new(),
                };
                Patch {
                    id: id.clone(),
                    state: PatchState::parse(&state),
                    detail: String::new(),
                    description: String::new(),
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The report written last (by any instance), with the instance key it belongs to and when it was written.
pub fn latest_report(dirs: &Dirs) -> Option<(String, std::time::SystemTime, Vec<Patch>)> {
    let rd = std::fs::read_dir(folder(dirs)).ok()?;
    let newest = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "report"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(t, _)| *t)?;
    let text = std::fs::read_to_string(&newest.1).ok()?;
    let key = newest.1.file_stem()?.to_string_lossy().to_string();
    Some((key, newest.0, parse_report(&text)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instances::InstanceFile;

    fn lines(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    fn instance(name: &str, root: &Path, launcher: &str) -> Instance {
        let mut file = InstanceFile::default();
        file.launcher = serde_json::from_str(&format!(r#"{{"name":"{name}"{launcher}}}"#)).unwrap();
        Instance { folder: "Folder1".into(), mod_count: 0, root: root.to_path_buf(), file }
    }

    #[test]
    fn the_agent_is_embedded_and_is_a_jar() {
        assert!(AGENT_JAR.starts_with(b"PK"));
        assert!(AGENT_JAR.len() > 1000);
    }

    #[test]
    fn the_instance_switch_wins_over_the_setting() {
        let mut s = Settings::default();
        assert!(s.lite, "on by default");
        let plain = instance("A", Path::new("/x"), "");
        let off = instance("A", Path::new("/x"), r#","lite":false"#);
        let on = instance("A", Path::new("/x"), r#","lite":true"#);
        assert!(enabled(&s, &plain) && !enabled(&s, &off));
        s.lite = false;
        assert!(!enabled(&s, &plain) && enabled(&s, &on));
        assert_eq!(instance_override(&plain), None);
        assert_eq!(instance_override(&off), Some(false));
    }

    #[test]
    fn instance_keys_are_plain_and_never_empty() {
        assert_eq!(instance_key(&instance("My Pack 2", Path::new("/x"), "")), "MyPack2");
        // a name without plain letters falls back to the folder
        assert_eq!(instance_key(&instance("Мой мир", Path::new("/x"), "")), "Folder1");
    }

    #[test]
    fn options_carry_the_report_mods_and_switched_off_changes() {
        let o = options(Path::new("/c/lite/A.report"), Some(Path::new("/c/lite/A.mods")), " fog ; leaves,", false);
        assert_eq!(o, "quiet,report=/c/lite/A.report,mods=/c/lite/A.mods,off=fog;leaves");
        assert_eq!(options(Path::new("/c/lite/A.report"), None, "", false), "quiet,report=/c/lite/A.report");
        // the agent splits at commas, so paths with one are left out
        assert_eq!(options(Path::new("/a,b/A.report"), Some(Path::new("/a,b/A.mods")), "", false), "quiet");
        assert_eq!(
            java_argument(Path::new("/c/blockforge-lite.jar"), "quiet"),
            "-javaagent:/c/blockforge-lite.jar=quiet"
        );
        assert_eq!(off_list(""), "");
    }

    #[test]
    fn the_balanced_level_is_asked_for_only_while_low_end_mode_is_on() {
        let r = Path::new("/c/lite/A.report");
        let normal = options(r, None, "", false);
        assert!(!normal.contains("level="), "{normal}");
        let balanced = options(r, None, "fog", true);
        assert_eq!(balanced, "quiet,level=2,report=/c/lite/A.report,off=fog;hidden-entities");
        // the one change kept off is not listed twice
        assert_eq!(
            options(r, None, "hidden-entities", true),
            "quiet,level=2,report=/c/lite/A.report,off=hidden-entities"
        );
    }

    #[test]
    fn it_goes_after_fast_start_and_never_twice() {
        let boost = "-javaagent:/c/blockforge-boost.jar=preload=/x".to_string();
        assert_eq!(insert_position(&[boost.clone(), "-Xmx2G".into()]), 1);
        assert_eq!(insert_position(&["-Xmx2G".into()]), 0);
        assert!(!already_there(&[boost]));
        assert!(already_there(&["-javaagent:/c/blockforge-lite.jar=quiet".into()]));
        // the player's own copy, on Windows, without options
        assert!(already_there(&["-javaagent:C:\\Games\\BlockForge-Lite.jar".into()]));
        assert!(!already_there(&["-javaagent:/x/other-lite.jar".into()]));
    }

    #[test]
    fn prepare_writes_the_jar_and_the_mod_list() {
        let base = std::env::temp_dir().join(format!("bf-lite-{}", std::process::id()));
        let game = base.join("game");
        std::fs::create_dir_all(game.join("mods")).unwrap();
        for f in ["sodium.jar", "lithium.JAR", "old.jar.disabled", "notes.txt"] {
            std::fs::write(game.join("mods").join(f), b"x").unwrap();
        }
        let dirs = Dirs::at(base.join("data"));
        let inst = instance("Pack", &game, "");
        let (added, notes) = prepare(&dirs, &inst, "a;b", false, &["-Xmx2G".into()]).unwrap();
        assert!(notes.is_empty());
        let Added::Argument(at, arg) = added else { panic!("not added") };
        assert_eq!(at, 0);
        assert!(arg.starts_with(&format!("-javaagent:{}=quiet,report=", jar_path(&dirs).display())));
        assert!(arg.ends_with(&format!("mods={},off=a;b", mods_path(&dirs, "Pack").display())));
        assert_eq!(std::fs::read(jar_path(&dirs)).unwrap(), AGENT_JAR);
        assert_eq!(std::fs::read_to_string(mods_path(&dirs, "Pack")).unwrap(), "lithium.JAR\nsodium.jar\n");
        // real jars also name their ids
        let jar = |name: &str, files: &[(&str, &str)]| {
            use std::io::Write;
            let mut z = zip::ZipWriter::new(std::fs::File::create(game.join("mods").join(name)).unwrap());
            for (n, t) in files {
                z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
                z.write_all(t.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        };
        jar("sodium.jar", &[("fabric.mod.json", r#"{"id":"sodium","provides":["indium"]}"#)]);
        let toml =
            "modLoader=\"javafml\"\n[[mods]]\nmodId = \"embeddium\"\n[[dependencies.embeddium]]\nmodId=\"minecraft\"\n";
        jar("emb.jar", &[("META-INF/mods.toml", toml)]);
        jar("of.jar", &[("optifine/OptiFineClassTransformer.class", "x")]);
        jar("q.jar", &[("quilt.mod.json", r#"{"quilt_loader":{"id":"qsl"}}"#)]);
        assert_eq!(
            mod_list(&game),
            [
                "emb.jar",
                "embeddium",
                "lithium.JAR",
                "of.jar",
                "optifine",
                "q.jar",
                "qsl",
                "sodium.jar",
                "sodium",
                "indium"
            ]
        );
        // the player starts it already
        let own = vec!["-javaagent:/elsewhere/blockforge-lite.jar".to_string()];
        assert_eq!(prepare(&dirs, &inst, "", false, &own).unwrap().0, Added::PlayersOwn);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn agent_paths_java_refuses_are_left_out() {
        assert!(usable_jar_path(Path::new("/home/a/.local/share/blockforge/cache/blockforge-lite.jar")));
        assert!(!usable_jar_path(Path::new("/home/a=b/cache/blockforge-lite.jar")));
        assert!(!usable_jar_path(Path::new("C:\\Users\\a,b\\cache\\blockforge-lite.jar")));
        let base = std::env::temp_dir().join(format!("bf-lite-eq={}", std::process::id()));
        let dirs = Dirs::at(&base);
        let inst = instance("Pack", &base, "");
        assert!(matches!(prepare(&dirs, &inst, "", false, &[]).unwrap().0, Added::UnusablePath(_)));
        assert!(!jar_path(&dirs).exists(), "nothing is written for it");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn runtime_names_follow_the_version_and_loader() {
        use RuntimeNames::*;
        assert_eq!(runtime_names("26.3", None), Mojang);
        assert_eq!(runtime_names("26.3", Some("Fabric")), Mojang);
        assert_eq!(runtime_names("1.21.1", Some("NeoForge")), Mojang);
        assert_eq!(runtime_names("1.20.6", Some("NeoForge")), Mojang);
        assert_eq!(runtime_names("1.20.1", Some("NeoForge")), Srg);
        assert_eq!(runtime_names("1.20.1", Some("Forge")), Srg);
        assert_eq!(runtime_names("1.12.2", Some("Forge")), Srg);
        assert_eq!(runtime_names("1.21.1", Some("Quilt")), Intermediary);
        assert_eq!(runtime_names("1.21.11", Some("Fabric")), Intermediary);
        assert_eq!(runtime_names("1.21.1", None), Obfuscated);
        assert_eq!(runtime_names("1.8.9", None), Obfuscated);
    }

    #[test]
    fn early_failures_blame_either_helper() {
        let quiet = lines(&["[main/INFO]: Loading Minecraft"]);
        let short = Duration::from_secs(6);
        let long = Duration::from_secs(600);
        assert!(may_be_the_cause(false, true, Some(1), short, &quiet));
        assert!(may_be_the_cause(true, false, Some(1), short, &quiet));
        assert!(!may_be_the_cause(false, false, Some(1), short, &quiet));
        assert!(!may_be_the_cause(true, true, Some(0), short, &quiet));
        assert!(!may_be_the_cause(true, true, None, short, &quiet));
        assert!(!may_be_the_cause(false, true, Some(1), long, &quiet));
        let problem = lines(&["[BlockForge Lite] problem: patching fog: java.lang.VerifyError"]);
        assert!(may_be_the_cause(false, true, Some(1), long, &problem));
        assert!(!may_be_the_cause(false, true, Some(0), long, &problem));
        let jvm = lines(&["Error opening zip file or JAR manifest missing : /x/blockforge-lite.jar"]);
        assert!(may_be_the_cause(false, true, None, long, &jvm));
    }

    #[test]
    fn two_early_failures_in_a_row_turn_it_off() {
        let base = std::env::temp_dir().join(format!("bf-lite-fail-{}", std::process::id()));
        let dirs = Dirs::at(&base);
        assert_eq!(record_start(&dirs, "A", true), 1);
        assert!(!should_turn_off(1));
        // a good start in between starts the count again
        assert_eq!(record_start(&dirs, "A", false), 0);
        assert_eq!(record_start(&dirs, "A", true), 1);
        assert_eq!(record_start(&dirs, "B", true), 1, "counted per instance");
        assert_eq!(record_start(&dirs, "A", true), 2);
        assert!(should_turn_off(2));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn reports_are_read_in_any_reasonable_form() {
        let agent = "# BlockForge Lite report. Columns: id, applied/skipped/failed, class, reason\n# 812 us\n\
            idle-fps\tapplied\tnet/minecraft/client/Minecraft\t\n\
            no-world-gc\tskipped\t\tnot this version (1.13 and newer don't have the calls)\n\
            other\tfailed\tfoo/Bar\tjava.lang.Error: x\n";
        let r = parse_report(agent);
        assert_eq!(r.len(), 3);
        assert_eq!((r[0].id.as_str(), r[0].state, r[0].detail.as_str()), ("idle-fps", PatchState::On, ""));
        assert_eq!(
            (r[1].state, r[1].detail.as_str()),
            (PatchState::Off, "not this version (1.13 and newer don't have the calls)")
        );
        assert_eq!((r[2].state, r[2].detail.as_str()), (PatchState::Problem, "java.lang.Error: x"));
        assert!(r.iter().all(|p| p.description.is_empty()));
        // with the 5th column: what the change is, in plain words
        let five = "worker-priority\tapplied\tnet/minecraft/Util\t\tworld generation gives way to the game\n\
                    short-fade\tskipped\t-\tno matching code was loaded\tthe loading screen goes away sooner\n";
        let r = parse_report(five);
        assert_eq!((r[0].id.as_str(), r[0].detail.as_str()), ("worker-priority", ""));
        assert_eq!(r[0].description, "world generation gives way to the game");
        assert_eq!(
            (r[1].state, r[1].detail.as_str(), r[1].description.as_str()),
            (PatchState::Off, "no matching code was loaded", "the loading screen goes away sooner")
        );
        let text = "# BlockForge Lite 1.0\nfog\ton\tthinner fog\nleaves off (Sodium does it)\nlight=failed: java.lang.Error\n\nsky: skipped\n";
        let r = parse_report(text);
        assert_eq!(r.len(), 4);
        assert_eq!((r[0].id.as_str(), r[0].state, r[0].detail.as_str()), ("fog", PatchState::On, "thinner fog"));
        assert_eq!((r[1].id.as_str(), r[1].state), ("leaves", PatchState::Off));
        assert_eq!(r[1].detail, "(Sodium does it)");
        assert_eq!((r[2].id.as_str(), r[2].state), ("light", PatchState::Problem));
        assert_eq!(r[2].detail, "java.lang.Error");
        assert_eq!(r[3].state, PatchState::Off);
        let json =
            parse_report(r#"{"patches":[{"id":"fog","status":"applied","reason":"x"},{"id":"sky","applied":false}]}"#);
        assert_eq!(json.len(), 2);
        assert_eq!((json[0].state, json[0].detail.as_str()), (PatchState::On, "x"));
        assert_eq!(json[1].state, PatchState::Off);
        let map = parse_report(r#"{"fog":"on","sky":"weird"}"#);
        assert_eq!(map.iter().find(|p| p.id == "sky").unwrap().state, PatchState::Other);
        assert!(parse_report("# placeholder: no patches\n").is_empty());
        assert!(parse_report("").is_empty());
    }
}
