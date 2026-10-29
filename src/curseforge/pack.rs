//! CurseForge modpacks: install one as a new instance (from CurseForge or from a downloaded `.zip`), update an
//! instance made that way, and find out whether a newer file exists. This follows `InstanceInstaller`
//! (`generatePackVersionFromCurseForgeManifest`, overrides, the launcher block) and `CurseForgeUpdateManager`
//! of the Java edition, and writes the launcher block the way Java does so Java can open and update the
//! instance too: `curseForgeProject`, `curseForgeFile`, `curseForgeProjectDescription`, `overridePaths` and a
//! `launcher.mods` list with the CurseForge objects of every mod.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use super::api;
use super::browser::{BrowserHub, BrowserRequest, fetch_by_hand};
use super::filter;
use super::install::{self, fetch_file, pack_record_for, record_current_path, update_instance};
use super::model::*;
use super::{CfSettings, JUMPLOADER_ID};
use crate::install::{self as core_install, Job, Progress};
use crate::instances::{self, Instance, LoaderVersion};
use crate::loaders::{self, Loader};
use crate::modrecord::ModRecord;
use crate::paths::Dirs;

/// How long the hand-downloads of a pack are waited for before giving up.
const HAND_DOWNLOAD_WAIT: Duration = Duration::from_secs(60 * 60);

// ---------------------------------------------------------------------------------------------------------
// the manifest

/// Read `manifest.json` from the root of a pack zip.
pub fn read_manifest(zip_path: &Path) -> Result<Manifest> {
    let file = std::fs::File::open(zip_path).with_context(|| format!("opening {}", zip_path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).context("that file is not a zip, so it is not a CurseForge modpack")?;
    let mut entry =
        archive.by_name("manifest.json").context("this zip has no manifest.json, so it is not a CurseForge modpack")?;
    let mut text = String::new();
    entry.read_to_string(&mut text)?;
    parse_manifest(&text)
}

/// Parse a manifest and make sure it describes a Minecraft modpack.
pub fn parse_manifest(text: &str) -> Result<Manifest> {
    let manifest: Manifest =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).context("reading the pack's manifest.json")?;
    if manifest.manifest_type != "minecraftModpack" {
        bail!("This is not a Minecraft modpack (its manifest says \"{}\")", manifest.manifest_type);
    }
    if manifest.minecraft.version.is_empty() {
        bail!("The modpack does not say which Minecraft version it needs");
    }
    Ok(manifest)
}

/// The kind of loader and the version an id like `forge-47.2.0` names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoaderId {
    pub loader: Loader,
    pub version: String,
}

pub fn parse_loader_id(id: &str) -> Result<LoaderId> {
    for (prefix, loader) in [
        ("forge-", Loader::Forge),
        ("neoforge-", Loader::NeoForge),
        ("fabric-", Loader::Fabric),
        ("quilt-", Loader::Quilt),
    ] {
        if let Some(v) = id.strip_prefix(prefix) {
            return Ok(LoaderId { loader, version: v.to_string() });
        }
    }
    bail!("The modpack asks for a mod loader called \"{id}\", which is not known")
}

/// The loader a pack needs and, if it names one, the exact version. A pack containing JumpLoader runs on the
/// newest Fabric, as in the Java edition.
pub async fn resolve_loader(dirs: &Dirs, manifest: &Manifest) -> Result<(Loader, Option<String>)> {
    let mc = &manifest.minecraft.version;
    if manifest.files.iter().any(|f| f.project_id == JUMPLOADER_ID) {
        return Ok((Loader::Fabric, None));
    }
    let primary =
        manifest.minecraft.mod_loaders.iter().find(|l| l.primary).or_else(|| manifest.minecraft.mod_loaders.first());
    let Some(primary) = primary else {
        // a pack with no loader at all runs on plain Minecraft
        return Ok((Loader::Vanilla, None));
    };
    let id = parse_loader_id(&primary.id)?;
    match id.loader {
        Loader::Forge => {
            let list = loaders::versions(dirs, Loader::Forge, mc).await.unwrap_or_default();
            let want = match id.version.as_str() {
                "recommended" => forge_recommended(mc).await.or_else(|| list.first().map(|v| strip_mc_prefix(v, mc))),
                "latest" => list.first().map(|v| strip_mc_prefix(v, mc)),
                v => Some(v.to_string()),
            };
            let Some(want) = want else { bail!("Forge has no version for Minecraft {mc}") };
            // the list holds the full names ("1.20.1-47.2.0"); old versions carry a suffix
            Ok((Loader::Forge, Some(loaders::full_forge_version(&list, mc, &want))))
        }
        Loader::NeoForge => {
            let list = loaders::versions(dirs, Loader::NeoForge, mc).await.unwrap_or_default();
            let candidates = [id.version.clone(), format!("{mc}-{}", id.version)];
            let found = candidates.iter().find(|c| list.contains(c)).cloned();
            // NeoForge for 1.20.1 is named "1.20.1-47.1.x" on the server
            let v = found
                .unwrap_or_else(|| if mc == "1.20.1" { format!("{mc}-{}", id.version) } else { id.version.clone() });
            Ok((Loader::NeoForge, Some(v)))
        }
        other => Ok((other, Some(id.version))),
    }
}

fn strip_mc_prefix(v: &str, mc: &str) -> String {
    v.strip_prefix(&format!("{mc}-")).unwrap_or(v).to_string()
}

/// Forge's promoted "recommended" build for a Minecraft version.
async fn forge_recommended(mc: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Promos {
        promos: HashMap<String, String>,
    }
    let p: Promos =
        crate::http::get_json("https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json")
            .await
            .ok()?;
    p.promos.get(&format!("{mc}-recommended")).cloned()
}

// ---------------------------------------------------------------------------------------------------------
// planning what to download

/// Where a file of the pack comes from.
#[derive(Clone, Debug)]
pub enum Route {
    /// CurseForge's own download address.
    Direct(String),
    /// The author blocks launchers, but Modrinth has the identical file.
    Modrinth { url: String, project: Value, version: Value },
    /// The player has to download it in a browser.
    Browser,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub required: bool,
    pub project: Project,
    pub file: File,
    pub route: Route,
}

impl Entry {
    /// The folder of the instance this entry's file belongs in.
    pub fn folder(&self) -> &'static str {
        match self.project.mod_type() {
            "resourcepack" => "resourcepacks",
            "shaderpack" => "shaderpacks",
            "datapack" => "datapacks",
            _ => "mods",
        }
    }
}

/// Look up every project and file of a manifest and decide where each comes from.
pub async fn plan_entries(manifest: &Manifest, progress: &Arc<Progress>) -> Result<Vec<Entry>> {
    progress.stage(format!("Looking up {} files on CurseForge", manifest.files.len()));
    let project_ids: Vec<i64> = manifest.files.iter().map(|f| f.project_id).collect();
    let file_ids: Vec<i64> = manifest.files.iter().map(|f| f.file_id).collect();
    let mut projects = api::projects_map(&project_ids).await?;
    let mut files: HashMap<i64, File> = api::files_by_ids(&file_ids).await?.into_iter().map(|f| (f.id, f)).collect();

    let mut entries = Vec::new();
    for mf in &manifest.files {
        if !projects.contains_key(&mf.project_id) {
            let p = api::project(mf.project_id)
                .await
                .with_context(|| format!("looking up CurseForge project {}", mf.project_id))?;
            projects.insert(p.id, p);
        }
        let project = projects[&mf.project_id].clone();
        if !files.contains_key(&mf.file_id) {
            // not in the bulk answer: ask for it directly, then look through the project's whole list
            let found = match api::file(mf.project_id, mf.file_id).await {
                Ok(f) => Some(f),
                Err(_) => {
                    api::files(mf.project_id).await.ok().and_then(|all| all.into_iter().find(|f| f.id == mf.file_id))
                }
            };
            match found {
                Some(f) => {
                    files.insert(f.id, f);
                }
                None => bail!(
                    "Failed to find fileId {} for mod {} ({}) in CurseForge API",
                    mf.file_id,
                    project.name,
                    project.id
                ),
            }
        }
        let file = files[&mf.file_id].clone();
        let route = match file.download_url.clone().filter(|u| !u.is_empty()) {
            Some(url) => Route::Direct(url),
            None => Route::Browser,
        };
        entries.push(Entry { required: mf.required, project, file, route });
    }

    // files the author blocks may exist unchanged on Modrinth
    let blocked: Vec<String> = entries
        .iter()
        .filter(|e| matches!(e.route, Route::Browser))
        .filter_map(|e| e.file.sha1().map(str::to_string))
        .collect();
    if !blocked.is_empty() {
        progress.stage(format!("Checking Modrinth for {} files CurseForge does not allow", blocked.len()));
        if let Ok(found) = modrinth_by_sha1(&blocked).await {
            for e in entries.iter_mut().filter(|e| matches!(e.route, Route::Browser)) {
                if let Some((url, project, version)) = e.file.sha1().and_then(|h| found.get(h)) {
                    e.route = Route::Modrinth { url: url.clone(), project: project.clone(), version: version.clone() };
                }
            }
        }
    }
    Ok(entries)
}

/// Modrinth versions (and their projects) for files by sha1: hash -> (download url, project, version).
async fn modrinth_by_sha1(hashes: &[String]) -> Result<HashMap<String, (String, Value, Value)>> {
    let client = crate::http::client();
    let versions: HashMap<String, Value> = client
        .post("https://api.modrinth.com/v2/version_files")
        .json(&json!({ "hashes": hashes, "algorithm": "sha1" }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if versions.is_empty() {
        return Ok(HashMap::new());
    }
    let ids: Vec<String> =
        versions.values().filter_map(|v| v.get("project_id").and_then(Value::as_str).map(str::to_string)).collect();
    let projects: Vec<Value> = client
        .get("https://api.modrinth.com/v2/projects")
        .query(&[("ids", serde_json::to_string(&ids)?)])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let mut out = HashMap::new();
    for (hash, version) in versions {
        let Some(pid) = version.get("project_id").and_then(Value::as_str) else { continue };
        let Some(project) = projects.iter().find(|p| p.get("id").and_then(Value::as_str) == Some(pid)) else {
            continue;
        };
        let url = version
            .get("files")
            .and_then(Value::as_array)
            .and_then(|fs| {
                fs.iter().find(|f| {
                    f.pointer("/hashes/sha1").and_then(Value::as_str).is_some_and(|h| h.eq_ignore_ascii_case(&hash))
                })
            })
            .and_then(|f| f.get("url").and_then(Value::as_str))
            .map(str::to_string);
        if let Some(url) = url {
            out.insert(hash.to_lowercase(), (url, project.clone(), version));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------
// the install itself

/// What a pack install produced for the instance folder.
pub struct Populated {
    pub records: Vec<ModRecord>,
    pub override_paths: Vec<String>,
}

fn safe_relative(p: &Path) -> bool {
    !p.is_absolute()
        && !p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_) | std::path::Component::RootDir
            )
        })
}

/// Download the pack's mods into the instance folder `root` and copy the pack's own files over it.
pub async fn populate(
    dirs: &Dirs,
    cf: &CfSettings,
    root: &Path,
    mc: &str,
    manifest: &Manifest,
    zip_path: &Path,
    progress: &Arc<Progress>,
    hub: Option<&Arc<BrowserHub>>,
) -> Result<Populated> {
    let entries = plan_entries(manifest, progress).await?;

    let mut jobs = Vec::new();
    let mut by_hand: Vec<usize> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let name =
            Path::new(&e.file.file_name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.is_empty() || name != e.file.file_name {
            bail!("The modpack lists a file with the odd name \"{}\"", e.file.file_name);
        }
        let dest = root.join(e.folder()).join(&name);
        match &e.route {
            Route::Direct(url) | Route::Modrinth { url, .. } => {
                let mut job = Job::new(url, dest);
                job.sha1 = e.file.sha1().map(str::to_string);
                job.size = Some(e.file.file_length).filter(|s| *s > 0).map(|s| s as u64);
                job.keep_on_mismatch = true;
                jobs.push(job);
            }
            Route::Browser => by_hand.push(i),
        }
    }
    progress.stage(format!("Downloading {} files from the modpack", jobs.len()));
    core_install::download_all(jobs, progress, true).await?;
    // files with only an md5 listed were not checked by the downloader
    for e in entries
        .iter()
        .filter(|e| !matches!(e.route, Route::Browser) && e.file.sha1().is_none() && e.file.md5().is_some())
    {
        let p = root.join(e.folder()).join(&e.file.file_name);
        if p.is_file() && !super::browser::contents_match(&p, e.file.md5(), None) {
            // kept, like a file whose SHA-1 disagrees (see `Job::keep_on_mismatch`), and said when the install ends
            crate::logs::warn(format!("Kept {} although its MD5 doesn't match CurseForge's", e.file.file_name));
            progress.note(e.file.file_name.clone());
        }
    }

    // files the player downloads in a browser
    let mut skipped: Vec<usize> = Vec::new();
    if !by_hand.is_empty() {
        progress.stage(format!("{} files must be downloaded in your browser", by_hand.len()));
        let local;
        let hub_ref = match hub {
            Some(h) => h,
            None => {
                local = BrowserHub::new(dirs, cf.custom_downloads_path.as_deref());
                &local
            }
        };
        let requests: Vec<BrowserRequest> = by_hand
            .iter()
            .map(|&i| {
                let e = &entries[i];
                BrowserRequest {
                    label: e.project.name.clone(),
                    file_name: e.file.file_name.clone(),
                    url: e.project.browser_download_url(&e.file),
                    size: e.file.file_length.max(0) as u64,
                    md5: e.file.md5().map(str::to_string),
                    sha1: e.file.sha1().map(str::to_string),
                }
            })
            .collect();
        // with no window to press "Open" in, open every page at once
        let found = fetch_by_hand(hub_ref, requests, hub.is_none(), HAND_DOWNLOAD_WAIT).await?;
        for (&i, got) in by_hand.iter().zip(found) {
            match got {
                Some(path) => {
                    let dest = root.join(entries[i].folder()).join(&entries[i].file.file_name);
                    if let Some(dir) = dest.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    std::fs::copy(&path, &dest).with_context(|| format!("copying {}", entries[i].file.file_name))?;
                }
                None => skipped.push(i),
            }
        }
    }

    // the records, in the pack's order
    let mut records: Vec<ModRecord> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let mut r = pack_record_for(&e.project, &e.file, e.required);
        if let Route::Modrinth { project, version, .. } = &e.route {
            r.modrinth_project = Some(project.clone());
            r.modrinth_version = Some(version.clone());
        }
        r.skipped = skipped.contains(&i);
        // a mod the pack lists as optional starts switched on, as in the Java edition
        records.push(r);
    }

    // the pack's own files
    progress.stage("Copying the modpack's own files");
    let override_paths = extract_overrides(zip_path, manifest.overrides_dir(), root)?;
    records.extend(override_mod_records(root, &override_paths, mc));
    Ok(Populated { records, override_paths })
}

/// Copy `<overrides>/...` from the zip over the instance folder. Returns the files written that belong to the pack,
/// relative to it. Two kinds of file are treated differently:
/// - `instance.json` is the launcher's, never the pack's: one shipped in a pack would replace the player's memory,
///   Java and command settings, so it is never copied.
/// - Worlds the pack ships (a skyblock map) become the player's once they are there: a file that already exists
///   under `saves/` is never overwritten (an update would otherwise reset the player's progress), and none of them
///   are listed as the pack's, so a later update doesn't delete them either.
pub fn extract_overrides(zip_path: &Path, overrides: &str, root: &Path) -> Result<Vec<String>> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?).context("reading the modpack zip")?;
    let prefix = format!("{}/", overrides.trim_matches('/'));
    let mut written = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(enclosed) = entry.enclosed_name() else { continue };
        let name = enclosed.to_string_lossy().replace('\\', "/");
        let Some(rel) = name.strip_prefix(&prefix).filter(|r| !r.is_empty()) else { continue };
        if entry.is_dir() {
            continue;
        }
        let rel_path = Path::new(rel);
        if !safe_relative(rel_path) || rel.eq_ignore_ascii_case("instance.json") {
            continue;
        }
        let player_data = crate::packs::is_player_data(rel_path);
        let out = root.join(rel_path);
        if player_data && std::fs::symlink_metadata(&out).is_ok() {
            continue;
        }
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::File::create(&out).with_context(|| format!("writing {}", out.display()))?;
        crate::packutil::copy_zip_entry(&mut entry, &mut f).with_context(|| format!("unpacking {name}"))?;
        if !player_data {
            written.push(rel.to_string());
        }
    }
    Ok(written)
}

/// Name, version and description of a mod jar, read from `fabric.mod.json` or `mcmod.info` the way Java's
/// `Utils.getMCModForFile` / `getFabricModForFile` do.
pub fn mod_info_from_jar(path: &Path) -> Option<(String, String, String)> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut read = |name: &str| -> Option<Value> {
        let mut e = archive.by_name(name).ok()?;
        let s = crate::packutil::zip_entry_text(&mut e).ok()?;
        serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()
    };
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    if let Some(info) = read("mcmod.info") {
        let first = match &info {
            Value::Array(a) => a.first().cloned(),
            Value::Object(o) => o.get("modList").and_then(Value::as_array).and_then(|a| a.first().cloned()),
            _ => None,
        };
        if let Some(m) = first {
            if let Some(name) = text(&m, "name") {
                return Some((
                    name,
                    text(&m, "version").unwrap_or_else(|| "Unknown".into()),
                    text(&m, "description").unwrap_or_default(),
                ));
            }
        }
    }
    if let Some(m) = read("fabric.mod.json") {
        let name = text(&m, "name").or_else(|| text(&m, "id"))?;
        return Some((
            name,
            text(&m, "version").unwrap_or_else(|| "Unknown".into()),
            text(&m, "description").unwrap_or_default(),
        ));
    }
    None
}

/// Records for the jars a pack ships in its own `mods/`, `mods/<mc>/` and `datapacks/` folders.
fn override_mod_records(root: &Path, written: &[String], mc: &str) -> Vec<ModRecord> {
    let mut out = Vec::new();
    for rel in written {
        let lower = rel.to_lowercase();
        if !(lower.ends_with(".jar") || lower.ends_with(".zip")) {
            continue;
        }
        let parts: Vec<&str> = rel.split('/').collect();
        let kind = match parts.as_slice() {
            ["mods", _] => "mods",
            ["mods", v, _] if *v == mc => "dependency",
            ["datapacks", _] => "datapack",
            _ => continue,
        };
        let file = parts[parts.len() - 1];
        let mut r = ModRecord::new(file, file, kind);
        r.optional = true;
        r.version = "Unknown".into();
        r.description = String::new();
        if let Some((name, version, description)) = mod_info_from_jar(&root.join(rel)) {
            r.name = name;
            r.version = version;
            r.description = description;
        }
        out.push(r);
    }
    out
}

// ---------------------------------------------------------------------------------------------------------
// where a pack comes from

pub enum PackSource {
    /// A modpack project on CurseForge. `None` picks the newest release (or newest file when alpha and beta
    /// files are allowed).
    Project { project: Project, file: Option<File> },
    /// A downloaded CurseForge `.zip`.
    Zip(PathBuf),
}

/// The newest file of a modpack project that the player's settings allow.
pub async fn choose_pack_file(cf: &CfSettings, project: &Project) -> Result<File> {
    let mut files: Vec<File> = api::files(project.id).await?.into_iter().filter(|f| f.is_available).collect();
    files.sort_by(|a, b| b.id.cmp(&a.id));
    filter::pick_latest(&files, cf.allow_alpha_beta)
        .cloned()
        .ok_or_else(|| anyhow!("{} has no files to install", project.name))
}

/// `https://www.curseforge.com/minecraft/modpacks/<slug>[/files/<id>|/download/<id>]` -> slug and file id.
pub fn parse_modpack_url(url: &str) -> Option<(String, Option<i64>)> {
    let rest = url.trim().strip_prefix("https://www.curseforge.com/minecraft/modpacks/")?;
    let mut parts = rest.split(['/', '?', '#']).filter(|p| !p.is_empty());
    let slug = parts.next()?;
    if !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let file = match parts.next() {
        Some("files") | Some("download") => parts.next().and_then(|n| n.parse().ok()),
        _ => None,
    };
    Some((slug.to_string(), file))
}

/// Install the pack a CurseForge modpack address points at.
pub async fn install_from_url(
    dirs: &Dirs,
    cf: &CfSettings,
    url: &str,
    name: Option<String>,
    progress: &Arc<Progress>,
    hub: Option<&Arc<BrowserHub>>,
) -> Result<Instance> {
    let (slug, file_id) = parse_modpack_url(url).context("That is not a CurseForge modpack address")?;
    let project = api::project_by_slug(&slug, api::Section::Modpacks)
        .await?
        .with_context(|| format!("CurseForge has no modpack called \"{slug}\""))?;
    let file = match file_id {
        Some(id) => Some(api::file(project.id, id).await?),
        None => None,
    };
    install_pack(dirs, cf, PackSource::Project { project, file }, name, progress, hub).await
}

/// Create the version part of an instance: Minecraft, its loader and the files they need.
async fn prepare_game(
    dirs: &Dirs,
    mc: &str,
    loader: Loader,
    loader_version: Option<&str>,
    progress: &Arc<Progress>,
) -> Result<(crate::model::VersionJson, Option<LoaderVersion>)> {
    progress.stage(format!("Reading Minecraft {mc}"));
    let vanilla = core_install::vanilla_json(dirs, mc).await?;
    let (merged, info) = loaders::install(dirs, loader, mc, loader_version, &vanilla, progress).await?;
    if let Some(jv) = &merged.java_version {
        if !jv.component.is_empty() {
            crate::java::ensure_runtime(dirs, &jv.component, progress).await?;
        }
    }
    core_install::ensure_game_files(dirs, &merged, progress, false).await?;
    let mut version = merged;
    version.id = mc.to_string();
    Ok((version, info))
}

fn put_defaults(l: &mut crate::instances::LauncherBlock) {
    let defaults = [
        ("isDev", json!(false)),
        ("isPlayable", json!(true)),
        ("assetsMapToResources", json!(false)),
        ("checkForUpdates", json!(true)),
        ("ignoredUpdates", json!([])),
        ("ignoreAllUpdates", json!(false)),
    ];
    for (k, v) in defaults {
        l.extras.entry(k.to_string()).or_insert(v);
    }
}

/// Fill the launcher block the way the Java installer does for a CurseForge pack.
fn set_pack_fields(
    inst: &mut Instance,
    project: Option<&Project>,
    description: Option<String>,
    file: Option<&File>,
    manifest: &Manifest,
    pack_name: &str,
) {
    let l = &mut inst.file.launcher;
    l.pack = pack_name.to_string();
    l.vanilla_instance = false;
    l.pack_id = 0;
    l.version = manifest.version.clone().unwrap_or_else(|| "1.0.0".to_string());
    l.enable_curse_forge_integration = true;
    l.enable_editing_mods = true;
    if let Some(p) = project {
        l.external_pack_id = p.id;
        l.description = p.summary.clone();
        l.extras.insert("curseForgeProject".into(), serde_json::to_value(p).unwrap_or(Value::Null));
    }
    if let Some(d) = description {
        l.extras.insert("curseForgeProjectDescription".into(), Value::String(d));
    }
    match file {
        Some(f) => {
            l.extras.insert("curseForgeFile".into(), serde_json::to_value(f).unwrap_or(Value::Null));
        }
        None => {
            l.extras.remove("curseForgeFile");
        }
    }
    // Java clears the old-style stored manifest when it installs
    l.extras.remove("curseForgeManifest");
    l.extras.remove("curseManifest");
    put_defaults(l);
}

/// Install a modpack as a new instance. `name` defaults to the pack's name. On failure the half-made instance
/// folder is removed again.
pub async fn install_pack(
    dirs: &Dirs,
    cf: &CfSettings,
    source: PackSource,
    name: Option<String>,
    progress: &Arc<Progress>,
    hub: Option<&Arc<BrowserHub>>,
) -> Result<Instance> {
    // 1. the zip
    let (zip_path, project, file, is_temp) = match source {
        PackSource::Zip(p) => (p, None, None, false),
        PackSource::Project { project, file } => {
            let file = match file {
                Some(f) => f,
                None => choose_pack_file(cf, &project).await?,
            };
            let got = fetch_file(dirs, cf, &project, &file, progress, hub, true).await?;
            let Some(path) = got else { bail!("The modpack file was not downloaded") };
            (path, Some(project), Some(file), true)
        }
    };
    let result = install_from_zip(dirs, cf, &zip_path, project, file, name, progress, hub).await;
    if is_temp {
        let _ = std::fs::remove_file(&zip_path);
    }
    result
}

async fn install_from_zip(
    dirs: &Dirs,
    cf: &CfSettings,
    zip_path: &Path,
    mut project: Option<Project>,
    file: Option<File>,
    name: Option<String>,
    progress: &Arc<Progress>,
    hub: Option<&Arc<BrowserHub>>,
) -> Result<Instance> {
    progress.stage("Reading the modpack");
    let manifest = read_manifest(zip_path)?;
    // a local zip may still say which CurseForge project it is
    if project.is_none() {
        if let Some(id) = manifest.project_id.filter(|id| *id != 0) {
            project = api::project(id).await.ok();
        }
    }
    let description = match &project {
        Some(p) => api::description(p.id).await.ok(),
        None => None,
    };
    let pack_name = project
        .as_ref()
        .map(|p| p.name.clone())
        .or_else(|| Some(manifest.name.clone()).filter(|n| !n.is_empty()))
        .unwrap_or_else(|| "CurseForge modpack".to_string());
    let instance_name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| pack_name.clone());
    let mc = manifest.minecraft.version.clone();

    let (loader, loader_version) = resolve_loader(dirs, &manifest).await?;
    let mut instance =
        instances::create(dirs, &instance_name, &mc, loader, loader_version.as_deref(), progress).await?;
    let outcome: Result<()> = async {
        let populated = populate(dirs, cf, &instance.root, &mc, &manifest, zip_path, progress, hub).await?;
        set_pack_fields(&mut instance, project.as_ref(), description, file.as_ref(), &manifest, &pack_name);
        instance.file.launcher.extras.insert("overridePaths".into(), json!(populated.override_paths));
        instance.set_mod_records(&populated.records);
        instance.save().context("saving instance.json")?;

        // the pack's picture
        if let Some(logo) = project.as_ref().and_then(|p| p.logo.as_ref()).filter(|l| !l.url.is_empty()) {
            let job = Job::new(&logo.url, instance.root.join("instance.png"));
            let _ = core_install::download_all(vec![job], progress, false).await;
        }
        // mods the pack ships itself may be known to CurseForge
        progress.stage("Checking the pack's own mods on CurseForge");
        let _ = install::identify_unknown_mods(cf, &instance).await;
        Ok(())
    }
    .await;
    if let Err(e) = outcome {
        let _ = instances::delete(&instance);
        return Err(e);
    }
    instances::load_one(&instance.root).context("reading the new instance back")
}

// ---------------------------------------------------------------------------------------------------------
// reading a pack instance, and updates

fn extra<'a>(inst: &'a Instance, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| inst.file.launcher.extras.get(*k)).filter(|v| !v.is_null())
}

/// The CurseForge project of a pack instance, as saved by Java or by this launcher.
pub fn pack_project(inst: &Instance) -> Option<Project> {
    extra(inst, &["curseForgeProject"]).and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// The CurseForge file the pack instance was installed from.
pub fn pack_file(inst: &Instance) -> Option<File> {
    extra(inst, &["curseForgeFile"]).and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// An instance installed from a CurseForge project (`isCurseForgePack`).
pub fn is_curseforge_pack(inst: &Instance) -> bool {
    pack_project(inst).is_some() && pack_file(inst).is_some()
}

/// The project id to check for updates: from an old-style stored manifest, else from the project.
pub fn pack_project_id(inst: &Instance) -> Option<i64> {
    if let Some(m) = extra(inst, &["curseForgeManifest", "curseManifest"]) {
        return m.get("projectID").and_then(Value::as_i64);
    }
    pack_project(inst).map(|p| p.id)
}

/// The memory (MB) an old-style stored manifest recommends, if it has one.
pub fn recommended_ram(inst: &Instance) -> Option<i64> {
    extra(inst, &["curseForgeManifest", "curseManifest"])?.pointer("/minecraft/recommendedRam")?.as_i64()
}

/// The web links of a CurseForge pack instance, for a "Get help" menu.
#[derive(Clone, Debug, Default)]
pub struct PackLinks {
    pub website: Option<String>,
    pub issues: Option<String>,
    pub wiki: Option<String>,
    pub discord: Option<String>,
}

/// The first Discord invite found in a project description (`CurseForgeUtils.parseDescriptionForDiscordInvite`):
/// one of the launcher's custom links, else a `discord.gg/...` or `discord.com/invite/...` address.
pub fn discord_invite_from_description(description: &str, custom_links: &[String]) -> Option<String> {
    if let Some(l) = custom_links.iter().find(|l| !l.is_empty() && description.contains(l.as_str())) {
        return Some(format!("https://{l}"));
    }
    for start in ["https://discord.gg/", "https://discord.com/invite/"] {
        if let Some(i) = description.find(start) {
            let rest = &description[i + start.len()..];
            let code: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if !code.is_empty() {
                return Some(format!("{start}{code}"));
            }
        }
    }
    None
}

/// Website, issues, wiki and Discord links for a pack instance, in the order the Java edition looks for the
/// Discord one: the project's own social link, the description, then the launcher's override lists.
pub fn pack_links(dirs: &Dirs, inst: &Instance) -> PackLinks {
    let Some(project) = pack_project(inst) else { return PackLinks::default() };
    let config: Value = std::fs::read(dirs.configs.join("json").join("config.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let matching = &config["discordLinkMatching"];
    let custom: Vec<String> = matching["customLinks"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let description = extra(inst, &["curseForgeProjectDescription"]).and_then(Value::as_str).map(str::to_string);
    let discord = project
        .social_link(SOCIAL_DISCORD)
        .map(str::to_string)
        .or_else(|| description.as_deref().and_then(|d| discord_invite_from_description(d, &custom)))
        .or_else(|| matching["curseForgeProjectIdsToDiscordLink"][project.id.to_string()].as_str().map(str::to_string))
        .or_else(|| {
            project.authors.first().and_then(|a| {
                matching["curseForgeAuthorIdsToDiscordLink"][a.id.to_string()].as_str().map(str::to_string)
            })
        });
    PackLinks {
        website: project.website_url().map(str::to_string),
        issues: project.issues_url().map(str::to_string),
        wiki: project.wiki_url().map(str::to_string),
        discord,
    }
}

#[derive(Clone, Debug)]
pub struct PackUpdate {
    pub latest: File,
    /// The player chose to ignore this update (or all updates for the instance).
    pub ignored: bool,
}

fn ignored(inst: &Instance, file_id: i64) -> bool {
    let l = &inst.file.launcher.extras;
    if l.get("ignoreAllUpdates").and_then(Value::as_bool).unwrap_or(false) {
        return true;
    }
    l.get("ignoredUpdates")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().filter_map(Value::as_str).any(|v| v.eq_ignore_ascii_case(&file_id.to_string())))
}

/// Remember that the player does not want this update (`Instance.ignoreUpdate`).
pub fn ignore_update(inst: &Instance, file_id: i64) -> Result<()> {
    update_instance(&inst.root, |i| {
        let list = i.file.launcher.extras.entry("ignoredUpdates".to_string()).or_insert_with(|| json!([]));
        if let Some(a) = list.as_array_mut() {
            if !a.iter().filter_map(Value::as_str).any(|v| v == file_id.to_string()) {
                a.push(Value::String(file_id.to_string()));
            }
        }
    })
}

/// Pick the newest of a project's `latest_files` for an instance, following `CurseForgeUpdateManager`: newest by
/// id, and unless alpha and beta files are allowed, a release pack only moves to releases and a beta pack to
/// releases or betas.
pub fn latest_pack_file(inst: &Instance, project: &Project, cf: &CfSettings) -> Option<File> {
    let installed_type = pack_file(inst).and_then(|f| f.release_type);
    let mut files = project.latest_files.clone();
    files.sort_by(|a, b| b.id.cmp(&a.id));
    files.into_iter().find(|f| filter::release_type_ok(installed_type, f, cf.allow_alpha_beta))
}

/// The update for every CurseForge pack in `instances`, by folder name. One request covers all of them.
pub async fn check_pack_updates(
    dirs: &Dirs,
    cf: &CfSettings,
    instances: &[Instance],
) -> Result<HashMap<String, PackUpdate>> {
    if !super::platform_enabled(dirs, "modpacksEnabled") {
        return Ok(HashMap::new());
    }
    let packs: Vec<(&Instance, i64)> = instances
        .iter()
        .filter(|i| is_curseforge_pack(i) || extra(i, &["curseForgeManifest", "curseManifest"]).is_some())
        .filter_map(|i| pack_project_id(i).map(|id| (i, id)))
        .collect();
    if packs.is_empty() {
        return Ok(HashMap::new());
    }
    let ids: Vec<i64> = packs.iter().map(|(_, id)| *id).collect();
    let projects = api::projects_map(&ids).await?;
    let mut out = HashMap::new();
    for (inst, id) in packs {
        let Some(project) = projects.get(&id) else { continue };
        let Some(latest) = latest_pack_file(inst, project, cf) else { continue };
        // an update is a newest file that is not the installed one (a Java instance with a stored manifest
        // has no installed file to compare, so it is reported only when it is known)
        let current = pack_file(inst).map(|f| f.id).or_else(|| {
            extra(inst, &["curseForgeManifest", "curseManifest"]).and_then(|m| m.get("fileID")).and_then(Value::as_i64)
        });
        if current == Some(latest.id) {
            continue;
        }
        let is_ignored = ignored(inst, latest.id);
        out.insert(inst.folder.clone(), PackUpdate { latest, ignored: is_ignored });
    }
    Ok(out)
}

/// The update for one pack instance, if any.
pub async fn check_pack_update(dirs: &Dirs, cf: &CfSettings, instance: &Instance) -> Result<Option<PackUpdate>> {
    Ok(check_pack_updates(dirs, cf, std::slice::from_ref(instance)).await?.remove(&instance.folder))
}

/// Remove the files a previous pack install put in the instance (the mods it listed and its override files),
/// leaving everything the player added.
fn remove_old_pack_files(inst: &Instance) {
    let mc = inst.mc_version().to_string();
    for rec in inst.mod_records().iter().filter(|m| !m.user_added) {
        if let Some(p) = record_current_path(&inst.root, rec, &mc) {
            let _ = std::fs::remove_file(p);
        }
    }
    let overrides: Vec<String> = inst
        .file
        .launcher
        .extras
        .get("overridePaths")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    for rel in overrides {
        let p = Path::new(&rel);
        // worlds are never removed, even when an older install listed them as the pack's
        if !safe_relative(p) || crate::packs::is_player_data(p) {
            continue;
        }
        let full = inst.root.join(p);
        if full.is_file() {
            let _ = std::fs::remove_file(&full);
            // tidy empty folders up to the instance root
            let mut dir = full.parent().map(Path::to_path_buf);
            while let Some(d) = dir {
                if d == inst.root || !d.starts_with(&inst.root) || std::fs::remove_dir(&d).is_err() {
                    break;
                }
                dir = d.parent().map(Path::to_path_buf);
            }
        }
    }
}

/// Update a pack instance to `file` (a newer file of the same project, or an older one to go back). The pack's
/// own mods and files are replaced; mods the player added, worlds, `options.txt` and `servers.dat` stay.
pub async fn update_pack(
    dirs: &Dirs,
    cf: &CfSettings,
    instance: &Instance,
    file: &File,
    progress: &Arc<Progress>,
    hub: Option<&Arc<BrowserHub>>,
) -> Result<Instance> {
    let inst0 = instances::load_one(&instance.root).unwrap_or_else(|| instance.clone());
    let project = match pack_project(&inst0) {
        Some(p) => api::project(p.id).await.unwrap_or(p),
        None => bail!("This instance is not a CurseForge modpack, so there is nothing to update from"),
    };
    let got = fetch_file(dirs, cf, &project, file, progress, hub, true).await?;
    let Some(zip_path) = got else { bail!("The modpack file was not downloaded") };
    let result = async {
        progress.stage("Reading the modpack");
        let manifest = read_manifest(&zip_path)?;
        let mc = manifest.minecraft.version.clone();
        let description = api::description(project.id).await.ok();

        // a different Minecraft or loader means the game files are rebuilt
        let (loader, loader_version) = resolve_loader(dirs, &manifest).await?;
        let cur_loader = inst0.loader().map(|(k, v)| (k.to_string(), v.to_string()));
        let same_loader = match (&cur_loader, loader) {
            (None, Loader::Vanilla) => true,
            (Some((k, v)), l) if l != Loader::Vanilla => {
                k.eq_ignore_ascii_case(l.label())
                    && loader_version
                        .as_deref()
                        .map(|w| w == v || w.ends_with(&format!("-{v}")) || w == format!("{}-{v}", inst0.mc_version()))
                        .unwrap_or(true)
            }
            _ => false,
        };
        let new_game = if inst0.mc_version() != mc || !same_loader {
            Some(prepare_game(dirs, &mc, loader, loader_version.as_deref(), progress).await?)
        } else {
            None
        };

        // keep what the player set up
        let keep: Vec<(&str, Option<Vec<u8>>)> =
            ["options.txt", "servers.dat"].into_iter().map(|n| (n, std::fs::read(inst0.root.join(n)).ok())).collect();
        let user_records: Vec<ModRecord> = inst0.mod_records().into_iter().filter(|m| m.user_added).collect();

        // the old pack files are removed before the new ones are fetched, and fetching can fail (the network, a
        // file CurseForge won't hand out): a snapshot taken first puts the instance back as it was if it does
        progress.stage("Saving a snapshot of the instance");
        let snapshot = match crate::snapshots::save_before(dirs, &inst0, "before-update") {
            Ok(s) => Some(s),
            Err(e) => {
                crate::logs::warn(format!("Couldn't save a snapshot before updating {}: {e:#}", inst0.name()));
                None
            }
        };
        remove_old_pack_files(&inst0);
        let replaced = async {
            let populated = populate(dirs, cf, &inst0.root, &mc, &manifest, &zip_path, progress, hub).await?;
            for (n, data) in &keep {
                if let Some(d) = data {
                    let _ = std::fs::write(inst0.root.join(n), d);
                }
            }

            // read back from the disk, so settings changed while the update ran are kept (a pack can't ship an
            // instance.json any more: `extract_overrides` leaves it out)
            let mut inst = instances::load_one(&inst0.root).unwrap_or_else(|| inst0.clone());
            if let Some((version, info)) = new_game {
                inst.file.version = version;
                inst.file.launcher.loader_version = info;
            }
            set_pack_fields(&mut inst, Some(&project), description, Some(file), &manifest, &project.name);
            inst.file.launcher.extras.insert("overridePaths".into(), json!(populated.override_paths));
            let mut records = populated.records;
            records.retain(|r| !user_records.iter().any(|u| u.file == r.file && u.kind == r.kind));
            records.extend(user_records);
            inst.set_mod_records(&records);
            inst.save().context("saving instance.json")?;
            anyhow::Ok(inst)
        }
        .await;
        let inst = match replaced {
            Ok(inst) => inst,
            Err(e) => {
                if let Some(s) = &snapshot {
                    let dir = crate::snapshots::folder_for(dirs, &inst0);
                    if let Err(back) = crate::snapshots::restore(s, &inst0.root, &dir) {
                        return Err(e.context(format!(
                            "the update failed, and putting the instance back failed too ({back:#}); the snapshot is {}",
                            s.file.display()
                        )));
                    }
                    return Err(e.context("the update failed; the instance was put back as it was"));
                }
                return Err(e);
            }
        };
        progress.stage("Checking the pack's own mods on CurseForge");
        let _ = install::identify_unknown_mods(cf, &inst).await;
        instances::load_one(&inst.root).context("reading the updated instance back")
    }
    .await;
    let _ = std::fs::remove_file(&zip_path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-made manifest (the real one is in `testdata/`).
    const MANIFEST: &str = r#"{
        "minecraft": {"version": "1.20.1", "modLoaders": [{"id": "forge-47.2.0", "primary": true}], "recommendedRam": 6144},
        "manifestType": "minecraftModpack",
        "manifestVersion": 1,
        "name": "Sample Pack",
        "version": "2.3.1",
        "author": "someone",
        "projectID": 123456,
        "files": [
            {"projectID": 238222, "fileID": 4665738, "required": true},
            {"projectID": 306612, "fileID": 4800001, "required": false}
        ],
        "overrides": "overrides"
    }"#;

    #[test]
    fn manifest_parses() {
        let m = parse_manifest(MANIFEST).unwrap();
        assert_eq!(m.name, "Sample Pack");
        assert_eq!(m.minecraft.version, "1.20.1");
        assert_eq!(m.minecraft.recommended_ram, Some(6144));
        assert_eq!(m.files.len(), 2);
        assert_eq!(m.files[0].project_id, 238222);
        assert!(!m.files[1].required);
        assert_eq!(m.project_id, Some(123456));
        assert_eq!(m.overrides_dir(), "overrides");
    }

    /// The manifest.json of the real "Fabulously Optimized" pack from CurseForge (first six files kept).
    #[test]
    fn real_manifest_parses() {
        let m = parse_manifest(include_str!("testdata/manifest-fabulously-optimized.json")).unwrap();
        assert_eq!(m.name, "Fabulously Optimized");
        assert_eq!(m.version.as_deref(), Some("13.1.0-alpha.2"));
        assert_eq!(m.minecraft.version, "26.1.1");
        assert_eq!(m.minecraft.mod_loaders.len(), 1);
        assert_eq!(
            parse_loader_id(&m.minecraft.mod_loaders[0].id).unwrap(),
            LoaderId { loader: Loader::Fabric, version: "0.18.6".into() }
        );
        assert!(m.minecraft.mod_loaders[0].primary);
        assert_eq!(m.files.len(), 6);
        assert!(m.files.iter().all(|f| f.project_id > 0 && f.file_id > 0));
        assert_eq!(m.overrides_dir(), "overrides");
    }

    #[test]
    fn manifest_checks_type_and_tolerates_missing_parts() {
        assert!(parse_manifest(r#"{"manifestType":"somethingElse","minecraft":{"version":"1.20.1"}}"#).is_err());
        let m = parse_manifest(r#"{"manifestType":"minecraftModpack","minecraft":{"version":"1.12.2"},"name":"x"}"#)
            .unwrap();
        assert!(m.files.is_empty());
        assert_eq!(m.overrides_dir(), "overrides");
        assert!(m.minecraft.mod_loaders.is_empty());
    }

    #[test]
    fn loader_ids() {
        assert_eq!(
            parse_loader_id("forge-47.2.0").unwrap(),
            LoaderId { loader: Loader::Forge, version: "47.2.0".into() }
        );
        assert_eq!(parse_loader_id("fabric-0.15.11").unwrap().loader, Loader::Fabric);
        assert_eq!(parse_loader_id("neoforge-21.1.57").unwrap().loader, Loader::NeoForge);
        assert_eq!(parse_loader_id("quilt-0.26.0").unwrap().version, "0.26.0");
        assert!(parse_loader_id("liteloader-1").is_err());
    }

    #[test]
    fn discord_invites_are_found_in_descriptions() {
        let d = "Join us <a href=\"https://discord.gg/AbC123\">here</a> or at https://discord.com/invite/zzz";
        assert_eq!(discord_invite_from_description(d, &[]).as_deref(), Some("https://discord.gg/AbC123"));
        assert_eq!(
            discord_invite_from_description("see https://discord.com/invite/zzz9!", &[]).as_deref(),
            Some("https://discord.com/invite/zzz9")
        );
        assert_eq!(
            discord_invite_from_description(
                "our server: lunapixel.studio/discord",
                &["lunapixel.studio/discord".to_string()]
            )
            .as_deref(),
            Some("https://lunapixel.studio/discord")
        );
        assert_eq!(discord_invite_from_description("nothing here", &[]), None);
    }

    #[test]
    fn pack_addresses() {
        assert_eq!(
            parse_modpack_url("https://www.curseforge.com/minecraft/modpacks/all-the-mods-9"),
            Some(("all-the-mods-9".into(), None))
        );
        assert_eq!(
            parse_modpack_url("https://www.curseforge.com/minecraft/modpacks/rlcraft/files/4612345"),
            Some(("rlcraft".into(), Some(4612345)))
        );
        assert_eq!(
            parse_modpack_url("https://www.curseforge.com/minecraft/modpacks/rlcraft/download/99"),
            Some(("rlcraft".into(), Some(99)))
        );
        assert_eq!(parse_modpack_url("https://www.curseforge.com/minecraft/mc-mods/jei"), None);
    }

    #[test]
    fn overrides_are_copied_safely() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("bf-cf-pack-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("pack.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file("manifest.json", opts).unwrap();
            z.write_all(MANIFEST.as_bytes()).unwrap();
            z.start_file("overrides/config/thing.toml", opts).unwrap();
            z.write_all(b"a = 1").unwrap();
            z.start_file("overrides/mods/local.jar", opts).unwrap();
            z.write_all(b"not really a jar").unwrap();
            z.start_file("overrides/../escape.txt", opts).unwrap();
            z.write_all(b"no").unwrap();
            z.start_file("other/file.txt", opts).unwrap();
            z.write_all(b"no").unwrap();
            z.finish().unwrap();
        }
        let manifest = read_manifest(&zip_path).unwrap();
        assert_eq!(manifest.name, "Sample Pack");
        let root = dir.join("inst");
        let written = extract_overrides(&zip_path, manifest.overrides_dir(), &root).unwrap();
        assert!(written.contains(&"config/thing.toml".to_string()));
        assert!(written.contains(&"mods/local.jar".to_string()));
        assert_eq!(written.len(), 2);
        assert_eq!(std::fs::read_to_string(root.join("config/thing.toml")).unwrap(), "a = 1");
        assert!(!dir.join("escape.txt").exists());
        let recs = override_mod_records(&root, &written, "1.20.1");
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].kind, "mods");
        assert_eq!(recs[0].file, "local.jar");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A skyblock-style pack ships a world and (by mistake or on purpose) an instance.json. Updating it must not
    /// reset the player's world or replace their settings.
    #[test]
    fn overrides_never_overwrite_worlds_or_the_instance_file() {
        use std::io::Write;
        let dir = crate::testutil::scratch("cf-overrides-world");
        let zip_path = dir.join("pack.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            for (name, body) in [
                ("manifest.json", MANIFEST.as_bytes()),
                ("overrides/saves/Sky/level.dat", b"pristine".as_slice()),
                ("overrides/saves/Sky/region/r.0.0.mca", b"pristine region"),
                ("overrides/instance.json", br#"{"launcher":{"javaArguments":"-Xmx1G","enableCommands":true}}"#),
                ("overrides/config/a.toml", b"a=2"),
            ] {
                z.start_file(name, opts).unwrap();
                z.write_all(body).unwrap();
            }
            z.finish().unwrap();
        }
        let root = dir.join("inst");
        // first install: the world arrives, but it is the player's from now on
        let written = extract_overrides(&zip_path, "overrides", &root).unwrap();
        assert_eq!(std::fs::read(root.join("saves/Sky/level.dat")).unwrap(), b"pristine");
        assert_eq!(written, vec!["config/a.toml".to_string()], "only the pack's own files are listed");
        assert!(!root.join("instance.json").exists(), "a pack never ships the launcher's file");
        // the player plays; then the pack is updated
        std::fs::write(root.join("saves/Sky/level.dat"), b"hours of progress").unwrap();
        std::fs::write(root.join("instance.json"), b"{\"mine\":true}").unwrap();
        extract_overrides(&zip_path, "overrides", &root).unwrap();
        assert_eq!(std::fs::read(root.join("saves/Sky/level.dat")).unwrap(), b"hours of progress");
        assert_eq!(std::fs::read(root.join("instance.json")).unwrap(), b"{\"mine\":true}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_the_old_pack_never_touches_worlds_or_escapes() {
        let dir = crate::testutil::scratch("cf-remove-old");
        let root = dir.join("inst");
        for f in ["saves/W/level.dat", "config/old.toml", "options.txt"] {
            std::fs::create_dir_all(root.join(f).parent().unwrap()).unwrap();
            std::fs::write(root.join(f), b"x").unwrap();
        }
        std::fs::write(dir.join("outside.txt"), b"x").unwrap();
        let mut inst = Instance { folder: "inst".into(), mod_count: 0, root: root.clone(), file: Default::default() };
        inst.file.launcher.extras.insert(
            "overridePaths".into(),
            json!(["saves/W/level.dat", "config/old.toml", "../outside.txt", "", "/etc/hostname"]),
        );
        remove_old_pack_files(&inst);
        assert!(root.join("saves/W/level.dat").exists());
        assert!(!root.join("config/old.toml").exists());
        assert!(dir.join("outside.txt").exists());
        assert!(root.join("options.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
