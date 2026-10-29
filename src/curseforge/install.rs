//! Putting CurseForge files into an instance, with the files they depend on, and finding newer files for what is
//! already there. The records written to `launcher.mods` follow `Instance.addFileFromCurseForge` in the Java
//! edition, so the Java launcher shows, updates and reinstalls them like its own.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures::StreamExt;

use super::api;
use super::browser::{BrowserHub, BrowserRequest, fetch_by_hand};
use super::filter::{self, Target};
use super::model::*;
use super::{CfSettings, fingerprint};
use crate::install::{self, Job, Progress};
use crate::instances::{self, Instance};
use crate::modrecord::ModRecord;
use crate::paths::Dirs;

/// How long a hand-download is waited for when nobody is there to skip it (the command line).
const UNATTENDED_WAIT: Duration = Duration::from_secs(15 * 60);

/// A file name that is only a name: nothing that could write outside the folder it is put in.
fn safe_name(name: &str) -> Result<String> {
    let base = Path::new(name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if base.is_empty() || base != name || name == "." || name == ".." {
        bail!("CurseForge gave a file the name \"{name}\", which is not a plain file name");
    }
    Ok(base)
}

/// Where a record's file is (or would be) in the instance, the way `DisableableMod.getFile` decides.
pub fn record_path(root: &Path, rec: &ModRecord, mc_version: &str) -> Option<PathBuf> {
    let dir: PathBuf = if let Some(p) = rec.path.as_deref().filter(|p| !p.is_empty()) {
        root.join(p)
    } else {
        match rec.kind.as_str() {
            "jar" | "forge" | "mcpc" => root.join("jarmods"),
            "texturepack" => root.join("texturepacks"),
            "resourcepack" => root.join("resourcepacks"),
            "mods" => root.join("mods"),
            "plugins" | "plugin" => root.join("plugins"),
            "ic2lib" => root.join("mods/ic2"),
            "denlib" => root.join("mods/denlib"),
            "coremods" => root.join("coremods"),
            "shaderpack" => root.join("shaderpacks"),
            "datapack" => root.join("datapacks"),
            "dependency" if !mc_version.is_empty() => root.join("mods").join(mc_version),
            _ => return None,
        }
    };
    if rec.file.is_empty() {
        return None;
    }
    Some(dir.join(&rec.file))
}

/// The file a record points at right now: in `disabledmods/` when disabled, else in its own folder.
pub fn record_current_path(root: &Path, rec: &ModRecord, mc_version: &str) -> Option<PathBuf> {
    if rec.disabled { Some(root.join("disabledmods").join(&rec.file)) } else { record_path(root, rec, mc_version) }
}

/// The record Java writes for a file the player added from CurseForge.
pub fn record_for(project: &Project, file: &File) -> ModRecord {
    let mut r = ModRecord::new(&project.name, &file.file_name, project.install_type());
    r.version = file.display_name.clone();
    r.description = project.summary.clone();
    r.optional = true;
    r.disabled = false;
    r.user_added = true;
    r.was_selected = true;
    r.skipped = false;
    r.curse_forge_project_id = Some(project.id);
    r.curse_forge_file_id = Some(file.id);
    r.curse_forge_project = serde_json::to_value(project).ok();
    r.curse_forge_file = serde_json::to_value(file).ok();
    r
}

/// The record for a file that comes with a modpack (`CurseForgeFile.convertToMod` then the installer):
/// not user added, optional when the pack says it is not required.
pub fn pack_record_for(project: &Project, file: &File, required: bool) -> ModRecord {
    let mut r = record_for(project, file);
    r.kind = project.mod_type().to_string();
    r.user_added = false;
    r.optional = !required;
    r
}

/// Reload the instance from disk, change it, save it. Doing it this way means two things installing at once
/// do not overwrite each other's records.
pub fn update_instance(root: &Path, change: impl FnOnce(&mut Instance)) -> Result<()> {
    let mut inst =
        instances::load_one(root).with_context(|| format!("reading {}", root.join("instance.json").display()))?;
    change(&mut inst);
    inst.save().context("saving instance.json")
}

fn fresh(instance: &Instance) -> Instance {
    instances::load_one(&instance.root).unwrap_or_else(|| instance.clone())
}

// ---------------------------------------------------------------------------------------------------------
// downloading one file

/// Put the file in `dirs.base/downloads/<name>` (the shared cache the Java edition uses), downloading it or, when
/// the author does not allow that, getting it through the browser. `None` means the player skipped it.
pub async fn fetch_file(
    dirs: &Dirs,
    cf: &CfSettings,
    project: &Project,
    file: &File,
    progress: &Arc<Progress>,
    hub: Option<&Arc<BrowserHub>>,
    open_browser: bool,
) -> Result<Option<PathBuf>> {
    let name = safe_name(&file.file_name)?;
    let staged = dirs.base.join("downloads").join(&name);
    match file.download_url.as_deref().filter(|u| !u.is_empty()) {
        Some(url) => {
            progress.stage(format!("Downloading {}", file.display_name));
            let mut job = Job::new(url, &staged);
            job.sha1 = file.sha1().map(str::to_string);
            job.size = Some(file.file_length).filter(|s| *s > 0).map(|s| s as u64);
            // a cached copy is reused only if it is the file we want
            if staged.is_file() && !super::browser::contents_match(&staged, file.md5(), file.sha1()) {
                let _ = std::fs::remove_file(&staged);
            }
            install::download_all(vec![job], progress, true).await?;
            // the downloader has already checked the sha1; with only an md5 listed, check that here
            let md5_only = if file.sha1().is_some() { None } else { file.md5() };
            if !super::browser::contents_match(&staged, md5_only, None) {
                let _ = std::fs::remove_file(&staged);
                bail!("{} arrived damaged; try again", file.file_name);
            }
            Ok(Some(staged))
        }
        None => {
            progress.stage(format!("Waiting for {} to be downloaded in your browser", file.display_name));
            let local;
            let hub = match hub {
                Some(h) => h,
                None => {
                    local = BrowserHub::new(dirs, cf.custom_downloads_path.as_deref());
                    &local
                }
            };
            let req = BrowserRequest {
                label: project.name.clone(),
                file_name: name,
                url: project.browser_download_url(file),
                size: file.file_length.max(0) as u64,
                md5: file.md5().map(str::to_string),
                sha1: file.sha1().map(str::to_string),
            };
            let mut got = fetch_by_hand(hub, vec![req], open_browser, UNATTENDED_WAIT).await?;
            Ok(got.pop().flatten())
        }
    }
}

/// Unpack a world's zip into `saves/`, refusing paths that would leave the folder.
pub fn extract_world(zip_path: &Path, saves: &Path) -> Result<()> {
    std::fs::create_dir_all(saves)?;
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?).context("that world file is not a zip")?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = saves.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::File::create(&out)?;
        crate::packutil::copy_zip_entry(&mut entry, &mut f)?;
    }
    Ok(())
}

/// Ask Modrinth whether it knows this exact file, and return its project and version objects to store next to
/// the CurseForge ones (the Java edition does this after every CurseForge install). Quietly gives nothing on any
/// failure; it is only a convenience.
async fn modrinth_lookup(path: &Path) -> Option<(serde_json::Value, serde_json::Value)> {
    let data = tokio::fs::read(path).await.ok()?;
    let sha1 = crate::http::sha1_hex(&data);
    let version: serde_json::Value =
        crate::http::get_json(&format!("https://api.modrinth.com/v2/version_file/{sha1}?algorithm=sha1")).await.ok()?;
    let project_id = version.get("project_id")?.as_str()?.to_string();
    let project: serde_json::Value =
        crate::http::get_json(&format!("https://api.modrinth.com/v2/project/{project_id}")).await.ok()?;
    Some((project, version))
}

/// Add one file to an instance (no dependencies): fetch it, replace older files of the same project, copy it
/// into the right folder and record it. Worlds are unpacked into `saves/` and not recorded. Returns `None` when
/// the file had to be downloaded by hand and the player skipped it.
pub async fn install_file(
    dirs: &Dirs,
    cf: &CfSettings,
    instance: &Instance,
    project: &Project,
    file: &File,
    progress: &Arc<Progress>,
    hub: Option<&Arc<BrowserHub>>,
) -> Result<Option<ModRecord>> {
    let Some(staged) = fetch_file(dirs, cf, project, file, progress, hub, true).await? else { return Ok(None) };
    let name = safe_name(&file.file_name)?;
    let root = instance.root.clone();
    let mc = instance.mc_version().to_string();
    let folder = root.join(project.instance_folder());

    progress.stage(format!("Adding {} to {}", project.name, instance.name()));
    let current = fresh(instance);
    // older files of the same project are removed once the new one is in place
    let older: Vec<PathBuf> = current
        .mod_records()
        .iter()
        .filter(|m| m.curse_forge_project_id == Some(project.id))
        .filter_map(|m| record_current_path(&root, m, &mc))
        .filter(|p| p.file_name().is_some_and(|n| *n != *name))
        .collect();

    if project.is_world() {
        extract_world(&staged, &root.join("saves"))?;
        return Ok(Some(record_for(project, file)));
    }
    std::fs::create_dir_all(&folder)?;
    let final_path = folder.join(&name);
    if final_path.exists() {
        std::fs::remove_file(&final_path)?;
    }
    std::fs::copy(&staged, &final_path).with_context(|| format!("copying {name} into the instance"))?;
    for p in older {
        let _ = std::fs::remove_file(p);
    }

    let mut record = record_for(project, file);
    if !cf.dont_check_modrinth {
        if let Some((p, v)) = modrinth_lookup(&final_path).await {
            record.modrinth_project = Some(p);
            record.modrinth_version = Some(v);
        }
    }
    let to_store = record.clone();
    // every record of this project is replaced, whatever file it had
    update_instance(&root, move |inst| inst.upsert_mod_record(to_store))?;
    Ok(Some(record))
}

// ---------------------------------------------------------------------------------------------------------
// dependencies

/// A file's dependency, with the project and the file that would be installed for it.
#[derive(Clone, Debug)]
pub struct Dependency {
    pub project: Project,
    /// The newest file that fits the instance, `None` when the project has nothing for it.
    pub file: Option<File>,
    /// [`REL_REQUIRED`] or [`REL_OPTIONAL`].
    pub relation: i64,
}

impl Dependency {
    pub fn is_required(&self) -> bool {
        self.relation == REL_REQUIRED
    }
}

/// Whether the instance already has CurseForge project `id` (or the same thing from Modrinth).
fn already_has(mods: &[ModRecord], id: i64, target: &Target) -> bool {
    if mods.iter().any(|m| m.curse_forge_project_id == Some(id) && m.curse_forge_file_id.is_some()) {
        return true;
    }
    let mr = |code: &str| mods.iter().any(|m| m.modrinth_project_id().is_some_and(|p| p.eq_ignore_ascii_case(code)));
    match id {
        super::FABRIC_API_ID => {
            mr(super::MODRINTH_FABRIC_API_ID)
                || (target.loader == filter::LoaderKind::Quilt && mr(super::MODRINTH_QSL_ID))
        }
        super::LEGACY_FABRIC_API_ID => mr(super::MODRINTH_LEGACY_FABRIC_API_ID),
        super::FORGIFIED_FABRIC_API_ID => mr(super::MODRINTH_FORGIFIED_FABRIC_API_ID),
        _ => false,
    }
}

/// The required and optional dependencies of `file` that the instance does not have yet. The Java edition
/// shows only the required ones; the optional ones are offered here as unticked choices.
pub async fn dependencies_of(
    dirs: &Dirs,
    cf: &CfSettings,
    instance: &Instance,
    file: &File,
) -> Result<Vec<Dependency>> {
    let instance = fresh(instance);
    let target = Target::from_instance(&instance, dirs, cf);
    let mods = instance.mod_records();
    let wanted: Vec<(i64, i64)> = file
        .dependencies
        .iter()
        .filter(|d| d.is_required() || d.is_optional())
        // Fabric API is not wanted next to Sinytra Connector
        .filter(|d| !(d.is_required() && d.mod_id == super::FABRIC_API_ID && target.sinytra))
        .filter(|d| !already_has(&mods, d.mod_id, &target))
        .map(|d| (d.mod_id, d.relation_type))
        .collect();
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = wanted.iter().map(|d| d.0).collect();
    let projects = api::projects_map(&ids).await?;
    let cf2 = cf.clone();
    let results: Vec<Option<Dependency>> = futures::stream::iter(wanted)
        .map(|(id, relation)| {
            let project = projects.get(&id).cloned();
            let target = target.clone();
            let cf = cf2.clone();
            async move {
                let project = match project {
                    Some(p) => p,
                    None => api::project(id).await.ok()?,
                };
                let files = api::files(id).await.ok().unwrap_or_default();
                let fitting = filter::files_for_target(&project, &files, &target, &cf);
                let chosen = filter::pick_latest(&fitting, cf.allow_alpha_beta).cloned();
                Some(Dependency { project, file: chosen, relation })
            }
        })
        .buffered(6)
        .collect()
        .await;
    let mut seen = HashSet::new();
    Ok(results.into_iter().flatten().filter(|d| seen.insert(d.project.id)).collect())
}

/// What an install did.
#[derive(Default, Debug)]
pub struct InstallReport {
    pub installed: Vec<ModRecord>,
    /// Files the player chose not to download by hand.
    pub skipped: Vec<String>,
    /// Dependencies that could not be added (the main file still went in): name and reason.
    pub problems: Vec<(String, String)>,
}

impl InstallReport {
    pub fn summary(&self) -> String {
        let names: Vec<&str> = self.installed.iter().map(|r| r.name.as_str()).collect();
        let mut s = format!("Added {}", if names.is_empty() { "nothing".to_string() } else { names.join(", ") });
        if !self.skipped.is_empty() {
            s.push_str(&format!("; skipped {}", self.skipped.join(", ")));
        }
        if !self.problems.is_empty() {
            s.push_str(&format!(
                "; could not add {}",
                self.problems.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
        s
    }
}

/// Install `file` and, from its dependencies, the projects in `include` (ids taken from [`dependencies_of`]),
/// along with whatever those need in turn. A dependency that cannot be added is reported, not fatal.
pub async fn install_with_dependencies(
    dirs: &Dirs,
    cf: &CfSettings,
    instance: &Instance,
    project: &Project,
    file: &File,
    include: &[i64],
    progress: &Arc<Progress>,
    hub: Option<&Arc<BrowserHub>>,
) -> Result<InstallReport> {
    let mut report = InstallReport::default();
    match install_file(dirs, cf, instance, project, file, progress, hub).await? {
        Some(r) => report.installed.push(r),
        None => report.skipped.push(project.name.clone()),
    }

    let mut queue: Vec<i64> = include.to_vec();
    let mut seen: HashSet<i64> = HashSet::new();
    seen.insert(project.id);
    while let Some(id) = queue.pop() {
        if !seen.insert(id) {
            continue;
        }
        let current = fresh(instance);
        let target = Target::from_instance(&current, dirs, cf);
        if already_has(&current.mod_records(), id, &target) {
            continue;
        }
        let step: Result<Option<(Project, File)>> = async {
            let p = api::project(id).await?;
            let files = api::files(id).await?;
            let fitting = filter::files_for_target(&p, &files, &target, cf);
            Ok(filter::pick_latest(&fitting, cf.allow_alpha_beta).cloned().map(|f| (p, f)))
        }
        .await;
        match step {
            Ok(Some((p, f))) => {
                // what this dependency needs is added too
                queue.extend(f.dependencies.iter().filter(|d| d.is_required()).map(|d| d.mod_id));
                match install_file(dirs, cf, &current, &p, &f, progress, hub).await {
                    Ok(Some(r)) => report.installed.push(r),
                    Ok(None) => report.skipped.push(p.name.clone()),
                    Err(e) => report.problems.push((p.name.clone(), format!("{e:#}"))),
                }
            }
            Ok(None) => report.problems.push((format!("project {id}"), "no file fits this instance".to_string())),
            Err(e) => report.problems.push((format!("project {id}"), format!("{e:#}"))),
        }
    }
    Ok(report)
}

/// Install the best file of a project for the instance together with every required dependency: what a single
/// "Install" click does.
pub async fn install_latest(
    dirs: &Dirs,
    cf: &CfSettings,
    instance: &Instance,
    project: &Project,
    progress: &Arc<Progress>,
    hub: Option<&Arc<BrowserHub>>,
) -> Result<InstallReport> {
    progress.stage(format!("Finding a version of {}", project.name));
    let target = Target::from_instance(instance, dirs, cf);
    let files = api::files(project.id).await?;
    let fitting = filter::files_for_target(project, &files, &target, cf);
    let Some(file) = filter::pick_latest(&fitting, cf.allow_alpha_beta).cloned() else {
        bail!(
            "No file of {} fits Minecraft {}{}. Loosen the \"add mod restriction\" setting to see others.",
            project.name,
            target.mc_version,
            if target.loader == filter::LoaderKind::None {
                String::new()
            } else {
                format!(" with {}", instance.loader().map(|l| l.0).unwrap_or("this loader"))
            }
        );
    };
    let deps = dependencies_of(dirs, cf, instance, &file).await?;
    let include: Vec<i64> = deps.iter().filter(|d| d.is_required() && d.file.is_some()).map(|d| d.project.id).collect();
    install_with_dependencies(dirs, cf, instance, project, &file, &include, progress, hub).await
}

// ---------------------------------------------------------------------------------------------------------
// update checks

/// A newer file for something installed.
#[derive(Clone, Debug)]
pub struct ModUpdate {
    /// The recorded file name of the installed mod.
    pub installed_file: String,
    pub installed_name: String,
    pub installed_file_id: i64,
    pub project: Project,
    pub latest: File,
}

/// The newest file that could replace record `rec`, if one fits the instance and the alpha/beta setting.
pub async fn update_for_record(
    dirs: &Dirs,
    cf: &CfSettings,
    instance: &Instance,
    rec: &ModRecord,
) -> Result<Option<ModUpdate>> {
    let (Some(project_id), Some(file_id)) = (rec.curse_forge_project_id, rec.curse_forge_file_id) else {
        return Ok(None);
    };
    let target = Target::from_instance(instance, dirs, cf);
    let files = api::files(project_id).await?;
    let project = match rec.curse_forge_project.clone().and_then(|v| serde_json::from_value::<Project>(v).ok()) {
        Some(p) => p,
        None => api::project(project_id).await?,
    };
    let fitting = filter::files_for_target(&project, &files, &target, cf);
    let installed_type = rec
        .curse_forge_file
        .as_ref()
        .and_then(|f| f.get("releaseType").and_then(|v| v.as_i64()))
        .or_else(|| files.iter().find(|f| f.id == file_id).and_then(|f| f.release_type));
    Ok(filter::newer_file(&fitting, file_id, installed_type, cf.allow_alpha_beta).map(|latest| ModUpdate {
        installed_file: rec.file.clone(),
        installed_name: rec.name.clone(),
        installed_file_id: file_id,
        project,
        latest: latest.clone(),
    }))
}

/// Check every CurseForge mod of an instance by its stored project and file ids. Returns the ones with a
/// newer file; a mod that cannot be checked is left out rather than failing the whole check.
pub async fn check_updates(dirs: &Dirs, cf: &CfSettings, instance: &Instance) -> Result<Vec<ModUpdate>> {
    let instance = fresh(instance);
    let records: Vec<ModRecord> = instance
        .mod_records()
        .into_iter()
        .filter(|m| m.curse_forge_project_id.is_some() && m.curse_forge_file_id.is_some() && !m.skipped)
        .collect();
    let results: Vec<Option<ModUpdate>> = futures::stream::iter(records)
        .map(|rec| {
            let (inst, dirs, cf) = (&instance, dirs, cf);
            async move { update_for_record(dirs, cf, inst, &rec).await.ok().flatten() }
        })
        .buffered(6)
        .collect()
        .await;
    Ok(results.into_iter().flatten().collect())
}

/// Check by fingerprint instead: identify the given files on CurseForge (so mods installed outside the launcher
/// are covered) and report the ones with a newer file for this instance.
pub async fn check_updates_by_fingerprint(
    dirs: &Dirs,
    cf: &CfSettings,
    instance: &Instance,
    files: Vec<PathBuf>,
) -> Result<Vec<ModUpdate>> {
    let matches = identify_files(files).await?;
    let instance = fresh(instance);
    let target = Target::from_instance(&instance, dirs, cf);
    let mut out = Vec::new();
    for m in matches {
        let all = api::files(m.project.id).await.unwrap_or_default();
        let fitting = filter::files_for_target(&m.project, &all, &target, cf);
        if let Some(latest) = filter::newer_file(&fitting, m.file.id, m.file.release_type, cf.allow_alpha_beta) {
            out.push(ModUpdate {
                installed_file: m.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                installed_name: m.project.name.clone(),
                installed_file_id: m.file.id,
                project: m.project.clone(),
                latest: latest.clone(),
            });
        }
    }
    Ok(out)
}

/// Install an update found by the checks above (the old file of the project is replaced).
pub async fn apply_update(
    dirs: &Dirs,
    cf: &CfSettings,
    instance: &Instance,
    update: &ModUpdate,
    progress: &Arc<Progress>,
    hub: Option<&Arc<BrowserHub>>,
) -> Result<Option<ModRecord>> {
    // keep the player's enabled / disabled choice for the mod
    let was_disabled =
        fresh(instance).mod_records().iter().any(|m| m.curse_forge_project_id == Some(update.project.id) && m.disabled);
    let record = install_file(dirs, cf, instance, &update.project, &update.latest, progress, hub).await?;
    if was_disabled {
        if let Some(r) = &record {
            let root = instance.root.clone();
            let (id, file) = (update.project.id, r.file.clone());
            let from = root.join(update.project.instance_folder()).join(&file);
            let to_dir = root.join("disabledmods");
            if from.is_file()
                && std::fs::create_dir_all(&to_dir).is_ok()
                && std::fs::rename(&from, to_dir.join(&file)).is_ok()
            {
                update_instance(&root, move |inst| {
                    let mut mods = inst.mod_records();
                    for m in mods.iter_mut().filter(|m| m.curse_forge_project_id == Some(id)) {
                        m.disabled = true;
                    }
                    inst.set_mod_records(&mods);
                })?;
            }
        }
    }
    Ok(record)
}

// ---------------------------------------------------------------------------------------------------------
// identifying files by fingerprint

/// A file on disk that CurseForge recognised.
#[derive(Clone, Debug)]
pub struct FileMatch {
    pub path: PathBuf,
    pub fingerprint: u32,
    pub project: Project,
    pub file: File,
}

/// Fingerprint `paths` and ask CurseForge which projects and files they are. Only approved projects (status 4)
/// count, as in the Java edition.
pub async fn identify_files(paths: Vec<PathBuf>) -> Result<Vec<FileMatch>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let hashed: Vec<(PathBuf, u32)> = tokio::task::spawn_blocking(move || {
        paths.into_iter().filter_map(|p| fingerprint::fingerprint_file(&p).ok().map(|h| (p, h))).collect()
    })
    .await?;
    if hashed.is_empty() {
        return Ok(Vec::new());
    }
    let prints: Vec<u32> = hashed.iter().map(|(_, h)| *h).collect();
    let answer = api::fingerprints(&prints).await?;
    if answer.exact_matches.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = answer.exact_matches.iter().map(|m| m.id).collect();
    let projects = api::projects_map(&ids).await?;
    let mut out = Vec::new();
    for m in answer.exact_matches {
        let Some(project) = projects.get(&m.id).filter(|p| p.status == 4) else { continue };
        let print = m.file.package_fingerprint as u32;
        for (path, h) in hashed.iter().filter(|(_, h)| *h == print) {
            out.push(FileMatch { path: path.clone(), fingerprint: *h, project: project.clone(), file: m.file.clone() });
        }
    }
    Ok(out)
}

/// Fill a record with what CurseForge says about the file it points at (`checkModsOnCurseForge`): the ids, the
/// project and file objects, the mod's proper name and its description.
pub fn apply_match(rec: &mut ModRecord, m: &FileMatch) {
    rec.curse_forge_project_id = Some(m.project.id);
    rec.curse_forge_file_id = Some(m.file.id);
    rec.curse_forge_project = serde_json::to_value(&m.project).ok();
    rec.curse_forge_file = serde_json::to_value(&m.file).ok();
    rec.name = m.project.name.clone();
    rec.description = m.project.summary.clone();
}

/// Identify the recorded mods that have no CurseForge information yet and save what is found. Returns how many
/// records were filled in. Does nothing when `dontCheckModsOnCurseForge` is on.
pub async fn identify_unknown_mods(cf: &CfSettings, instance: &Instance) -> Result<usize> {
    if cf.dont_check_mods {
        return Ok(0);
    }
    let current = fresh(instance);
    let mc = current.mc_version().to_string();
    let paths: Vec<(String, PathBuf)> = current
        .mod_records()
        .iter()
        .filter(|m| m.curse_forge_project.is_none() && m.curse_forge_file.is_none())
        .filter_map(|m| record_current_path(&current.root, m, &mc).filter(|p| p.is_file()).map(|p| (m.file.clone(), p)))
        .collect();
    if paths.is_empty() {
        return Ok(0);
    }
    let matches = identify_files(paths.iter().map(|(_, p)| p.clone()).collect()).await?;
    if matches.is_empty() {
        return Ok(0);
    }
    let mut filled = 0;
    update_instance(&current.root, |inst| {
        let mut mods = inst.mod_records();
        for rec in mods.iter_mut().filter(|m| m.curse_forge_project.is_none() && m.curse_forge_file.is_none()) {
            let Some(path) = paths.iter().find(|(f, _)| *f == rec.file).map(|(_, p)| p) else { continue };
            if let Some(m) = matches.iter().find(|m| &m.path == path) {
                apply_match(rec, m);
                filled += 1;
            }
        }
        inst.set_mod_records(&mods);
    })?;
    Ok(filled)
}
