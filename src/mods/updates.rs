//! Looking for newer versions of the mods in an instance, updating them, rolling them back to an older version,
//! and keeping the lists of updates the player chose to ignore.
//!
//! Modrinth is handled here. CurseForge lives in another module and plugs in through [`ExternalSource`]: it offers
//! updates for the mods it knows and applies them, and everything here (the offers list, "Update all") treats its
//! offers like any other.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use futures::future::BoxFuture;
use serde_json::{Value, json};

use super::api::{MrVersion, loaders_for_listing};
use super::files::{self, ModKey};
use super::identify;
use super::install;
use super::metadata::ContentKind;
use super::versions;
use super::{ModContext, ModSettings};
use crate::install::Progress;
use crate::instances::Instance;
use crate::modrecord::ModRecord;

/// Where a newer version comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Platform {
    Modrinth,
    CurseForge,
}

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Platform::Modrinth => "Modrinth",
            Platform::CurseForge => "CurseForge",
        }
    }
}

/// A newer version of an installed mod.
#[derive(Clone, Debug)]
pub struct UpdateOffer {
    pub key: ModKey,
    /// The mod's name.
    pub name: String,
    pub kind: ContentKind,
    pub platform: Platform,
    /// What is installed, as a label ("Sodium 0.9.1").
    pub current: String,
    /// What it would become.
    pub newer: String,
    /// The Modrinth version, for Modrinth offers.
    pub modrinth: Option<MrVersion>,
    /// Whatever the platform module needs to apply its own offers.
    pub external: Option<Value>,
}

/// Another mod platform's update checks (CurseForge). `src/curseforge.rs` implements this and the launcher hands
/// the implementation to [`find_updates_with`] and [`apply_update_with`].
pub trait ExternalSource: Send + Sync {
    fn platform(&self) -> Platform;

    /// Newer versions of the mods of `kind` this platform knows about and is responsible for
    /// (see [`preferred_platform`]). Returns an error if the platform can't be reached.
    fn find_updates<'a>(
        &'a self,
        instance: &'a Instance,
        ctx: &'a ModContext,
        kind: ContentKind,
    ) -> BoxFuture<'a, Result<Vec<UpdateOffer>>>;

    /// Installs the version in `offer`, replacing the mod; the instance's mod list is updated and saved.
    fn apply<'a>(
        &'a self,
        instance: &'a mut Instance,
        ctx: &'a ModContext,
        offer: &'a UpdateOffer,
        progress: &'a Arc<Progress>,
    ) -> BoxFuture<'a, Result<()>>;
}

/// Which platform checks a mod for updates: the Java edition's rule is that a mod known to both goes by the
/// `defaultModPlatform` setting, and a mod known to one goes by that one.
pub fn preferred_platform(rec: &ModRecord, settings: &ModSettings) -> Option<Platform> {
    let (cf, mr) = (files::is_from_curseforge(rec), files::is_from_modrinth(rec));
    match (cf, mr) {
        (true, true) => {
            Some(if settings.default_mod_platform == "CURSEFORGE" { Platform::CurseForge } else { Platform::Modrinth })
        }
        (true, false) => Some(Platform::CurseForge),
        (false, true) => Some(Platform::Modrinth),
        _ => None,
    }
}

/// Projects that must stay as they are because an installed mod needs one exact version of them (Iris needs a
/// matching Sodium and would break if Sodium moved on).
fn pinned_projects(records: &[ModRecord]) -> HashSet<String> {
    let mut pinned = HashSet::new();
    for m in records {
        if !files::is_from_modrinth(m) {
            continue;
        }
        let Some(deps) = m.modrinth_version.as_ref().and_then(|v| v.get("dependencies")).and_then(Value::as_array)
        else {
            continue;
        };
        for d in deps {
            let required = d.get("dependency_type").and_then(Value::as_str) == Some("required");
            if required && d.get("version_id").and_then(Value::as_str).is_some() {
                if let Some(p) = d.get("project_id").and_then(Value::as_str) {
                    pinned.insert(p.to_string());
                }
            }
        }
    }
    pinned
}

/// Whether moving from `current` to `newest` is an update the player would want: it must be newer, and someone on
/// a full release isn't moved to a beta (installs pick releases first too).
fn wanted_update(newest: &MrVersion, current: Option<&MrVersion>) -> bool {
    let riskier = newest.version_type.as_deref().is_some_and(|t| t != "release")
        && current.is_none_or(|c| c.version_type.as_deref().is_none_or(|t| t == "release"));
    let pair = |v: &MrVersion| (v.id.clone(), v.date_published.clone());
    let (n, c) = (pair(newest), current.map(pair));
    !riskier && versions::is_newer(Some((&n.0, &n.1)), c.as_ref().map(|(a, b)| (a.as_str(), b.as_str())))
}

/// Finds newer versions of the mods of one kind that came from Modrinth (one request for all of them).
pub async fn find_updates(instance: &Instance, ctx: &ModContext, kind: ContentKind) -> Result<Vec<UpdateOffer>> {
    find_updates_with(instance, ctx, kind, &[]).await
}

/// Like [`find_updates`], and also asks the other platforms' modules; their offers are added to the list.
pub async fn find_updates_with(
    instance: &Instance,
    ctx: &ModContext,
    kind: ContentKind,
    external: &[&dyn ExternalSource],
) -> Result<Vec<UpdateOffer>> {
    let records = instance.mod_records();
    let pinned = pinned_projects(&records);

    let mut candidates: Vec<(ModRecord, std::path::PathBuf)> = Vec::new();
    for e in files::entries(instance, kind) {
        let rec = e.record;
        if !files::is_from_modrinth(&rec) || preferred_platform(&rec, &ctx.settings) != Some(Platform::Modrinth) {
            continue;
        }
        if rec.modrinth_project_id().is_some_and(|id| pinned.contains(id)) {
            continue;
        }
        if let Some(p) = e.path.filter(|p| p.is_file()) {
            candidates.push((rec, p));
        }
    }
    let mut offers = Vec::new();
    if !candidates.is_empty() {
        let hashed = identify::hash_files(candidates.iter().map(|(_, p)| p.clone()).collect()).await;
        let mut by_hash: Vec<(String, &ModRecord)> = Vec::new();
        for (path, hash) in &hashed {
            if let Some((rec, _)) = candidates.iter().find(|(_, p)| p == path) {
                by_hash.push((hash.clone(), rec));
            }
        }
        let hashes: Vec<String> = by_hash.iter().map(|(h, _)| h.clone()).collect();
        let loaders = if kind == ContentKind::Mods { ctx.target.accepted_loaders() } else { Vec::new() };
        let game_versions = ctx.update_game_versions();
        let latest = ctx.api.latest_from_sha1(&hashes, &loaders, &game_versions).await?;
        for (hash, rec) in by_hash {
            let Some(newest) = latest.get(&hash) else { continue };
            let current = rec.modrinth_version.clone().and_then(MrVersion::from_value);
            if wanted_update(newest, current.as_ref()) {
                offers.push(UpdateOffer {
                    key: ModKey::of(rec),
                    name: files::name_of(rec),
                    kind,
                    platform: Platform::Modrinth,
                    current: current.map(|c| c.name).filter(|n| !n.is_empty()).unwrap_or_else(|| rec.version.clone()),
                    newer: newest.name.clone(),
                    modrinth: Some(newest.clone()),
                    external: None,
                });
            }
        }
    }
    for source in external {
        match source.find_updates(instance, ctx, kind).await {
            Ok(mut more) => offers.append(&mut more),
            Err(e) => bail!("Couldn't check {} for updates: {e:#}", source.platform().label()),
        }
    }
    offers.sort_by_key(|o| o.name.to_lowercase());
    Ok(offers)
}

/// Replaces a mod with a newer version of the same project, keeping it switched off if it was.
pub async fn apply_update(
    instance: &mut Instance,
    ctx: &ModContext,
    offer: &UpdateOffer,
    progress: &Arc<Progress>,
) -> Result<()> {
    apply_update_with(instance, ctx, offer, progress, &[]).await
}

pub async fn apply_update_with(
    instance: &mut Instance,
    ctx: &ModContext,
    offer: &UpdateOffer,
    progress: &Arc<Progress>,
    external: &[&dyn ExternalSource],
) -> Result<()> {
    match offer.platform {
        Platform::Modrinth => {
            let version = offer.modrinth.clone().context("this update has no version to install")?;
            install_over(instance, ctx, &offer.key, &version, progress).await
        }
        platform => {
            let source = external
                .iter()
                .find(|s| s.platform() == platform)
                .with_context(|| format!("{} updates aren't available", platform.label()))?;
            source.apply(instance, ctx, offer, progress).await
        }
    }
}

/// Installs `version` over the mod `key`: the old file goes, the new one comes, and a switched-off mod stays off.
pub async fn install_over(
    instance: &mut Instance,
    ctx: &ModContext,
    key: &ModKey,
    version: &MrVersion,
    progress: &Arc<Progress>,
) -> Result<()> {
    let rec = instance
        .mod_records()
        .into_iter()
        .find(|m| m.file == key.file && m.kind == key.kind)
        .with_context(|| format!("{} isn't in this instance any more", key.file))?;
    let project_id = rec.modrinth_project_id().context("this mod didn't come from Modrinth")?.to_string();
    // the project as Modrinth has it now, or as it was saved when that isn't possible
    let project = match ctx.api.project(&project_id).await {
        Ok(p) => p,
        Err(_) => {
            rec.modrinth_project.clone().context("Couldn't reach Modrinth. Check your connection and try again.")?
        }
    };
    let was_disabled = rec.disabled;
    // the install only clears out the switched-on copy of the old version; the switched-off one is removed once
    // the new version is downloaded, so a failed update keeps the old file
    let old_disabled = if was_disabled { files::disabled_path(instance, &rec).filter(|p| p.exists()) } else { None };
    let new_key = install::install_version(instance, &project, version, Some(&rec.kind), progress).await?;
    if let Some(p) = old_disabled.filter(|p| p.exists()) {
        std::fs::remove_file(&p).with_context(|| format!("removing the old {}", p.display()))?;
    }
    if was_disabled {
        files::set_enabled(instance, &new_key, false)?;
    }
    Ok(())
}

/// What updating several mods did.
#[derive(Clone, Debug, Default)]
pub struct UpdateAllReport {
    pub updated: Vec<String>,
    /// Name and the reason it failed.
    pub failed: Vec<(String, String)>,
}

/// Updates every offer, one at a time. One that fails doesn't stop the rest.
pub async fn update_all(
    instance: &mut Instance,
    ctx: &ModContext,
    offers: &[UpdateOffer],
    progress: &Arc<Progress>,
    external: &[&dyn ExternalSource],
) -> UpdateAllReport {
    let mut report = UpdateAllReport::default();
    for (i, offer) in offers.iter().enumerate() {
        progress.stage(format!("Updating {} ({}/{})", offer.name, i + 1, offers.len()));
        match apply_update_with(instance, ctx, offer, progress, external).await {
            Ok(()) => report.updated.push(offer.name.clone()),
            Err(e) => report.failed.push((offer.name.clone(), format!("{e:#}"))),
        }
    }
    report
}

/// The versions of a mod's project the player may pick from (for an update, a rollback or a reinstall), newest
/// first, following the Java edition's version selector.
pub async fn available_versions(instance: &Instance, ctx: &ModContext, key: &ModKey) -> Result<Vec<MrVersion>> {
    let rec = instance
        .mod_records()
        .into_iter()
        .find(|m| m.file == key.file && m.kind == key.kind)
        .context("that mod isn't in this instance")?;
    let project_id = rec.modrinth_project_id().context("this mod didn't come from Modrinth")?.to_string();
    let project_type = rec
        .modrinth_project
        .as_ref()
        .and_then(|p| p.get("project_type"))
        .and_then(Value::as_str)
        .unwrap_or("mod")
        .to_string();
    let all = ctx.api.versions(&project_id, None, &[]).await?;
    let with_sinytra = files::forge_like_with_sinytra(instance, &ctx.settings);
    Ok(install::selectable_versions(ctx, &all, &project_type, &rec.kind, with_sinytra))
}

/// The newest version of one mod that is newer than the one installed, or `None` when it is up to date. This is
/// the "Check for updates" of a single mod in the Java edition's Edit Mods window.
pub async fn check_single(instance: &Instance, ctx: &ModContext, key: &ModKey) -> Result<Option<MrVersion>> {
    let rec = instance
        .mod_records()
        .into_iter()
        .find(|m| m.file == key.file && m.kind == key.kind)
        .context("that mod isn't in this instance")?;
    let project_id = rec.modrinth_project_id().context("this mod didn't come from Modrinth")?.to_string();
    let installed = rec.modrinth_version.clone().and_then(MrVersion::from_value);
    // updates are always for this instance's Minecraft version, whatever the browser is set to show
    let mc_filter = Some(ctx.target.minecraft_version.as_str());
    let loaders =
        loaders_for_listing(ctx.target.loader_key, &ctx.target.minecraft_version, &ctx.target.neoforge_forge_versions);
    let all = ctx.api.versions(&project_id, mc_filter, &loaders).await?;
    let project_type = rec
        .modrinth_project
        .as_ref()
        .and_then(|p| p.get("project_type"))
        .and_then(Value::as_str)
        .unwrap_or("mod")
        .to_string();
    let with_sinytra = files::forge_like_with_sinytra(instance, &ctx.settings);
    let list = install::selectable_versions(ctx, &all, &project_type, &rec.kind, with_sinytra);
    Ok(list
        .into_iter()
        .find(|v| newer_by_a_second(&v.date_published, installed.as_ref().map(|i| i.date_published.as_str()))))
}

/// The Java edition counts a version as an update when it was published more than a second after the installed one.
fn newer_by_a_second(candidate: &str, installed: Option<&str>) -> bool {
    let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).ok();
    match (parse(candidate), installed.and_then(parse)) {
        (Some(c), Some(i)) => c - chrono::Duration::seconds(1) > i,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Installs the exact version `version_id` of a mod's project: for rolling back, picking an older version, or
/// reinstalling the same one.
pub async fn install_specific(
    instance: &mut Instance,
    ctx: &ModContext,
    key: &ModKey,
    version_id: &str,
    progress: &Arc<Progress>,
) -> Result<()> {
    progress.stage("Getting the version");
    let version = ctx.api.version(version_id).await?;
    install_over(instance, ctx, key, &version, progress).await
}

/// Downloads the installed version again (the Edit Mods window's "Reinstall").
pub async fn reinstall(
    instance: &mut Instance,
    ctx: &ModContext,
    key: &ModKey,
    progress: &Arc<Progress>,
) -> Result<()> {
    let rec = instance
        .mod_records()
        .into_iter()
        .find(|m| m.file == key.file && m.kind == key.kind)
        .context("that mod isn't in this instance")?;
    let version_id = rec.modrinth_version_id().context("this mod didn't come from Modrinth")?.to_string();
    install_specific(instance, ctx, key, &version_id, progress).await
}

/// Older versions than the installed one, newest first (for the rollback list).
pub fn older_than_installed<'a>(instance: &Instance, key: &ModKey, list: &'a [MrVersion]) -> Vec<&'a MrVersion> {
    let installed = instance
        .mod_records()
        .into_iter()
        .find(|m| m.file == key.file && m.kind == key.kind)
        .and_then(|r| r.modrinth_version)
        .and_then(MrVersion::from_value);
    let Some(installed) = installed else { return list.iter().collect() };
    list.iter().filter(|v| v.id != installed.id && v.date_published < installed.date_published).collect()
}

// ---------------------------------------------------------------------------------------------------------
// Modpack updates and the lists of updates the player ignores

fn launcher_str<'a>(instance: &'a Instance, key: &str) -> Option<&'a Value> {
    instance.file.launcher.extras.get(key).filter(|v| !v.is_null())
}

/// Whether the instance was installed from a Modrinth modpack.
pub fn is_modrinth_pack(instance: &Instance) -> bool {
    ["modrinthManifest", "modrinthProject", "modrinthVersion"].iter().all(|k| launcher_str(instance, k).is_some())
}

/// The project id and installed version id of the Modrinth modpack an instance came from.
pub fn modrinth_pack_ids(instance: &Instance) -> Option<(String, String)> {
    if !is_modrinth_pack(instance) {
        return None;
    }
    let project = launcher_str(instance, "modrinthProject")?.get("id")?.as_str()?.to_string();
    let version = launcher_str(instance, "modrinthVersion")?.get("id")?.as_str()?.to_string();
    Some((project, version))
}

/// The newest version of the Modrinth modpack an instance came from, or `None` when it isn't from one or the pack
/// has no versions.
pub async fn latest_modrinth_pack_version(instance: &Instance, ctx: &ModContext) -> Result<Option<MrVersion>> {
    let Some((project, _)) = modrinth_pack_ids(instance).filter(|(p, _)| !p.is_empty()) else { return Ok(None) };
    let mut all = ctx.api.versions(&project, None, &[]).await?;
    all.sort_by(|a, b| b.date_published.cmp(&a.date_published));
    Ok(all.into_iter().next())
}

/// Whether the latest version of a modpack is an update the player hasn't waved away.
pub fn pack_update_wanted(instance: &Instance, latest_version_id: &str, installed_version_id: &str) -> bool {
    !latest_version_id.is_empty()
        && latest_version_id != installed_version_id
        && !has_update_been_ignored(instance, latest_version_id)
}

/// `ignoreAllUpdates` on, or `version` in the `ignoredUpdates` list (case does not matter).
pub fn has_update_been_ignored(instance: &Instance, version: &str) -> bool {
    if ignore_all_updates_set(instance) {
        return true;
    }
    if version.is_empty() {
        return false;
    }
    ignored_updates(instance).iter().any(|v| v.eq_ignore_ascii_case(version))
}

pub fn ignored_updates(instance: &Instance) -> Vec<String> {
    launcher_str(instance, "ignoredUpdates")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

pub fn ignore_all_updates_set(instance: &Instance) -> bool {
    launcher_str(instance, "ignoreAllUpdates").and_then(Value::as_bool).unwrap_or(false)
}

/// "Ignore this update": remembers the version so it isn't offered again. Vanilla instances have nothing to update.
pub fn ignore_update(instance: &mut Instance, version: &str) -> Result<()> {
    if instance.file.launcher.vanilla_instance || has_update_been_ignored(instance, version) {
        return Ok(());
    }
    let mut list = ignored_updates(instance);
    list.push(version.to_string());
    instance.file.launcher.extras.insert("ignoredUpdates".into(), json!(list));
    instance.save()
}

/// "Ignore all updates" for this instance.
pub fn ignore_all_updates(instance: &mut Instance) -> Result<()> {
    instance.file.launcher.extras.insert("ignoreAllUpdates".into(), json!(true));
    instance.save()
}

/// Lets updates be offered again: forgets the ignored versions and the ignore-all switch.
pub fn clear_ignored_updates(instance: &mut Instance) -> Result<()> {
    instance.file.launcher.extras.insert("ignoredUpdates".into(), json!([]));
    instance.file.launcher.extras.insert("ignoreAllUpdates".into(), json!(false));
    instance.save()
}

/// The `checkForUpdates` field of an instance (on by default).
pub fn check_for_updates(instance: &Instance) -> bool {
    launcher_str(instance, "checkForUpdates").and_then(Value::as_bool).unwrap_or(true)
}

pub fn set_check_for_updates(instance: &mut Instance, on: bool) -> Result<()> {
    instance.file.launcher.extras.insert("checkForUpdates".into(), json!(on));
    instance.save()
}

/// Mods in `instance` by Modrinth project id, for a quick "is this project here".
pub fn installed_by_project(instance: &Instance) -> HashMap<String, ModKey> {
    instance
        .mod_records()
        .iter()
        .filter(|m| files::is_from_modrinth(m))
        .filter_map(|m| Some((m.modrinth_project_id()?.to_string(), ModKey::of(m))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::files::tests_support::temp_instance;

    fn version(id: &str, date: &str, kind: &str) -> MrVersion {
        MrVersion::from_value(json!({"id": id, "project_id": "p", "name": id, "date_published": date, "version_type": kind, "loaders": ["fabric"], "game_versions": ["1.20.1"], "files": []})).unwrap()
    }

    #[test]
    fn updates_must_be_newer_and_not_riskier() {
        let installed = version("a", "2026-01-01T00:00:00Z", "release");
        assert!(wanted_update(&version("b", "2026-02-01T00:00:00Z", "release"), Some(&installed)));
        assert!(!wanted_update(&version("a", "2026-01-01T00:00:00Z", "release"), Some(&installed)));
        assert!(!wanted_update(&version("old", "2025-01-01T00:00:00Z", "release"), Some(&installed)));
        // somebody on a release isn't moved to a beta, but somebody on a beta can go to a newer beta
        assert!(!wanted_update(&version("beta", "2026-03-01T00:00:00Z", "beta"), Some(&installed)));
        let on_beta = version("b1", "2026-01-01T00:00:00Z", "beta");
        assert!(wanted_update(&version("b2", "2026-03-01T00:00:00Z", "beta"), Some(&on_beta)));
    }

    #[test]
    fn a_second_apart_is_not_an_update() {
        assert!(newer_by_a_second("2026-01-01T00:00:05Z", Some("2026-01-01T00:00:00Z")));
        assert!(!newer_by_a_second("2026-01-01T00:00:01Z", Some("2026-01-01T00:00:00Z")));
        assert!(!newer_by_a_second("2025-12-31T00:00:00Z", Some("2026-01-01T00:00:00Z")));
        assert!(newer_by_a_second("2026-01-01T00:00:00Z", None));
    }

    #[test]
    fn the_platform_follows_the_default_setting_when_both_know_a_mod() {
        let mut r = ModRecord::new("x", "x.jar", "mods");
        let mut s = ModSettings::default();
        assert_eq!(preferred_platform(&r, &s), None);
        r.modrinth_project = Some(json!({"id": "p"}));
        r.modrinth_version = Some(json!({"id": "v"}));
        assert_eq!(preferred_platform(&r, &s), Some(Platform::Modrinth));
        r.curse_forge_project_id = Some(1);
        r.curse_forge_file_id = Some(2);
        assert_eq!(preferred_platform(&r, &s), Some(Platform::Modrinth));
        s.default_mod_platform = "CURSEFORGE".into();
        assert_eq!(preferred_platform(&r, &s), Some(Platform::CurseForge));
        r.modrinth_project = None;
        s.default_mod_platform = "MODRINTH".into();
        assert_eq!(preferred_platform(&r, &s), Some(Platform::CurseForge));
    }

    #[test]
    fn mods_that_need_an_exact_version_are_pinned() {
        let mut iris = ModRecord::new("Iris", "iris.jar", "mods");
        iris.modrinth_project = Some(json!({"id": "iris"}));
        iris.modrinth_version = Some(json!({"id": "v", "dependencies": [
            {"project_id": "sodium", "version_id": "s1", "dependency_type": "required"},
            {"project_id": "other", "dependency_type": "required"},
            {"project_id": "opt", "version_id": "o1", "dependency_type": "optional"}]}));
        let pinned = pinned_projects(&[iris]);
        assert!(pinned.contains("sodium") && !pinned.contains("other") && !pinned.contains("opt"));
    }

    #[test]
    fn ignored_updates_are_remembered_and_case_insensitive() {
        let mut inst = temp_instance("ignore");
        assert!(!has_update_been_ignored(&inst, "AbC123"));
        ignore_update(&mut inst, "AbC123").unwrap();
        assert!(has_update_been_ignored(&inst, "abc123"));
        assert!(!has_update_been_ignored(&inst, "other"));
        assert_eq!(ignored_updates(&inst), vec!["AbC123"]);
        // saved as the Java edition's field
        let saved: Value = serde_json::from_slice(&std::fs::read(inst.root.join("instance.json")).unwrap()).unwrap();
        assert_eq!(saved["launcher"]["ignoredUpdates"], json!(["AbC123"]));
        ignore_all_updates(&mut inst).unwrap();
        assert!(has_update_been_ignored(&inst, "anything"));
        clear_ignored_updates(&mut inst).unwrap();
        assert!(!has_update_been_ignored(&inst, "AbC123"));
        assert!(check_for_updates(&inst));
        set_check_for_updates(&mut inst, false).unwrap();
        assert!(!check_for_updates(&inst));
        // vanilla instances have nothing to ignore
        inst.file.launcher.vanilla_instance = true;
        ignore_update(&mut inst, "zzz").unwrap();
        assert!(ignored_updates(&inst).is_empty());
        let _ = std::fs::remove_dir_all(&inst.root);
    }

    #[test]
    fn a_pack_update_needs_a_new_unignored_version() {
        let mut inst = temp_instance("packs");
        assert!(pack_update_wanted(&inst, "new", "old"));
        assert!(!pack_update_wanted(&inst, "old", "old"));
        ignore_update(&mut inst, "new").unwrap();
        assert!(!pack_update_wanted(&inst, "new", "old"));
        assert!(!is_modrinth_pack(&inst));
        for (k, v) in [
            ("modrinthManifest", json!({})),
            ("modrinthProject", json!({"id": "pp"})),
            ("modrinthVersion", json!({"id": "vv"})),
        ] {
            inst.file.launcher.extras.insert(k.into(), v);
        }
        assert_eq!(modrinth_pack_ids(&inst), Some(("pp".into(), "vv".into())));
        let _ = std::fs::remove_dir_all(&inst.root);
    }

    #[test]
    fn rollback_lists_only_older_versions() {
        let mut inst = temp_instance("rollback");
        let mut r = ModRecord::new("M", "m.jar", "mods");
        r.modrinth_project = Some(json!({"id": "p"}));
        r.modrinth_version = Some(json!({"id": "v2", "project_id": "p", "date_published": "2026-02-01T00:00:00Z"}));
        inst.set_mod_records(&[r.clone()]);
        let list = vec![
            version("v3", "2026-03-01T00:00:00Z", "release"),
            version("v2", "2026-02-01T00:00:00Z", "release"),
            version("v1", "2026-01-01T00:00:00Z", "release"),
        ];
        let older = older_than_installed(&inst, &ModKey::of(&r), &list);
        assert_eq!(older.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), vec!["v1"]);
        let _ = std::fs::remove_dir_all(&inst.root);
    }
}
