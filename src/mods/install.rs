//! Putting a Modrinth version into an instance and recording it in `launcher.mods` the way the Java edition does
//! (the whole Modrinth project and version are saved next to the mod, so both editions can update it later),
//! together with the mods it needs.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::api::MrVersion;
use super::files::{self, ModKey};
use super::metadata::ContentKind;
use super::versions;
use super::{AddModRestriction, ModContext};
use crate::dependencies::{self, ModVersion};
use crate::install::{self, Job, Progress};
use crate::instances::Instance;
use crate::modrecord::ModRecord;

/// What an install did.
#[derive(Clone, Debug, Default)]
pub struct InstallResult {
    /// Names of everything that was added, the thing asked for last.
    pub installed: Vec<String>,
    /// Things that didn't go to plan but didn't stop the install.
    pub warnings: Vec<String>,
}

/// Nothing is made for the instance.
#[derive(Debug)]
pub struct NoCompatibleVersion(pub String);

impl std::fmt::Display for NoCompatibleVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NoCompatibleVersion {}

/// How a file of a Modrinth project is recorded: `datapack`, `mods`, `shaderpack` or `resourcepack`.
pub fn record_type_for(project: &Value, version: &MrVersion, requested: Option<&str>) -> &'static str {
    if requested == Some("datapack") {
        return "datapack";
    }
    let project_type = project.get("project_type").and_then(Value::as_str).unwrap_or_default();
    if project_type == "datapack" || version.loaders.iter().any(|l| l == "datapack") {
        return "datapack";
    }
    match project_type {
        "mod" => "mods",
        "shader" => "shaderpack",
        _ => "resourcepack",
    }
}

fn folder_for(record_type: &str) -> &'static str {
    match record_type {
        "datapack" => "datapacks",
        "shaderpack" => "shaderpacks",
        "resourcepack" => "resourcepacks",
        _ => "mods",
    }
}

/// Downloads one version into the instance and records it, replacing an older version of the same project.
/// `requested` is the type the caller wants it filed as (`None` works it out from the project).
pub async fn install_version(
    instance: &mut Instance,
    project: &Value,
    version: &MrVersion,
    requested: Option<&str>,
    progress: &Arc<Progress>,
) -> Result<ModKey> {
    let file = version.primary_file().with_context(|| format!("{} has no file to download", version.name))?;
    let project_id = project.get("id").and_then(Value::as_str).unwrap_or(&version.project_id).to_string();
    let title = project.get("title").and_then(Value::as_str).unwrap_or(&file.filename).to_string();
    let record_type = record_type_for(project, version, requested);
    if !crate::packutil::safe_file_name(&file.filename) {
        bail!("{title} has a file name that isn't allowed: {}", file.filename);
    }
    let final_path = instance.root.join(folder_for(record_type)).join(&file.filename);

    // an older version of the same project, of the same sort (a project can be both a data pack and a resource
    // pack, each installed on its own), is replaced
    let records = instance.mod_records();
    let is_same = |m: &ModRecord| {
        m.modrinth_project_id().is_some_and(|id| id.eq_ignore_ascii_case(&project_id))
            && folder_for(&m.kind) == folder_for(record_type)
    };
    let same: Vec<ModRecord> = records.iter().filter(|m| is_same(m)).cloned().collect();

    progress.stage(format!("Downloading {title}"));
    let mut job = Job::new(&file.url, &final_path);
    job.sha1 = file.hashes.get("sha1").cloned();
    job.size = Some(file.size).filter(|s| *s > 0);
    // the download lands under another name and takes the place of a file of the same name only once it is whole
    // and checked, so the old version is removed only after that: a failed or cancelled download leaves it working
    install::download_all(vec![job], progress, true)
        .await
        .with_context(|| format!("Couldn't download {}. Please try again later.", file.filename))?;
    for old in &same {
        if let Some(p) = files::enabled_path(instance, old).filter(|p| p.is_file() && *p != final_path) {
            let _ = std::fs::remove_file(p);
        }
        // a switched-off copy of the old version is replaced too, not left behind with no record
        if let Some(p) = files::disabled_path(instance, old).filter(|p| old.disabled && p.is_file()) {
            let _ = std::fs::remove_file(p);
        }
    }

    let mut rec = ModRecord::new(&title, &file.filename, record_type);
    rec.version = version.name.clone();
    rec.optional = true;
    rec.description = project.get("description").and_then(Value::as_str).unwrap_or_default().to_string();
    rec.disabled = false;
    rec.user_added = true;
    rec.was_selected = true;
    rec.skipped = false;
    rec.modrinth_project = Some(project.clone());
    rec.modrinth_version = Some(version.raw.clone());

    let key = ModKey::of(&rec);
    let mut list: Vec<ModRecord> = records.into_iter().filter(|m| !is_same(m)).collect();
    list.push(rec);
    instance.set_mod_records(&list);
    instance.save()?;
    Ok(key)
}

/// The version that would be installed: the newest made for this instance, releases first.
pub fn pick<'a>(ctx: &ModContext, kind: ContentKind, versions_list: &'a [MrVersion]) -> Option<&'a MrVersion> {
    let game = Some(ctx.target.minecraft_version.as_str());
    if kind != ContentKind::Mods {
        return versions::pick(versions_list, &[], game, &ctx.nearby_versions);
    }
    let accepted = ctx.target.accepted_loaders();
    // some projects publish one file that is both a mod and a data pack; the launcher would file that as a data
    // pack, so a mod-only file is preferred
    let mod_only: Vec<MrVersion> =
        versions_list.iter().filter(|v| !v.loaders.iter().any(|l| l == "datapack")).cloned().collect();
    if let Some(chosen) = versions::pick(&mod_only, &accepted, game, &ctx.nearby_versions) {
        return versions_list.iter().find(|v| v.id == chosen.id);
    }
    versions::pick(versions_list, &accepted, game, &ctx.nearby_versions)
}

/// Installs the newest version of a project made for this instance, and for mods everything it needs.
pub async fn install_project(
    instance: &mut Instance,
    ctx: &ModContext,
    project_id: &str,
    kind: ContentKind,
    progress: &Arc<Progress>,
) -> Result<InstallResult> {
    progress.stage("Looking up versions");
    let project =
        ctx.api.project(project_id).await.context("Couldn't reach Modrinth. Check your connection and try again.")?;
    let all = ctx
        .api
        .versions(project_id, None, &[])
        .await
        .context("Couldn't reach Modrinth. Check your connection and try again.")?;
    let title = project.get("title").and_then(Value::as_str).unwrap_or(project_id).to_string();
    let Some(chosen) = pick(ctx, kind, &all).cloned() else {
        let loader = match (kind, ctx.target.loader_key) {
            (ContentKind::Mods, Some(k)) => format!(" with {}", versions::display_name(Some(k))),
            _ => String::new(),
        };
        return Err(NoCompatibleVersion(format!(
            "{title} has no version for Minecraft {}{loader}",
            ctx.target.minecraft_version
        ))
        .into());
    };
    install_chosen(instance, ctx, &project, chosen, kind, progress).await
}

/// Installs one exact version, and for mods whatever it needs.
pub async fn install_chosen(
    instance: &mut Instance,
    ctx: &ModContext,
    project: &Value,
    chosen: MrVersion,
    kind: ContentKind,
    progress: &Arc<Progress>,
) -> Result<InstallResult> {
    let mut result = InstallResult::default();
    let mut to_install: Vec<MrVersion> = Vec::new();
    if kind == ContentKind::Mods {
        let (adds, warnings) = plan_dependencies(instance, ctx, &chosen).await?;
        result.warnings = warnings;
        to_install = adds;
        if to_install.last().map(|v| v.id.as_str()) != Some(chosen.id.as_str()) {
            to_install.retain(|v| v.id != chosen.id);
            to_install.push(chosen.clone());
        }
    } else {
        to_install.push(chosen.clone());
    }

    let total = to_install.len();
    for (i, version) in to_install.into_iter().enumerate() {
        let main = version.id == chosen.id;
        let p = if main {
            project.clone()
        } else {
            ctx.api
                .project(&version.project_id)
                .await
                .context("Couldn't reach Modrinth. Check your connection and try again.")?
        };
        let title = p.get("title").and_then(Value::as_str).unwrap_or(&version.project_id).to_string();
        progress.stage(if total > 1 { format!("{title} ({}/{total})", i + 1) } else { title.clone() });
        install_version(instance, &p, &version, Some(if main { kind.record_type() } else { "mods" }), progress).await?;
        result.installed.push(title);
    }
    Ok(result)
}

/// Works out the dependencies of a mod version. Returns the versions to add, dependencies first, and the problems
/// found (which don't stop the install; the player is told).
pub async fn plan_dependencies(
    instance: &Instance,
    ctx: &ModContext,
    chosen: &MrVersion,
) -> Result<(Vec<MrVersion>, Vec<String>)> {
    // what is installed from Modrinth: project id to the version installed
    let mut installed_versions: HashMap<String, MrVersion> = HashMap::new();
    for m in instance.mod_records() {
        if files::is_from_modrinth(&m) {
            if let (Some(id), Some(v)) = (
                m.modrinth_project_id().map(str::to_string),
                m.modrinth_version.clone().and_then(MrVersion::from_value),
            ) {
                installed_versions.insert(id, v);
            }
        }
    }
    // only what the game will load counts as provided: a switched-off dependency is fetched (switched on) again
    let provided = files::enabled_project_ids(instance);

    let mut wanted = ModVersion::from_modrinth(chosen);
    // the version was already picked for this instance, maybe from a nearby Minecraft version
    wanted.loaders.clear();
    wanted.game_versions.clear();
    let mut request = dependencies::Request::new(wanted);
    request.game_version = Some(ctx.target.minecraft_version.clone());
    for id in &provided {
        let version = installed_versions
            .get(id)
            .map(|v| v.id.clone())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "installed".into());
        request.installed.insert(id.clone(), version);
    }
    request.installed.remove(&chosen.project_id);
    // a mod that needs one exact version of another (Iris and Sodium) gets it, replacing the one installed
    for dep in &chosen.dependencies {
        let (Some(pid), Some(vid)) = (&dep.project_id, &dep.version_id) else { continue };
        if dep.dependency_type == "required"
            && request.installed.get(pid).is_some_and(|have| have != vid)
            && installed_versions.contains_key(pid)
        {
            request.installed.remove(pid);
        }
    }

    let loaders = ctx.target.accepted_loaders();
    let mut cache: HashMap<String, Vec<MrVersion>> = HashMap::new();
    let mut known: HashMap<String, MrVersion> = HashMap::new();
    known.insert(chosen.id.clone(), chosen.clone());
    loop {
        // the planner is asked synchronously, so what it needs from the network is noted and fetched afterwards
        let (plan, need) = {
            let missing = std::cell::RefCell::new(Vec::<String>::new());
            let catalog = |project_id: &str| -> Vec<ModVersion> {
                if request.installed.contains_key(project_id) {
                    // only asked about to look for clashes: what's installed is known without the network
                    return installed_versions
                        .get(project_id)
                        .map(|v| vec![ModVersion::from_modrinth(v)])
                        .unwrap_or_default();
                }
                match cache.get(project_id) {
                    Some(list) => list.iter().map(ModVersion::from_modrinth).collect(),
                    None => {
                        missing.borrow_mut().push(project_id.to_string());
                        Vec::new()
                    }
                }
            };
            let plan = dependencies::plan(&request, &catalog);
            (plan, missing.into_inner())
        };
        if need.is_empty() {
            let mut adds = Vec::new();
            for step in &plan.adds {
                if let Some(v) = known.get(&step.version.version_id) {
                    adds.push(v.clone());
                }
            }
            let mut warnings = plan.problems.clone();
            warnings.extend(plan.conflicts.clone());
            return Ok((adds, warnings));
        }
        let mut seen = HashSet::new();
        for id in need {
            if !seen.insert(id.clone()) || cache.contains_key(&id) {
                continue;
            }
            let all = ctx.api.versions(&id, None, &[]).await.unwrap_or_default();
            let fits: Vec<MrVersion> = versions::compatible(&all, &loaders, Some(&ctx.target.minecraft_version), &[])
                .into_iter()
                .cloned()
                .collect();
            for v in &fits {
                known.insert(v.id.clone(), v.clone());
            }
            cache.insert(id, fits);
        }
    }
}

/// Whether anything of a project is made for this instance. Needs the network.
pub async fn is_available(ctx: &ModContext, project_id: &str, kind: ContentKind) -> bool {
    match ctx.api.versions(project_id, None, &[]).await {
        Ok(all) => pick(ctx, kind, &all).is_some(),
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------------------------------------
// Shaders and the performance pack

/// What shader packs need before they do anything on this instance.
pub fn shader_setup(instance: &Instance, target: &super::InstanceTarget) -> versions::ShaderSetup {
    let mods: Vec<ModRecord> = instance
        .mod_records()
        .into_iter()
        .filter(|m| m.kind == "mods" && m.was_selected && !m.skipped && !m.disabled)
        .collect();
    let ids: Vec<&str> =
        mods.iter().filter(|m| files::is_from_modrinth(m)).filter_map(|m| m.modrinth_project_id()).collect();
    let names: Vec<&str> = mods.iter().map(|m| m.file.as_str()).collect();
    versions::shader_setup(target.loader_key, versions::has_shader_loader(ids, names))
}

/// The project that runs shaders on this instance (Iris or Oculus), or `None` if there is none.
pub fn shader_loader_project(target: &super::InstanceTarget) -> Option<&'static str> {
    match versions::shader_setup(target.loader_key, false) {
        versions::ShaderSetup::AddIris => Some(versions::IRIS),
        versions::ShaderSetup::AddOculus => Some(versions::OCULUS),
        _ => None,
    }
}

/// Installs Iris (with Sodium) or Oculus, whichever fits the loader.
pub async fn install_shader_loader(
    instance: &mut Instance,
    ctx: &ModContext,
    progress: &Arc<Progress>,
) -> Result<InstallResult> {
    let loader = shader_loader_project(&ctx.target)
        .ok_or_else(|| NoCompatibleVersion("Shaders need a mod loader such as Fabric".into()))?;
    let mut result = install_project(instance, ctx, loader, ContentKind::Mods, progress).await?;
    if loader == versions::IRIS
        && !files::installed_project_ids(instance).contains(versions::SODIUM)
        && ctx.target.loader_key != Some(versions::FORGE)
        && is_available(ctx, versions::SODIUM, ContentKind::Mods).await
    {
        let sodium = install_project(instance, ctx, versions::SODIUM, ContentKind::Mods, progress).await?;
        result.installed.extend(sodium.installed);
    }
    Ok(result)
}

/// What the performance pack did.
#[derive(Clone, Debug, Default)]
pub struct PackResult {
    pub installed: Vec<String>,
    pub warnings: Vec<String>,
    /// Names of the mods nothing was available for, or that failed.
    pub skipped: Vec<String>,
}

/// Installs the performance mods that are available for this instance and not there yet.
pub async fn install_performance_pack(
    instance: &mut Instance,
    ctx: &ModContext,
    progress: &Arc<Progress>,
) -> PackResult {
    use super::performance;
    let mut result = PackResult::default();
    let slots = performance::slots_for(ctx.target.loader_key);
    progress.stage("Checking what's available");
    let installed: Vec<String> = files::installed_project_ids(instance).into_iter().collect();
    let mut available: HashSet<String> = HashSet::new();
    for slot in &slots {
        for id in &slot.project_ids {
            if is_available(ctx, id, ContentKind::Mods).await {
                available.insert(id.to_string());
            }
        }
    }
    let ids = performance::choose(&slots, |id| available.contains(id), &installed);
    for slot in &slots {
        let covered =
            slot.project_ids.iter().any(|id| ids.iter().any(|i| i == id) || installed.iter().any(|i| i == id));
        if !covered {
            result.skipped.push(performance::name_of(slot.project_ids[0]));
        }
    }
    for id in ids {
        match install_project(instance, ctx, &id, ContentKind::Mods, progress).await {
            Ok(one) => {
                result.installed.push(performance::name_of(&id));
                result.warnings.extend(one.warnings);
            }
            Err(e) => {
                result.warnings.push(format!("{e:#}"));
                result.skipped.push(performance::name_of(&id));
            }
        }
    }
    result
}

/// The versions of a project the player may pick from for an instance, newest first: the Java edition's version
/// selector rules (loader, data pack, and the `addModRestriction` setting).
pub fn selectable_versions(
    ctx: &ModContext,
    all: &[MrVersion],
    project_type: &str,
    record_type: &str,
    with_sinytra: bool,
) -> Vec<MrVersion> {
    let mut list: Vec<MrVersion> = all.to_vec();
    list.sort_by(|a, b| b.date_published.cmp(&a.date_published));

    let mc = ctx.target.minecraft_version.as_str();
    if ctx.settings.add_mod_restriction != AddModRestriction::None
        && ctx.target.loader_key.is_some()
        && project_type == "mod"
        && record_type != "datapack"
    {
        let key = ctx.target.loader_key.unwrap_or_default();
        let neo_forge_compat = ctx.target.neoforge_forge_versions.iter().any(|v| v == mc);
        let has_neoforge_version = all.iter().any(|v| {
            v.loaders.iter().any(|l| l == "neoforge") || (neo_forge_compat && v.loaders.iter().any(|l| l == "forge"))
        });
        let has_forge_version = all.iter().any(|v| v.loaders.iter().any(|l| l == "forge"));
        list.retain(|v| {
            let has = |l: &str| v.loaders.iter().any(|x| x == l);
            if has("fabric")
                && (key == versions::FABRIC
                    || key == versions::LEGACY_FABRIC
                    || key == versions::QUILT
                    || (with_sinytra && key == versions::FORGE && !has_forge_version)
                    || (with_sinytra && key == versions::NEOFORGE && !has_neoforge_version))
            {
                return true;
            }
            if has("legacy-fabric") && key == versions::LEGACY_FABRIC {
                return true;
            }
            if has("neoforge") && key == versions::NEOFORGE {
                return true;
            }
            if has("forge") && (key == versions::FORGE || (key == versions::NEOFORGE && neo_forge_compat)) {
                return true;
            }
            has("quilt") && key == versions::QUILT
        });
    }
    if record_type == "datapack" {
        list.retain(|v| v.loaders.iter().any(|l| l == "datapack"));
    }
    match ctx.settings.add_mod_restriction {
        AddModRestriction::Strict => list.retain(|v| v.game_versions.iter().any(|g| g == mc)),
        AddModRestriction::Lax if !ctx.nearby_versions.is_empty() => {
            list.retain(|v| v.game_versions.iter().any(|g| ctx.nearby_versions.contains(g)))
        }
        _ => {}
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::{InstanceTarget, ModSettings};
    use serde_json::json;

    fn version(id: &str, date: &str, loaders: &[&str], games: &[&str]) -> MrVersion {
        MrVersion::from_value(json!({"id": id, "project_id": "p", "name": id, "date_published": date, "version_type": "release", "loaders": loaders, "game_versions": games, "files": []})).unwrap()
    }

    fn ctx(loader: &str, mc: &str, restriction: AddModRestriction) -> ModContext {
        let mut settings = ModSettings::default();
        settings.add_mod_restriction = restriction;
        ModContext {
            api: Default::default(),
            settings,
            target: InstanceTarget {
                minecraft_version: mc.into(),
                loader_key: versions::loader_key(loader),
                loader_type: Some(loader.into()),
                neoforge_forge_versions: vec!["1.20.1".into()],
            },
            nearby_versions: vec!["1.20".into(), "1.20.1".into(), "1.20.2".into()],
        }
    }

    #[test]
    fn updates_are_only_ever_for_the_instances_own_minecraft() {
        for r in [AddModRestriction::Strict, AddModRestriction::Lax, AddModRestriction::None] {
            assert_eq!(ctx("Fabric", "1.20.1", r).update_game_versions(), vec!["1.20.1".to_string()]);
        }
        // the browser still follows the setting
        assert_eq!(ctx("Fabric", "1.20.1", AddModRestriction::None).allowed_game_versions(), None);
        assert_eq!(ctx("Fabric", "1.20.1", AddModRestriction::Lax).allowed_game_versions().unwrap().len(), 3);
    }

    #[test]
    fn types_follow_the_project() {
        let v = version("v", "2026-01-01", &["fabric"], &["1.20.1"]);
        assert_eq!(record_type_for(&json!({"project_type": "mod"}), &v, None), "mods");
        assert_eq!(record_type_for(&json!({"project_type": "shader"}), &v, None), "shaderpack");
        assert_eq!(record_type_for(&json!({"project_type": "resourcepack"}), &v, None), "resourcepack");
        assert_eq!(record_type_for(&json!({"project_type": "mod"}), &v, Some("datapack")), "datapack");
        let dp = version("v", "2026-01-01", &["datapack"], &["1.20.1"]);
        assert_eq!(record_type_for(&json!({"project_type": "mod"}), &dp, None), "datapack");
    }

    #[test]
    fn the_version_list_follows_the_restriction_and_loader() {
        let all = vec![
            version("fab-new", "2026-03-01", &["fabric"], &["1.20.1"]),
            version("fab-old-mc", "2026-02-01", &["fabric"], &["1.19.4"]),
            version("forge", "2026-04-01", &["forge"], &["1.20.1"]),
            version("fab-near", "2026-05-01", &["fabric"], &["1.20.2"]),
        ];
        let ids = |c: &ModContext| {
            selectable_versions(c, &all, "mod", "mods", false).iter().map(|v| v.id.clone()).collect::<Vec<_>>()
        };
        assert_eq!(ids(&ctx("Fabric", "1.20.1", AddModRestriction::Strict)), vec!["fab-new"]);
        assert_eq!(ids(&ctx("Fabric", "1.20.1", AddModRestriction::Lax)), vec!["fab-near", "fab-new"]);
        // with no restriction at all nothing is filtered, not even by loader
        assert_eq!(
            ids(&ctx("Fabric", "1.20.1", AddModRestriction::None)),
            vec!["fab-near", "forge", "fab-new", "fab-old-mc"]
        );
        assert_eq!(ids(&ctx("Forge", "1.20.1", AddModRestriction::Strict)), vec!["forge"]);
        // Quilt takes Fabric builds; NeoForge takes Forge ones on 1.20.1
        assert_eq!(ids(&ctx("Quilt", "1.20.1", AddModRestriction::Strict)), vec!["fab-new"]);
        assert_eq!(ids(&ctx("NeoForge", "1.20.1", AddModRestriction::Strict)), vec!["forge"]);
        // Sinytra Connector lets Forge run Fabric builds when there is no Forge build
        let only_fabric = vec![version("fab", "2026-01-01", &["fabric"], &["1.20.1"])];
        assert!(
            selectable_versions(&ctx("Forge", "1.20.1", AddModRestriction::Strict), &only_fabric, "mod", "mods", false)
                .is_empty()
        );
        assert_eq!(
            selectable_versions(&ctx("Forge", "1.20.1", AddModRestriction::Strict), &only_fabric, "mod", "mods", true)
                .len(),
            1
        );
    }

    #[test]
    fn picking_prefers_mod_only_files_and_releases() {
        let mixed = version("both", "2026-06-01", &["fabric", "datapack"], &["1.20.1"]);
        let plain = version("plain", "2026-01-01", &["fabric"], &["1.20.1"]);
        let c = ctx("Fabric", "1.20.1", AddModRestriction::Strict);
        let list = vec![mixed.clone(), plain];
        assert_eq!(pick(&c, ContentKind::Mods, &list).unwrap().id, "plain");
        // when only the combined file exists it is still used
        assert_eq!(pick(&c, ContentKind::Mods, &[mixed]).unwrap().id, "both");
        // packs ignore the loader
        let shader = version("sh", "2026-01-01", &["iris"], &["1.20.1"]);
        assert_eq!(pick(&c, ContentKind::Shaders, &[shader]).unwrap().id, "sh");
    }
}

/// Installing and updating as it goes wrong for players: the download fails half way through "Update all", a
/// project is both a data pack and a resource pack, a mod site sends a strange file name.
#[cfg(test)]
mod robustness_tests {
    use super::*;
    use crate::mods::files::tests_support::temp_instance;
    use crate::testutil::{Reply, Server, runtime, sha1};
    use serde_json::json;

    fn version_with_file(url: &str, filename: &str, body: &[u8]) -> MrVersion {
        MrVersion::from_value(json!({
            "id": "v2", "project_id": "p", "name": "2.0", "date_published": "2026-01-01",
            "files": [{"url": url, "filename": filename, "primary": true,
                       "hashes": {"sha1": sha1(body)}, "size": body.len()}]
        }))
        .unwrap()
    }

    fn record(name: &str, file: &str, kind: &str) -> ModRecord {
        let mut r = ModRecord::new(name, file, kind);
        r.modrinth_project = Some(json!({"id": "p"}));
        r
    }

    #[test]
    fn a_failed_update_keeps_the_old_version_and_a_good_one_replaces_it() {
        runtime().block_on(async {
            let server = Server::start().await;
            let mut inst = temp_instance("install-update");
            std::fs::write(inst.root.join("mods/old.jar"), b"old").unwrap();
            // the same Modrinth project, installed as a data pack too
            std::fs::create_dir_all(inst.root.join("datapacks")).unwrap();
            std::fs::write(inst.root.join("datapacks/p.zip"), b"data").unwrap();
            inst.set_mod_records(&[record("P", "old.jar", "mods"), record("P data", "p.zip", "datapack")]);
            let project = json!({"id": "p", "title": "P", "project_type": "mod"});
            let body = b"new jar".to_vec();

            // the server is down: the old version must still be there and still recorded
            let url = server.route("/new.jar", Reply::status(500));
            let v = version_with_file(&url, "new.jar", &body);
            assert!(install_version(&mut inst, &project, &v, Some("mods"), &Progress::new()).await.is_err());
            assert_eq!(std::fs::read(inst.root.join("mods/old.jar")).unwrap(), b"old");
            assert!(inst.mod_records().iter().any(|m| m.file == "old.jar"));

            // the server is back: the new version replaces the old one, and only the old mod
            server.route("/new.jar", Reply::ok(body.clone()));
            install_version(&mut inst, &project, &v, Some("mods"), &Progress::new()).await.unwrap();
            assert!(!inst.root.join("mods/old.jar").exists());
            assert_eq!(std::fs::read(inst.root.join("mods/new.jar")).unwrap(), body);
            assert_eq!(std::fs::read(inst.root.join("datapacks/p.zip")).unwrap(), b"data", "the data pack stays");
            let files: Vec<String> = inst.mod_records().iter().map(|m| m.file.clone()).collect();
            assert!(files.contains(&"p.zip".to_string()) && files.contains(&"new.jar".to_string()), "{files:?}");
            assert!(!files.contains(&"old.jar".to_string()));
            let _ = std::fs::remove_dir_all(&inst.root);
        });
    }

    #[test]
    fn updating_to_a_file_of_the_same_name_keeps_it_when_the_download_fails() {
        runtime().block_on(async {
            let server = Server::start().await;
            let mut inst = temp_instance("install-samename");
            std::fs::write(inst.root.join("mods/same.jar"), b"v1").unwrap();
            inst.set_mod_records(&[record("P", "same.jar", "mods")]);
            let project = json!({"id": "p", "title": "P", "project_type": "mod"});
            let url = server.route("/same.jar", Reply::cut(b"version two".to_vec(), 3));
            let v = version_with_file(&url, "same.jar", b"version two");
            assert!(install_version(&mut inst, &project, &v, Some("mods"), &Progress::new()).await.is_err());
            assert_eq!(std::fs::read(inst.root.join("mods/same.jar")).unwrap(), b"v1");
            let _ = std::fs::remove_dir_all(&inst.root);
        });
    }

    #[test]
    fn file_names_that_could_leave_the_folder_are_refused_before_downloading() {
        runtime().block_on(async {
            let server = Server::start().await;
            let mut inst = temp_instance("install-names");
            let project = json!({"id": "p", "title": "P", "project_type": "mod"});
            for evil in
                ["../evil.jar", "..\\evil.jar", "C:evil.jar", "a.jar:stream", "", "..", "/etc/x.jar", "a\nb.jar"]
            {
                let url = server.route("/f", Reply::ok(b"x".to_vec()));
                let v = version_with_file(&url, evil, b"x");
                assert!(install_version(&mut inst, &project, &v, None, &Progress::new()).await.is_err(), "{evil:?}");
            }
            assert_eq!(server.total(), 0, "nothing was downloaded");
            let _ = std::fs::remove_dir_all(&inst.root);
        });
    }
}
