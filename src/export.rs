//! Exporting an instance as a modpack other people can import: Modrinth (`.mrpack`), CurseForge (`.zip`), both,
//! or MultiMC / Prism (`.zip`). Mods that Modrinth or CurseForge know are listed by their ids and hashes so the
//! archive stays small; everything else the player picked goes into the archive itself (`overrides/`).
//!
//! The formats match what the Java edition writes (same files, fields and names), and the `lastExport*`
//! fields in the `launcher` block remember the dialog's answers for next time.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};

use crate::cf_seam::CurseForgeSource;
use crate::config::Settings;
use crate::imports::multimc::{self, Properties};
use crate::install::Progress;
use crate::instances::Instance;
use crate::modcheck;
use crate::modrecord::ModRecord;
use crate::paths::{self, Dirs};

// ---------------------------------------------------------------------------------------------------------
// Choices

/// Which format to export as (`InstanceExportFormat`). The names are the ones the Java edition saves in
/// `defaultExportFormat`.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExportFormat {
    #[serde(rename = "CURSEFORGE")]
    CurseForge,
    #[serde(rename = "MODRINTH")]
    Modrinth,
    #[serde(rename = "CURSEFORGE_AND_MODRINTH")]
    CurseForgeAndModrinth,
    #[serde(rename = "MULTIMC")]
    MultiMc,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 4] =
        [ExportFormat::CurseForge, ExportFormat::Modrinth, ExportFormat::CurseForgeAndModrinth, ExportFormat::MultiMc];

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::CurseForge => "CurseForge",
            ExportFormat::Modrinth => "Modrinth",
            ExportFormat::CurseForgeAndModrinth => "CurseForge & Modrinth",
            ExportFormat::MultiMc => "MultiMC",
        }
    }

    /// The format the launcher setting `defaultExportFormat` names (Modrinth when unset, as in Java).
    pub fn from_settings(settings: &Settings) -> ExportFormat {
        settings
            .extras
            .get("defaultExportFormat")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or(ExportFormat::Modrinth)
    }
}

/// What the player decides in the export dialog.
#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub name: String,
    pub version: String,
    pub author: String,
    pub format: ExportFormat,
    /// The folder the archive is written to.
    pub save_to: PathBuf,
    /// Paths (relative to the instance, `/`-separated) of the folders and files to include.
    pub overrides: Vec<String>,
}

/// Folders ticked by default, compared without regard to case.
const DEFAULT_FOLDERS: [&str; 8] =
    ["config", "mods", "oresources", "resourcepacks", "shaderpacks", "datapacks", "resources", "scripts"];

/// Names at the top of an instance that are never offered (`disabledmods` and the instance's own files).
const HIDDEN: [&str; 4] = ["disabledmods", "instance.json", ".fabric", ".quilt"];

/// Whether the instance has a mod loader, which every export format needs (`Instance.canBeExported`).
pub fn can_be_exported(instance: &Instance) -> bool {
    instance.file.launcher.loader_version.as_ref().is_some_and(|l| !l.kind.is_empty())
}

/// The dialog's starting values: what was used last time, else the instance's own name and version, the
/// account's name, the instance folder, and the configured default format.
pub fn default_options(instance: &Instance, account_name: Option<&str>, settings: &Settings) -> ExportOptions {
    let l = &instance.file.launcher;
    let last = |k: &str| l.extras.get(k).and_then(Value::as_str).map(str::to_string);
    ExportOptions {
        name: last("lastExportName").unwrap_or_else(|| l.name.clone()),
        version: last("lastExportVersion").unwrap_or_else(|| l.version.clone()),
        author: last("lastExportAuthor").unwrap_or_else(|| account_name.unwrap_or("").to_string()),
        format: ExportFormat::from_settings(settings),
        save_to: last("lastExportSaveTo").map(PathBuf::from).unwrap_or_else(|| instance.root.clone()),
        overrides: default_overrides(instance),
    }
}

/// Remember the answers in `launcher.lastExport*` (the caller saves the instance).
pub fn remember(instance: &mut Instance, opts: &ExportOptions) {
    let e = &mut instance.file.launcher.extras;
    e.insert("lastExportName".into(), json!(opts.name));
    e.insert("lastExportVersion".into(), json!(opts.version));
    e.insert("lastExportAuthor".into(), json!(opts.author));
    e.insert("lastExportSaveTo".into(), json!(opts.save_to.to_string_lossy()));
}

// ---------------------------------------------------------------------------------------------------------
// The tree of files to include

/// One file or folder of the instance, as shown in the export tree.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeEntry {
    pub name: String,
    /// Relative to the instance, `/`-separated.
    pub rel: String,
    pub is_dir: bool,
    pub size: u64,
}

/// The entries of `rel` (the instance itself when empty), folders first. At the top, the launcher's own files
/// and empty folders are left out, like in the Java dialog.
pub fn list_dir(instance_root: &Path, rel: &str) -> Vec<TreeEntry> {
    let dir = if rel.is_empty() { instance_root.to_path_buf() } else { instance_root.join(rel) };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(ty) = e.file_type() else { continue };
        if ty.is_symlink() {
            continue;
        }
        if rel.is_empty() {
            if HIDDEN.iter().any(|h| h.eq_ignore_ascii_case(&name)) {
                continue;
            }
            if ty.is_dir() && std::fs::read_dir(e.path()).map(|mut r| r.next().is_none()).unwrap_or(true) {
                continue;
            }
        }
        out.push(TreeEntry {
            rel: if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") },
            name,
            is_dir: ty.is_dir(),
            size: if ty.is_file() { e.metadata().map(|m| m.len()).unwrap_or(0) } else { 0 },
        });
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// The folders ticked by default: `config`, `mods`, `resourcepacks`, `shaderpacks`, `datapacks` ... that exist.
pub fn default_overrides(instance: &Instance) -> Vec<String> {
    list_dir(&instance.root, "")
        .into_iter()
        .filter(|e| DEFAULT_FOLDERS.iter().any(|d| d.eq_ignore_ascii_case(&e.name)))
        .map(|e| e.rel)
        .collect()
}

/// Which paths are ticked in the tree. A ticked folder includes everything inside it; unticking something
/// inside a ticked folder splits the folder into its other entries.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    checked: BTreeSet<String>,
}

impl Selection {
    pub fn new(paths: impl IntoIterator<Item = String>) -> Selection {
        Selection { checked: paths.into_iter().collect() }
    }

    /// Whether `rel` is included (itself or through a ticked parent folder).
    pub fn is_checked(&self, rel: &str) -> bool {
        self.checked.iter().any(|c| c == rel || rel.strip_prefix(c.as_str()).is_some_and(|rest| rest.starts_with('/')))
    }

    /// Whether some but not all of what is inside `rel` is included.
    pub fn is_partial(&self, rel: &str) -> bool {
        !self.is_checked(rel)
            && self.checked.iter().any(|c| c.strip_prefix(rel).is_some_and(|rest| rest.starts_with('/')))
    }

    pub fn set(&mut self, instance_root: &Path, rel: &str, on: bool) {
        if on {
            // a ticked folder covers whatever was ticked inside it
            let prefix = format!("{rel}/");
            self.checked.retain(|c| !c.starts_with(&prefix));
            if !self.is_checked(rel) {
                self.checked.insert(rel.to_string());
            }
            return;
        }
        // unticking inside a ticked folder: replace the folder by its other entries
        while let Some(parent) = self
            .checked
            .iter()
            .find(|c| c.as_str() != rel && rel.strip_prefix(c.as_str()).is_some_and(|r| r.starts_with('/')))
            .cloned()
        {
            self.checked.remove(&parent);
            let mut cur = parent;
            for part in rel[cur.len() + 1..].split('/') {
                let next = format!("{cur}/{part}");
                for sibling in list_dir(instance_root, &cur) {
                    if sibling.rel != next {
                        self.checked.insert(sibling.rel);
                    }
                }
                cur = next;
            }
            // `cur` is now `rel` itself and is left out
        }
        self.checked.remove(rel);
        let prefix = format!("{rel}/");
        self.checked.retain(|c| !c.starts_with(&prefix));
    }

    /// The paths to hand to [`export`].
    pub fn to_overrides(&self) -> Vec<String> {
        self.checked.iter().cloned().collect()
    }
}

// ---------------------------------------------------------------------------------------------------------
// Scanning the instance's mods before an export

/// Make `launcher.mods` match the disk before an export: files that are in `mods/`, `disabledmods/`, `datapacks/`,
/// `resourcepacks/`, `shaderpacks/` or `jarmods/` but not in the list are added (and looked up on CurseForge
/// and Modrinth), records of mods whose file is gone are dropped, and duplicates are dropped
/// (`Instance.scanMissingMods`). Returns the number of files added.
pub async fn scan_missing_mods(
    instance: &mut Instance,
    cf: Option<&dyn CurseForgeSource>,
    check_modrinth: bool,
) -> Result<usize> {
    let root = instance.root.clone();
    let mc = instance.mc_version().to_string();
    let mut records = instance.mod_records();

    let mut added: Vec<ModRecord> = Vec::new();
    for (dir, kind) in [
        ("mods", "mods"),
        ("disabledmods", "mods"),
        ("datapacks", "datapack"),
        ("resourcepacks", "resourcepack"),
        ("shaderpacks", "shaderpack"),
        ("jarmods", "jar"),
    ] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
        files.sort();
        for p in files {
            let file = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            let lower = file.to_lowercase();
            if !(lower.ends_with(".jar") || lower.ends_with(".zip") || lower.ends_with(".litemod")) {
                continue;
            }
            if records.iter().chain(added.iter()).any(|r| r.kind == kind && r.file == file) {
                continue;
            }
            let (name, version, description) = crate::imports::detect::mod_metadata(&p);
            let mut rec = ModRecord::new(name.as_deref().unwrap_or(&file), &file, kind);
            rec.version = version.unwrap_or_else(|| "Unknown".into());
            rec.description = description.unwrap_or_default();
            rec.disabled = dir == "disabledmods";
            rec.user_added = true;
            added.push(rec);
        }
    }
    let count = added.len();
    if !added.is_empty() {
        if let Some(cf) = cf {
            let _ = modcheck::attach_curseforge(&root, &mc, &mut added, cf, true).await;
        }
        if check_modrinth {
            if modcheck::attach_modrinth(&root, &mc, &mut added, false).await.unwrap_or(0) > 0 {
                for rec in
                    added.iter_mut().filter(|r| modcheck::is_from_modrinth(r) && !modcheck::is_from_curseforge(r))
                {
                    if let Some(p) = &rec.modrinth_project {
                        rec.name = p["title"].as_str().unwrap_or(&rec.name).to_string();
                        rec.description = p["description"].as_str().unwrap_or(&rec.description).to_string();
                    }
                }
            }
        }
        records.extend(added);
    }

    // records whose file is gone, and duplicates
    let before = records.len();
    records.retain(|r| {
        if !r.was_selected || r.skipped || r.kind != "mods" {
            return true;
        }
        modcheck::current_file(&root, &mc, r).is_none_or(|f| f.exists())
    });
    let mut seen = HashSet::new();
    records.retain(|r| {
        if !r.was_selected || r.skipped || r.kind != "mods" {
            return true;
        }
        match modcheck::current_file(&root, &mc, r) {
            Some(f) => seen.insert(f),
            None => true,
        }
    });
    if count > 0 || records.len() != before {
        instance.set_mod_records(&records);
        instance.save()?;
    }
    Ok(count)
}

// ---------------------------------------------------------------------------------------------------------
// Exporting

/// What an export produced.
#[derive(Clone, Debug, Default)]
pub struct ExportOutcome {
    /// The archive that was written (the second one when two formats were asked for).
    pub path: PathBuf,
    /// Every archive written.
    pub paths: Vec<PathBuf>,
    /// Modrinth only: the jar and zip files that ended up inside the archive, one per line. Modrinth's reviewers
    /// ask for permission to distribute them; empty when there are none.
    pub overrides_note: String,
}

/// Characters that can't be in a file name on some system are dropped from the archive's name.
pub fn safe_file_name(name: &str) -> String {
    name.chars().filter(|c| !matches!(c, '\\' | '"' | '?' | ':' | '*' | '<' | '>' | '|')).collect()
}

/// Export `instance`. The mods Modrinth and CurseForge recognise are looked up first (and remembered in the
/// instance's records), the archive is written next to nothing else, and the answers are remembered in
/// `launcher.lastExport*`. Nothing is written if a step fails.
pub async fn export(
    dirs: &Dirs,
    settings: &Settings,
    instance: &mut Instance,
    opts: &ExportOptions,
    cf: &dyn CurseForgeSource,
    progress: &Arc<Progress>,
) -> Result<ExportOutcome> {
    if !can_be_exported(instance) {
        bail!("This instance has no mod loader, so it can't be exported. Add Fabric, Forge, NeoForge or Quilt first.");
    }
    if opts.name.trim().is_empty() {
        bail!("Give the export a name");
    }
    std::fs::create_dir_all(&opts.save_to).with_context(|| format!("creating {}", opts.save_to.display()))?;
    let overrides: Vec<String> = opts
        .overrides
        .iter()
        .map(|o| o.replace('\\', "/").trim_matches('/').to_string())
        .filter(|o| !o.is_empty())
        .collect();
    for o in &overrides {
        if crate::packutil::safe_relative(o).is_none() {
            bail!("{o} isn't a path inside the instance");
        }
    }

    let mut outcome = ExportOutcome::default();
    match opts.format {
        ExportFormat::CurseForge => {
            outcome.paths.push(export_curseforge(dirs, instance, opts, &overrides, cf, progress).await?)
        }
        ExportFormat::Modrinth => {
            let (path, note) = export_modrinth(dirs, instance, opts, &overrides, progress).await?;
            outcome.paths.push(path);
            outcome.overrides_note = note;
        }
        ExportFormat::CurseForgeAndModrinth => {
            outcome.paths.push(export_curseforge(dirs, instance, opts, &overrides, cf, progress).await?);
            let (path, note) = export_modrinth(dirs, instance, opts, &overrides, progress).await?;
            outcome.paths.push(path);
            outcome.overrides_note = note;
        }
        ExportFormat::MultiMc => outcome.paths.push(export_multimc(instance, opts, &overrides, settings, progress)?),
    }
    outcome.path = outcome.paths.last().cloned().unwrap_or_default();
    remember(instance, opts);
    instance.save().context("saving instance.json")?;
    Ok(outcome)
}

enum Content {
    File(PathBuf),
    Bytes(Vec<u8>),
}

struct Entry {
    name: String,
    content: Content,
}

/// Every file under the chosen paths, as archive entries below `prefix` (`overrides/`, `.minecraft/` ...).
/// `skip` lists absolute paths that must not be included (mods listed by id, the archive itself).
fn collect(root: &Path, overrides: &[String], prefix: &str, skip: &HashSet<PathBuf>) -> Vec<Entry> {
    fn walk(root: &Path, path: &Path, prefix: &str, skip: &HashSet<PathBuf>, out: &mut Vec<Entry>) {
        let Ok(meta) = std::fs::symlink_metadata(path) else { return };
        if meta.file_type().is_symlink() || skip.contains(path) {
            return;
        }
        if meta.is_dir() {
            if let Ok(rd) = std::fs::read_dir(path) {
                for e in rd.flatten() {
                    walk(root, &e.path(), prefix, skip, out);
                }
            }
        } else if meta.is_file() {
            if path.file_name().is_some_and(|n| n == ".DS_Store") {
                return;
            }
            if let Ok(rel) = path.strip_prefix(root) {
                let name =
                    rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/");
                out.push(Entry { name: format!("{prefix}{name}"), content: Content::File(path.to_path_buf()) });
            }
        }
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for o in overrides {
        let path = root.join(o);
        if path.exists() {
            let before = out.len();
            walk(root, &path, prefix, skip, &mut out);
            // a path chosen twice (a folder and a file in it) is stored once
            let mut i = before;
            while i < out.len() {
                if seen.insert(out[i].name.clone()) {
                    i += 1
                } else {
                    out.remove(i);
                }
            }
        }
    }
    out
}

fn write_zip(dest: &Path, mut entries: Vec<Entry>) -> Result<()> {
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let part = dest.with_extension(format!(
        "{}.part",
        dest.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default()
    ));
    let result = (|| -> Result<()> {
        let file = std::fs::File::create(&part).with_context(|| format!("creating {}", part.display()))?;
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .large_file(true);
        for e in entries {
            zip.start_file(e.name.as_str(), opts)?;
            match e.content {
                Content::Bytes(b) => zip.write_all(&b)?,
                Content::File(p) => {
                    let mut f = std::fs::File::open(&p).with_context(|| format!("reading {}", p.display()))?;
                    std::io::copy(&mut f, &mut zip)?;
                }
            }
        }
        zip.finish()?.flush()?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            std::fs::rename(&part, dest).with_context(|| format!("saving {}", dest.display()))?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

fn pretty(v: &impl Serialize) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(v)?)
}

/// Whether the record's file is inside one of the chosen paths (`relativize(path).startsWith(override)`).
fn included(root: &Path, file: &Path, overrides: &[String]) -> bool {
    let Ok(rel) = file.strip_prefix(root) else { return false };
    overrides.iter().any(|o| rel.starts_with(o))
}

// -- CurseForge -------------------------------------------------------------------------------------------

#[derive(Serialize)]
struct CfManifest {
    minecraft: CfMinecraft,
    #[serde(rename = "manifestType")]
    manifest_type: &'static str,
    #[serde(rename = "manifestVersion")]
    manifest_version: i32,
    name: String,
    version: String,
    author: String,
    files: Vec<CfManifestFile>,
    overrides: &'static str,
}

#[derive(Serialize)]
struct CfMinecraft {
    version: String,
    #[serde(rename = "modLoaders")]
    mod_loaders: Vec<CfModLoader>,
}

#[derive(Serialize)]
struct CfModLoader {
    id: String,
    primary: bool,
}

#[derive(Serialize)]
struct CfManifestFile {
    #[serde(rename = "projectID")]
    project_id: i64,
    #[serde(rename = "fileID")]
    file_id: i64,
    required: bool,
}

async fn export_curseforge(
    dirs: &Dirs,
    instance: &mut Instance,
    opts: &ExportOptions,
    overrides: &[String],
    cf: &dyn CurseForgeSource,
    progress: &Arc<Progress>,
) -> Result<PathBuf> {
    let _ = dirs;
    let safe = safe_file_name(&opts.name);
    let to = opts.save_to.join(format!("{safe} {}.zip", opts.version));
    let root = instance.root.clone();
    let mc = instance.mc_version().to_string();

    // mods that CurseForge doesn't have on record are looked up by fingerprint
    progress.stage("Looking the mods up on CurseForge");
    let mut records = instance.mod_records();
    if let Err(e) = modcheck::attach_curseforge(&root, &mc, &mut records, cf, false).await {
        eprintln!("[export] CurseForge lookup failed: {e:#}");
    }
    instance.set_mod_records(&records);
    instance.save()?;

    let (loader_type, loader_version) = {
        let l = instance.file.launcher.loader_version.as_ref().context("This instance has no mod loader")?;
        (l.kind.to_lowercase(), l.version.clone())
    };
    let listed: Vec<&ModRecord> = records
        .iter()
        .filter(|m| {
            !m.disabled
                && modcheck::is_from_curseforge(m)
                && modcheck::has_full_curseforge_info(m)
                && m.kind != "worlds"
        })
        .filter(|m| modcheck::current_file(&root, &mc, m).is_some_and(|f| included(&root, &f, overrides)))
        .collect();
    let available =
        |m: &ModRecord| m.curse_forge_file.as_ref().and_then(|f| f["isAvailable"].as_bool()).unwrap_or(false);
    let mut seen = HashSet::new();
    let files: Vec<CfManifestFile> = listed
        .iter()
        .filter(|m| seen.insert(m.curse_forge_file_id))
        // #875 - files CurseForge says are no longer available are rejected by it, so they stay in overrides
        .filter(|m| available(m))
        .map(|m| CfManifestFile {
            project_id: m.curse_forge_project_id.unwrap_or(0),
            file_id: m.curse_forge_file_id.unwrap_or(0),
            required: true,
        })
        .collect();

    let manifest = CfManifest {
        minecraft: CfMinecraft {
            version: mc.clone(),
            mod_loaders: vec![CfModLoader { id: format!("{loader_type}-{loader_version}"), primary: true }],
        },
        manifest_type: "minecraftModpack",
        manifest_version: 1,
        name: opts.name.clone(),
        version: opts.version.clone(),
        author: opts.author.clone(),
        files,
        overrides: "overrides",
    };

    let mut html = String::from("<ul>");
    for m in listed.iter().filter(|m| available(m)) {
        match m.curse_forge_project.as_ref().and_then(|p| p["links"]["websiteUrl"].as_str()).filter(|u| !u.is_empty()) {
            Some(url) => html.push_str(&format!("<li><a href=\"{url}\">{}</a></li>", m.name)),
            None => html.push_str(&format!("<li>{}</li>", m.name)),
        }
    }
    html.push_str("</ul>");

    // files that are listed by id are not stored a second time
    let mut skip: HashSet<PathBuf> = HashSet::new();
    skip.insert(to.clone());
    for m in records.iter().filter(|m| {
        !m.disabled
            && modcheck::is_from_curseforge(m)
            && modcheck::has_full_curseforge_info(m)
            && m.kind != "worlds"
            && available(m)
    }) {
        if let Some(f) = modcheck::enabled_file(&root, &mc, m) {
            skip.insert(f);
        }
    }
    for m in records.iter().filter(|m| {
        !m.disabled && modcheck::is_from_curseforge(m) && modcheck::has_full_curseforge_info(m) && !available(m)
    }) {
        eprintln!(
            "[export] File {} is no longer available according to the CurseForge api, so putting it in overrides",
            m.file
        );
    }

    progress.stage("Writing the archive");
    let mut entries = vec![
        Entry { name: "manifest.json".into(), content: Content::Bytes(pretty(&manifest)?) },
        Entry { name: "modlist.html".into(), content: Content::Bytes(html.into_bytes()) },
    ];
    entries.extend(collect(&root, &overrides_without_output(overrides, &safe), "overrides/", &skip));
    write_zip(&to, entries)?;
    Ok(to)
}

/// The archive itself may sit in the instance folder; it must not be packed into itself.
fn overrides_without_output(overrides: &[String], safe_name: &str) -> Vec<String> {
    overrides.iter().filter(|o| !o.eq_ignore_ascii_case(&format!("{safe_name}.zip"))).cloned().collect()
}

// -- Modrinth ---------------------------------------------------------------------------------------------

#[derive(Serialize)]
struct MrManifest {
    #[serde(rename = "formatVersion")]
    format_version: i32,
    game: &'static str,
    #[serde(rename = "versionId")]
    version_id: String,
    name: String,
    summary: String,
    files: Vec<MrFile>,
    dependencies: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct MrFile {
    path: String,
    hashes: BTreeMap<String, String>,
    env: BTreeMap<String, String>,
    downloads: Vec<String>,
    #[serde(rename = "fileSize")]
    file_size: u64,
}

/// The key a loader has in `modrinth.index.json`'s `dependencies` (`LoaderVersion.getTypeForModrinthExport`).
pub fn modrinth_loader_key(kind: &str) -> &'static str {
    match kind.to_lowercase().as_str() {
        "fabric" | "legacyfabric" => "fabric-loader",
        "quilt" => "quilt-loader",
        "neoforge" => "neoforge",
        _ => "forge",
    }
}

async fn export_modrinth(
    dirs: &Dirs,
    instance: &mut Instance,
    opts: &ExportOptions,
    overrides: &[String],
    progress: &Arc<Progress>,
) -> Result<(PathBuf, String)> {
    let _ = dirs;
    let safe = safe_file_name(&opts.name);
    let to = opts.save_to.join(format!("{safe} {}.mrpack", opts.version));
    let root = instance.root.clone();
    let mc = instance.mc_version().to_string();

    // mods that Modrinth has not been asked about yet are looked up by their SHA-1
    progress.stage("Looking the mods up on Modrinth");
    let mut records = instance.mod_records();
    match modcheck::attach_modrinth(&root, &mc, &mut records, true).await {
        Ok(n) if n > 0 => {
            instance.set_mod_records(&records);
            instance.save()?;
        }
        Ok(_) => {}
        Err(e) => eprintln!("[export] Modrinth lookup failed: {e:#}"),
    }

    progress.stage("Hashing the mods");
    let listed: Vec<&ModRecord> = records
        .iter()
        .filter(|m| !m.disabled && m.modrinth_version.as_ref().is_some_and(|v| !v.is_null()))
        .filter(|m| modcheck::enabled_file(&root, &mc, m).is_some_and(|f| f.exists() && included(&root, &f, overrides)))
        .collect();
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for m in listed.into_iter().filter(|m| seen.insert(m.modrinth_version_id().map(str::to_string))) {
        let Some(path) = modcheck::enabled_file(&root, &mc, m) else { continue };
        let (sha1, sha512, size) = hash_file(&path)?;
        let Some(version) = m.modrinth_version.as_ref() else { continue };
        let Some(url) = modcheck::modrinth_file_by_sha1(version, &sha1).and_then(|f| f["url"].as_str()) else {
            continue;
        };
        let mut env = BTreeMap::new();
        // mods are always required on the client ALWAYS ALWAYS ALWAYS (for now)
        env.insert("client".to_string(), "required".to_string());
        let server_unsupported =
            m.modrinth_project.as_ref().is_some_and(|p| p["server_side"].as_str() == Some("unsupported"));
        env.insert("server".to_string(), if server_unsupported { "unsupported" } else { "required" }.to_string());
        let rel = path
            .strip_prefix(&root)
            .map(|r| r.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/"))
            .unwrap_or_default();
        files.push(MrFile {
            path: rel,
            hashes: BTreeMap::from([("sha1".to_string(), sha1), ("sha512".to_string(), sha512)]),
            env,
            downloads: vec![url.to_string()],
            file_size: size,
        });
    }

    let mut dependencies = BTreeMap::new();
    dependencies.insert("minecraft".to_string(), mc.clone());
    if let Some(l) = &instance.file.launcher.loader_version {
        dependencies.insert(modrinth_loader_key(&l.kind).to_string(), l.version.clone());
    }
    let manifest = MrManifest {
        format_version: 1,
        game: "minecraft",
        version_id: opts.version.clone(),
        name: opts.name.clone(),
        summary: instance.file.launcher.description.clone(),
        files,
        dependencies,
    };

    // files Modrinth lists by hash are not stored in the archive
    let mut skip: HashSet<PathBuf> = HashSet::new();
    skip.insert(to.clone());
    for m in records.iter().filter(|m| !m.disabled && m.modrinth_version.as_ref().is_some_and(|v| !v.is_null())) {
        if let Some(f) = modcheck::enabled_file(&root, &mc, m) {
            skip.insert(f);
        }
    }
    progress.stage("Writing the archive");
    let mut entries = vec![Entry { name: "modrinth.index.json".into(), content: Content::Bytes(pretty(&manifest)?) }];
    entries.extend(collect(&root, &overrides_without_output(overrides, &safe), "overrides/", &skip));

    // the jar and zip files stored in the archive, for the permissions note
    let note: String = entries
        .iter()
        .filter(|e| e.name.starts_with("overrides/") && (e.name.ends_with(".jar") || e.name.ends_with(".zip")))
        .map(|e| format!("{}\n", e.name))
        .collect();
    write_zip(&to, entries)?;
    Ok((to, note))
}

/// SHA-1, SHA-512 and size of a file, read once.
pub fn hash_file(path: &Path) -> Result<(String, String, u64)> {
    let mut f = std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let (mut a, mut b) = (Sha1::new(), Sha512::new());
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        a.update(&buf[..n]);
        b.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hex::encode(a.finalize()), hex::encode(b.finalize()), total))
}

// -- MultiMC ----------------------------------------------------------------------------------------------

fn component(uid: &str, name: &str, version: &str) -> multimc::Component {
    multimc::Component {
        cached_name: Some(name.into()),
        cached_version: Some(version.into()),
        uid: uid.into(),
        version: Some(version.into()),
        ..Default::default()
    }
}

fn requires(uid: &str, equals: Option<&str>) -> multimc::Require {
    multimc::Require { equals: equals.map(str::to_string), suggests: None, uid: uid.into() }
}

/// The `mmc-pack.json` of an instance (Minecraft, LWJGL 3, the loader and its mappings).
pub fn mmc_manifest(instance: &Instance) -> multimc::Manifest {
    let mc = instance.mc_version().to_string();
    let mut components = Vec::new();
    let mut minecraft = component("net.minecraft", "Minecraft", &mc);
    minecraft.important = Some(true);

    let lwjgl = instance
        .file
        .version
        .libraries
        .iter()
        .find(|l| l.name.contains("org.lwjgl:lwjgl:"))
        .map(|l| l.name.replace("org.lwjgl:lwjgl:", ""));
    if let Some(v) = &lwjgl {
        let mut c = component("org.lwjgl3", "LWJGL 3", v);
        c.cached_volatile = Some(true);
        c.dependency_only = Some(true);
        components.push(c);
        minecraft.cached_requires = Some(vec![multimc::Require {
            equals: Some(v.clone()),
            suggests: Some(v.clone()),
            uid: "org.lwjgl3".into(),
        }]);
    }
    components.push(minecraft);

    if let Some(l) = &instance.file.launcher.loader_version {
        let v = l.version.as_str();
        let mappings = |uid: &str, name: &str| {
            let mut c = component(uid, name, &mc);
            c.cached_requires = Some(vec![requires("net.minecraft", Some(&mc))]);
            c.cached_volatile = Some(true);
            c.dependency_only = Some(true);
            c
        };
        match l.kind.to_lowercase().as_str() {
            "fabric" | "legacyfabric" => {
                components.push(mappings("net.fabricmc.intermediary", "Intermediary Mappings"));
                let mut c = component("net.fabricmc.fabric-loader", "Fabric Loader", v);
                c.cached_requires = Some(vec![requires("net.fabricmc.intermediary", None)]);
                components.push(c);
            }
            "neoforge" => {
                let mut c = component("net.neoforged", "NeoForge", v);
                c.cached_requires = Some(vec![requires("net.minecraft", Some(&mc))]);
                components.push(c);
            }
            "forge" => {
                let mut c = component("net.minecraftforge", "Forge", v);
                c.cached_requires = Some(vec![requires("net.minecraft", Some(&mc))]);
                components.push(c);
            }
            "quilt" => {
                components.push(mappings("org.quiltmc.hashed", "Hashed Mappings"));
                let mut c = component("org.quiltmc.quilt-loader", "Quilt Loader", v);
                c.cached_requires = Some(vec![requires("org.quiltmc.hashed", None)]);
                components.push(c);
            }
            _ => {}
        }
    }
    multimc::Manifest { components, format_version: 1 }
}

/// The `instance.cfg` of an exported instance.
pub fn mmc_instance_cfg(instance: &Instance, settings: &Settings, icon_key: &str) -> Properties {
    let l = &instance.file.launcher;
    let extra_str = |k: &str| l.extras.get(k).and_then(Value::as_str).map(str::to_string);
    let setting_str = |k: &str| settings.extras.get(k).and_then(Value::as_str).map(str::to_string);
    let pre = extra_str("preLaunchCommand");
    let post = extra_str("postExitCommand");
    let wrapper = l.wrapper_command.clone();
    let java = match l.java_path.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => settings.java_path.clone(),
    };
    let sep = std::path::MAIN_SEPARATOR;
    let exe = if cfg!(target_os = "windows") { "javaw.exe" } else { "java" };

    let mut p = Properties::default();
    let mut set = |k: &str, v: String| p.set(k, v);
    set("AutoCloseConsole", "false".into());
    set("ForgeVersion", "false".into());
    set("InstanceType", "OneSix".into());
    set("IntendedVersion", String::new());
    set("JavaPath", if java.is_empty() { String::new() } else { format!("{java}{sep}bin{sep}{exe}") });
    set("JVMArgs", l.java_arguments.clone().unwrap_or_default());
    set("LWJGLVersion", String::new());
    set("LaunchMaximized", "false".into());
    set("LiteloaderVersion", String::new());
    set("LogPrePostOutput", "true".into());
    set("MCLaunchMethod", "LauncherPart".into());
    set("MaxMemAlloc", l.maximum_memory.unwrap_or(settings.maximum_memory).to_string());
    set("MinecraftWinHeight", settings.window_height.to_string());
    set("MinecraftWinWidth", settings.window_width.to_string());
    set("OverrideCommands", (post.is_some() || pre.is_some() || wrapper.is_some()).to_string());
    set("OverrideConsole", "false".into());
    set("OverrideJava", l.java_path.is_some().to_string());
    set("OverrideJavaArgs", l.java_arguments.is_some().to_string());
    set("OverrideJavaLocation", "false".into());
    set("OverrideMCLaunchMethod", "false".into());
    set("OverrideMemory", l.maximum_memory.is_some().to_string());
    set("OverrideNativeWorkarounds", "false".into());
    set("OverrideWindow", "false".into());
    set("PermGen", l.perm_gen.unwrap_or(settings.metaspace).to_string());
    set("PostExitCommand", post.or_else(|| setting_str("postExitCommand")).unwrap_or_default());
    set("PreLaunchCommand", pre.or_else(|| setting_str("preLaunchCommand")).unwrap_or_default());
    set("ShowConsole", "false".into());
    set("ShowConsoleOnError", "true".into());
    set("UseNativeGLFW", "false".into());
    set("UseNativeOpenAL", "false".into());
    set("WrapperCommand", wrapper.or_else(|| settings.wrapper_command.clone()).unwrap_or_default());
    set("iconKey", icon_key.to_string());
    set("name", l.name.clone());
    set("lastLaunchTime", String::new());
    set("notes", String::new());
    set("totalTimePlayed", "0".into());
    p
}

fn export_multimc(
    instance: &Instance,
    opts: &ExportOptions,
    overrides: &[String],
    settings: &Settings,
    progress: &Arc<Progress>,
) -> Result<PathBuf> {
    let safe = safe_file_name(&opts.name);
    let to = opts.save_to.join(format!("{safe}.zip"));
    let root = instance.root.clone();
    let manifest = mmc_manifest(instance);
    let mc = instance.mc_version().to_string();
    progress.stage("Writing the archive");

    let mut entries = vec![Entry { name: "mmc-pack.json".into(), content: Content::Bytes(pretty(&manifest)?) }];

    // Legacy Fabric needs its intermediary mappings as a patch
    if instance.file.launcher.loader_version.as_ref().is_some_and(|l| l.kind == "LegacyFabric") {
        let patch = json!({
            "formatVersion": 1,
            "name": "Intermediary Mappings",
            "uid": "net.fabricmc.intermediary",
            "version": mc,
            "+libraries": [{"name": format!("net.fabricmc:intermediary:{mc}"), "url": "https://repo.legacyfabric.net/repository/legacyfabric/"}],
        });
        entries.push(Entry {
            name: "patches/net.fabricmc.intermediary.json".into(),
            content: Content::Bytes(pretty(&patch)?),
        });
    }

    let mut icon_key = "default".to_string();
    let custom = root.join("instance.png");
    if custom.is_file() {
        let key = format!("atlauncher_{}", paths::folder_name(&instance.file.launcher.name).to_lowercase());
        entries.push(Entry { name: format!("{key}.png"), content: Content::File(custom) });
        icon_key = key;
    }
    entries.push(Entry {
        name: "instance.cfg".into(),
        content: Content::Bytes(
            mmc_instance_cfg(instance, settings, &icon_key).store("Exported by BlockForge").into_bytes(),
        ),
    });
    entries.push(Entry { name: ".packignore".into(), content: Content::Bytes(Vec::new()) });

    let mut skip = HashSet::new();
    skip.insert(to.clone());
    entries.extend(collect(&root, &overrides_without_output(overrides, &safe), ".minecraft/", &skip));
    write_zip(&to, entries)?;
    Ok(to)
}

// ---------------------------------------------------------------------------------------------------------
// SHA-512 (the dependency set has SHA-1 and MD5 only)

struct Sha512 {
    h: [u64; 8],
    buf: Vec<u8>,
    len: u128,
}

const K512: [u64; 80] = [
    0x428a2f98d728ae22,
    0x7137449123ef65cd,
    0xb5c0fbcfec4d3b2f,
    0xe9b5dba58189dbbc,
    0x3956c25bf348b538,
    0x59f111f1b605d019,
    0x923f82a4af194f9b,
    0xab1c5ed5da6d8118,
    0xd807aa98a3030242,
    0x12835b0145706fbe,
    0x243185be4ee4b28c,
    0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f,
    0x80deb1fe3b1696b1,
    0x9bdc06a725c71235,
    0xc19bf174cf692694,
    0xe49b69c19ef14ad2,
    0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5,
    0x240ca1cc77ac9c65,
    0x2de92c6f592b0275,
    0x4a7484aa6ea6e483,
    0x5cb0a9dcbd41fbd4,
    0x76f988da831153b5,
    0x983e5152ee66dfab,
    0xa831c66d2db43210,
    0xb00327c898fb213f,
    0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2,
    0xd5a79147930aa725,
    0x06ca6351e003826f,
    0x142929670a0e6e70,
    0x27b70a8546d22ffc,
    0x2e1b21385c26c926,
    0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df,
    0x650a73548baf63de,
    0x766a0abb3c77b2a8,
    0x81c2c92e47edaee6,
    0x92722c851482353b,
    0xa2bfe8a14cf10364,
    0xa81a664bbc423001,
    0xc24b8b70d0f89791,
    0xc76c51a30654be30,
    0xd192e819d6ef5218,
    0xd69906245565a910,
    0xf40e35855771202a,
    0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8,
    0x1e376c085141ab53,
    0x2748774cdf8eeb99,
    0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63,
    0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc,
    0x78a5636f43172f60,
    0x84c87814a1f0ab72,
    0x8cc702081a6439ec,
    0x90befffa23631e28,
    0xa4506cebde82bde9,
    0xbef9a3f7b2c67915,
    0xc67178f2e372532b,
    0xca273eceea26619c,
    0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e,
    0xf57d4f7fee6ed178,
    0x06f067aa72176fba,
    0x0a637dc5a2c898a6,
    0x113f9804bef90dae,
    0x1b710b35131c471b,
    0x28db77f523047d84,
    0x32caab7b40c72493,
    0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6,
    0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec,
    0x6c44198c4a475817,
];

impl Sha512 {
    fn new() -> Sha512 {
        Sha512 {
            h: [
                0x6a09e667f3bcc908,
                0xbb67ae8584caa73b,
                0x3c6ef372fe94f82b,
                0xa54ff53a5f1d36f1,
                0x510e527fade682d1,
                0x9b05688c2b3e6c1f,
                0x1f83d9abfb41bd6b,
                0x5be0cd19137e2179,
            ],
            buf: Vec::with_capacity(128),
            len: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.len += data.len() as u128;
        self.buf.extend_from_slice(data);
        while self.buf.len() >= 128 {
            let block: Vec<u8> = self.buf.drain(..128).collect();
            self.block(&block);
        }
    }

    fn block(&mut self, b: &[u8]) {
        let mut w = [0u64; 80];
        for i in 0..16 {
            w[i] = u64::from_be_bytes(b[i * 8..i * 8 + 8].try_into().unwrap_or([0; 8]));
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b_, mut c, mut d, mut e, mut f, mut g, mut h] = self.h;
        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K512[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b_) ^ (a & c) ^ (b_ & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b_;
            b_ = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in self.h.iter_mut().zip([a, b_, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(v);
        }
    }

    fn finalize(mut self) -> Vec<u8> {
        let bit_len = self.len * 8;
        let mut tail = vec![0x80u8];
        while (self.buf.len() + tail.len()) % 128 != 112 {
            tail.push(0);
        }
        tail.extend_from_slice(&bit_len.to_be_bytes());
        let buf = std::mem::take(&mut self.buf);
        let mut all = buf;
        all.extend_from_slice(&tail);
        for chunk in all.chunks(128) {
            self.block(chunk);
        }
        self.h.iter().flat_map(|v| v.to_be_bytes()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instances::{InstanceFile, LoaderVersion};

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bf-exp-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: &Path, c: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, c).unwrap();
    }

    fn instance(root: &Path, kind: &str, version: &str) -> Instance {
        let mut file = InstanceFile::default();
        file.launcher.name = "My Pack".into();
        file.launcher.version = "1.0".into();
        file.launcher.description = "A pack".into();
        file.launcher.loader_version = Some(LoaderVersion {
            kind: kind.into(),
            version: version.into(),
            raw_version: version.into(),
            ..Default::default()
        });
        file.version.id = "1.20.1".into();
        Instance { folder: "MyPack".into(), mod_count: 0, root: root.to_path_buf(), file }
    }

    #[test]
    fn sha512_matches_known_vectors() {
        let h = |data: &[u8]| {
            let mut s = Sha512::new();
            s.update(data);
            hex::encode(s.finalize())
        };
        assert_eq!(
            h(b""),
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
        assert_eq!(
            h(b"abc"),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        // a message longer than one block, fed in pieces
        let long = vec![b'a'; 1000];
        let mut s = Sha512::new();
        for c in long.chunks(37) {
            s.update(c);
        }
        let mut whole = Sha512::new();
        whole.update(&long);
        assert_eq!(hex::encode(s.finalize()), hex::encode(whole.finalize()));
    }

    #[test]
    fn format_names_match_the_java_settings() {
        assert_eq!(serde_json::to_string(&ExportFormat::CurseForgeAndModrinth).unwrap(), "\"CURSEFORGE_AND_MODRINTH\"");
        let f: ExportFormat = serde_json::from_str("\"MULTIMC\"").unwrap();
        assert_eq!(f, ExportFormat::MultiMc);
        let s = Settings::default();
        assert_eq!(ExportFormat::from_settings(&s), ExportFormat::Modrinth);
    }

    #[test]
    fn file_names_lose_characters_windows_refuses() {
        assert_eq!(safe_file_name("My: \"Pack\"? <1>|*"), "My Pack 1");
    }

    #[test]
    fn dialog_defaults_follow_last_export() {
        let t = temp();
        let mut inst = instance(&t, "Fabric", "0.16.0");
        let s = Settings::default();
        let d = default_options(&inst, Some("Steve"), &s);
        assert_eq!((d.name.as_str(), d.version.as_str(), d.author.as_str()), ("My Pack", "1.0", "Steve"));
        assert_eq!(d.save_to, t);
        let mut o = d.clone();
        o.name = "Renamed".into();
        o.version = "2.0".into();
        o.author = "Alex".into();
        o.save_to = t.join("out");
        remember(&mut inst, &o);
        let again = default_options(&inst, Some("Steve"), &s);
        assert_eq!((again.name.as_str(), again.version.as_str(), again.author.as_str()), ("Renamed", "2.0", "Alex"));
        assert_eq!(again.save_to, t.join("out"));
        assert_eq!(inst.file.launcher.extras["lastExportName"], "Renamed");
    }

    #[test]
    fn the_tree_hides_launcher_files_and_empty_folders_and_ticks_the_usual_folders() {
        let t = temp();
        write(&t.join("mods/a.jar"), "x");
        write(&t.join("config/a.toml"), "x");
        write(&t.join("saves/W/level.dat"), "x");
        write(&t.join("options.txt"), "x");
        write(&t.join("instance.json"), "{}");
        write(&t.join("disabledmods/b.jar"), "x");
        std::fs::create_dir_all(t.join("emptydir")).unwrap();
        std::fs::create_dir_all(t.join(".fabric")).unwrap();
        let names: Vec<String> = list_dir(&t, "").into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["config", "mods", "saves", "options.txt"]);
        let inst = instance(&t, "Fabric", "0.16.0");
        assert_eq!(default_overrides(&inst), vec!["config", "mods"]);
        let inner: Vec<String> = list_dir(&t, "saves").into_iter().map(|e| e.rel).collect();
        assert_eq!(inner, vec!["saves/W"]);
    }

    #[test]
    fn unticking_inside_a_ticked_folder_splits_it() {
        let t = temp();
        write(&t.join("config/a.toml"), "x");
        write(&t.join("config/sub/b.toml"), "x");
        write(&t.join("config/sub/c.toml"), "x");
        write(&t.join("mods/m.jar"), "x");
        let mut s = Selection::new(vec!["config".to_string(), "mods".to_string()]);
        assert!(s.is_checked("config/sub/b.toml") && s.is_checked("config"));
        s.set(&t, "config/sub/b.toml", false);
        assert!(!s.is_checked("config/sub/b.toml"));
        assert!(!s.is_checked("config") && s.is_partial("config") && s.is_partial("config/sub"));
        assert!(s.is_checked("config/a.toml") && s.is_checked("config/sub/c.toml") && s.is_checked("mods/m.jar"));
        assert_eq!(s.to_overrides(), vec!["config/a.toml", "config/sub/c.toml", "mods"]);
        // ticking the folder again covers everything
        s.set(&t, "config", true);
        assert_eq!(s.to_overrides(), vec!["config", "mods"]);
        s.set(&t, "mods", false);
        assert_eq!(s.to_overrides(), vec!["config"]);
    }

    #[test]
    fn mmc_manifest_matches_the_java_exporter() {
        let t = temp();
        let mut inst = instance(&t, "Fabric", "0.16.0");
        inst.file
            .version
            .libraries
            .push(crate::model::Library { name: "org.lwjgl:lwjgl:3.3.3".into(), ..Default::default() });
        let m = mmc_manifest(&inst);
        let uids: Vec<&str> = m.components.iter().map(|c| c.uid.as_str()).collect();
        assert_eq!(
            uids,
            vec!["org.lwjgl3", "net.minecraft", "net.fabricmc.intermediary", "net.fabricmc.fabric-loader"]
        );
        assert_eq!(m.components[1].cached_requires.as_ref().unwrap()[0].equals.as_deref(), Some("3.3.3"));
        assert_eq!(m.components[3].version.as_deref(), Some("0.16.0"));
        assert_eq!(m.format_version, 1);

        let forge = mmc_manifest(&instance(&t, "Forge", "47.2.0"));
        assert_eq!(forge.components.last().unwrap().uid, "net.minecraftforge");
        let neo = mmc_manifest(&instance(&t, "NeoForge", "21.1.57"));
        assert_eq!(neo.components.last().unwrap().uid, "net.neoforged");
        let quilt = mmc_manifest(&instance(&t, "Quilt", "0.26.4"));
        let q: Vec<&str> = quilt.components.iter().map(|c| c.uid.as_str()).collect();
        assert_eq!(q, vec!["net.minecraft", "org.quiltmc.hashed", "org.quiltmc.quilt-loader"]);
    }

    #[test]
    fn modrinth_loader_keys() {
        assert_eq!(modrinth_loader_key("Fabric"), "fabric-loader");
        assert_eq!(modrinth_loader_key("LegacyFabric"), "fabric-loader");
        assert_eq!(modrinth_loader_key("Quilt"), "quilt-loader");
        assert_eq!(modrinth_loader_key("NeoForge"), "neoforge");
        assert_eq!(modrinth_loader_key("Forge"), "forge");
    }

    #[test]
    fn instance_cfg_has_what_prism_needs() {
        let t = temp();
        let mut inst = instance(&t, "Fabric", "0.16.0");
        inst.file.launcher.maximum_memory = Some(6144);
        inst.file.launcher.java_arguments = Some("-Dx=1".into());
        let mut settings = Settings::default();
        settings.java_path = "/opt/jdk".into();
        let p = mmc_instance_cfg(&inst, &settings, "default");
        assert_eq!(p.get("name"), Some("My Pack"));
        assert_eq!(p.get("MaxMemAlloc"), Some("6144"));
        assert_eq!(p.get("OverrideMemory"), Some("true"));
        assert_eq!(p.get("OverrideJava"), Some("false"));
        assert_eq!(p.get("JVMArgs"), Some("-Dx=1"));
        assert_eq!(p.get("InstanceType"), Some("OneSix"));
        assert!(p.get("JavaPath").unwrap().starts_with("/opt/jdk"));
        assert_eq!(p.get("iconKey"), Some("default"));
        // and the importer reads it back
        let back = multimc::InstanceConfig::from_properties(&Properties::parse(&p.store("x")));
        assert_eq!(back.maximum_memory, Some(6144));
        assert_eq!(back.java_arguments.as_deref(), Some("-Dx=1"));
        assert_eq!(back.name, "My Pack");
    }

    #[test]
    fn collection_skips_listed_mods_links_and_the_archive_itself() {
        let t = temp();
        write(&t.join("mods/listed.jar"), "a");
        write(&t.join("mods/own.jar"), "b");
        write(&t.join("config/c.toml"), "c");
        write(&t.join("config/.DS_Store"), "x");
        let mut skip = HashSet::new();
        skip.insert(t.join("mods/listed.jar"));
        let entries =
            collect(&t, &["mods".to_string(), "config".to_string(), "config/c.toml".to_string()], "overrides/", &skip);
        let mut names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        assert_eq!(
            names,
            vec!["overrides/config/c.toml", "overrides/mods/own.jar"],
            "a file chosen twice is stored once"
        );
        assert!(included(&t, &t.join("mods/a.jar"), &["mods".into()]));
        assert!(!included(&t, &t.join("modsx/a.jar"), &["mods".into()]));
    }

    #[tokio::test]
    async fn a_pack_without_a_loader_is_refused() {
        let t = temp();
        let mut inst = instance(&t, "Fabric", "1");
        inst.file.launcher.loader_version = None;
        assert!(!can_be_exported(&inst));
        let dirs = Dirs::at(&t);
        let opts = ExportOptions {
            name: "x".into(),
            version: "1".into(),
            author: "".into(),
            format: ExportFormat::Modrinth,
            save_to: t.join("out"),
            overrides: vec![],
        };
        let err =
            export(&dirs, &Settings::default(), &mut inst, &opts, &crate::cf_seam::MinimalCurseForge, &Progress::new())
                .await
                .unwrap_err();
        assert!(format!("{err:#}").contains("no mod loader"));
    }

    fn read_zip_entry(zip: &Path, name: &str) -> Option<String> {
        let mut z = zip::ZipArchive::new(std::fs::File::open(zip).unwrap()).unwrap();
        let mut e = z.by_name(name).ok()?;
        let mut s = String::new();
        e.read_to_string(&mut s).ok()?;
        Some(s)
    }

    #[tokio::test]
    async fn every_format_writes_a_readable_archive_without_the_network() {
        let t = temp();
        let root = t.join("inst");
        write(&root.join("mods/own.jar"), "jar bytes");
        write(&root.join("config/c.toml"), "a = 1");
        write(&root.join("options.txt"), "fov:90");
        write(&root.join("instance.json"), "{}");
        write(&root.join("saves/W/level.dat"), "w");
        let mut inst = instance(&root, "Forge", "47.2.0");
        let dirs = Dirs::at(&t);
        let settings = Settings::default();
        let out = t.join("out");
        let mut opts = ExportOptions {
            name: "My: Pack".into(),
            version: "2.0".into(),
            author: "Alex".into(),
            format: ExportFormat::Modrinth,
            save_to: out.clone(),
            overrides: vec!["mods".into(), "config".into(), "options.txt".into()],
        };
        let cf = crate::cf_seam::MinimalCurseForge;
        let progress = Progress::new();

        let mr = export(&dirs, &settings, &mut inst, &opts, &cf, &progress).await.unwrap();
        assert_eq!(mr.path, out.join("My Pack 2.0.mrpack"));
        let index: Value = serde_json::from_str(&read_zip_entry(&mr.path, "modrinth.index.json").unwrap()).unwrap();
        assert_eq!(index["game"], "minecraft");
        assert_eq!(index["versionId"], "2.0");
        assert_eq!(index["dependencies"]["minecraft"], "1.20.1");
        assert_eq!(index["dependencies"]["forge"], "47.2.0");
        assert_eq!(index["files"].as_array().unwrap().len(), 0);
        assert_eq!(read_zip_entry(&mr.path, "overrides/config/c.toml").as_deref(), Some("a = 1"));
        assert!(read_zip_entry(&mr.path, "overrides/instance.json").is_none());
        assert!(read_zip_entry(&mr.path, "overrides/saves/W/level.dat").is_none(), "only what was ticked");
        assert_eq!(
            mr.overrides_note, "overrides/mods/own.jar\n",
            "jars stored in the archive are listed for the permissions note"
        );
        assert_eq!(inst.file.launcher.extras["lastExportVersion"], "2.0");

        opts.format = ExportFormat::CurseForge;
        let cfz = export(&dirs, &settings, &mut inst, &opts, &cf, &progress).await.unwrap();
        assert_eq!(cfz.path, out.join("My Pack 2.0.zip"));
        let manifest: Value = serde_json::from_str(&read_zip_entry(&cfz.path, "manifest.json").unwrap()).unwrap();
        assert_eq!(manifest["manifestType"], "minecraftModpack");
        assert_eq!(manifest["minecraft"]["modLoaders"][0]["id"], "forge-47.2.0");
        assert_eq!(manifest["author"], "Alex");
        assert_eq!(manifest["overrides"], "overrides");
        assert_eq!(read_zip_entry(&cfz.path, "modlist.html").as_deref(), Some("<ul></ul>"));
        assert_eq!(read_zip_entry(&cfz.path, "overrides/mods/own.jar").as_deref(), Some("jar bytes"));

        opts.format = ExportFormat::MultiMc;
        let mmc = export(&dirs, &settings, &mut inst, &opts, &cf, &progress).await.unwrap();
        assert_eq!(mmc.path, out.join("My Pack.zip"));
        let unpacked = t.join("unpacked");
        let folder = multimc::extract_export(&mmc.path, &unpacked).unwrap();
        let back = multimc::MmcInstance::load(&folder).unwrap();
        assert_eq!(back.manifest.minecraft_version(), Some("1.20.1"));
        assert_eq!(back.manifest.loader(), Some((crate::loaders::Loader::Forge, "47.2.0".to_string())));
        assert_eq!(back.config.name, "My Pack");
        assert_eq!(std::fs::read_to_string(folder.join(".minecraft/options.txt")).unwrap(), "fov:90");
        assert!(folder.join(".packignore").is_file());

        opts.format = ExportFormat::CurseForgeAndModrinth;
        let both = export(&dirs, &settings, &mut inst, &opts, &cf, &progress).await.unwrap();
        assert_eq!(both.paths.len(), 2);
        // exporting again into the instance folder never packs the earlier archives into the new one
        opts.save_to = root.clone();
        opts.overrides = vec![".".into()];
        assert!(
            export(&dirs, &settings, &mut inst, &opts, &cf, &progress).await.is_err(),
            "a path that leaves the list is refused"
        );
    }
}
