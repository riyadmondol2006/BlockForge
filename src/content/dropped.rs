//! What a file or folder the player dropped on the launcher turned out to be.
//!
//! The extension alone isn't enough: shader packs, resource packs, data packs, worlds and modpacks are all zip
//! files. So the names inside are looked at, and for a mod its metadata is read for the loader, the name, the
//! version and the Minecraft versions it asks for.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::kind::ContentKind;
use super::loader_support;
use super::version_ranges;

/// Metadata files bigger than this are not read.
pub const MAX_TEXT: u64 = 512 * 1024;
/// At most this many names are read from a zip or folder; enough to recognise any pack.
pub const MAX_ENTRIES: usize = 4000;

pub const LITELOADER: &str = "liteloader";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DroppedKind {
    Mod,
    ResourcePack,
    DataPack,
    ShaderPack,
    ModrinthModpack,
    CurseforgeModpack,
    /// an instance exported from Prism Launcher or MultiMC
    PrismExport,
    /// A Minecraft world (`level.dat`), zipped or as a folder. Not in the Java edition, which has no way to add
    /// one; it is copied into the instance's `saves/` folder.
    WorldSave,
    Unknown,
}

impl DroppedKind {
    /// Modpacks become instances of their own instead of going into one.
    pub fn is_modpack(self) -> bool {
        matches!(self, DroppedKind::ModrinthModpack | DroppedKind::CurseforgeModpack | DroppedKind::PrismExport)
    }

    /// Whether it goes into an instance (a data pack goes into its worlds).
    pub fn goes_into_instance(self) -> bool {
        matches!(
            self,
            DroppedKind::Mod
                | DroppedKind::ResourcePack
                | DroppedKind::DataPack
                | DroppedKind::ShaderPack
                | DroppedKind::WorldSave
        )
    }

    /// The tab of the content manager it shows on; `None` for data packs, worlds and modpacks.
    pub fn content_kind(self) -> Option<ContentKind> {
        match self {
            DroppedKind::Mod => Some(ContentKind::Mods),
            DroppedKind::ResourcePack => Some(ContentKind::ResourcePacks),
            DroppedKind::ShaderPack => Some(ContentKind::Shaders),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DroppedFile {
    /// where it is on disk (`None` for something classified from names alone)
    pub path: Option<PathBuf>,
    pub file_name: String,
    pub kind: DroppedKind,
    /// the loader a mod is made for (fabric, quilt, forge, neoforge, liteloader), or `None` if unknown
    pub loader: Option<String>,
    /// the name inside the file, or `None` if it has none
    pub name: Option<String>,
    pub version: Option<String>,
    /// Minecraft version requirements as the mod writes them; any one is enough
    pub minecraft_versions: Vec<String>,
}

impl DroppedFile {
    fn new(
        path: Option<&Path>,
        file_name: &str,
        kind: DroppedKind,
        loader: Option<&str>,
        name: Option<String>,
        version: Option<String>,
        minecraft_versions: Vec<String>,
    ) -> DroppedFile {
        DroppedFile {
            path: path.map(Path::to_path_buf),
            file_name: file_name.to_string(),
            kind,
            loader: loader.map(str::to_string),
            name,
            version,
            minecraft_versions,
        }
    }

    fn plain(path: Option<&Path>, file_name: &str, kind: DroppedKind) -> DroppedFile {
        DroppedFile::new(path, file_name, kind, None, None, None, Vec::new())
    }

    /// The best name to show for it.
    pub fn display_name(&self) -> &str {
        match &self.name {
            Some(n) if !n.trim().is_empty() => n,
            _ => &self.file_name,
        }
    }

    /// Why this might not work in an instance, or `None` when it looks fine.
    ///
    /// * `loader_key`: the instance's loader as Modrinth names it, `None` for plain Minecraft
    /// * `minecraft_version`: the instance's Minecraft version
    pub fn problem_for(&self, loader_key: Option<&str>, minecraft_version: &str) -> Option<String> {
        if self.kind == DroppedKind::Mod {
            let Some(instance_loader) = loader_key else {
                return Some("Mods need a mod loader such as Fabric".to_string());
            };
            if self.loader.as_deref() == Some(LITELOADER) {
                return Some(format!("{} is a LiteLoader mod", self.display_name()));
            }
            if let Some(mod_loader) = self.loader.as_deref() {
                if !loader_runs(instance_loader, mod_loader, minecraft_version) {
                    return Some(format!(
                        "{} is made for {}",
                        self.display_name(),
                        loader_support::display_name(Some(mod_loader))
                    ));
                }
            }
            let fits = if self.minecraft_versions.is_empty() {
                None
            } else {
                version_ranges::matches_any(&self.minecraft_versions, minecraft_version)
            };
            if fits == Some(false) {
                return Some(format!(
                    "{} is made for Minecraft {}",
                    self.display_name(),
                    self.minecraft_versions.join(" or ")
                ));
            }
        }
        if self.kind == DroppedKind::ShaderPack && loader_key.is_none() {
            return Some("Shaders need Fabric with Iris".to_string());
        }
        None
    }
}

/// Whether an instance with one loader runs a mod made for another.
pub fn loader_runs(instance_loader: &str, mod_loader: &str, minecraft_version: &str) -> bool {
    if instance_loader == mod_loader {
        return true;
    }
    if instance_loader == loader_support::QUILT && mod_loader == loader_support::FABRIC {
        return true;
    }
    // NeoForge still loaded Forge mods on 1.20.1, where it started
    instance_loader == loader_support::NEOFORGE && mod_loader == loader_support::FORGE && minecraft_version == "1.20.1"
}

// ---------------------------------------------------------------------------------------------------------
// Looking at the file

/// Reads a file or folder from disk and tells what it is. Things that can't be read are `Unknown`.
pub fn detect(path: &Path) -> DroppedFile {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if path.is_dir() {
        return detect_folder(path, &name);
    }
    let lower = name.to_lowercase();
    if !(lower.ends_with(".jar")
        || lower.ends_with(".zip")
        || lower.ends_with(".mrpack")
        || lower.ends_with(".litemod"))
    {
        return DroppedFile::plain(Some(path), &name, DroppedKind::Unknown);
    }
    let Ok(file) = std::fs::File::open(path) else {
        return DroppedFile::plain(Some(path), &name, DroppedKind::Unknown);
    };
    // not a zip at all: a .jar or .litemod that isn't one can't be a working mod either
    let Ok(archive) = zip::ZipArchive::new(file) else {
        return DroppedFile::plain(Some(path), &name, DroppedKind::Unknown);
    };
    let archive = std::cell::RefCell::new(archive);
    let entries = names_in(&mut archive.borrow_mut());
    let read = |entry: &str| read_entry(&mut archive.borrow_mut(), entry);
    classify(Some(path), &name, &entries, Some(&read))
}

/// A folder is looked at the same way as a zip: by the names inside it, a few levels down.
fn detect_folder(path: &Path, name: &str) -> DroppedFile {
    let mut entries = Vec::new();
    walk(path, "", 0, &mut entries);
    let read = |entry: &str| {
        let file = path.join(entry);
        let meta = std::fs::metadata(&file).ok()?;
        if !meta.is_file() || meta.len() > MAX_TEXT {
            return None;
        }
        std::fs::read(&file).ok().map(|b| String::from_utf8_lossy(&b).to_string())
    };
    let found = classify(Some(path), name, &entries, Some(&read));
    // a folder can't be copied in as a mod or imported as a CurseForge/Modrinth pack file
    match found.kind {
        DroppedKind::Mod | DroppedKind::ModrinthModpack | DroppedKind::CurseforgeModpack => {
            DroppedFile::plain(Some(path), name, DroppedKind::Unknown)
        }
        _ => found,
    }
}

fn walk(root: &Path, prefix: &str, depth: usize, out: &mut Vec<String>) {
    if depth > 3 || out.len() >= MAX_ENTRIES {
        return;
    }
    let Ok(rd) = std::fs::read_dir(root.join(prefix)) else { return };
    let mut list: Vec<_> = rd.flatten().collect();
    list.sort_by_key(|e| e.file_name());
    for e in list {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let name = e.file_name().to_string_lossy().to_string();
        let rel = if prefix.is_empty() { name } else { format!("{prefix}{name}") };
        if e.path().is_dir() {
            out.push(format!("{rel}/"));
            walk(root, &format!("{rel}/"), depth + 1, out);
        } else {
            out.push(rel);
        }
    }
}

fn names_in<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>) -> Vec<String> {
    let mut names = Vec::new();
    for i in 0..archive.len().min(MAX_ENTRIES) {
        if let Ok(entry) = archive.by_index_raw(i) {
            if let Ok(name) = entry.name() {
                names.push(name.to_string());
            }
        }
    }
    names
}

fn read_entry<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let entry = archive.by_name(name).ok()?;
    if entry.size() > MAX_TEXT {
        return None;
    }
    let mut bytes = Vec::new();
    entry.take(MAX_TEXT + 1).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).to_string())
}

/// The names inside a zip file, or `None` if it isn't a zip that can be read.
pub fn zip_entries(path: &Path) -> Option<Vec<String>> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    Some(names_in(&mut archive))
}

/// Tells what a file is from its name and the names inside it.
///
/// `read` gives the text of an entry, or `None` if it can't be read; it may itself be `None`.
pub fn classify(
    path: Option<&Path>,
    file_name: &str,
    raw_entries: &[String],
    read: Option<&dyn Fn(&str) -> Option<String>>,
) -> DroppedFile {
    let reader = |entry: &str| read.and_then(|r| r(entry));
    let lower = file_name.to_lowercase();
    let entries: HashSet<String> = raw_entries.iter().map(|e| e.replace('\\', "/")).collect();

    if lower.ends_with(".mrpack") || entries.contains("modrinth.index.json") {
        let json = json(reader("modrinth.index.json").as_deref());
        return DroppedFile::new(
            path,
            file_name,
            DroppedKind::ModrinthModpack,
            None,
            string(json.as_ref(), "name"),
            string(json.as_ref(), "versionId"),
            Vec::new(),
        );
    }
    if lower.ends_with(".litemod") {
        let json = json(reader("litemod.json").as_deref());
        return DroppedFile::new(
            path,
            file_name,
            DroppedKind::Mod,
            Some(LITELOADER),
            string(json.as_ref(), "name"),
            string(json.as_ref(), "version"),
            list_of(string(json.as_ref(), "mcversion")),
        );
    }
    if at_root_or_one_down(&entries, "mmc-pack.json") || at_root_or_one_down(&entries, "instance.cfg") {
        return DroppedFile::plain(path, file_name, DroppedKind::PrismExport);
    }
    if entries.contains("manifest.json") {
        let json = json(reader("manifest.json").as_deref());
        if let Some(mc) = json.as_ref().and_then(|j| j.get("minecraft")).filter(|m| m.is_object()) {
            return DroppedFile::new(
                path,
                file_name,
                DroppedKind::CurseforgeModpack,
                None,
                string(json.as_ref(), "name"),
                string(json.as_ref(), "version"),
                list_of(string(Some(mc), "version")),
            );
        }
    }
    if at_root_or_one_down(&entries, "level.dat") {
        return DroppedFile::plain(path, file_name, DroppedKind::WorldSave);
    }

    if let Some(found) = mod_metadata(path, file_name, &entries, &reader) {
        return found;
    }
    if lower.ends_with(".jar") {
        // a jar without any loader's metadata: probably a library or a very old mod
        return DroppedFile::plain(path, file_name, DroppedKind::Mod);
    }

    let mcmeta = entries.contains("pack.mcmeta");
    let mut assets = false;
    let mut data = false;
    let mut shaders = false;
    for entry in &entries {
        assets |= entry.starts_with("assets/");
        data |= entry.starts_with("data/");
        let parts: Vec<&str> = entry.split('/').collect();
        // Java's String.split drops trailing empties: "shaders/" is just ["shaders"]
        let real: Vec<&str> = {
            let mut p = parts.clone();
            while p.last().is_some_and(|x| x.is_empty()) {
                p.pop();
            }
            p
        };
        if (real.len() >= 2 && real[0] == "shaders") || (real.len() >= 3 && real[1] == "shaders") {
            shaders = true;
        }
        if entry == "shaders/" {
            shaders = true;
        }
    }
    if shaders && !assets {
        return DroppedFile::plain(path, file_name, DroppedKind::ShaderPack);
    }
    if mcmeta {
        let description = pack_description(reader("pack.mcmeta").as_deref());
        let kind = if data && !assets { DroppedKind::DataPack } else { DroppedKind::ResourcePack };
        return DroppedFile::new(path, file_name, kind, None, None, description, Vec::new());
    }
    DroppedFile::plain(path, file_name, DroppedKind::Unknown)
}

fn at_root_or_one_down(entries: &HashSet<String>, name: &str) -> bool {
    entries.iter().any(|entry| {
        if entry == name {
            return true;
        }
        let parts: Vec<&str> = entry.split('/').collect();
        let mut real = parts;
        while real.last().is_some_and(|x| x.is_empty()) {
            real.pop();
        }
        real.len() == 2 && real[1] == name
    })
}

/// Reads whichever loader's metadata a mod has; `None` if it has none.
fn mod_metadata(
    path: Option<&Path>,
    file_name: &str,
    entries: &HashSet<String>,
    read: &dyn Fn(&str) -> Option<String>,
) -> Option<DroppedFile> {
    if entries.contains("quilt.mod.json") {
        let json = json(read("quilt.mod.json").as_deref());
        let loader = json.as_ref().and_then(|j| j.get("quilt_loader")).filter(|l| l.is_object());
        let meta = loader.and_then(|l| l.get("metadata")).filter(|m| m.is_object());
        let mut mc = Vec::new();
        if let Some(deps) = loader.and_then(|l| l.get("depends")).and_then(Value::as_array) {
            for dep in deps {
                if dep.is_object() && string(Some(dep), "id").as_deref() == Some("minecraft") {
                    mc.extend(versions(dep.get("versions")));
                }
            }
        }
        return Some(DroppedFile::new(
            path,
            file_name,
            DroppedKind::Mod,
            Some(loader_support::QUILT),
            string(meta, "name"),
            clean(string(loader, "version")),
            mc,
        ));
    }
    if entries.contains("fabric.mod.json") {
        let json = json(read("fabric.mod.json").as_deref());
        let mut mc = Vec::new();
        if let Some(depends) = json.as_ref().and_then(|j| j.get("depends")).filter(|d| d.is_object()) {
            mc.extend(versions(depends.get("minecraft")));
        }
        return Some(DroppedFile::new(
            path,
            file_name,
            DroppedKind::Mod,
            Some(loader_support::FABRIC),
            string(json.as_ref(), "name"),
            clean(string(json.as_ref(), "version")),
            mc,
        ));
    }
    if entries.contains("META-INF/neoforge.mods.toml") {
        return Some(toml(path, file_name, read("META-INF/neoforge.mods.toml").as_deref(), loader_support::NEOFORGE));
    }
    if entries.contains("META-INF/mods.toml") {
        return Some(toml(path, file_name, read("META-INF/mods.toml").as_deref(), loader_support::FORGE));
    }
    if entries.contains("mcmod.info") {
        let root = parse_lenient(read("mcmod.info").as_deref());
        let first: Option<Value> = match &root {
            Some(Value::Array(list)) if list.first().is_some_and(Value::is_object) => list.first().cloned(),
            Some(Value::Object(o)) => {
                o.get("modList").and_then(Value::as_array).and_then(|l| l.first()).filter(|f| f.is_object()).cloned()
            }
            _ => None,
        };
        return Some(DroppedFile::new(
            path,
            file_name,
            DroppedKind::Mod,
            Some(loader_support::FORGE),
            string(first.as_ref(), "name"),
            clean(string(first.as_ref(), "version")),
            list_of(clean(string(first.as_ref(), "mcversion"))),
        ));
    }
    None
}

/// Forge and NeoForge describe mods in TOML. Only the few keys needed here are read: the display name, the version
/// and the Minecraft dependency. A Forge-style file that depends on NeoForge is a NeoForge mod.
pub fn toml(path: Option<&Path>, file_name: &str, text: Option<&str>, loader: &str) -> DroppedFile {
    let mut table = String::new();
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut dep_mod_id: Option<String> = None;
    let mut dep_range: Option<String> = None;
    let mut mc: Vec<String> = Vec::new();
    let mut depends_on_neoforge = false;
    if let Some(text) = text {
        for raw in text.lines() {
            let line = raw.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                if dep_mod_id.as_deref() == Some("minecraft") {
                    if let Some(r) = dep_range.take() {
                        mc.push(r);
                    }
                }
                if dep_mod_id.as_deref() == Some("neoforge") {
                    depends_on_neoforge = true;
                }
                dep_mod_id = None;
                dep_range = None;
                table = line.replace(['[', ']'], "").trim().to_string();
                continue;
            }
            let Some(eq) = line.find('=') else { continue };
            let key = line[..eq].trim();
            let value = unquote(line[eq + 1..].trim());
            if table == "mods" {
                if key == "displayName" && name.is_none() {
                    name = Some(value);
                } else if key == "version" && version.is_none() {
                    version = clean(Some(value));
                }
            } else if table.starts_with("dependencies") {
                if key == "modId" {
                    dep_mod_id = Some(value);
                } else if key == "versionRange" {
                    dep_range = Some(value);
                }
            }
        }
        if dep_mod_id.as_deref() == Some("minecraft") {
            if let Some(r) = dep_range.take() {
                mc.push(r);
            }
        }
        if dep_mod_id.as_deref() == Some("neoforge") {
            depends_on_neoforge = true;
        }
    }
    let effective = if depends_on_neoforge { loader_support::NEOFORGE } else { loader };
    DroppedFile::new(path, file_name, DroppedKind::Mod, Some(effective), name, version, mc)
}

fn unquote(value: &str) -> String {
    let mut v = value;
    if let Some(hash) = v.find(" #").filter(|h| *h > 0) {
        if !v.starts_with('"') && !v.starts_with('\'') {
            v = v[..hash].trim();
        }
    }
    if v.chars().count() >= 2 && (v.starts_with('"') || v.starts_with('\'')) {
        let quote = v.chars().next().unwrap_or('"');
        let rest = &v[1..];
        return match rest.find(quote) {
            Some(end) => rest[..end].to_string(),
            None => rest.to_string(),
        };
    }
    v.to_string()
}

/// Placeholders like `${version}` are filled in when a mod is built; one left in means the value is unknown.
fn clean(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.contains("${") && !v.trim().is_empty()).map(|v| v.trim().to_string())
}

fn list_of(value: Option<String>) -> Vec<String> {
    value.into_iter().collect()
}

fn versions(element: Option<&Value>) -> Vec<String> {
    match element {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    }
}

fn pack_description(mcmeta: Option<&str>) -> Option<String> {
    let json = json(mcmeta)?;
    match json.get("pack")?.get("description")? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn json(text: Option<&str>) -> Option<Value> {
    parse_lenient(text).filter(Value::is_object)
}

fn string(json: Option<&Value>, key: &str) -> Option<String> {
    match json?.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Parses the JSON mods ship, which is often not strictly valid: a byte order mark at the start, comments, trailing
/// commas, single quotes, and raw line breaks inside strings (Java's lenient reader accepts all of these).
pub fn parse_lenient(text: Option<&str>) -> Option<Value> {
    let text = text?.trim_start_matches('\u{feff}');
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return Some(v);
    }
    serde_json::from_str::<Value>(&repair_json(text)).ok()
}

fn repair_json(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut in_string: Option<char> = None;
    let mut escape = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(quote) = in_string {
            if escape {
                // \' is not a JSON escape
                if c == '\'' {
                    out.pop();
                }
                out.push(c);
                escape = false;
            } else if c == '\\' {
                out.push(c);
                escape = true;
            } else if c == quote {
                out.push('"');
                in_string = None;
            } else if c == '"' {
                out.push_str("\\\"");
            } else if c == '\n' {
                out.push_str("\\n");
            } else if c == '\r' {
                out.push_str("\\r");
            } else if c == '\t' {
                out.push_str("\\t");
            } else if (c as u32) < 0x20 {
                out.push_str(&format!("\\u{:04x}", c as u32));
            } else {
                out.push(c);
            }
        } else if c == '"' || c == '\'' {
            in_string = Some(c);
            out.push('"');
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        } else if c == ',' {
            // a comma right before a closing bracket is dropped
            let next = chars[i + 1..].iter().find(|x| !x.is_whitespace());
            if !matches!(next, Some('}') | Some(']')) {
                out.push(c);
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Writes a zip with the given entries; an entry ending in / is a folder.
    fn zip(dir: &Path, name: &str, entries: &[(&str, Option<&str>)]) -> PathBuf {
        let file = dir.join(name);
        let mut z = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (entry, body) in entries {
            if entry.ends_with('/') {
                z.add_directory(*entry, options).unwrap();
            } else {
                z.start_file(*entry, options).unwrap();
                if let Some(b) = body {
                    z.write_all(b.as_bytes()).unwrap();
                }
            }
        }
        z.finish().unwrap();
        file
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("blockforge-dropped-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn fabric_mod_with_name_version_and_minecraft_range() {
        let dir = temp("fabric");
        let f = detect(&zip(
            &dir,
            "sodium.jar",
            &[
                (
                    "fabric.mod.json",
                    Some(
                        r#"{"schemaVersion":1,"id":"sodium","name":"Sodium","version":"0.9.2","depends":{"minecraft":">=26.3 <26.4","fabricloader":">=0.16"}}"#,
                    ),
                ),
                ("net/caffeinemc/Sodium.class", Some("")),
            ],
        ));
        assert_eq!(f.kind, DroppedKind::Mod);
        assert_eq!(f.loader.as_deref(), Some(loader_support::FABRIC));
        assert_eq!(f.name.as_deref(), Some("Sodium"));
        assert_eq!(f.version.as_deref(), Some("0.9.2"));
        assert_eq!(f.minecraft_versions, s(&[">=26.3 <26.4"]));
        assert_eq!(f.problem_for(Some(loader_support::FABRIC), "26.3"), None);
        assert_eq!(f.problem_for(Some(loader_support::QUILT), "26.3"), None);
        assert!(f.problem_for(Some(loader_support::FABRIC), "26.2").unwrap().contains("Minecraft"));
        assert!(f.problem_for(Some(loader_support::NEOFORGE), "26.3").unwrap().contains("Fabric"));
        assert!(f.problem_for(None, "26.3").unwrap().contains("loader"));
    }

    #[test]
    fn fabric_version_lists_mean_any_of_them() {
        let dir = temp("multi");
        let f = detect(&zip(
            &dir,
            "multi.jar",
            &[(
                "fabric.mod.json",
                Some(r#"{"name":"Multi","version":"${version}","depends":{"minecraft":["1.20.1","1.20.4"]}}"#),
            )],
        ));
        assert_eq!(f.version, None);
        assert_eq!(f.minecraft_versions, s(&["1.20.1", "1.20.4"]));
        assert_eq!(f.problem_for(Some(loader_support::FABRIC), "1.20.4"), None);
        assert!(f.problem_for(Some(loader_support::FABRIC), "1.21").unwrap().contains("1.20.1 or 1.20.4"));
    }

    #[test]
    fn quilt_mod() {
        let dir = temp("quilt");
        let f = detect(&zip(
            &dir,
            "q.jar",
            &[(
                "quilt.mod.json",
                Some(
                    r#"{"quilt_loader":{"id":"q","version":"2.0","metadata":{"name":"Quilty"},"depends":["quilt_loader",{"id":"minecraft","versions":">=1.20"}]}}"#,
                ),
            )],
        ));
        assert_eq!(f.kind, DroppedKind::Mod);
        assert_eq!(f.loader.as_deref(), Some(loader_support::QUILT));
        assert_eq!(f.name.as_deref(), Some("Quilty"));
        assert_eq!(f.version.as_deref(), Some("2.0"));
        assert_eq!(f.minecraft_versions, s(&[">=1.20"]));
        assert!(f.problem_for(Some(loader_support::FABRIC), "1.21.1").unwrap().contains("Quilt"));
    }

    #[test]
    fn neoforge_mod_from_its_own_toml() {
        let dir = temp("neo");
        let toml_text = "modLoader=\"javafml\"\nloaderVersion=\"[4,)\"\n[[mods]]\nmodId=\"neo\"\nversion=\"1.4.0\" # comment\n\
            displayName=\"Neo Thing\"\n[[dependencies.neo]]\nmodId=\"neoforge\"\ntype=\"required\"\n\
            versionRange=\"[21.1,)\"\n[[dependencies.neo]]\nmodId=\"minecraft\"\ntype=\"required\"\n\
            versionRange=\"[1.21.1,1.22)\"\n";
        let f = detect(&zip(&dir, "neo.jar", &[("META-INF/neoforge.mods.toml", Some(toml_text))]));
        assert_eq!(f.loader.as_deref(), Some(loader_support::NEOFORGE));
        assert_eq!(f.name.as_deref(), Some("Neo Thing"));
        assert_eq!(f.version.as_deref(), Some("1.4.0"));
        assert_eq!(f.minecraft_versions, s(&["[1.21.1,1.22)"]));
        assert_eq!(f.problem_for(Some(loader_support::NEOFORGE), "1.21.1"), None);
        assert!(f.problem_for(Some(loader_support::NEOFORGE), "1.20.1").unwrap().contains("Minecraft"));
    }

    #[test]
    fn forge_mods_toml_and_neoforge_ones_that_still_use_it() {
        let dir = temp("forge");
        let forge_toml = "[[mods]]\nmodId='jei'\nversion='${file.jarVersion}'\ndisplayName='Just Enough Items'\n\
            [[dependencies.jei]]\nmodId='forge'\nversionRange='[47,)'\n\
            [[dependencies.jei]]\nmodId='minecraft'\nversionRange='[1.20.1]'\n";
        let forge = detect(&zip(&dir, "jei.jar", &[("META-INF/mods.toml", Some(forge_toml))]));
        assert_eq!(forge.loader.as_deref(), Some(loader_support::FORGE));
        assert_eq!(forge.name.as_deref(), Some("Just Enough Items"));
        assert_eq!(forge.version, None);
        assert_eq!(forge.problem_for(Some(loader_support::FORGE), "1.20.1"), None);
        // NeoForge ran Forge mods on 1.20.1
        assert_eq!(forge.problem_for(Some(loader_support::NEOFORGE), "1.20.1"), None);
        assert!(forge.problem_for(Some(loader_support::FABRIC), "1.20.1").unwrap().contains("Forge"));

        let old_neo = detect(&zip(
            &dir,
            "oldneo.jar",
            &[(
                "META-INF/mods.toml",
                Some("[[mods]]\nmodId=\"x\"\n[[dependencies.x]]\nmodId=\"neoforge\"\nversionRange=\"[20.4,)\"\n"),
            )],
        ));
        assert_eq!(old_neo.loader.as_deref(), Some(loader_support::NEOFORGE));
    }

    #[test]
    fn old_forge_mod_with_mcmod_info() {
        let dir = temp("mcmod");
        let f = detect(&zip(
            &dir,
            "old.jar",
            &[("mcmod.info", Some(r#"[{"modid":"old","name":"Old Mod","version":"1.0","mcversion":"1.7.10"}]"#))],
        ));
        assert_eq!(f.kind, DroppedKind::Mod);
        assert_eq!(f.loader.as_deref(), Some(loader_support::FORGE));
        assert_eq!(f.name.as_deref(), Some("Old Mod"));
        assert_eq!(f.minecraft_versions, s(&["1.7.10"]));
        assert!(f.problem_for(Some(loader_support::FORGE), "1.12.2").unwrap().contains("1.7.10"));
    }

    #[test]
    fn liteloader_and_plain_jars() {
        let dir = temp("lite");
        let lite = detect(&zip(
            &dir,
            "voxel.litemod",
            &[("litemod.json", Some(r#"{"name":"VoxelMap","version":"1.7","mcversion":"1.12.2"}"#))],
        ));
        assert_eq!(lite.kind, DroppedKind::Mod);
        assert_eq!(lite.loader.as_deref(), Some(LITELOADER));
        assert!(lite.problem_for(Some(loader_support::FORGE), "1.12.2").unwrap().contains("LiteLoader"));

        let library = detect(&zip(&dir, "library.jar", &[("a/B.class", Some(""))]));
        assert_eq!(library.kind, DroppedKind::Mod);
        assert_eq!(library.loader, None);
        assert_eq!(library.problem_for(Some(loader_support::FABRIC), "26.3"), None);
    }

    #[test]
    fn resource_data_and_shader_packs() {
        let dir = temp("packs");
        let rp = detect(&zip(
            &dir,
            "Faithful.zip",
            &[
                ("pack.mcmeta", Some(r#"{"pack":{"pack_format":46,"description":"Faithful 32x"}}"#)),
                ("assets/minecraft/textures/a.png", Some("")),
            ],
        ));
        assert_eq!(rp.kind, DroppedKind::ResourcePack);
        assert_eq!(rp.kind.content_kind(), Some(ContentKind::ResourcePacks));
        assert_eq!(rp.version.as_deref(), Some("Faithful 32x"));

        let dp = detect(&zip(
            &dir,
            "Terralith.zip",
            &[("pack.mcmeta", Some("{}")), ("data/terralith/worldgen/x.json", Some("{}"))],
        ));
        assert_eq!(dp.kind, DroppedKind::DataPack);
        assert_eq!(dp.kind.content_kind(), None);
        assert!(dp.kind.goes_into_instance());

        let flat =
            detect(&zip(&dir, "BSL.zip", &[("shaders/", None), ("shaders/composite.fsh", Some("void main(){}"))]));
        assert_eq!(flat.kind, DroppedKind::ShaderPack);
        let wrapped = detect(&zip(
            &dir,
            "Complementary.zip",
            &[("Complementary/", None), ("Complementary/shaders/final.fsh", Some(""))],
        ));
        assert_eq!(wrapped.kind, DroppedKind::ShaderPack);
        assert_eq!(wrapped.kind.content_kind(), Some(ContentKind::Shaders));
        assert_eq!(wrapped.problem_for(Some(loader_support::FABRIC), "26.3"), None);
        assert!(wrapped.problem_for(None, "26.3").unwrap().contains("Iris"));
    }

    #[test]
    fn modpacks_of_every_format() {
        let dir = temp("modpacks");
        let mrpack = detect(&zip(
            &dir,
            "Fabulously Optimized.mrpack",
            &[("modrinth.index.json", Some(r#"{"formatVersion":1,"name":"Fabulously Optimized","versionId":"6.0"}"#))],
        ));
        assert_eq!(mrpack.kind, DroppedKind::ModrinthModpack);
        assert_eq!(mrpack.name.as_deref(), Some("Fabulously Optimized"));
        assert!(mrpack.kind.is_modpack());
        assert!(!mrpack.kind.goes_into_instance());

        let mrzip = detect(&zip(&dir, "pack.zip", &[("modrinth.index.json", Some("{}"))]));
        assert_eq!(mrzip.kind, DroppedKind::ModrinthModpack);

        let curse = detect(&zip(
            &dir,
            "All the Mods.zip",
            &[
                (
                    "manifest.json",
                    Some(
                        r#"{"minecraft":{"version":"1.20.1","modLoaders":[{"id":"forge-47.2.0"}]},"name":"All the Mods 9","version":"0.2","files":[]}"#,
                    ),
                ),
                ("overrides/config/x.toml", Some("")),
            ],
        ));
        assert_eq!(curse.kind, DroppedKind::CurseforgeModpack);
        assert_eq!(curse.name.as_deref(), Some("All the Mods 9"));
        assert_eq!(curse.minecraft_versions, s(&["1.20.1"]));

        let prism = detect(&zip(
            &dir,
            "Exported.zip",
            &[
                ("My Pack/", None),
                ("My Pack/mmc-pack.json", Some("{}")),
                ("My Pack/instance.cfg", Some("name=My Pack")),
            ],
        ));
        assert_eq!(prism.kind, DroppedKind::PrismExport);
        let multimc = detect(&zip(&dir, "Old.zip", &[("instance.cfg", Some("name=x"))]));
        assert_eq!(multimc.kind, DroppedKind::PrismExport);

        // a manifest.json that isn't CurseForge's is not a modpack
        let other = detect(&zip(&dir, "other.zip", &[("manifest.json", Some("{\"a\":1}"))]));
        assert_eq!(other.kind, DroppedKind::Unknown);
    }

    #[test]
    fn unrecognised_files_are_unknown() {
        let dir = temp("unknown");
        let text = dir.join("notes.txt");
        std::fs::write(&text, "hi").unwrap();
        assert_eq!(detect(&text).kind, DroppedKind::Unknown);

        let broken = dir.join("broken.jar");
        std::fs::write(&broken, [1u8, 2, 3]).unwrap();
        assert_eq!(detect(&broken).kind, DroppedKind::Unknown);

        assert_eq!(detect(&zip(&dir, "random.zip", &[("photos/cat.jpg", Some("x"))])).kind, DroppedKind::Unknown);
        assert_eq!(detect(&dir.join("missing.jar")).kind, DroppedKind::Unknown);
    }

    #[test]
    fn bad_metadata_still_counts_as_a_mod() {
        let dir = temp("badmeta");
        let f = detect(&zip(&dir, "broken-meta.jar", &[("fabric.mod.json", Some("{ not json"))]));
        assert_eq!(f.kind, DroppedKind::Mod);
        assert_eq!(f.loader.as_deref(), Some(loader_support::FABRIC));
        assert_eq!(f.name, None);
        assert_eq!(f.display_name(), "broken-meta.jar");
    }

    #[test]
    fn sloppy_json_that_mods_really_ship_is_still_read() {
        // a raw line break inside a string, a comment and a trailing comma: Gson's lenient reader takes all three
        let text = "\u{feff}{\n // the name\n \"name\": \"Two\nLines\",\n \"version\": '1.0',\n \"depends\": {\"minecraft\": [\"1.21\",],},\n}";
        let v = parse_lenient(Some(text)).unwrap();
        assert_eq!(v["name"], "Two\nLines");
        assert_eq!(v["version"], "1.0");
        assert_eq!(v["depends"]["minecraft"][0], "1.21");
        assert!(parse_lenient(Some("{ not json")).is_none());
    }

    #[test]
    fn worlds_and_folders() {
        let dir = temp("worlds");
        let zipped = detect(&zip(
            &dir,
            "My World.zip",
            &[("My World/", None), ("My World/level.dat", Some("x")), ("My World/region/r.0.0.mca", Some(""))],
        ));
        assert_eq!(zipped.kind, DroppedKind::WorldSave);
        assert!(zipped.kind.goes_into_instance());

        let world = dir.join("Survival");
        std::fs::create_dir_all(world.join("region")).unwrap();
        std::fs::write(world.join("level.dat"), "x").unwrap();
        assert_eq!(detect(&world).kind, DroppedKind::WorldSave);

        let pack = dir.join("MyPack");
        std::fs::create_dir_all(pack.join("assets/minecraft")).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), r#"{"pack":{"description":"hi"}}"#).unwrap();
        let found = detect(&pack);
        assert_eq!(found.kind, DroppedKind::ResourcePack);
        assert_eq!(found.version.as_deref(), Some("hi"));

        let shaders = dir.join("Shaders");
        std::fs::create_dir_all(shaders.join("shaders")).unwrap();
        std::fs::write(shaders.join("shaders/final.fsh"), "").unwrap();
        assert_eq!(detect(&shaders).kind, DroppedKind::ShaderPack);

        // an unpacked mod folder can't be dropped into mods/
        let unpacked = dir.join("unpacked");
        std::fs::create_dir_all(&unpacked).unwrap();
        std::fs::write(unpacked.join("fabric.mod.json"), "{}").unwrap();
        assert_eq!(detect(&unpacked).kind, DroppedKind::Unknown);

        let prism = dir.join("Inst");
        std::fs::create_dir_all(&prism).unwrap();
        std::fs::write(prism.join("mmc-pack.json"), "{}").unwrap();
        assert_eq!(detect(&prism).kind, DroppedKind::PrismExport);
    }
}
