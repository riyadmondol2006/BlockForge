//! Working out what a file, folder or web address is: a modpack in one of the formats BlockForge can import, an
//! instance folder from another launcher, a mod, a pack of resources or shaders, a world. The Java edition's
//! `DroppedFileDetector` (what a dropped file is) and `ImportPackUtils` (which importer to call) both live here.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

/// What a dropped file turned out to be.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileKind {
    Mod,
    ResourcePack,
    DataPack,
    ShaderPack,
    ModrinthModpack,
    CurseForgeModpack,
    /// A zip exported from Prism Launcher or MultiMC.
    PrismExport,
    Unknown,
}

impl FileKind {
    /// Modpacks become instances of their own instead of going into one.
    pub fn is_modpack(self) -> bool {
        matches!(self, FileKind::ModrinthModpack | FileKind::CurseForgeModpack | FileKind::PrismExport)
    }

    /// Whether it goes into an instance (a data pack goes into its worlds).
    pub fn goes_into_instance(self) -> bool {
        matches!(self, FileKind::Mod | FileKind::ResourcePack | FileKind::DataPack | FileKind::ShaderPack)
    }
}

/// The loader a mod is made for, lowercase: `fabric`, `quilt`, `forge`, `neoforge`, `liteloader`.
#[derive(Clone, Debug)]
pub struct DroppedFile {
    pub path: PathBuf,
    pub file_name: String,
    pub kind: FileKind,
    pub loader: Option<String>,
    /// The name inside the file, if it has one.
    pub name: Option<String>,
    pub version: Option<String>,
    /// Minecraft version requirements as the mod writes them; any one is enough.
    pub minecraft_versions: Vec<String>,
}

impl DroppedFile {
    fn new(path: &Path, file_name: &str, kind: FileKind) -> DroppedFile {
        DroppedFile {
            path: path.to_path_buf(),
            file_name: file_name.to_string(),
            kind,
            loader: None,
            name: None,
            version: None,
            minecraft_versions: Vec::new(),
        }
    }

    /// The best name to show for it.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(&self.file_name)
    }
}

/// Entries that are listed at most (a zip with millions of files isn't a pack).
const MAX_ENTRIES: usize = 200_000;
/// Metadata files bigger than this are not read.
const MAX_TEXT: u64 = 512 * 1024;

/// Reads a file from disk and tells what it is. Files that can't be read are `Unknown`.
pub fn detect_file(path: &Path) -> DroppedFile {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let lower = name.to_lowercase();
    if !(lower.ends_with(".jar")
        || lower.ends_with(".zip")
        || lower.ends_with(".mrpack")
        || lower.ends_with(".litemod"))
    {
        return DroppedFile::new(path, &name, FileKind::Unknown);
    }
    let Ok(file) = std::fs::File::open(path) else { return DroppedFile::new(path, &name, FileKind::Unknown) };
    let Ok(mut archive) = zip::ZipArchive::new(file) else { return DroppedFile::new(path, &name, FileKind::Unknown) };
    let entries = entry_names(&mut archive);

    classify(path, &name, &entries, &mut |entry: &str| read_entry(&mut archive, entry))
}

fn entry_names<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>) -> Vec<String> {
    (0..archive.len().min(MAX_ENTRIES))
        .filter_map(|i| archive.name_for_index(i).and_then(|n| n.ok()).map(|n| n.to_string()))
        .collect()
}

/// The text of one entry, or `None` when it isn't there, is too big or can't be read.
pub fn read_entry<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let entry = archive.by_name(name).ok()?;
    if entry.size() > MAX_TEXT {
        return None;
    }
    let mut text = Vec::new();
    entry.take(MAX_TEXT).read_to_end(&mut text).ok()?;
    Some(String::from_utf8_lossy(&text).to_string())
}

/// Tells what a file is from its name and the names inside it. `read` gives the text of an entry.
pub fn classify(
    path: &Path,
    file_name: &str,
    raw_entries: &[String],
    read: &mut dyn FnMut(&str) -> Option<String>,
) -> DroppedFile {
    let lower = file_name.to_lowercase();
    let entries: HashSet<String> = raw_entries.iter().map(|e| e.replace('\\', "/")).collect();
    let has = |n: &str| entries.contains(n);

    if lower.ends_with(".mrpack") || has("modrinth.index.json") {
        let json = parse_json(read("modrinth.index.json"));
        let mut f = DroppedFile::new(path, file_name, FileKind::ModrinthModpack);
        f.name = string(&json, "name");
        f.version = string(&json, "versionId");
        return f;
    }
    if lower.ends_with(".litemod") {
        let json = parse_json(read("litemod.json"));
        let mut f = DroppedFile::new(path, file_name, FileKind::Mod);
        f.loader = Some("liteloader".into());
        f.name = string(&json, "name");
        f.version = string(&json, "version");
        f.minecraft_versions = string(&json, "mcversion").into_iter().collect();
        return f;
    }
    if at_root_or_one_down(&entries, "mmc-pack.json") || at_root_or_one_down(&entries, "instance.cfg") {
        return DroppedFile::new(path, file_name, FileKind::PrismExport);
    }
    if has("manifest.json") {
        let json = parse_json(read("manifest.json"));
        if let Some(mc) = json.as_ref().and_then(|j| j.get("minecraft")).filter(|m| m.is_object()) {
            let mut f = DroppedFile::new(path, file_name, FileKind::CurseForgeModpack);
            f.name = string(&json, "name");
            f.version = string(&json, "version");
            f.minecraft_versions = mc.get("version").and_then(Value::as_str).map(str::to_string).into_iter().collect();
            return f;
        }
    }

    if let Some(m) = mod_file(path, file_name, &entries, read) {
        return m;
    }
    if lower.ends_with(".jar") {
        // a jar without any loader's metadata: probably a library or a very old mod
        return DroppedFile::new(path, file_name, FileKind::Mod);
    }

    let mut assets = false;
    let mut data = false;
    let mut shaders = false;
    for entry in &entries {
        assets |= entry.starts_with("assets/");
        data |= entry.starts_with("data/");
        let parts: Vec<&str> = entry.split('/').collect();
        if (parts.len() >= 2 && parts[0] == "shaders")
            || (parts.len() >= 3 && parts[1] == "shaders")
            || entry == "shaders/"
        {
            shaders = true;
        }
    }
    if shaders && !assets {
        return DroppedFile::new(path, file_name, FileKind::ShaderPack);
    }
    if has("pack.mcmeta") {
        let description = parse_json(read("pack.mcmeta"))
            .and_then(|j| j.get("pack").and_then(|p| p.get("description")).and_then(Value::as_str).map(str::to_string));
        let kind = if data && !assets { FileKind::DataPack } else { FileKind::ResourcePack };
        let mut f = DroppedFile::new(path, file_name, kind);
        f.version = description;
        return f;
    }
    DroppedFile::new(path, file_name, FileKind::Unknown)
}

fn at_root_or_one_down(entries: &HashSet<String>, name: &str) -> bool {
    entries.iter().any(|e| {
        if e == name {
            return true;
        }
        let parts: Vec<&str> = e.split('/').collect();
        parts.len() == 2 && parts[1] == name
    })
}

/// Reads whichever loader's metadata a mod has; `None` if it has none.
fn mod_file(
    path: &Path,
    file_name: &str,
    entries: &HashSet<String>,
    read: &mut dyn FnMut(&str) -> Option<String>,
) -> Option<DroppedFile> {
    let has = |n: &str| entries.contains(n);
    if has("quilt.mod.json") {
        let json = parse_json(read("quilt.mod.json"));
        let loader = json.as_ref().and_then(|j| j.get("quilt_loader")).filter(|l| l.is_object());
        let meta = loader.and_then(|l| l.get("metadata")).filter(|m| m.is_object());
        let mut mc = Vec::new();
        if let Some(deps) = loader.and_then(|l| l.get("depends")).and_then(Value::as_array) {
            for dep in deps.iter().filter(|d| d.is_object()) {
                if dep.get("id").and_then(Value::as_str) == Some("minecraft") {
                    mc.extend(versions(dep.get("versions")));
                }
            }
        }
        let mut f = DroppedFile::new(path, file_name, FileKind::Mod);
        f.loader = Some("quilt".into());
        f.name = meta.and_then(|m| m.get("name")).and_then(Value::as_str).map(str::to_string);
        f.version = loader.and_then(|l| l.get("version")).and_then(Value::as_str).and_then(clean);
        f.minecraft_versions = mc;
        return Some(f);
    }
    if has("fabric.mod.json") {
        let json = parse_json(read("fabric.mod.json"));
        let mut f = DroppedFile::new(path, file_name, FileKind::Mod);
        f.loader = Some("fabric".into());
        f.name = string(&json, "name");
        f.version = string(&json, "version").and_then(|v| clean(&v));
        if let Some(deps) = json.as_ref().and_then(|j| j.get("depends")).filter(|d| d.is_object()) {
            f.minecraft_versions = versions(deps.get("minecraft"));
        }
        return Some(f);
    }
    if has("META-INF/neoforge.mods.toml") {
        return Some(toml_mod(path, file_name, read("META-INF/neoforge.mods.toml").as_deref(), "neoforge"));
    }
    if has("META-INF/mods.toml") {
        return Some(toml_mod(path, file_name, read("META-INF/mods.toml").as_deref(), "forge"));
    }
    if has("mcmod.info") {
        let root = parse_json(read("mcmod.info"));
        let first = match &root {
            Some(Value::Array(a)) => a.first().cloned(),
            Some(o @ Value::Object(_)) => o.get("modList").and_then(Value::as_array).and_then(|l| l.first().cloned()),
            _ => None,
        };
        let first = first.filter(Value::is_object);
        let mut f = DroppedFile::new(path, file_name, FileKind::Mod);
        f.loader = Some("forge".into());
        f.name = string(&first, "name");
        f.version = string(&first, "version").and_then(|v| clean(&v));
        f.minecraft_versions = string(&first, "mcversion").and_then(|v| clean(&v)).into_iter().collect();
        return Some(f);
    }
    None
}

/// Forge and NeoForge describe mods in TOML. Only the few keys needed here are read: the display name, the
/// version and the Minecraft dependency. A Forge-style file that depends on NeoForge is a NeoForge mod.
pub fn toml_mod(path: &Path, file_name: &str, text: Option<&str>, loader: &str) -> DroppedFile {
    let mut table = String::new();
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut dep_mod: Option<String> = None;
    let mut dep_range: Option<String> = None;
    let mut mc = Vec::new();
    let mut depends_on_neoforge = false;
    let flush = |dep_mod: &Option<String>, dep_range: &Option<String>, mc: &mut Vec<String>, neo: &mut bool| {
        if dep_mod.as_deref() == Some("minecraft") {
            if let Some(r) = dep_range {
                mc.push(r.clone());
            }
        }
        if dep_mod.as_deref() == Some("neoforge") {
            *neo = true;
        }
    };
    for raw in text.unwrap_or("").lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            flush(&dep_mod, &dep_range, &mut mc, &mut depends_on_neoforge);
            dep_mod = None;
            dep_range = None;
            table = line.replace(['[', ']'], "").trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), unquote(value.trim()));
        if table == "mods" {
            if key == "displayName" && name.is_none() {
                name = Some(value);
            } else if key == "version" && version.is_none() {
                version = clean(&value);
            }
        } else if table.starts_with("dependencies") {
            if key == "modId" {
                dep_mod = Some(value);
            } else if key == "versionRange" {
                dep_range = Some(value);
            }
        }
    }
    flush(&dep_mod, &dep_range, &mut mc, &mut depends_on_neoforge);
    let mut f = DroppedFile::new(path, file_name, FileKind::Mod);
    f.loader = Some(if depends_on_neoforge { "neoforge" } else { loader }.to_string());
    f.name = name;
    f.version = version;
    f.minecraft_versions = mc;
    f
}

fn unquote(value: &str) -> String {
    let mut v = value.to_string();
    if !v.starts_with('"') && !v.starts_with('\'') {
        if let Some(i) = v.find(" #") {
            v.truncate(i);
            v = v.trim().to_string();
        }
    }
    if v.len() >= 2 && (v.starts_with('"') || v.starts_with('\'')) {
        let q = v.chars().next().unwrap_or('"');
        return match v[1..].find(q) {
            Some(end) => v[1..1 + end].to_string(),
            None => v[1..].to_string(),
        };
    }
    v
}

/// Placeholders like `${version}` are filled in when a mod is built; one left in means the value is unknown.
fn clean(value: &str) -> Option<String> {
    if value.contains("${") || value.trim().is_empty() { None } else { Some(value.trim().to_string()) }
}

fn versions(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    }
}

fn string(json: &Option<Value>, key: &str) -> Option<String> {
    match json.as_ref()?.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Parses JSON the way mod metadata is written in the wild: with a byte order mark, trailing commas and `//` lines.
pub fn parse_json(text: Option<String>) -> Option<Value> {
    let text = text?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if let Ok(v) = serde_json::from_str(text) {
        return Some(v);
    }
    serde_json::from_str(&strip_loose_json(text)).ok()
}

fn strip_loose_json(text: &str) -> String {
    // drop `//` comment lines and trailing commas outside strings
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == ',' {
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if !matches!(next, Some(']') | Some('}')) {
                out.push(c);
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// The name, version and description a mod file declares about itself (`mcmod.info` or `fabric.mod.json`), which
/// instance records show. Missing pieces are `None`.
pub fn mod_metadata(path: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let Ok(file) = std::fs::File::open(path) else { return (None, None, None) };
    let Ok(mut archive) = zip::ZipArchive::new(file) else { return (None, None, None) };
    if let Some(info) = parse_json(read_entry(&mut archive, "mcmod.info")) {
        let first = match &info {
            Value::Array(a) => a.first().cloned(),
            other => other.get("modList").and_then(Value::as_array).and_then(|l| l.first().cloned()),
        };
        if let Some(m) = first.filter(Value::is_object) {
            let f = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
            return (f("name"), f("version"), f("description"));
        }
    }
    if let Some(m) = parse_json(read_entry(&mut archive, "fabric.mod.json")) {
        let f = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
        return (f("name"), f("version"), f("description"));
    }
    (None, None, None)
}

// ---------------------------------------------------------------------------------------------------------
// Which importer to use

/// What an import source is, from the importer's point of view.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Source {
    /// A `.mrpack`, or a zip with `modrinth.index.json`.
    Modrinth(PathBuf),
    /// A zip with a CurseForge `manifest.json`. Handled by the CurseForge package.
    CurseForge(PathBuf),
    /// A Prism / MultiMC export: a zip with `mmc-pack.json` at the top or one folder down.
    MultiMcArchive(PathBuf),
    /// A folder from another launcher (Prism, MultiMC, CurseForge, GDLauncher, ATLauncher).
    LauncherFolder(PathBuf),
    /// A zip holding a Minecraft world (`level.dat`). BlockForge adds worlds to an existing instance; it does
    /// not make an instance from one.
    World(PathBuf),
    /// A plain `.minecraft`-style folder (has `saves/`, `mods/` or `options.txt`). Same note as for worlds.
    MinecraftFolder(PathBuf),
    Unknown,
}

/// Look at a file or folder and say what to import it as. Archives are checked in the Java edition's order:
/// MultiMC/Prism, CurseForge, Modrinth.
pub fn detect_path(path: &Path) -> Source {
    if path.is_dir() {
        if super::detect_folder(path).is_some() {
            return Source::LauncherFolder(path.to_path_buf());
        }
        if ["saves", "mods", "resourcepacks"].iter().any(|d| path.join(d).is_dir())
            || path.join("options.txt").is_file()
        {
            return Source::MinecraftFolder(path.to_path_buf());
        }
        return Source::Unknown;
    }
    let Ok(file) = std::fs::File::open(path) else { return Source::Unknown };
    let Ok(mut archive) = zip::ZipArchive::new(file) else { return Source::Unknown };
    let entries: HashSet<String> = entry_names(&mut archive).into_iter().map(|e| e.replace('\\', "/")).collect();
    if entries.contains("mmc-pack.json") {
        return Source::MultiMcArchive(path.to_path_buf());
    }
    if single_top_folder(&entries).is_some_and(|top| entries.contains(&format!("{top}/mmc-pack.json"))) {
        return Source::MultiMcArchive(path.to_path_buf());
    }
    if entries.contains("manifest.json") {
        return Source::CurseForge(path.to_path_buf());
    }
    if entries.contains("modrinth.index.json") {
        return Source::Modrinth(path.to_path_buf());
    }
    if entries.contains("level.dat")
        || single_top_folder(&entries).is_some_and(|top| entries.contains(&format!("{top}/level.dat")))
    {
        return Source::World(path.to_path_buf());
    }
    Source::Unknown
}

/// The one folder everything in an archive sits in, if there is exactly one.
pub fn single_top_folder(entries: &HashSet<String>) -> Option<String> {
    let mut tops = HashSet::new();
    for e in entries {
        let first = e.split('/').next().unwrap_or("");
        if first.is_empty() {
            continue;
        }
        // a file at the top level means there is no single folder
        if !e.contains('/') {
            return None;
        }
        tops.insert(first.to_string());
    }
    if tops.len() == 1 { tops.into_iter().next() } else { None }
}

/// A web address, understood. See [`detect_url`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UrlSource {
    /// A CurseForge modpack page: `https://www.curseforge.com/minecraft/modpacks/<slug>[/files/<id>]`.
    CurseForge { slug: String, file_id: Option<i64> },
    /// A Modrinth modpack page: `https://modrinth.com/modpack/<slug>`.
    Modrinth { slug: String },
    /// A pack page on feed-the-beast.com: `https://feed-the-beast.com/modpacks/<id>-<slug>`.
    Ftb { pack_id: i64 },
    /// Anything else is downloaded and its contents decide.
    File { url: String, extension: &'static str },
}

pub fn detect_url(url: &str) -> Option<UrlSource> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }
    let rest = url.split_once("://")?.1;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.trim_start_matches("www.").to_lowercase();
    let path = path.split(['?', '#']).next().unwrap_or("");
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();

    if host == "curseforge.com" || host == "legacy.curseforge.com" {
        if parts.len() >= 3 && parts[0] == "minecraft" && parts[1] == "modpacks" {
            let slug = parts[2];
            if !slug.is_empty() && slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                let file_id = if parts.len() >= 5 && matches!(parts[3], "files" | "download") {
                    parts[4].parse().ok()
                } else {
                    None
                };
                return Some(UrlSource::CurseForge { slug: slug.to_string(), file_id });
            }
        }
    } else if host == "modrinth.com" {
        if parts.len() >= 2
            && parts[0] == "modpack"
            && parts[1].chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Some(UrlSource::Modrinth { slug: parts[1].to_string() });
        }
    } else if host == "feed-the-beast.com" && parts.len() >= 2 && parts[0] == "modpacks" {
        let id: String = parts[1].chars().take_while(char::is_ascii_digit).collect();
        if let Ok(pack_id) = id.parse() {
            return Some(UrlSource::Ftb { pack_id });
        }
    }
    let extension = if path.to_lowercase().ends_with(".mrpack") { "mrpack" } else { "zip" };
    Some(UrlSource::File { url: url.to_string(), extension })
}

/// The files and folders in a list of dropped paths that can be imported, in order, with what each is.
pub fn dropped_sources(paths: &[PathBuf]) -> Vec<(PathBuf, Source)> {
    paths.iter().map(|p| (p.clone(), detect_path(p))).collect()
}

/// Read one entry of an archive that must exist.
pub fn require_entry(archive_path: &Path, name: &str) -> Result<String> {
    let file = std::fs::File::open(archive_path).with_context(|| format!("opening {}", archive_path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).with_context(|| format!("{} isn't a zip file", archive_path.display()))?;
    let mut entry = archive.by_name(name).with_context(|| format!("{} isn't in {}", name, archive_path.display()))?;
    let mut text = String::new();
    entry.read_to_string(&mut text)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Writes a zip with the given entries; an entry whose name ends in `/` is a folder.
    fn zip_at(dir: &Path, name: &str, entries: &[(&str, &str)]) -> PathBuf {
        let path = dir.join(name);
        let mut z = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for (n, content) in entries {
            if n.ends_with('/') {
                z.add_directory(*n, opts).unwrap();
            } else {
                z.start_file(*n, opts).unwrap();
                z.write_all(content.as_bytes()).unwrap();
            }
        }
        z.finish().unwrap();
        path
    }

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bf-detect-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn fabric_mod_with_name_version_and_minecraft_range() {
        let d = temp();
        let f = detect_file(&zip_at(
            &d,
            "sodium.jar",
            &[
                (
                    "fabric.mod.json",
                    r#"{"schemaVersion":1,"id":"sodium","name":"Sodium","version":"0.9.2","depends":{"minecraft":">=26.3 <26.4","fabricloader":">=0.16"}}"#,
                ),
                ("net/caffeinemc/Sodium.class", ""),
            ],
        ));
        assert_eq!(f.kind, FileKind::Mod);
        assert_eq!(f.loader.as_deref(), Some("fabric"));
        assert_eq!(f.name.as_deref(), Some("Sodium"));
        assert_eq!(f.version.as_deref(), Some("0.9.2"));
        assert_eq!(f.minecraft_versions, vec![">=26.3 <26.4"]);
    }

    #[test]
    fn fabric_version_lists_and_placeholders() {
        let d = temp();
        let f = detect_file(&zip_at(
            &d,
            "multi.jar",
            &[(
                "fabric.mod.json",
                r#"{"name":"Multi","version":"${version}","depends":{"minecraft":["1.20.1","1.20.4"]}}"#,
            )],
        ));
        assert_eq!(f.version, None);
        assert_eq!(f.minecraft_versions, vec!["1.20.1", "1.20.4"]);
    }

    #[test]
    fn quilt_mod() {
        let d = temp();
        let f = detect_file(&zip_at(
            &d,
            "q.jar",
            &[(
                "quilt.mod.json",
                r#"{"quilt_loader":{"id":"q","version":"2.0","metadata":{"name":"Quilty"},"depends":["quilt_loader",{"id":"minecraft","versions":">=1.20"}]}}"#,
            )],
        ));
        assert_eq!(f.loader.as_deref(), Some("quilt"));
        assert_eq!((f.name.as_deref(), f.version.as_deref()), (Some("Quilty"), Some("2.0")));
        assert_eq!(f.minecraft_versions, vec![">=1.20"]);
    }

    #[test]
    fn neoforge_mod_from_its_own_toml() {
        let d = temp();
        let toml = "modLoader=\"javafml\"\nloaderVersion=\"[4,)\"\n[[mods]]\nmodId=\"neo\"\nversion=\"1.4.0\" # comment\ndisplayName=\"Neo Thing\"\n[[dependencies.neo]]\nmodId=\"neoforge\"\ntype=\"required\"\nversionRange=\"[21.1,)\"\n[[dependencies.neo]]\nmodId=\"minecraft\"\ntype=\"required\"\nversionRange=\"[1.21.1,1.22)\"\n";
        let f = detect_file(&zip_at(&d, "neo.jar", &[("META-INF/neoforge.mods.toml", toml)]));
        assert_eq!(f.loader.as_deref(), Some("neoforge"));
        assert_eq!((f.name.as_deref(), f.version.as_deref()), (Some("Neo Thing"), Some("1.4.0")));
        assert_eq!(f.minecraft_versions, vec!["[1.21.1,1.22)"]);
    }

    #[test]
    fn forge_mods_toml_and_neoforge_ones_that_still_use_it() {
        let d = temp();
        let forge = "[[mods]]\nmodId='jei'\nversion='${file.jarVersion}'\ndisplayName='Just Enough Items'\n[[dependencies.jei]]\nmodId='forge'\nversionRange='[47,)'\n[[dependencies.jei]]\nmodId='minecraft'\nversionRange='[1.20.1]'\n";
        let f = detect_file(&zip_at(&d, "jei.jar", &[("META-INF/mods.toml", forge)]));
        assert_eq!(f.loader.as_deref(), Some("forge"));
        assert_eq!(f.name.as_deref(), Some("Just Enough Items"));
        assert_eq!(f.version, None);
        assert_eq!(f.minecraft_versions, vec!["[1.20.1]"]);
        let old_neo = "[[mods]]\nmodId=\"x\"\n[[dependencies.x]]\nmodId=\"neoforge\"\nversionRange=\"[20.4,)\"\n";
        let n = detect_file(&zip_at(&d, "oldneo.jar", &[("META-INF/mods.toml", old_neo)]));
        assert_eq!(n.loader.as_deref(), Some("neoforge"));
    }

    #[test]
    fn old_forge_mod_with_mcmod_info() {
        let d = temp();
        let f = detect_file(&zip_at(
            &d,
            "old.jar",
            &[("mcmod.info", r#"[{"modid":"old","name":"Old Mod","version":"1.0","mcversion":"1.7.10"}]"#)],
        ));
        assert_eq!(f.kind, FileKind::Mod);
        assert_eq!(f.loader.as_deref(), Some("forge"));
        assert_eq!(f.name.as_deref(), Some("Old Mod"));
        assert_eq!(f.minecraft_versions, vec!["1.7.10"]);
        // mcmod.info files are often written loosely
        let loose = detect_file(&zip_at(
            &d,
            "loose.jar",
            &[("mcmod.info", "\u{feff}[\n  {\n    \"name\": \"Loose\", // comment\n    \"version\": \"2\",\n  },\n]")],
        ));
        assert_eq!(loose.name.as_deref(), Some("Loose"));
    }

    #[test]
    fn liteloader_and_plain_jars() {
        let d = temp();
        let lite = detect_file(&zip_at(
            &d,
            "voxel.litemod",
            &[("litemod.json", r#"{"name":"VoxelMap","version":"1.7","mcversion":"1.12.2"}"#)],
        ));
        assert_eq!((lite.kind, lite.loader.as_deref()), (FileKind::Mod, Some("liteloader")));
        let lib = detect_file(&zip_at(&d, "library.jar", &[("a/B.class", "")]));
        assert_eq!((lib.kind, lib.loader), (FileKind::Mod, None));
    }

    #[test]
    fn resource_data_and_shader_packs() {
        let d = temp();
        let rp = detect_file(&zip_at(
            &d,
            "Faithful.zip",
            &[
                ("pack.mcmeta", r#"{"pack":{"pack_format":46,"description":"Faithful 32x"}}"#),
                ("assets/minecraft/textures/a.png", ""),
            ],
        ));
        assert_eq!(rp.kind, FileKind::ResourcePack);
        assert_eq!(rp.version.as_deref(), Some("Faithful 32x"));
        let dp = detect_file(&zip_at(
            &d,
            "Terralith.zip",
            &[("pack.mcmeta", "{}"), ("data/terralith/worldgen/x.json", "{}")],
        ));
        assert_eq!(dp.kind, FileKind::DataPack);
        assert!(dp.kind.goes_into_instance());
        let flat = detect_file(&zip_at(&d, "BSL.zip", &[("shaders/", ""), ("shaders/composite.fsh", "void main(){}")]));
        assert_eq!(flat.kind, FileKind::ShaderPack);
        let wrapped = detect_file(&zip_at(
            &d,
            "Complementary.zip",
            &[("Complementary/", ""), ("Complementary/shaders/final.fsh", "")],
        ));
        assert_eq!(wrapped.kind, FileKind::ShaderPack);
    }

    #[test]
    fn modpacks_of_every_format() {
        let d = temp();
        let mr = detect_file(&zip_at(
            &d,
            "Fabulously Optimized.mrpack",
            &[("modrinth.index.json", r#"{"formatVersion":1,"name":"Fabulously Optimized","versionId":"6.0"}"#)],
        ));
        assert_eq!(mr.kind, FileKind::ModrinthModpack);
        assert_eq!(mr.name.as_deref(), Some("Fabulously Optimized"));
        assert!(mr.kind.is_modpack() && !mr.kind.goes_into_instance());
        assert_eq!(
            detect_file(&zip_at(&d, "pack.zip", &[("modrinth.index.json", "{}")])).kind,
            FileKind::ModrinthModpack
        );

        let cf = detect_file(&zip_at(
            &d,
            "All the Mods.zip",
            &[
                (
                    "manifest.json",
                    r#"{"minecraft":{"version":"1.20.1","modLoaders":[{"id":"forge-47.2.0"}]},"name":"All the Mods 9","version":"0.2","files":[]}"#,
                ),
                ("overrides/config/x.toml", ""),
            ],
        ));
        assert_eq!(cf.kind, FileKind::CurseForgeModpack);
        assert_eq!(cf.name.as_deref(), Some("All the Mods 9"));

        let prism = detect_file(&zip_at(
            &d,
            "Exported.zip",
            &[("My Pack/", ""), ("My Pack/mmc-pack.json", "{}"), ("My Pack/instance.cfg", "name=My Pack")],
        ));
        assert_eq!(prism.kind, FileKind::PrismExport);
        assert_eq!(detect_file(&zip_at(&d, "Old.zip", &[("instance.cfg", "name=x")])).kind, FileKind::PrismExport);

        // a manifest.json that isn't CurseForge's is not a modpack
        assert_eq!(detect_file(&zip_at(&d, "other.zip", &[("manifest.json", r#"{"a":1}"#)])).kind, FileKind::Unknown);
    }

    #[test]
    fn unrecognised_files_are_unknown() {
        let d = temp();
        let text = d.join("notes.txt");
        std::fs::write(&text, "hi").unwrap();
        assert_eq!(detect_file(&text).kind, FileKind::Unknown);
        let broken = d.join("broken.jar");
        std::fs::write(&broken, [1u8, 2, 3]).unwrap();
        assert_eq!(detect_file(&broken).kind, FileKind::Unknown);
        assert_eq!(detect_file(&zip_at(&d, "random.zip", &[("photos/cat.jpg", "x")])).kind, FileKind::Unknown);
    }

    #[test]
    fn bad_metadata_still_counts_as_a_mod() {
        let d = temp();
        let f = detect_file(&zip_at(&d, "broken-meta.jar", &[("fabric.mod.json", "{ not json")]));
        assert_eq!((f.kind, f.loader.as_deref(), f.name.clone()), (FileKind::Mod, Some("fabric"), None));
        assert_eq!(f.display_name(), "broken-meta.jar");
    }

    #[test]
    fn import_sources_are_told_apart_in_the_java_order() {
        let d = temp();
        let mmc = zip_at(&d, "a.zip", &[("mmc-pack.json", "{}"), ("instance.cfg", "")]);
        assert_eq!(detect_path(&mmc), Source::MultiMcArchive(mmc.clone()));
        let nested = zip_at(&d, "b.zip", &[("Pack/", ""), ("Pack/mmc-pack.json", "{}"), ("Pack/instance.cfg", "")]);
        assert_eq!(detect_path(&nested), Source::MultiMcArchive(nested.clone()));
        let cf = zip_at(&d, "c.zip", &[("manifest.json", "{}"), ("overrides/", "")]);
        assert_eq!(detect_path(&cf), Source::CurseForge(cf.clone()));
        let mr = zip_at(&d, "d.mrpack", &[("modrinth.index.json", "{}")]);
        assert_eq!(detect_path(&mr), Source::Modrinth(mr.clone()));
        let world = zip_at(&d, "e.zip", &[("My World/", ""), ("My World/level.dat", "")]);
        assert_eq!(detect_path(&world), Source::World(world.clone()));
        let junk = zip_at(&d, "f.zip", &[("x.txt", "")]);
        assert_eq!(detect_path(&junk), Source::Unknown);
        assert_eq!(detect_path(&d.join("missing.zip")), Source::Unknown);
        let plain = d.join("dotminecraft");
        std::fs::create_dir_all(plain.join("saves")).unwrap();
        assert_eq!(detect_path(&plain), Source::MinecraftFolder(plain.clone()));
        let prism = d.join("prism");
        std::fs::create_dir_all(&prism).unwrap();
        std::fs::write(prism.join("instance.cfg"), "name=x").unwrap();
        std::fs::write(prism.join("mmc-pack.json"), r#"{"components":[{"uid":"net.minecraft","version":"1.21"}]}"#)
            .unwrap();
        assert_eq!(detect_path(&prism), Source::LauncherFolder(prism.clone()));
    }

    #[test]
    fn web_addresses_are_understood() {
        assert_eq!(
            detect_url("https://www.curseforge.com/minecraft/modpacks/all-the-mods-9/files/5000000?x=1"),
            Some(UrlSource::CurseForge { slug: "all-the-mods-9".into(), file_id: Some(5000000) })
        );
        assert_eq!(
            detect_url("https://www.curseforge.com/minecraft/modpacks/rlcraft"),
            Some(UrlSource::CurseForge { slug: "rlcraft".into(), file_id: None })
        );
        assert_eq!(
            detect_url("https://modrinth.com/modpack/fabulously-optimized/version/6.0"),
            Some(UrlSource::Modrinth { slug: "fabulously-optimized".into() })
        );
        assert_eq!(
            detect_url("https://feed-the-beast.com/modpacks/100-ftb-stoneblock-3"),
            Some(UrlSource::Ftb { pack_id: 100 })
        );
        assert_eq!(
            detect_url("https://example.com/packs/a.mrpack?dl=1"),
            Some(UrlSource::File { url: "https://example.com/packs/a.mrpack?dl=1".into(), extension: "mrpack" })
        );
        assert_eq!(
            detect_url("https://example.com/packs/a.zip"),
            Some(UrlSource::File { url: "https://example.com/packs/a.zip".into(), extension: "zip" })
        );
        assert_eq!(detect_url("/home/me/pack.zip"), None);
        assert_eq!(detect_url("not a url"), None);
    }
}
