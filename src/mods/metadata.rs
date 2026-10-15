//! Looking inside a mod, resource pack, data pack or shader pack to see what it is: its name, version,
//! description, authors and picture, which mod loader it is made for, and which Minecraft versions it asks for.
//!
//! Every loader writes its own file: `fabric.mod.json`, `quilt.mod.json`, `META-INF/neoforge.mods.toml`,
//! `META-INF/mods.toml`, `mcmod.info` (old Forge), `litemod.json` (LiteLoader) and `pack.mcmeta` (packs). The order
//! they are tried in, and the way the loader is told apart, follow the Java edition's `DroppedFileDetector`.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::versions::{self, FABRIC, FORGE, LITELOADER, NEOFORGE, QUILT};

/// Metadata files bigger than this are not read.
pub const MAX_TEXT: u64 = 512 * 1024;
/// At most this many names are read from a zip; enough to recognise any pack.
pub const MAX_ENTRIES: usize = 4000;
/// Pictures bigger than this are not extracted.
const MAX_ICON: u64 = 4 * 1024 * 1024;

/// What a file turned out to be.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileKind {
    Mod,
    ResourcePack,
    DataPack,
    ShaderPack,
    ModrinthModpack,
    CurseForgeModpack,
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

    /// The tab of the content manager it shows on, or `None` for data packs and modpacks.
    pub fn content_kind(self) -> Option<ContentKind> {
        match self {
            FileKind::Mod => Some(ContentKind::Mods),
            FileKind::ResourcePack => Some(ContentKind::ResourcePacks),
            FileKind::ShaderPack => Some(ContentKind::Shaders),
            _ => None,
        }
    }
}

/// The kinds of things a player adds to an instance, and where each one lives inside it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ContentKind {
    Mods,
    ResourcePacks,
    Shaders,
}

impl ContentKind {
    pub const ALL: [ContentKind; 3] = [ContentKind::Mods, ContentKind::ResourcePacks, ContentKind::Shaders];

    /// How the launcher records a file of this kind (`type` in the instance's mod list).
    pub fn record_type(self) -> &'static str {
        match self {
            ContentKind::Mods => "mods",
            ContentKind::ResourcePacks => "resourcepack",
            ContentKind::Shaders => "shaderpack",
        }
    }

    /// The folder inside the instance.
    pub fn folder(self) -> &'static str {
        match self {
            ContentKind::Mods => "mods",
            ContentKind::ResourcePacks => "resourcepacks",
            ContentKind::Shaders => "shaderpacks",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ContentKind::Mods => "Mods",
            ContentKind::ResourcePacks => "Resource packs",
            ContentKind::Shaders => "Shaders",
        }
    }

    /// The Modrinth project type used when searching.
    pub fn project_type(self) -> &'static str {
        match self {
            ContentKind::Mods => "mod",
            ContentKind::ResourcePacks => "resourcepack",
            ContentKind::Shaders => "shader",
        }
    }

    /// The kind that a recorded `type` belongs to, or `None` if it isn't one of these.
    pub fn of_record_type(t: &str) -> Option<ContentKind> {
        ContentKind::ALL.into_iter().find(|k| k.record_type() == t)
    }
}

/// Everything read from inside a file. Fields are `None` or empty when the file doesn't say.
#[derive(Clone, Debug, Default)]
pub struct ModInfo {
    /// The loader to treat it as (the first of `loaders`), or `None` when no loader metadata was found.
    pub loader: Option<String>,
    /// Every loader whose metadata is in the file; a few jars carry both Fabric and Forge files.
    pub loaders: Vec<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub authors: Vec<String>,
    pub license: Option<String>,
    pub homepage: Option<String>,
    pub sources: Option<String>,
    pub issues: Option<String>,
    /// Path of the logo inside the file.
    pub icon_path: Option<String>,
    /// Minecraft version requirements as the mod writes them; any one is enough.
    pub minecraft_versions: Vec<String>,
    /// `client`, `server` or `*` when the mod says where it runs.
    pub environment: Option<String>,
    /// Ids of the mods this one needs (loader and Minecraft left out).
    pub depends: Vec<String>,
}

/// What a file turned out to be, with what could be read from inside it.
#[derive(Clone, Debug)]
pub struct Detected {
    pub path: PathBuf,
    pub file_name: String,
    pub kind: FileKind,
    /// The loader a mod is made for (fabric, quilt, forge, neoforge, liteloader), or `None` if unknown.
    pub loader: Option<String>,
    /// The name inside the file, or `None` if it has none.
    pub name: Option<String>,
    pub version: Option<String>,
    /// Minecraft version requirements as the mod writes them; any one is enough.
    pub minecraft_versions: Vec<String>,
    /// For packs, the description from `pack.mcmeta`.
    pub description: Option<String>,
    pub info: ModInfo,
}

impl Detected {
    fn plain(path: &Path, file_name: &str, kind: FileKind) -> Detected {
        Detected {
            path: path.to_path_buf(),
            file_name: file_name.to_string(),
            kind,
            loader: None,
            name: None,
            version: None,
            minecraft_versions: Vec::new(),
            description: None,
            info: ModInfo::default(),
        }
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
    /// `loader_key` is the instance's loader as Modrinth names it (`None` for plain Minecraft).
    pub fn problem_for(&self, loader_key: Option<&str>, minecraft_version: &str) -> Option<String> {
        if self.kind == FileKind::Mod {
            let Some(instance_loader) = loader_key else {
                return Some("Mods need a mod loader such as Fabric".into());
            };
            if self.loader.as_deref() == Some(LITELOADER) {
                return Some(format!("{} is a LiteLoader mod", self.display_name()));
            }
            if let Some(mod_loader) = &self.loader {
                // a jar that carries metadata for several loaders runs on any of them
                let runs =
                    self.info.loaders.iter().any(|l| versions::loader_runs(instance_loader, l, minecraft_version))
                        || versions::loader_runs(instance_loader, mod_loader, minecraft_version);
                if !runs {
                    return Some(format!(
                        "{} is made for {}",
                        self.display_name(),
                        versions::display_name(Some(mod_loader))
                    ));
                }
            }
            if !self.minecraft_versions.is_empty()
                && versions::range_matches_any(&self.minecraft_versions, minecraft_version) == Some(false)
            {
                return Some(format!(
                    "{} is made for Minecraft {}",
                    self.display_name(),
                    self.minecraft_versions.join(" or ")
                ));
            }
        }
        if self.kind == FileKind::ShaderPack && loader_key.is_none() {
            return Some("Shaders need Fabric with Iris".into());
        }
        None
    }
}

/// Reads a file from disk and tells what it is. Files that can't be read are `Unknown`.
pub fn detect(file: &Path) -> Detected {
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let lower = name.to_lowercase();
    if !(lower.ends_with(".jar")
        || lower.ends_with(".zip")
        || lower.ends_with(".mrpack")
        || lower.ends_with(".litemod"))
    {
        return Detected::plain(file, &name, FileKind::Unknown);
    }
    let Ok(f) = std::fs::File::open(file) else { return Detected::plain(file, &name, FileKind::Unknown) };
    // not a zip at all: a .jar or .litemod that isn't one can't be a working mod either
    let Ok(mut zip) = zip::ZipArchive::new(f) else { return Detected::plain(file, &name, FileKind::Unknown) };
    let entries: Vec<String> =
        zip.file_names().take(MAX_ENTRIES).filter_map(|n| n.ok().map(|c| c.to_string())).collect();
    let mut cache: Vec<(String, Option<String>)> = Vec::new();
    let mut detected = {
        let mut reader = |entry: &str| -> Option<String> {
            if let Some((_, v)) = cache.iter().find(|(k, _)| k == entry) {
                return v.clone();
            }
            let v = read_text(&mut zip, entry);
            cache.push((entry.to_string(), v.clone()));
            v
        };
        classify(file, &name, &entries, &mut reader)
    };
    detected.path = file.to_path_buf();
    detected
}

/// Reads the text of one entry in a zip, or `None` when it is missing, too big or not text.
pub fn read_text<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, entry: &str) -> Option<String> {
    let mut e = zip.by_name(entry).ok()?;
    if e.size() > MAX_TEXT {
        return None;
    }
    let mut buf = Vec::new();
    e.by_ref().take(MAX_TEXT + 1).read_to_end(&mut buf).ok()?;
    if buf.len() as u64 > MAX_TEXT {
        return None;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Reads the bytes of one entry in a zip (a logo), or `None` when it is missing or too big.
pub fn read_bytes<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, entry: &str) -> Option<Vec<u8>> {
    let mut e = zip.by_name(entry).ok()?;
    if e.size() > MAX_ICON {
        return None;
    }
    let mut buf = Vec::new();
    e.by_ref().take(MAX_ICON + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 <= MAX_ICON).then_some(buf)
}

/// Tells what a file is from its name and the names inside it.
///
/// `read` gives the text of an entry, or `None` if it can't be read.
pub fn classify(
    path: &Path,
    file_name: &str,
    raw_entries: &[String],
    read: &mut dyn FnMut(&str) -> Option<String>,
) -> Detected {
    let lower = file_name.to_lowercase();
    let entries: BTreeSet<String> = raw_entries.iter().map(|e| e.replace('\\', "/")).collect();
    let has = |name: &str| entries.contains(name);

    if lower.ends_with(".mrpack") || has("modrinth.index.json") {
        let json = read("modrinth.index.json").and_then(|t| lenient_json(&t));
        let mut d = Detected::plain(path, file_name, FileKind::ModrinthModpack);
        d.name = json_string(json.as_ref(), "name");
        d.version = json_string(json.as_ref(), "versionId");
        return d;
    }
    if lower.ends_with(".litemod") {
        let json = read("litemod.json").and_then(|t| lenient_json(&t));
        let mut d = Detected::plain(path, file_name, FileKind::Mod);
        d.loader = Some(LITELOADER.into());
        d.name = json_string(json.as_ref(), "name");
        d.version = json_string(json.as_ref(), "version");
        d.minecraft_versions = list_of(json_string(json.as_ref(), "mcversion"));
        d.info = ModInfo {
            loader: Some(LITELOADER.into()),
            loaders: vec![LITELOADER.into()],
            name: d.name.clone(),
            version: d.version.clone(),
            description: json_string(json.as_ref(), "description"),
            authors: json_string(json.as_ref(), "author").into_iter().collect(),
            minecraft_versions: d.minecraft_versions.clone(),
            ..Default::default()
        };
        return d;
    }
    if at_root_or_one_down(&entries, "mmc-pack.json") || at_root_or_one_down(&entries, "instance.cfg") {
        return Detected::plain(path, file_name, FileKind::PrismExport);
    }
    if has("manifest.json") {
        let json = read("manifest.json").and_then(|t| lenient_json(&t));
        if let Some(mc) = json.as_ref().and_then(|j| j.get("minecraft")).filter(|m| m.is_object()) {
            let mut d = Detected::plain(path, file_name, FileKind::CurseForgeModpack);
            d.name = json_string(json.as_ref(), "name");
            d.version = json_string(json.as_ref(), "version");
            d.minecraft_versions = list_of(json_string(Some(mc), "version"));
            return d;
        }
    }

    if let Some(info) = mod_info(&entries, read) {
        let mut d = Detected::plain(path, file_name, FileKind::Mod);
        d.loader = info.loader.clone();
        d.name = info.name.clone();
        d.version = info.version.clone();
        d.minecraft_versions = info.minecraft_versions.clone();
        d.info = info;
        return d;
    }
    if lower.ends_with(".jar") {
        // a jar without any loader's metadata: probably a library or a very old mod
        return Detected::plain(path, file_name, FileKind::Mod);
    }

    let mcmeta = has("pack.mcmeta");
    let (mut assets, mut data, mut shaders) = (false, false, false);
    for entry in &entries {
        assets |= entry.starts_with("assets/");
        data |= entry.starts_with("data/");
        let parts: Vec<&str> = entry.split('/').collect();
        if (parts.len() >= 2 && parts[0] == "shaders") || (parts.len() >= 3 && parts[1] == "shaders") {
            shaders = true;
        }
        if entry == "shaders/" {
            shaders = true;
        }
    }
    if shaders && !assets {
        return Detected::plain(path, file_name, FileKind::ShaderPack);
    }
    if mcmeta {
        let description = pack_description(read("pack.mcmeta").as_deref());
        let kind = if data && !assets { FileKind::DataPack } else { FileKind::ResourcePack };
        let mut d = Detected::plain(path, file_name, kind);
        // the Java edition shows a pack's description where a mod has its version
        d.version = description.clone();
        d.description = description.clone();
        d.info.description = description;
        d.info.icon_path = has("pack.png").then(|| "pack.png".to_string());
        return d;
    }
    Detected::plain(path, file_name, FileKind::Unknown)
}

fn at_root_or_one_down(entries: &BTreeSet<String>, name: &str) -> bool {
    entries.iter().any(|e| {
        if e == name {
            return true;
        }
        let parts: Vec<&str> = e.split('/').collect();
        parts.len() == 2 && parts[1] == name
    })
}

/// Reads whichever loader's metadata a mod has; `None` if it has none.
fn mod_info(entries: &BTreeSet<String>, read: &mut dyn FnMut(&str) -> Option<String>) -> Option<ModInfo> {
    let has = |name: &str| entries.contains(name);
    let mut found: Vec<ModInfo> = Vec::new();

    if has("quilt.mod.json") {
        found.push(quilt_info(read("quilt.mod.json").as_deref()));
    }
    if has("fabric.mod.json") {
        found.push(fabric_info(read("fabric.mod.json").as_deref()));
    }
    if has("META-INF/neoforge.mods.toml") {
        found.push(toml_info(read("META-INF/neoforge.mods.toml").as_deref(), NEOFORGE, read));
    }
    if has("META-INF/mods.toml") {
        found.push(toml_info(read("META-INF/mods.toml").as_deref(), FORGE, read));
    }
    if has("mcmod.info") {
        found.push(mcmod_info(read("mcmod.info").as_deref()));
    }
    let mut first = found.first()?.clone();
    first.loaders = Vec::new();
    for f in &found {
        if let Some(l) = &f.loader {
            if !first.loaders.contains(l) {
                first.loaders.push(l.clone());
            }
        }
    }
    // an icon is found by its own file name, so a jar that has several loaders' files still gets one
    if first.icon_path.is_none() {
        first.icon_path = found.iter().find_map(|f| f.icon_path.clone());
    }
    Some(first)
}

fn fabric_info(text: Option<&str>) -> ModInfo {
    let json = text.and_then(lenient_json);
    let j = json.as_ref();
    let mut info = ModInfo { loader: Some(FABRIC.into()), ..Default::default() };
    info.id = json_string(j, "id");
    info.name = json_string(j, "name");
    info.version = clean(json_string(j, "version"));
    info.description = json_string(j, "description");
    if let Some(authors) = j.and_then(|j| j.get("authors")).and_then(Value::as_array) {
        for a in authors {
            match a {
                Value::String(s) => info.authors.push(s.clone()),
                Value::Object(_) => info.authors.extend(json_string(Some(a), "name")),
                _ => {}
            }
        }
    }
    info.license = match j.and_then(|j| j.get("license")) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(a)) => {
            Some(a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).filter(|s| !s.is_empty())
        }
        _ => None,
    };
    if let Some(c) = j.and_then(|j| j.get("contact")) {
        info.homepage = json_string(Some(c), "homepage");
        info.sources = json_string(Some(c), "sources");
        info.issues = json_string(Some(c), "issues");
    }
    info.icon_path = icon_of(j.and_then(|j| j.get("icon")));
    info.environment = json_string(j, "environment");
    if let Some(depends) = j.and_then(|j| j.get("depends")).and_then(Value::as_object) {
        info.minecraft_versions = version_strings(depends.get("minecraft"));
        info.depends = depends
            .keys()
            .filter(|k| !matches!(k.as_str(), "minecraft" | "java" | "fabricloader" | "fabric-loader"))
            .cloned()
            .collect();
    }
    info
}

fn quilt_info(text: Option<&str>) -> ModInfo {
    let json = text.and_then(lenient_json);
    let loader = json.as_ref().and_then(|j| j.get("quilt_loader")).filter(|l| l.is_object());
    let meta = loader.and_then(|l| l.get("metadata")).filter(|m| m.is_object());
    let mut info = ModInfo { loader: Some(QUILT.into()), ..Default::default() };
    info.id = json_string(loader, "id");
    info.name = json_string(meta, "name");
    info.version = clean(json_string(loader, "version"));
    info.description = json_string(meta, "description");
    if let Some(c) = meta.and_then(|m| m.get("contributors")).and_then(Value::as_object) {
        info.authors = c.keys().cloned().collect();
    }
    info.license = match meta.and_then(|m| m.get("license")) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Object(o)) => o.get("id").and_then(Value::as_str).map(str::to_string),
        Some(Value::Array(a)) => Some(
            a.iter()
                .filter_map(|l| l.as_str().map(str::to_string).or_else(|| json_string(Some(l), "id")))
                .collect::<Vec<_>>()
                .join(", "),
        )
        .filter(|s| !s.is_empty()),
        _ => None,
    };
    if let Some(c) = meta.and_then(|m| m.get("contact")) {
        info.homepage = json_string(Some(c), "homepage");
        info.sources = json_string(Some(c), "sources");
        info.issues = json_string(Some(c), "issues");
    }
    info.icon_path = icon_of(meta.and_then(|m| m.get("icon")));
    if let Some(deps) = loader.and_then(|l| l.get("depends")).and_then(Value::as_array) {
        for dep in deps {
            let id = match dep {
                Value::String(s) => Some(s.clone()),
                Value::Object(_) => json_string(Some(dep), "id"),
                _ => None,
            };
            let Some(id) = id else { continue };
            if id == "minecraft" {
                info.minecraft_versions.extend(version_strings(dep.get("versions")));
            } else if !matches!(id.as_str(), "quilt_loader" | "java" | "fabricloader") {
                info.depends.push(id);
            }
        }
    }
    info
}

fn mcmod_info(text: Option<&str>) -> ModInfo {
    let root = text.and_then(lenient_json);
    let first = match &root {
        Some(Value::Array(a)) => a.first().filter(|v| v.is_object()),
        Some(Value::Object(o)) => {
            o.get("modList").and_then(Value::as_array).and_then(|a| a.first()).filter(|v| v.is_object())
        }
        _ => None,
    };
    let mut info = ModInfo { loader: Some(FORGE.into()), ..Default::default() };
    info.id = json_string(first, "modid");
    info.name = json_string(first, "name");
    info.version = clean(json_string(first, "version"));
    info.description = json_string(first, "description");
    for key in ["authorList", "authors"] {
        if let Some(a) = first.and_then(|f| f.get(key)).and_then(Value::as_array) {
            info.authors.extend(a.iter().filter_map(Value::as_str).map(str::to_string));
            break;
        }
    }
    info.homepage = json_string(first, "url").filter(|u| !u.is_empty());
    info.icon_path = json_string(first, "logoFile").filter(|u| !u.is_empty());
    info.minecraft_versions = list_of(clean(json_string(first, "mcversion")));
    info
}

/// Forge and NeoForge describe mods in TOML. A Forge-style file that depends on NeoForge is a NeoForge mod.
fn toml_info(text: Option<&str>, loader: &str, read: &mut dyn FnMut(&str) -> Option<String>) -> ModInfo {
    let mut info = ModInfo::default();
    let mut depends_on_neoforge = false;
    let mut parsed = false;
    if let Some(text) = text {
        let clean_text = text.strip_prefix('\u{feff}').unwrap_or(text);
        if let Ok(table) = clean_text.parse::<toml::Table>() {
            parsed = true;
            toml_from_table(&table, &mut info, &mut depends_on_neoforge);
        }
        if !parsed || info.name.is_none() && info.version.is_none() {
            // not valid TOML, or an unusual shape: fall back to reading it line by line
            let mut line_info = ModInfo::default();
            let mut neo = false;
            toml_from_lines(text, &mut line_info, &mut neo);
            if !parsed {
                info = line_info;
                depends_on_neoforge = neo;
            } else {
                info.name = info.name.or(line_info.name);
                info.version = info.version.or(line_info.version);
                depends_on_neoforge |= neo;
            }
        }
    }
    // Forge fills this placeholder in from the jar's manifest when it builds a mod
    if info.version.as_deref().is_some_and(|v| v.contains("${file.jarVersion}")) {
        info.version = clean_manifest_version(read("META-INF/MANIFEST.MF").as_deref());
    }
    info.version = clean(info.version.take());
    info.name = info.name.filter(|n| !n.contains("${"));
    info.loader = Some(if depends_on_neoforge { NEOFORGE } else { loader }.to_string());
    info
}

fn clean_manifest_version(manifest: Option<&str>) -> Option<String> {
    let manifest = manifest?;
    manifest
        .lines()
        .find_map(|l| l.strip_prefix("Implementation-Version:").map(|v| v.trim().to_string()))
        .filter(|v| !v.is_empty())
}

fn toml_from_table(table: &toml::Table, info: &mut ModInfo, depends_on_neoforge: &mut bool) {
    info.license = table.get("license").and_then(toml::Value::as_str).map(str::to_string);
    info.issues = table.get("issueTrackerURL").and_then(toml::Value::as_str).map(str::to_string);
    let top_logo = table.get("logoFile").and_then(toml::Value::as_str).map(str::to_string);
    let top_authors = table.get("authors").and_then(toml::Value::as_str).map(str::to_string);
    if let Some(mods) = table.get("mods").and_then(toml::Value::as_array) {
        if let Some(first) = mods.iter().find_map(toml::Value::as_table) {
            let s = |k: &str| first.get(k).and_then(toml::Value::as_str).map(str::to_string);
            info.id = s("modId");
            info.name = s("displayName");
            info.version = s("version");
            info.description = s("description").map(|d| d.trim().to_string()).filter(|d| !d.is_empty());
            info.homepage = s("displayURL");
            info.icon_path = s("logoFile").or(top_logo).filter(|l| !l.is_empty());
            if let Some(a) = s("authors").or(top_authors) {
                info.authors = a.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
            }
            if let Some(i) = s("issueTrackerURL") {
                info.issues = Some(i);
            }
        }
    }
    if let Some(deps) = table.get("dependencies").and_then(toml::Value::as_table) {
        for list in deps.values() {
            let Some(list) = list.as_array() else { continue };
            for dep in list.iter().filter_map(toml::Value::as_table) {
                let id = dep.get("modId").and_then(toml::Value::as_str).unwrap_or_default();
                let range = dep.get("versionRange").and_then(toml::Value::as_str);
                match id {
                    "minecraft" => info.minecraft_versions.extend(range.map(str::to_string)),
                    "neoforge" => *depends_on_neoforge = true,
                    "forge" | "java" | "" => {}
                    other => {
                        // optional and unordered dependencies are not needed to run
                        let mandatory = dep.get("mandatory").and_then(toml::Value::as_bool).unwrap_or(true)
                            && dep.get("type").and_then(toml::Value::as_str).is_none_or(|t| t == "required");
                        if mandatory && !info.depends.iter().any(|d| d == other) {
                            info.depends.push(other.to_string());
                        }
                    }
                }
            }
        }
    }
}

/// The Java edition's reading of the TOML: only the display name, the version and the Minecraft dependency.
fn toml_from_lines(text: &str, info: &mut ModInfo, depends_on_neoforge: &mut bool) {
    let mut table = String::new();
    let (mut dep_mod_id, mut dep_range): (Option<String>, Option<String>) = (None, None);
    let flush = |dep_mod_id: &Option<String>, dep_range: &Option<String>, info: &mut ModInfo, neo: &mut bool| {
        if dep_mod_id.as_deref() == Some("minecraft") {
            if let Some(r) = dep_range {
                info.minecraft_versions.push(r.clone());
            }
        }
        if dep_mod_id.as_deref() == Some("neoforge") {
            *neo = true;
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            flush(&dep_mod_id, &dep_range, info, depends_on_neoforge);
            dep_mod_id = None;
            dep_range = None;
            table = line.replace(['[', ']'], "").trim().to_string();
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim();
        let value = unquote(line[eq + 1..].trim());
        if table == "mods" {
            if key == "displayName" && info.name.is_none() {
                info.name = Some(value);
            } else if key == "version" && info.version.is_none() {
                info.version = Some(value);
            }
        } else if table.starts_with("dependencies") {
            if key == "modId" {
                dep_mod_id = Some(value);
            } else if key == "versionRange" {
                dep_range = Some(value);
            }
        }
    }
    flush(&dep_mod_id, &dep_range, info, depends_on_neoforge);
}

fn unquote(value: &str) -> String {
    let mut v = value.to_string();
    if let Some(hash) = v.find(" #") {
        if hash > 0 && !v.starts_with('"') && !v.starts_with('\'') {
            v = v[..hash].trim().to_string();
        }
    }
    let first = v.chars().next();
    if v.len() >= 2 && matches!(first, Some('"') | Some('\'')) {
        let quote = first.unwrap_or('"');
        return match v[1..].find(quote) {
            Some(end) => v[1..1 + end].to_string(),
            None => v[1..].to_string(),
        };
    }
    v
}

/// Placeholders like `${version}` are filled in when a mod is built; one left in means the value is unknown.
fn clean(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.contains("${") && !v.trim().is_empty()).map(|v| v.trim().to_string())
}

fn list_of(value: Option<String>) -> Vec<String> {
    value.into_iter().collect()
}

fn version_strings(element: Option<&Value>) -> Vec<String> {
    match element {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// Fabric and Quilt give the icon as one path, or a size-to-path map; the biggest is used.
fn icon_of(icon: Option<&Value>) -> Option<String> {
    match icon? {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o
            .iter()
            .filter_map(|(k, v)| Some((k.parse::<u32>().unwrap_or(0), v.as_str()?)))
            .max_by_key(|(size, _)| *size)
            .map(|(_, p)| p.to_string()),
        _ => None,
    }
    .filter(|p| !p.is_empty())
}

fn pack_description(mcmeta: Option<&str>) -> Option<String> {
    let json = mcmeta.and_then(lenient_json)?;
    let d = json.get("pack")?.get("description")?;
    text_component(d).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// The plain text of a Minecraft text component (a string, an object with `text` and `extra`, or a list of them).
fn text_component(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(strip_formatting(s)),
        Value::Array(a) => Some(a.iter().filter_map(text_component).collect::<Vec<_>>().join("")),
        Value::Object(o) => {
            let mut out = o.get("text").and_then(Value::as_str).map(strip_formatting).unwrap_or_default();
            if let Some(extra) = o.get("extra") {
                out.push_str(&text_component(extra).unwrap_or_default());
            }
            Some(out)
        }
        _ => None,
    }
}

/// Removes the section-sign colour codes some packs put in their descriptions.
fn strip_formatting(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{a7}' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

fn json_string(json: Option<&Value>, key: &str) -> Option<String> {
    match json?.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Parses JSON the way mods write it, not the way the standard says it should be: a byte order mark, comments,
/// trailing commas and raw line breaks inside strings (old `mcmod.info` files have all of them) are accepted.
pub fn lenient_json(text: &str) -> Option<Value> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return Some(v);
    }
    serde_json::from_str(&sanitize_json(text)).ok()
}

fn sanitize_json(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut in_string) = (0usize, false);
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            match c {
                '\\' => {
                    out.push(c);
                    if let Some(n) = chars.get(i + 1) {
                        out.push(*n);
                        i += 1;
                    }
                }
                '"' => {
                    in_string = false;
                    out.push(c);
                }
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push(' '),
                c => out.push(c),
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
            ',' => {
                // a comma right before a closing bracket is a trailing comma
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if !matches!(chars.get(j), Some(']') | Some('}')) {
                    out.push(c);
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------------------------------------
// Reading a file on disk

/// Reads the metadata of a jar or zip on disk. Anything that can't be read gives an empty result.
pub fn read_info(file: &Path) -> ModInfo {
    let d = detect(file);
    let mut info = d.info;
    if info.loader.is_none() {
        info.loader = d.loader;
    }
    if info.name.is_none() {
        info.name = d.name;
    }
    if info.version.is_none() {
        info.version = d.version;
    }
    if info.description.is_none() {
        info.description = d.description;
    }
    info
}

/// A mod's picture: the logo named in its metadata, or the usual file names when it doesn't name one.
pub fn read_icon(file: &Path, info: &ModInfo) -> Option<Vec<u8>> {
    let f = std::fs::File::open(file).ok()?;
    let mut zip = zip::ZipArchive::new(f).ok()?;
    let mut candidates: Vec<String> = Vec::new();
    if let Some(p) = &info.icon_path {
        candidates.push(p.trim_start_matches('/').to_string());
    }
    candidates.push("pack.png".into());
    candidates.push("icon.png".into());
    candidates.push("logo.png".into());
    if let Some(id) = &info.id {
        candidates.push(format!("assets/{id}/icon.png"));
        candidates.push(format!("assets/{id}/logo.png"));
    }
    candidates
        .into_iter()
        .find_map(|c| read_bytes(&mut zip, &c).filter(|b| b.len() > 8 && b.starts_with(&[0x89, b'P', b'N', b'G'])))
}

/// Which folder a file in a Minecraft folder goes in, from its name and contents (the routing used when a file is
/// dropped on a tab). `entries` is `None` when the file isn't a zip that could be read.
pub fn route(file_name: &str, entries: Option<&[String]>, current: ContentKind) -> Option<ContentKind> {
    let name = file_name.to_lowercase();
    if name.ends_with(".jar") || name.ends_with(".litemod") {
        return Some(ContentKind::Mods);
    }
    if !name.ends_with(".zip") {
        return None;
    }
    if let Some(entries) = entries {
        let found = classify(Path::new(""), file_name, entries, &mut |_| None);
        if found.kind != FileKind::Unknown {
            return found.kind.content_kind();
        }
    }
    // an unrecognised zip only makes sense in a pack folder, and only if that's where it was dropped
    if current == ContentKind::Mods { None } else { Some(current) }
}

/// The names inside a zip file, or `None` if it isn't a zip that can be read.
pub fn zip_entries(file: &Path) -> Option<Vec<String>> {
    let f = std::fs::File::open(file).ok()?;
    let zip = zip::ZipArchive::new(f).ok()?;
    Some(zip.file_names().take(MAX_ENTRIES).filter_map(|n| n.ok().map(|c| c.to_string())).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Writes a zip with the given entries; an entry ending in `/` is a folder.
    fn zip_file(dir: &Path, name: &str, entries: &[(&str, &str)]) -> PathBuf {
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

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("blockforge-meta-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fabric_mod_with_name_version_and_minecraft_range() {
        let dir = temp("fabric");
        let f = detect(&zip_file(
            &dir,
            "sodium.jar",
            &[
                (
                    "fabric.mod.json",
                    r#"{"schemaVersion":1,"id":"sodium","name":"Sodium","version":"0.9.2","authors":["jellysquid3",{"name":"IMS"}],
                    "icon":{"16":"a16.png","128":"a128.png"},"depends":{"minecraft":">=26.3 <26.4","fabricloader":">=0.16","fabric-api":"*"}}"#,
                ),
                ("net/caffeinemc/Sodium.class", ""),
            ],
        ));
        assert_eq!(f.kind, FileKind::Mod);
        assert_eq!(f.loader.as_deref(), Some(FABRIC));
        assert_eq!(f.name.as_deref(), Some("Sodium"));
        assert_eq!(f.version.as_deref(), Some("0.9.2"));
        assert_eq!(f.minecraft_versions, vec![">=26.3 <26.4".to_string()]);
        assert_eq!(f.info.authors, vec!["jellysquid3", "IMS"]);
        assert_eq!(f.info.icon_path.as_deref(), Some("a128.png"));
        assert_eq!(f.info.depends, vec!["fabric-api"]);
        assert_eq!(f.problem_for(Some(FABRIC), "26.3"), None);
        assert_eq!(f.problem_for(Some(QUILT), "26.3"), None);
        assert!(f.problem_for(Some(FABRIC), "26.2").unwrap().contains("Minecraft"));
        assert!(f.problem_for(Some(NEOFORGE), "26.3").unwrap().contains("Fabric"));
        assert!(f.problem_for(None, "26.3").unwrap().contains("loader"));
    }

    #[test]
    fn fabric_version_lists_mean_any_of_them() {
        let dir = temp("multi");
        let f = detect(&zip_file(
            &dir,
            "multi.jar",
            &[(
                "fabric.mod.json",
                r#"{"name":"Multi","version":"${version}","depends":{"minecraft":["1.20.1","1.20.4"]}}"#,
            )],
        ));
        assert_eq!(f.version, None);
        assert_eq!(f.minecraft_versions, vec!["1.20.1", "1.20.4"]);
        assert_eq!(f.problem_for(Some(FABRIC), "1.20.4"), None);
        assert!(f.problem_for(Some(FABRIC), "1.21").unwrap().contains("1.20.1 or 1.20.4"));
    }

    #[test]
    fn quilt_mod() {
        let dir = temp("quilt");
        let f = detect(&zip_file(
            &dir,
            "q.jar",
            &[(
                "quilt.mod.json",
                r#"{"quilt_loader":{"id":"q","version":"2.0","metadata":{"name":"Quilty","contributors":{"Ann":"Owner"}},
                "depends":["quilt_loader",{"id":"minecraft","versions":">=1.20"}]}}"#,
            )],
        ));
        assert_eq!(f.kind, FileKind::Mod);
        assert_eq!(f.loader.as_deref(), Some(QUILT));
        assert_eq!(f.name.as_deref(), Some("Quilty"));
        assert_eq!(f.version.as_deref(), Some("2.0"));
        assert_eq!(f.minecraft_versions, vec![">=1.20".to_string()]);
        assert_eq!(f.info.authors, vec!["Ann"]);
        assert!(f.problem_for(Some(FABRIC), "1.21.1").unwrap().contains("Quilt"));
    }

    #[test]
    fn neoforge_mod_from_its_own_toml() {
        let dir = temp("neo");
        let f = detect(&zip_file(
            &dir,
            "neo.jar",
            &[(
                "META-INF/neoforge.mods.toml",
                "modLoader=\"javafml\"\nloaderVersion=\"[4,)\"\n[[mods]]\nmodId=\"neo\"\nversion=\"1.4.0\" # comment\n\
                 displayName=\"Neo Thing\"\n[[dependencies.neo]]\nmodId=\"neoforge\"\ntype=\"required\"\n\
                 versionRange=\"[21.1,)\"\n[[dependencies.neo]]\nmodId=\"minecraft\"\ntype=\"required\"\n\
                 versionRange=\"[1.21.1,1.22)\"\n",
            )],
        ));
        assert_eq!(f.loader.as_deref(), Some(NEOFORGE));
        assert_eq!(f.name.as_deref(), Some("Neo Thing"));
        assert_eq!(f.version.as_deref(), Some("1.4.0"));
        assert_eq!(f.minecraft_versions, vec!["[1.21.1,1.22)".to_string()]);
        assert_eq!(f.problem_for(Some(NEOFORGE), "1.21.1"), None);
        assert!(f.problem_for(Some(NEOFORGE), "1.20.1").unwrap().contains("Minecraft"));
    }

    #[test]
    fn forge_mods_toml_and_neoforge_ones_that_still_use_it() {
        let dir = temp("forge");
        let forge_toml = "[[mods]]\nmodId='jei'\nversion='${file.jarVersion}'\ndisplayName='Just Enough Items'\n\
            [[dependencies.jei]]\nmodId='forge'\nversionRange='[47,)'\n\
            [[dependencies.jei]]\nmodId='minecraft'\nversionRange='[1.20.1]'\n";
        let forge = detect(&zip_file(&dir, "jei.jar", &[("META-INF/mods.toml", forge_toml)]));
        assert_eq!(forge.loader.as_deref(), Some(FORGE));
        assert_eq!(forge.name.as_deref(), Some("Just Enough Items"));
        assert_eq!(forge.version, None);
        assert_eq!(forge.problem_for(Some(FORGE), "1.20.1"), None);
        // NeoForge ran Forge mods on 1.20.1
        assert_eq!(forge.problem_for(Some(NEOFORGE), "1.20.1"), None);
        assert!(forge.problem_for(Some(FABRIC), "1.20.1").unwrap().contains("Forge"));

        let old_neo = detect(&zip_file(
            &dir,
            "oldneo.jar",
            &[(
                "META-INF/mods.toml",
                "[[mods]]\nmodId=\"x\"\n[[dependencies.x]]\nmodId=\"neoforge\"\nversionRange=\"[20.4,)\"\n",
            )],
        ));
        assert_eq!(old_neo.loader.as_deref(), Some(NEOFORGE));
    }

    #[test]
    fn the_jar_version_placeholder_comes_from_the_manifest() {
        let dir = temp("manifest");
        let f = detect(&zip_file(
            &dir,
            "m.jar",
            &[
                ("META-INF/mods.toml", "[[mods]]\nmodId=\"m\"\nversion=\"${file.jarVersion}\"\ndisplayName=\"M\"\n"),
                ("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\nImplementation-Version: 2.5.1\n"),
            ],
        ));
        assert_eq!(f.version.as_deref(), Some("2.5.1"));
    }

    #[test]
    fn invalid_toml_is_read_line_by_line() {
        let dir = temp("badtoml");
        let f = detect(&zip_file(
            &dir,
            "bad.jar",
            &[(
                "META-INF/mods.toml",
                "[[mods]]\nmodId=\"b\"\ndisplayName=\"Broken\"\nversion=\"1.0\"\nthis is = = not toml\n",
            )],
        ));
        assert_eq!(f.name.as_deref(), Some("Broken"));
        assert_eq!(f.version.as_deref(), Some("1.0"));
    }

    #[test]
    fn old_forge_mod_with_mcmod_info() {
        let dir = temp("old");
        let f = detect(&zip_file(
            &dir,
            "old.jar",
            &[(
                "mcmod.info",
                "[{\"modid\":\"old\",\"name\":\"Old Mod\",\"version\":\"1.0\",\"mcversion\":\"1.7.10\",\"description\":\"line one\nline two\"}]",
            )],
        ));
        assert_eq!(f.kind, FileKind::Mod);
        assert_eq!(f.loader.as_deref(), Some(FORGE));
        assert_eq!(f.name.as_deref(), Some("Old Mod"));
        assert_eq!(f.minecraft_versions, vec!["1.7.10".to_string()]);
        // a raw line break inside a string is accepted
        assert_eq!(f.info.description.as_deref(), Some("line one\nline two"));
        assert!(f.problem_for(Some(FORGE), "1.12.2").unwrap().contains("1.7.10"));
    }

    #[test]
    fn lite_loader_and_plain_jars() {
        let dir = temp("lite");
        let lite = detect(&zip_file(
            &dir,
            "voxel.litemod",
            &[("litemod.json", r#"{"name":"VoxelMap","version":"1.7","mcversion":"1.12.2"}"#)],
        ));
        assert_eq!(lite.kind, FileKind::Mod);
        assert_eq!(lite.loader.as_deref(), Some(LITELOADER));
        assert!(lite.problem_for(Some(FORGE), "1.12.2").unwrap().contains("LiteLoader"));

        let library = detect(&zip_file(&dir, "library.jar", &[("a/B.class", "")]));
        assert_eq!(library.kind, FileKind::Mod);
        assert_eq!(library.loader, None);
        assert_eq!(library.problem_for(Some(FABRIC), "26.3"), None);
    }

    #[test]
    fn resource_data_and_shader_packs() {
        let dir = temp("packs");
        let rp = detect(&zip_file(
            &dir,
            "Faithful.zip",
            &[
                ("pack.mcmeta", r#"{"pack":{"pack_format":46,"description":"Faithful 32x"}}"#),
                ("assets/minecraft/textures/a.png", ""),
            ],
        ));
        assert_eq!(rp.kind, FileKind::ResourcePack);
        assert_eq!(rp.kind.content_kind(), Some(ContentKind::ResourcePacks));
        assert_eq!(rp.version.as_deref(), Some("Faithful 32x"));

        let dp = detect(&zip_file(
            &dir,
            "Terralith.zip",
            &[("pack.mcmeta", "{}"), ("data/terralith/worldgen/x.json", "{}")],
        ));
        assert_eq!(dp.kind, FileKind::DataPack);
        assert_eq!(dp.kind.content_kind(), None);
        assert!(dp.kind.goes_into_instance());

        let flat = detect(&zip_file(&dir, "BSL.zip", &[("shaders/", ""), ("shaders/composite.fsh", "void main(){}")]));
        assert_eq!(flat.kind, FileKind::ShaderPack);
        let wrapped = detect(&zip_file(
            &dir,
            "Complementary.zip",
            &[("Complementary/", ""), ("Complementary/shaders/final.fsh", "")],
        ));
        assert_eq!(wrapped.kind, FileKind::ShaderPack);
        assert_eq!(wrapped.kind.content_kind(), Some(ContentKind::Shaders));
        assert_eq!(wrapped.problem_for(Some(FABRIC), "26.3"), None);
        assert!(wrapped.problem_for(None, "26.3").unwrap().contains("Iris"));
    }

    #[test]
    fn pack_descriptions_can_be_text_components() {
        assert_eq!(
            pack_description(Some(
                r#"{"pack":{"description":[{"text":"Fancy ","color":"red"},{"text":"Pack","extra":[{"text":"!"}]}]}}"#
            ))
            .as_deref(),
            Some("Fancy Pack!")
        );
        assert_eq!(pack_description(Some("{\"pack\":{\"description\":\"\u{a7}6Gold\"}}")).as_deref(), Some("Gold"));
    }

    #[test]
    fn modpacks_of_every_format() {
        let dir = temp("modpacks");
        let mrpack = detect(&zip_file(
            &dir,
            "Fabulously Optimized.mrpack",
            &[("modrinth.index.json", r#"{"formatVersion":1,"name":"Fabulously Optimized","versionId":"6.0"}"#)],
        ));
        assert_eq!(mrpack.kind, FileKind::ModrinthModpack);
        assert_eq!(mrpack.name.as_deref(), Some("Fabulously Optimized"));
        assert!(mrpack.kind.is_modpack());
        assert!(!mrpack.kind.goes_into_instance());

        let mrzip = detect(&zip_file(&dir, "pack.zip", &[("modrinth.index.json", "{}")]));
        assert_eq!(mrzip.kind, FileKind::ModrinthModpack);

        let curse = detect(&zip_file(
            &dir,
            "All the Mods.zip",
            &[
                (
                    "manifest.json",
                    r#"{"minecraft":{"version":"1.20.1","modLoaders":[{"id":"forge-47.2.0"}]},"name":"All the Mods 9","version":"0.2","files":[]}"#,
                ),
                ("overrides/config/x.toml", ""),
            ],
        ));
        assert_eq!(curse.kind, FileKind::CurseForgeModpack);
        assert_eq!(curse.name.as_deref(), Some("All the Mods 9"));

        let prism = detect(&zip_file(
            &dir,
            "Exported.zip",
            &[("My Pack/", ""), ("My Pack/mmc-pack.json", "{}"), ("My Pack/instance.cfg", "name=My Pack")],
        ));
        assert_eq!(prism.kind, FileKind::PrismExport);
        assert_eq!(detect(&zip_file(&dir, "Old.zip", &[("instance.cfg", "name=x")])).kind, FileKind::PrismExport);
        // a manifest.json that isn't CurseForge's is not a modpack
        assert_eq!(detect(&zip_file(&dir, "other.zip", &[("manifest.json", "{\"a\":1}")])).kind, FileKind::Unknown);
    }

    #[test]
    fn unrecognised_files_are_unknown() {
        let dir = temp("unknown");
        let text = dir.join("notes.txt");
        std::fs::write(&text, "hi").unwrap();
        assert_eq!(detect(&text).kind, FileKind::Unknown);
        let broken = dir.join("broken.jar");
        std::fs::write(&broken, [1, 2, 3]).unwrap();
        assert_eq!(detect(&broken).kind, FileKind::Unknown);
        assert_eq!(detect(&zip_file(&dir, "random.zip", &[("photos/cat.jpg", "x")])).kind, FileKind::Unknown);
    }

    #[test]
    fn bad_metadata_still_counts_as_a_mod() {
        let dir = temp("badmeta");
        let f = detect(&zip_file(&dir, "broken-meta.jar", &[("fabric.mod.json", "{ not json")]));
        assert_eq!(f.kind, FileKind::Mod);
        assert_eq!(f.loader.as_deref(), Some(FABRIC));
        assert_eq!(f.name, None);
        assert_eq!(f.display_name(), "broken-meta.jar");
    }

    #[test]
    fn jars_with_several_loaders_run_on_each() {
        let dir = temp("both");
        let f = detect(&zip_file(
            &dir,
            "DistantHorizons-fabric-neoforge.jar",
            &[
                ("fabric.mod.json", r#"{"name":"DH","version":"1"}"#),
                ("META-INF/neoforge.mods.toml", "[[mods]]\nmodId=\"dh\"\ndisplayName=\"DH\"\nversion=\"1\"\n"),
            ],
        ));
        assert_eq!(f.loader.as_deref(), Some(FABRIC));
        assert_eq!(f.info.loaders, vec![FABRIC, NEOFORGE]);
        assert_eq!(f.problem_for(Some(NEOFORGE), "26.3"), None);
        assert_eq!(f.problem_for(Some(FABRIC), "26.3"), None);
        assert!(f.problem_for(Some(FORGE), "26.3").is_some());
    }

    #[test]
    fn routes_files_to_a_folder() {
        let dir = temp("route");
        assert_eq!(route("a.jar", None, ContentKind::Shaders), Some(ContentKind::Mods));
        assert_eq!(route("notes.txt", None, ContentKind::Mods), None);
        let rp = zip_file(&dir, "r.zip", &[("pack.mcmeta", "{}"), ("assets/x", "")]);
        assert_eq!(route("r.zip", zip_entries(&rp).as_deref(), ContentKind::Mods), Some(ContentKind::ResourcePacks));
        assert_eq!(route("mystery.zip", None, ContentKind::Mods), None);
        assert_eq!(route("mystery.zip", None, ContentKind::Shaders), Some(ContentKind::Shaders));
        let dp = zip_file(&dir, "d.zip", &[("pack.mcmeta", "{}"), ("data/x", "")]);
        assert_eq!(route("d.zip", zip_entries(&dp).as_deref(), ContentKind::ResourcePacks), None);
    }

    #[test]
    fn lenient_json_accepts_what_mods_write() {
        assert!(lenient_json("\u{feff}{\"a\":1}").is_some());
        assert_eq!(lenient_json("{\"a\":1, // note\n \"b\":[1,2,],}").unwrap()["b"][1], 2);
        assert_eq!(lenient_json("/* c */ {\"a\":\"x\ty\"}").unwrap()["a"], "x\ty");
        assert!(lenient_json("{ nope").is_none());
    }
}
