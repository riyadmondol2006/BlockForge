//! Everything the content manager does to an instance's mods, resource packs and shaders: searching Modrinth,
//! installing the right version with whatever it needs, updating, switching on and off, removing and adding files
//! from the computer. It keeps the instance's mod list (`launcher.mods` in `instance.json`) in the Java edition's
//! format, so both editions see the same content.
//!
//! A [`ContentService`] owns a copy of the [`Instance`] it works on and saves it after every change; read the new
//! state back with [`ContentService::instance`]. CurseForge is reached through the [`CurseForgeSide`] seam, which
//! the CurseForge package fills in.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use futures::future::BoxFuture;
use serde_json::Value;

use super::dropped::{self, DroppedFile, DroppedKind};
use super::kind::ContentKind;
use super::loader_support::{self, ShaderSetup};
use super::mr::{self, Project, SearchResult, Version};
use super::performance_pack;
use super::version_picker;
use crate::config::Settings;
use crate::http;
use crate::install::{self, Job, Progress};
use crate::instances::Instance;
use crate::modrecord::ModRecord;
use crate::paths::Dirs;

// ---------------------------------------------------------------------------------------------------------
// Results and settings

/// What an install did.
#[derive(Clone, Debug, Default)]
pub struct InstallResult {
    /// names of everything that was added, the thing asked for last
    pub installed: Vec<String>,
    /// things that didn't go to plan but didn't stop the install
    pub warnings: Vec<String>,
    /// names of things that were skipped because nothing fits this instance
    pub skipped: Vec<String>,
}

/// What happened to files that were dropped in.
#[derive(Clone, Debug, Default)]
pub struct AddFilesResult {
    pub added: BTreeMap<ContentKind, Vec<String>>,
    /// files that already exist in the instance
    pub duplicates: Vec<String>,
    /// files that aren't a mod, resource pack or shader pack (or couldn't be copied)
    pub rejected: Vec<String>,
    /// data packs added, with the world each went into
    pub data_packs: Vec<String>,
    /// worlds added to `saves/`
    pub worlds: Vec<String>,
}

impl AddFilesResult {
    pub fn added_count(&self) -> usize {
        self.added.values().map(Vec::len).sum::<usize>() + self.data_packs.len() + self.worlds.len()
    }
}

/// Nothing of a project is made for the instance. The message says what is missing, in words for a player.
#[derive(Debug)]
pub struct NoCompatibleVersion(pub String);

impl std::fmt::Display for NoCompatibleVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NoCompatibleVersion {}

/// What to do when a dropped file has the same name as one already in the instance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Conflict {
    Replace,
    KeepBoth,
    Skip,
}

/// How strictly mods must match the instance's Minecraft version (`addModRestriction` in the Java edition).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddModRestriction {
    /// only versions made for exactly this Minecraft version
    Strict,
    /// also versions made for the same major Minecraft version (1.21 for 1.21.1)
    Lax,
    /// any version
    None,
}

/// The settings the service reads from `BlockForge.json`.
#[derive(Clone, Debug)]
pub struct ServiceOptions {
    pub restriction: AddModRestriction,
    /// Minecraft versions where NeoForge runs Forge mods (`loaders.neoforge.forgeCompatibleMinecraftVersions`)
    pub neoforge_runs_forge_on: Vec<String>,
    pub modrinth_api_key: Option<String>,
    /// `dontCheckModsOnCurseForge`
    pub dont_check_curseforge: bool,
    /// Whether files added from the computer are looked up on Modrinth to get a proper name and update checks.
    pub identify_files: bool,
}

impl Default for ServiceOptions {
    fn default() -> Self {
        ServiceOptions {
            restriction: AddModRestriction::Strict,
            neoforge_runs_forge_on: loader_support::DEFAULT_NEOFORGE_RUNS_FORGE_ON
                .iter()
                .map(|s| s.to_string())
                .collect(),
            modrinth_api_key: None,
            dont_check_curseforge: false,
            identify_files: true,
        }
    }
}

impl ServiceOptions {
    /// Reads `addModRestriction` (or the older `disableAddModRestrictions`), `modrinthApiKey` and
    /// `dontCheckModsOnCurseForge` from the settings, which keep the Java edition's keys in `extras`.
    pub fn from_settings(settings: &Settings) -> ServiceOptions {
        let extras = &settings.extras;
        let restriction =
            match extras.get("addModRestriction").and_then(Value::as_str).map(str::to_ascii_uppercase).as_deref() {
                Some("LAX") => AddModRestriction::Lax,
                Some("NONE") => AddModRestriction::None,
                Some("STRICT") => AddModRestriction::Strict,
                _ => {
                    if extras.get("disableAddModRestrictions").and_then(Value::as_bool).unwrap_or(false) {
                        AddModRestriction::None
                    } else {
                        AddModRestriction::Strict
                    }
                }
            };
        ServiceOptions {
            restriction,
            modrinth_api_key: extras
                .get("modrinthApiKey")
                .and_then(Value::as_str)
                .filter(|k| !k.is_empty())
                .map(str::to_string),
            dont_check_curseforge: extras.get("dontCheckModsOnCurseForge").and_then(Value::as_bool).unwrap_or(false),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// The CurseForge seam

/// What kind of CurseForge project to install.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CurseKind {
    Mod,
    ResourcePack,
    Shader,
    DataPack,
    World,
}

/// A CurseForge project (and, when the player picked one, a file of it) to put into the instance.
#[derive(Clone, Debug)]
pub struct CurseInstall {
    pub project_id: i64,
    pub file_id: Option<i64>,
    pub kind: CurseKind,
}

/// A file that CurseForge recognised by its fingerprint.
#[derive(Clone, Debug)]
pub struct CurseMatch {
    pub project_id: i64,
    pub file_id: i64,
    pub project: Value,
    pub file: Value,
}

/// The CurseForge half of content, filled in by the CurseForge package and handed to
/// [`ContentService::with_curseforge`]. The service never talks to CurseForge itself.
pub trait CurseForgeSide: Send + Sync {
    /// Download `request` into the instance (dependencies included, as the Java edition does) and return the
    /// records of what was added. The service adds them to the instance's mod list and saves it.
    fn install<'a>(
        &'a self,
        instance: &'a Instance,
        request: CurseInstall,
        progress: &'a Arc<Progress>,
    ) -> BoxFuture<'a, Result<Vec<ModRecord>>>;

    /// Looks a file in the instance up on CurseForge by its fingerprint, so a Modrinth install also remembers its
    /// CurseForge twin. Left out (the default) when CurseForge isn't available.
    fn identify<'a>(&'a self, _file: &'a Path) -> BoxFuture<'a, Option<CurseMatch>> {
        Box::pin(async { None })
    }
}

// ---------------------------------------------------------------------------------------------------------
// Where a record's file lives

/// Where a record's file lives while it is switched on (`getFile` in the Java edition).
pub fn record_file(root: &Path, record: &ModRecord, minecraft_version: &str) -> Option<PathBuf> {
    let dir = match record.path.as_deref() {
        Some(p) => root.join(p),
        None => match record.kind.as_str() {
            "jar" | "forge" | "mcpc" => root.join("jarmods"),
            "texturepack" => root.join("texturepacks"),
            "resourcepack" => root.join("resourcepacks"),
            "mods" => root.join("mods"),
            "plugins" => root.join("plugins"),
            "ic2lib" => root.join("mods/ic2"),
            "denlib" => root.join("mods/denlib"),
            "coremods" => root.join("coremods"),
            "shaderpack" => root.join("shaderpacks"),
            "datapack" => root.join("datapacks"),
            "dependency" => root.join("mods").join(minecraft_version),
            _ => return None,
        },
    };
    Some(dir.join(&record.file))
}

/// Where a record's file lives while it is switched off.
pub fn record_disabled_file(root: &Path, record: &ModRecord) -> PathBuf {
    root.join("disabledmods").join(&record.file)
}

fn record_current_file(root: &Path, record: &ModRecord, mc: &str) -> Option<PathBuf> {
    if record.disabled { Some(record_disabled_file(root, record)) } else { record_file(root, record, mc) }
}

pub fn from_modrinth(record: &ModRecord) -> bool {
    record.modrinth_project.is_some() && record.modrinth_version.is_some()
}

/// The name to show for an installed item.
pub fn name_of(record: &ModRecord) -> String {
    if from_modrinth(record) {
        if let Some(title) = record
            .modrinth_project
            .as_ref()
            .and_then(|p| p.get("title"))
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
        {
            return title.to_string();
        }
    }
    if record.name.is_empty() { record.file.clone() } else { record.name.clone() }
}

fn is_same(a: &ModRecord, b: &ModRecord) -> bool {
    a.file == b.file && a.kind == b.kind
}

/// A file name that is only a name, never a path.
fn safe_file_name(name: &str) -> Result<String> {
    let only = Path::new(name).file_name().map(|n| n.to_string_lossy().to_string());
    match only {
        Some(n) if n == name && !n.is_empty() && n != ".." => Ok(n),
        _ => bail!("Modrinth gave a file name that points outside the instance: {name}"),
    }
}

/// "mod.jar" becomes "mod (2).jar", "mod (3).jar" and so on until the name is free.
pub fn unique_name(file_name: &str, taken: impl Fn(&str) -> bool) -> String {
    let dot = file_name.rfind('.').filter(|d| *d > 0);
    let (base, ext) = match dot {
        Some(d) => (&file_name[..d], &file_name[d..]),
        None => (file_name, ""),
    };
    for i in 2..1000 {
        let candidate = format!("{base} ({i}){ext}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{base} ({}){ext}", chrono::Utc::now().timestamp_millis())
}

/// The folder names of an instance's worlds, sorted without regard to case.
pub fn worlds(root: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(root.join("saves")) else { return Vec::new() };
    let mut out: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().join("level.dat").is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    out.sort_by_key(|n| n.to_lowercase());
    out
}

/// Minecraft versions near enough to `version` that mods made for one usually run on the other: the releases of
/// the same major version (`1.21` for `1.21.1`). `known` is Mojang's list of `(id, type)`.
pub fn major_versions(version: &str, known: &[(String, String)]) -> Vec<String> {
    let Some((_, kind)) = known.iter().find(|(id, _)| id == version) else { return Vec::new() };
    // this doesn't apply for anything other than releases
    if kind != "release" {
        return vec![version.to_string()];
    }
    let family = version.rfind('.').map(|d| &version[..d]).unwrap_or(version);
    let prefix = if version.split('.').count() == 2 { version } else { family };
    known
        .iter()
        .filter(|(id, kind)| kind == "release" && (id == prefix || id.starts_with(&format!("{prefix}."))))
        .map(|(id, _)| id.clone())
        .collect()
}

// ---------------------------------------------------------------------------------------------------------
// Dependency planning

/// What has to be installed alongside a mod, and whether anything stops it.
#[derive(Debug, Default)]
pub struct DependencyPlan {
    /// in install order: dependencies come before what needs them
    pub adds: Vec<Version>,
    /// things that stop the install, with the reason
    pub conflicts: Vec<String>,
    /// needs that can't be met, with the reason
    pub problems: Vec<String>,
    /// extras that go well with this mod but aren't needed
    pub suggestions: Vec<String>,
    /// needs that are already met by what is installed
    pub already_installed: Vec<String>,
}

/// The same planning the Java edition's `DependencyPlanner` does for Modrinth versions.
///
/// * `installed`: project id to the version id installed
/// * `installed_versions`: the full versions of what is installed, to spot mods that rule the new one out
/// * `catalog`: project id to its versions that fit the instance, best first
pub fn plan_dependencies(
    wanted: &Version,
    game_version: &str,
    installed: &HashMap<String, String>,
    installed_versions: &HashMap<String, Version>,
    catalog: &HashMap<String, Vec<Version>>,
) -> DependencyPlan {
    struct Step {
        version: Version,
    }
    struct State<'a> {
        game_version: &'a str,
        installed: &'a HashMap<String, String>,
        catalog: &'a HashMap<String, Vec<Version>>,
        plan: DependencyPlan,
        planned: Vec<Step>,
        visiting: HashSet<String>,
    }

    fn describe(game_version: &str) -> String {
        if game_version.is_empty() { "this Minecraft version".to_string() } else { format!("Minecraft {game_version}") }
    }

    fn label(v: &Version) -> String {
        if v.name.is_empty() { v.project_id.clone() } else { v.name.clone() }
    }

    fn add(st: &mut State, version: &Version, check_game_version: bool) {
        if st.planned.iter().any(|s| s.version.project_id == version.project_id) {
            return;
        }
        if !st.visiting.insert(version.project_id.clone()) {
            return;
        }
        if check_game_version
            && !version.game_versions.is_empty()
            && !version.game_versions.iter().any(|g| g == st.game_version)
        {
            st.plan.problems.push(format!("{} isn't made for {}", label(version), describe(st.game_version)));
        }
        st.planned.push(Step { version: version.clone() });
        for dep in &version.dependencies {
            match dep.dependency_type.as_str() {
                "optional" => {
                    if let Some(p) = &dep.project_id {
                        st.plan.suggestions.push(format!("{} works well with {p}", label(version)));
                    }
                }
                "required" => resolve(st, version, dep),
                _ => {}
            }
        }
    }

    fn resolve(st: &mut State, parent: &Version, dep: &mr::Dependency) {
        let Some(project_id) = dep.project_id.as_deref() else { return };
        if let Some(have) = st.installed.get(project_id) {
            match dep.version_id.as_deref() {
                Some(pin) if pin != have => st
                    .plan
                    .conflicts
                    .push(format!("{} needs version {pin} of {project_id} but {have} is installed", label(parent))),
                _ => st.plan.already_installed.push(format!("{project_id} (needed by {})", label(parent))),
            }
            return;
        }
        let versions = st.catalog.get(project_id).map(Vec::as_slice).unwrap_or(&[]);
        let wanted = dep.version_id.as_deref();
        let chosen = versions.iter().find(|c| wanted.is_none_or(|w| w == c.id));
        match chosen {
            None => st.plan.problems.push(format!(
                "{} needs {project_id}, but {}",
                label(parent),
                if versions.is_empty() {
                    "it couldn't be found".to_string()
                } else if let Some(w) = wanted {
                    format!("version {w} isn't available for {}", describe(st.game_version))
                } else {
                    format!("no version of it works with {}", describe(st.game_version))
                }
            )),
            Some(chosen) => {
                let chosen = chosen.clone();
                add(st, &chosen, true);
            }
        }
    }

    let mut st = State {
        game_version,
        installed,
        catalog,
        plan: DependencyPlan::default(),
        planned: Vec::new(),
        visiting: HashSet::new(),
    };
    // the wanted version was already picked for this instance, maybe from a nearby Minecraft version
    add(&mut st, wanted, false);

    // anything that is incompatible with something that's installed or planned
    let mut conflicts = Vec::new();
    for step in &st.planned {
        for dep in &step.version.dependencies {
            let Some(pid) = dep.project_id.as_deref() else { continue };
            if dep.dependency_type == "incompatible"
                && (installed.contains_key(pid) || st.planned.iter().any(|s| s.version.project_id == pid))
            {
                conflicts.push(format!(
                    "{} doesn't work together with {pid}{}",
                    label(&step.version),
                    if installed.contains_key(pid) {
                        ", which you have installed"
                    } else {
                        ", which would also be installed"
                    }
                ));
            }
        }
    }
    // the other way round: something installed that rules out what's being added
    for step in &st.planned {
        for id in installed.keys() {
            let Some(have) = installed_versions.get(id) else { continue };
            if installed.get(id) != Some(&have.id) {
                continue;
            }
            for dep in &have.dependencies {
                if dep.dependency_type == "incompatible"
                    && dep.project_id.as_deref() == Some(step.version.project_id.as_str())
                {
                    let message =
                        format!("{} (installed) doesn't work together with {}", label(have), label(&step.version));
                    if !conflicts.contains(&message) {
                        conflicts.push(message);
                    }
                }
            }
        }
    }
    st.plan.conflicts.extend(conflicts);

    // dependencies first
    let mut ordered: Vec<Version> = st.planned.into_iter().map(|s| s.version).collect();
    ordered.reverse();
    st.plan.adds = ordered;
    st.plan
}

// ---------------------------------------------------------------------------------------------------------
// The service

enum Placement {
    /// the folder of the kind, or wherever the record type says
    Standard,
    /// into the `datapacks` folder of each of these worlds
    DataPacks(Vec<String>),
}

pub struct ContentService {
    dirs: Dirs,
    instance: Instance,
    options: ServiceOptions,
    api: mr::Api,
    curseforge: Option<Arc<dyn CurseForgeSide>>,
    version_cache: HashMap<String, Vec<Version>>,
    project_cache: HashMap<String, Project>,
    nearby: Option<Vec<String>>,
}

impl ContentService {
    pub fn new(dirs: &Dirs, instance: Instance, options: ServiceOptions) -> ContentService {
        let api = mr::Api { key: options.modrinth_api_key.clone() };
        ContentService {
            dirs: dirs.clone(),
            instance,
            options,
            api,
            curseforge: None,
            version_cache: HashMap::new(),
            project_cache: HashMap::new(),
            nearby: None,
        }
    }

    /// Plugs in the CurseForge side.
    pub fn with_curseforge(mut self, side: Arc<dyn CurseForgeSide>) -> ContentService {
        self.curseforge = Some(side);
        self
    }

    /// The instance as the service has it now (after installs, removals and so on).
    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    pub fn into_instance(self) -> Instance {
        self.instance
    }

    pub fn options(&self) -> &ServiceOptions {
        &self.options
    }

    pub fn minecraft_version(&self) -> String {
        self.instance.mc_version().to_string()
    }

    /// Modrinth's name for the instance's loader, or `None` for plain Minecraft.
    pub fn loader_key(&self) -> Option<&'static str> {
        loader_support::key(self.instance.loader().map(|(kind, _)| kind))
    }

    pub fn accepted_loaders(&self) -> Vec<String> {
        loader_support::accepted_loaders(
            self.loader_key(),
            self.instance.mc_version(),
            &self.options.neoforge_runs_forge_on,
        )
    }

    fn save(&self) -> Result<()> {
        self.instance.save().context("saving the instance")
    }

    /// Minecraft versions near enough to the instance's when nothing is made for the exact one; empty unless the
    /// player allows it (`addModRestriction` set to lax).
    async fn ensure_nearby(&mut self) {
        if self.nearby.is_some() {
            return;
        }
        let mut found = Vec::new();
        if self.options.restriction == AddModRestriction::Lax {
            if let Ok(manifest) = install::manifest(&self.dirs).await {
                let known: Vec<(String, String)> =
                    manifest.versions.iter().map(|v| (v.id.clone(), v.kind.clone())).collect();
                found = major_versions(self.instance.mc_version(), &known);
            }
        }
        self.nearby = Some(found);
    }

    fn nearby_versions(&self) -> &[String] {
        self.nearby.as_deref().unwrap_or(&[])
    }

    // ------------------------------------------------------------------------------------------ searching

    /// Searches Modrinth for one kind of content made for this instance. `sort` is Modrinth's index: relevance,
    /// downloads, follows, newest or updated.
    pub async fn search(&mut self, kind: ContentKind, query: &str, page: u32, sort: &str) -> Result<SearchResult> {
        self.ensure_nearby().await;
        let mc = self.minecraft_version();
        let game_versions: Option<Vec<String>> = match self.options.restriction {
            AddModRestriction::None => None,
            AddModRestriction::Lax => {
                let mut v = self.nearby_versions().to_vec();
                if v.is_empty() {
                    v.push(mc);
                }
                Some(v)
            }
            AddModRestriction::Strict => Some(vec![mc]),
        };
        let accepted = self.accepted_loaders();
        let categories: Vec<Vec<String>> =
            if kind == ContentKind::Mods && !accepted.is_empty() { vec![accepted] } else { Vec::new() };
        let sort = if sort.is_empty() { "relevance" } else { sort };
        self.api.search(game_versions.as_deref(), query, page, sort, &categories, Some(kind.project_type())).await
    }

    /// Data packs, which Modrinth files under `datapack` and, for older ones, under mods.
    pub async fn search_data_packs(&mut self, query: &str, page: u32, sort: &str) -> Result<SearchResult> {
        let mc = self.minecraft_version();
        let versions = match self.options.restriction {
            AddModRestriction::None => None,
            _ => Some(vec![mc]),
        };
        let categories = vec![vec!["datapack".to_string()]];
        let sort = if sort.is_empty() { "relevance" } else { sort };
        let result = self.api.search(versions.as_deref(), query, page, sort, &categories, Some("datapack")).await?;
        if result.hits.is_empty() {
            return self.api.search(versions.as_deref(), query, page, sort, &categories, Some("mod")).await;
        }
        Ok(result)
    }

    /// The most downloaded projects of a kind for this instance, for empty pages.
    pub async fn popular(&mut self, kind: ContentKind, count: usize) -> Result<Vec<mr::SearchHit>> {
        let mut result = self.search(kind, "", 0, "downloads").await?;
        result.hits.truncate(count);
        Ok(result.hits)
    }

    /// Every version of a project. Remembered while the service lives.
    pub async fn versions(&mut self, project_id: &str) -> Result<Vec<Version>> {
        if let Some(cached) = self.version_cache.get(project_id) {
            return Ok(cached.clone());
        }
        let fetched = self.api.versions(project_id).await?;
        self.version_cache.insert(project_id.to_string(), fetched.clone());
        Ok(fetched)
    }

    pub async fn project(&mut self, project_id: &str) -> Result<Project> {
        if let Some(cached) = self.project_cache.get(project_id) {
            return Ok(cached.clone());
        }
        let fetched = self.api.project(project_id).await?;
        if !fetched.slug.is_empty() {
            self.project_cache.insert(fetched.slug.clone(), fetched.clone());
        }
        self.project_cache.insert(project_id.to_string(), fetched.clone());
        Ok(fetched)
    }

    /// The version that would be installed, or `None` if none fits. Call after something that loads the nearby
    /// Minecraft versions (any of the async methods does).
    pub fn pick<'a>(&self, kind: ContentKind, versions: &'a [Version]) -> Option<&'a Version> {
        let mc = self.minecraft_version();
        if kind != ContentKind::Mods {
            return version_picker::pick(versions, &[], Some(&mc), self.nearby_versions());
        }
        // some projects publish one file that is both a mod and a data pack; the launcher would file that as a
        // data pack, so a mod-only file is preferred
        let accepted = self.accepted_loaders();
        let mod_only: Vec<&Version> = versions.iter().filter(|v| !v.loaders.iter().any(|l| l == "datapack")).collect();
        let best = {
            let owned: Vec<Version> = mod_only.iter().map(|v| (*v).clone()).collect();
            version_picker::pick(&owned, &accepted, Some(&mc), self.nearby_versions()).map(|v| v.id.clone())
        };
        match best {
            Some(id) => versions.iter().find(|v| v.id == id),
            None => version_picker::pick(versions, &accepted, Some(&mc), self.nearby_versions()),
        }
    }

    /// Whether something of a project is made for this instance. Needs the network; a project that can't be
    /// reached counts as not available.
    pub async fn is_available(&mut self, project_id: &str, kind: ContentKind) -> bool {
        self.ensure_nearby().await;
        match self.versions(project_id).await {
            Ok(versions) => self.pick(kind, &versions).is_some(),
            Err(_) => false,
        }
    }

    // ------------------------------------------------------------------------------------------ what's installed

    /// The instance's content of one kind, sorted by name.
    pub fn installed(&self, kind: ContentKind) -> Vec<ModRecord> {
        let mut list: Vec<ModRecord> = self
            .instance
            .mod_records()
            .into_iter()
            .filter(|m| m.kind == kind.record_type() && m.was_selected && !m.skipped)
            .collect();
        list.sort_by_key(|m| name_of(m).to_lowercase());
        list
    }

    /// Modrinth projects that are in the instance, counting stand-ins (Quilt's QSL does the job of Fabric API).
    pub fn installed_project_ids(&self) -> HashSet<String> {
        let mut ids = HashSet::new();
        for m in self.instance.mod_records() {
            if from_modrinth(&m) {
                if let Some(id) = m.modrinth_project_id() {
                    ids.insert(id.to_string());
                    if id == loader_support::QSL {
                        ids.insert(loader_support::FABRIC_API.to_string());
                    }
                }
            }
            if m.curse_forge_project_id.is_some()
                && m.curse_forge_file_id.is_some()
                && m.curse_forge_project_id == Some(loader_support::CURSEFORGE_FABRIC_API)
            {
                ids.insert(loader_support::FABRIC_API.to_string());
            }
        }
        ids
    }

    /// Projects installed from Modrinth, to the version installed.
    fn installed_versions(&self) -> HashMap<String, Version> {
        let mut out = HashMap::new();
        for m in self.instance.mod_records() {
            if !from_modrinth(&m) {
                continue;
            }
            if let (Some(id), Some(v)) = (m.modrinth_project_id(), m.modrinth_version.as_ref()) {
                if let Ok(version) = serde_json::from_value::<Version>(v.clone()) {
                    out.insert(id.to_string(), version);
                }
            }
        }
        out
    }

    // ------------------------------------------------------------------------------------------ installing

    /// Installs the newest version of a project that is made for this instance, and for mods everything it needs.
    pub async fn install(
        &mut self,
        project_id: &str,
        kind: ContentKind,
        progress: &Arc<Progress>,
    ) -> Result<InstallResult> {
        progress.stage("Looking up versions");
        self.ensure_nearby().await;
        let project = self
            .project(project_id)
            .await
            .map_err(|e| e.context("Couldn't reach Modrinth. Check your connection and try again."))?;
        let versions = self
            .versions(project_id)
            .await
            .map_err(|e| e.context("Couldn't reach Modrinth. Check your connection and try again."))?;
        let Some(chosen) = self.pick(kind, &versions).cloned() else {
            let loader = self.loader_key();
            return Err(NoCompatibleVersion(format!(
                "{} has no version for Minecraft {}{}",
                project.title,
                self.minecraft_version(),
                if kind == ContentKind::Mods && loader.is_some() {
                    format!(" with {}", loader_support::display_name(loader))
                } else {
                    String::new()
                }
            ))
            .into());
        };
        self.install_version(&project, &chosen, kind, progress).await
    }

    /// Installs one exact version, and for mods whatever it needs.
    pub async fn install_version(
        &mut self,
        project: &Project,
        chosen: &Version,
        kind: ContentKind,
        progress: &Arc<Progress>,
    ) -> Result<InstallResult> {
        self.ensure_nearby().await;
        let mut result = InstallResult::default();
        let mut to_install: Vec<Version> = Vec::new();

        if kind == ContentKind::Mods {
            let plan = self.plan_for(chosen).await;
            to_install.extend(plan.adds.iter().cloned());
            result.warnings.extend(plan.problems.iter().cloned());
            result.warnings.extend(plan.conflicts.iter().cloned());
            if to_install.last().is_none_or(|v| v.id != chosen.id) {
                to_install.retain(|v| v.id != chosen.id);
                to_install.push(chosen.clone());
            }
        } else {
            to_install.push(chosen.clone());
        }

        let total = to_install.len();
        for (i, version) in to_install.iter().enumerate() {
            let main = version.id == chosen.id;
            let p = if main { project.clone() } else { self.project(&version.project_id).await? };
            let label = if total > 1 { format!("{} ({}/{total})", p.title, i + 1) } else { p.title.clone() };
            progress.stage(label);
            self.download(&p, version, if main { kind } else { ContentKind::Mods }, Placement::Standard, progress)
                .await?;
            result.installed.push(p.title.clone());
        }
        Ok(result)
    }

    /// Installs the newest version of a data pack into each of the given worlds (`saves/<world>/datapacks`).
    pub async fn install_data_pack(
        &mut self,
        project_id: &str,
        worlds: &[String],
        progress: &Arc<Progress>,
    ) -> Result<InstallResult> {
        if worlds.is_empty() {
            bail!(
                "Data packs are added to a world, and {} has none yet. Play it once to make a world, then add the data pack again.",
                self.instance.name()
            );
        }
        progress.stage("Looking up versions");
        let project = self
            .project(project_id)
            .await
            .map_err(|e| e.context("Couldn't reach Modrinth. Check your connection and try again."))?;
        let versions = self
            .versions(project_id)
            .await
            .map_err(|e| e.context("Couldn't reach Modrinth. Check your connection and try again."))?;
        let mc = self.minecraft_version();
        let chosen = version_picker::pick(&versions, &[], Some(&mc), &[]).cloned().ok_or_else(|| {
            anyhow::Error::from(NoCompatibleVersion(format!("{} has no version for Minecraft {mc}", project.title)))
        })?;
        progress.stage(project.title.clone());
        self.download(&project, &chosen, ContentKind::ResourcePacks, Placement::DataPacks(worlds.to_vec()), progress)
            .await?;
        Ok(InstallResult { installed: vec![project.title], ..Default::default() })
    }

    /// Installs something from CurseForge through the plugged-in side, and records what it added.
    pub async fn install_from_curseforge(
        &mut self,
        request: CurseInstall,
        progress: &Arc<Progress>,
    ) -> Result<InstallResult> {
        let Some(side) = self.curseforge.clone() else {
            bail!("CurseForge isn't available in this build.");
        };
        let records = side.install(&self.instance, request, progress).await?;
        let mut result = InstallResult::default();
        for record in records {
            result.installed.push(name_of(&record));
            self.instance.upsert_mod_record(record);
        }
        self.save()?;
        Ok(result)
    }

    /// Works out what has to come with a mod version. Asks Modrinth for the versions of everything it needs.
    async fn plan_for(&mut self, chosen: &Version) -> DependencyPlan {
        let installed_versions = self.installed_versions();
        let provided = self.installed_project_ids();
        let mut installed: HashMap<String, String> = HashMap::new();
        for id in &provided {
            let version_id = installed_versions
                .get(id)
                .filter(|v| !v.id.is_empty())
                .map(|v| v.id.clone())
                .unwrap_or_else(|| "installed".to_string());
            installed.insert(id.clone(), version_id);
        }
        installed.remove(&chosen.project_id);
        // a mod that needs one exact version of another (Iris and Sodium) gets it, replacing the one installed
        for dep in &chosen.dependencies {
            if let (Some(pid), Some(vid)) = (&dep.project_id, &dep.version_id) {
                if dep.dependency_type == "required"
                    && installed.get(pid).is_some_and(|have| have != vid)
                    && installed_versions.contains_key(pid)
                {
                    installed.remove(pid);
                }
            }
        }

        // fetch the versions of everything the plan may need
        let accepted = self.accepted_loaders();
        let mc = self.minecraft_version();
        let mut catalog: HashMap<String, Vec<Version>> = HashMap::new();
        let mut stack: Vec<Version> = vec![chosen.clone()];
        let mut expanded: HashSet<String> = HashSet::new();
        while let Some(version) = stack.pop() {
            if !expanded.insert(version.project_id.clone()) {
                continue;
            }
            for dep in version.dependencies.iter().filter(|d| d.dependency_type == "required") {
                let Some(pid) = dep.project_id.clone() else { continue };
                if installed.contains_key(&pid) {
                    continue;
                }
                if !catalog.contains_key(&pid) {
                    let all = self.versions(&pid).await.unwrap_or_default();
                    let fitting: Vec<Version> =
                        version_picker::compatible(&all, &accepted, Some(&mc), &[]).into_iter().cloned().collect();
                    catalog.insert(pid.clone(), fitting);
                }
                let chosen_dep = catalog
                    .get(&pid)
                    .and_then(|list| list.iter().find(|c| dep.version_id.as_deref().is_none_or(|w| w == c.id)))
                    .cloned();
                if let Some(c) = chosen_dep {
                    stack.push(c);
                }
            }
        }
        plan_dependencies(chosen, &mc, &installed, &installed_versions, &catalog)
    }

    /// Puts one version's file into the instance and records it, replacing older files of the same project.
    async fn download(
        &mut self,
        project: &Project,
        version: &Version,
        kind: ContentKind,
        placement: Placement,
        progress: &Arc<Progress>,
    ) -> Result<()> {
        let Some(file) = version.primary_file().cloned() else { bail!("{} has no file to download", project.title) };
        let file_name = safe_file_name(&file.filename)?;
        let mc = self.minecraft_version();
        let root = self.instance.root.clone();

        let record_type = record_type_for(project, version, kind, &placement);
        let (targets, path_for_record): (Vec<(PathBuf, Option<String>)>, bool) = match &placement {
            Placement::DataPacks(worlds) => (
                worlds
                    .iter()
                    .map(|w| {
                        let rel = format!("saves/{w}/datapacks");
                        (root.join(&rel).join(&file_name), Some(rel))
                    })
                    .collect(),
                true,
            ),
            Placement::Standard => {
                let probe = ModRecord::new("", &file_name, record_type);
                match record_file(&root, &probe, &mc) {
                    Some(p) => (vec![(p, None)], false),
                    None => bail!("Don't know where to put a file of type {record_type}"),
                }
            }
        };
        let _ = path_for_record;

        // older files of the same project go away; a data pack only replaces itself in the same world
        let mut records = self.instance.mod_records();
        let same_project = |m: &ModRecord| {
            from_modrinth(m) && m.modrinth_project_id().is_some_and(|id| id.eq_ignore_ascii_case(&project.id))
        };
        let datapack_paths: Vec<Option<String>> = targets.iter().map(|(_, p)| p.clone()).collect();
        let mut old: Vec<ModRecord> = Vec::new();
        records.retain(|m| {
            let replaced =
                same_project(m) && (matches!(placement, Placement::Standard) || datapack_paths.contains(&m.path));
            if replaced {
                old.push(m.clone());
            }
            !replaced
        });
        let was_update = !old.is_empty();
        let _ = was_update;
        for m in &old {
            if let Some(p) = record_file(&root, m, &mc) {
                if !targets.iter().any(|(t, _)| *t == p) {
                    let _ = std::fs::remove_file(p);
                }
            }
            let disabled = record_disabled_file(&root, m);
            let _ = std::fs::remove_file(disabled);
        }

        let mut jobs = Vec::new();
        for (dest, _) in &targets {
            let mut job = Job::new(&file.url, dest);
            job.sha1 = file.hashes.get("sha1").cloned();
            job.size = file.size.filter(|s| *s > 0);
            jobs.push(job);
        }
        install::download_all(jobs, progress, true)
            .await
            .map_err(|e| e.context(format!("Couldn't download {}. Please try again.", project.title)))?;

        for (dest, rel) in &targets {
            let mut record = ModRecord::new(&project.title, &file_name, record_type);
            record.version = version.name.clone();
            record.optional = true;
            record.description = project.description.clone();
            record.disabled = false;
            record.user_added = true;
            record.was_selected = true;
            record.skipped = false;
            record.path = rel.clone();
            record.modrinth_project = Some(project.record_value());
            record.modrinth_version = Some(version.record_value());
            // check for the same mod on CurseForge
            if !self.options.dont_check_curseforge {
                if let Some(side) = self.curseforge.clone() {
                    if let Some(m) = side.identify(dest).await {
                        record.curse_forge_project_id = Some(m.project_id);
                        record.curse_forge_file_id = Some(m.file_id);
                        record.curse_forge_project = Some(m.project);
                        record.curse_forge_file = Some(m.file);
                    }
                }
            }
            records.push(record);
        }
        self.instance.set_mod_records(&records);
        self.save()
    }

    // ------------------------------------------------------------------------------------------ updates

    /// Finds newer versions of the content of one kind that came from Modrinth.
    pub async fn find_updates(&mut self, kind: ContentKind) -> Result<Vec<Update>> {
        self.ensure_nearby().await;
        let mc = self.minecraft_version();
        let root = self.instance.root.clone();
        // a mod that needs one exact version of another (Iris needs a matching Sodium) would break if that one moved on
        let mut pinned: HashSet<String> = HashSet::new();
        for m in self.instance.mod_records() {
            if from_modrinth(&m) {
                if let Some(v) =
                    m.modrinth_version.as_ref().and_then(|v| serde_json::from_value::<Version>(v.clone()).ok())
                {
                    for dep in v.dependencies {
                        if dep.dependency_type == "required" && dep.version_id.is_some() {
                            if let Some(p) = dep.project_id {
                                pinned.insert(p);
                            }
                        }
                    }
                }
            }
        }

        let mut by_hash: Vec<(String, ModRecord)> = Vec::new();
        for m in self.installed(kind) {
            if !from_modrinth(&m) || m.modrinth_project_id().is_some_and(|id| pinned.contains(id)) {
                continue;
            }
            let Some(path) = record_current_file(&root, &m, &mc).filter(|p| p.is_file()) else { continue };
            let Ok(bytes) = std::fs::read(&path) else { continue };
            by_hash.push((http::sha1_hex(&bytes), m));
        }
        if by_hash.is_empty() {
            return Ok(Vec::new());
        }
        let hashes: Vec<String> = by_hash.iter().map(|(h, _)| h.clone()).collect();
        let loaders = if kind == ContentKind::Mods { Some(self.accepted_loaders()) } else { None };
        let latest = self.api.latest_from_sha1(&hashes, loaders.as_deref(), Some(&[mc])).await?;

        let mut updates = Vec::new();
        for (hash, record) in by_hash {
            let Some(newest) = latest.get(&hash) else { continue };
            let current: Option<Version> =
                record.modrinth_version.as_ref().and_then(|v| serde_json::from_value(v.clone()).ok());
            // someone on a full release isn't moved to a beta; installs pick releases first too
            let riskier = !newest.is_release() && current.as_ref().is_none_or(Version::is_release);
            if !riskier && version_picker::is_newer(Some(newest), current.as_ref()) {
                updates.push(Update { record, newest: newest.clone() });
            }
        }
        Ok(updates)
    }

    /// Replaces an installed item with a newer version, keeping it switched off if it was.
    pub async fn update(&mut self, record: &ModRecord, version: &Version, progress: &Arc<Progress>) -> Result<()> {
        self.ensure_nearby().await;
        let kind = ContentKind::from_record_type(&record.kind);
        let project_id = record.modrinth_project_id().context("this item didn't come from Modrinth")?.to_string();
        let project = match self.project(&project_id).await {
            Ok(p) => p,
            Err(_) => record
                .modrinth_project
                .as_ref()
                .and_then(|p| serde_json::from_value(p.clone()).ok())
                .context("Couldn't reach Modrinth. Check your connection and try again.")?,
        };
        let was_disabled = record.disabled;
        if was_disabled {
            // the download code only clears out the switched-on copy of the old version
            let _ = std::fs::remove_file(record_disabled_file(&self.instance.root, record));
        }
        progress.stage(project.title.clone());
        self.download(&project, version, kind.unwrap_or(ContentKind::Mods), Placement::Standard, progress).await?;
        if was_disabled {
            for now in self.instance.mod_records() {
                if from_modrinth(&now) && now.modrinth_project_id() == Some(project_id.as_str()) {
                    self.set_enabled(&now, false)?;
                }
            }
            self.save()?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------ switching on and off, removing

    /// Switches an item on or off by moving its file between its folder and `disabledmods/`.
    pub fn set_enabled(&mut self, record: &ModRecord, enabled: bool) -> Result<()> {
        let mc = self.minecraft_version();
        let root = self.instance.root.clone();
        let mut records = self.instance.mod_records();
        let Some(mine) = records.iter_mut().find(|m| is_same(m, record)) else {
            bail!("{} isn't in this instance any more", name_of(record))
        };
        if enabled && mine.disabled {
            let to = record_file(&root, mine, &mc).context("Don't know where this file goes")?;
            let from = record_disabled_file(&root, mine);
            if let Some(dir) = to.parent() {
                std::fs::create_dir_all(dir)?;
            }
            move_file(&from, &to).with_context(|| format!("switching on {}", name_of(mine)))?;
            mine.disabled = false;
        } else if !enabled && !mine.disabled {
            let from = record_file(&root, mine, &mc).context("Don't know where this file is")?;
            let to = record_disabled_file(&root, mine);
            std::fs::create_dir_all(root.join("disabledmods"))?;
            move_file(&from, &to).with_context(|| format!("switching off {}", name_of(mine)))?;
            mine.disabled = true;
        }
        self.instance.set_mod_records(&records);
        self.save()
    }

    /// Takes an item out of the instance and deletes its file.
    pub fn remove(&mut self, record: &ModRecord) -> Result<()> {
        let mc = self.minecraft_version();
        let root = self.instance.root.clone();
        let mut records = self.instance.mod_records();
        let Some(i) = records.iter().position(|m| is_same(m, record)) else { return Ok(()) };
        let gone = records.remove(i);
        if let Some(path) = record_current_file(&root, &gone, &mc) {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("deleting {}", path.display())),
            }
        }
        self.instance.set_mod_records(&records);
        self.save()
    }

    // ------------------------------------------------------------------------------------------ files from the computer

    /// The folder names of the instance's worlds.
    pub fn worlds(&self) -> Vec<String> {
        worlds(&self.instance.root)
    }

    /// Copies dropped files into the right place: mods, resource packs and shader packs into their folders, where
    /// they show up in the content manager, data packs into the given worlds and worlds into `saves/`.
    ///
    /// `conflicts` decides what happens when a file of the same name is already there (nothing is replaced
    /// without it saying so; with no handler, such files are skipped). Modpacks and unknown files are reported as
    /// rejected.
    pub async fn add_dropped(
        &mut self,
        files: &[DroppedFile],
        data_pack_worlds: &[String],
        mut conflicts: Option<&mut dyn FnMut(&DroppedFile, &str) -> Conflict>,
    ) -> AddFilesResult {
        let mut result = AddFilesResult::default();
        let mut added: Vec<ModRecord> = Vec::new();
        let instance_name = self.instance.name().to_string();
        for file in files {
            if file.kind == DroppedKind::DataPack {
                for world in data_pack_worlds {
                    let folder = self.instance.root.join("saves").join(world).join("datapacks");
                    if self.place(file, &folder, None, world, &mut conflicts, &mut result).is_some() {
                        result.data_packs.push(format!("{} ({world})", file.file_name));
                    }
                }
                continue;
            }
            if file.kind == DroppedKind::WorldSave {
                if let Some(name) = self.place_world(file, &mut conflicts, &mut result) {
                    result.worlds.push(name);
                }
                continue;
            }
            let Some(kind) = file.kind.content_kind() else {
                result.rejected.push(file.file_name.clone());
                continue;
            };
            let folder = self.instance.root.join(kind.folder());
            let Some(target) = self.place(file, &folder, Some(kind), &instance_name, &mut conflicts, &mut result)
            else {
                continue;
            };
            if target.is_file() {
                let mut record = generate_record(&target, kind.record_type(), true);
                if let Some(name) = &file.name {
                    if record.name.is_empty()
                        || Some(record.name.as_str()) == target.file_name().map(|n| n.to_string_lossy()).as_deref()
                    {
                        record.name = name.clone();
                    }
                }
                if let Some(version) = &file.version {
                    if record.version.is_empty() || record.version == "Unknown" {
                        record.version = version.clone();
                    }
                }
                added.push(record);
            }
            result.added.entry(kind).or_default().push(file.file_name.clone());
        }
        if !added.is_empty() {
            if self.options.identify_files {
                self.identify(&mut added).await;
            }
            let mut records = self.instance.mod_records();
            records.extend(added);
            self.instance.set_mod_records(&records);
            if let Err(e) = self.save() {
                result.rejected.push(format!("(couldn't save the instance: {e})"));
            }
        }
        result
    }

    /// Copies one file into a folder, asking what to do if one of that name is already there. Returns where it
    /// went, or `None` if it was skipped or failed (and was noted in `result`).
    fn place(
        &mut self,
        file: &DroppedFile,
        folder: &Path,
        kind: Option<ContentKind>,
        where_: &str,
        conflicts: &mut Option<&mut dyn FnMut(&DroppedFile, &str) -> Conflict>,
        result: &mut AddFilesResult,
    ) -> Option<PathBuf> {
        let Some(source) = file.path.clone() else {
            result.rejected.push(file.file_name.clone());
            return None;
        };
        let mut target = folder.join(&file.file_name);
        let disabled = kind.map(|_| self.instance.root.join("disabledmods").join(&file.file_name));
        let taken = target.exists() || disabled.as_ref().is_some_and(|d| d.exists());
        let attempt = (|| -> Result<Option<PathBuf>> {
            // dragged out of this very folder: it's already where it belongs, and replacing would delete it
            if taken {
                let same = |p: &Path| p.exists() && same_file(&source, p);
                if same(&target) || disabled.as_deref().is_some_and(same) {
                    result.duplicates.push(file.file_name.clone());
                    return Ok(None);
                }
                let choice = match conflicts {
                    Some(handler) => handler(file, where_),
                    None => Conflict::Skip,
                };
                match choice {
                    Conflict::Skip => {
                        result.duplicates.push(file.file_name.clone());
                        return Ok(None);
                    }
                    Conflict::Replace => {
                        remove_any(&target)?;
                        if let Some(d) = &disabled {
                            remove_any(d)?;
                        }
                        if let Some(kind) = kind {
                            let mut records = self.instance.mod_records();
                            records.retain(|m| !(m.kind == kind.record_type() && m.file == file.file_name));
                            self.instance.set_mod_records(&records);
                        }
                    }
                    Conflict::KeepBoth => {
                        let name = unique_name(&file.file_name, |n| {
                            folder.join(n).exists()
                                || disabled.as_ref().and_then(|d| d.parent()).is_some_and(|p| p.join(n).exists())
                        });
                        target = folder.join(name);
                    }
                }
            }
            std::fs::create_dir_all(folder)?;
            copy_any(&source, &target)?;
            Ok(Some(target.clone()))
        })();
        match attempt {
            Ok(path) => path,
            Err(_) => {
                result.rejected.push(file.file_name.clone());
                None
            }
        }
    }

    /// A world goes into `saves/` under the name of its folder (or of the zip). Returns the name it was given.
    fn place_world(
        &mut self,
        file: &DroppedFile,
        conflicts: &mut Option<&mut dyn FnMut(&DroppedFile, &str) -> Conflict>,
        result: &mut AddFilesResult,
    ) -> Option<String> {
        let Some(source) = file.path.clone() else {
            result.rejected.push(file.file_name.clone());
            return None;
        };
        let saves = self.instance.root.join("saves");
        let base = if source.is_dir() {
            file.file_name.clone()
        } else {
            Path::new(&file.file_name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| file.file_name.clone())
        };
        let mut name = base.clone();
        let outcome = (|| -> Result<Option<String>> {
            if saves.join(&name).exists() {
                if source.is_dir() && same_file(&source, &saves.join(&name)) {
                    result.duplicates.push(file.file_name.clone());
                    return Ok(None);
                }
                let choice = match conflicts {
                    Some(handler) => handler(file, self.instance.name()),
                    None => Conflict::Skip,
                };
                match choice {
                    Conflict::Skip => {
                        result.duplicates.push(file.file_name.clone());
                        return Ok(None);
                    }
                    Conflict::Replace => remove_any(&saves.join(&name))?,
                    Conflict::KeepBoth => {
                        let candidate = unique_name(&base, |n| saves.join(n).exists());
                        name = candidate;
                    }
                }
            }
            std::fs::create_dir_all(&saves)?;
            let dest = saves.join(&name);
            if source.is_dir() {
                copy_any(&source, &dest)?;
            } else {
                extract_world(&source, &dest)?;
            }
            Ok(Some(name.clone()))
        })();
        match outcome {
            Ok(n) => n,
            Err(_) => {
                result.rejected.push(file.file_name.clone());
                None
            }
        }
    }

    /// Records files that were put into the instance's folders by hand, so they show up and can be switched off.
    /// Returns true if anything new was found.
    pub async fn scan_untracked(&mut self) -> bool {
        let records = self.instance.mod_records();
        let mut found: Vec<ModRecord> = Vec::new();
        for kind in ContentKind::ALL {
            let folder = self.instance.root.join(kind.folder());
            let Ok(rd) = std::fs::read_dir(&folder) else { continue };
            let mut names: Vec<(String, PathBuf)> = rd
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
                .filter(|(n, _)| accepted_mod_file(n))
                .collect();
            names.sort();
            for (name, path) in names {
                let known = records.iter().chain(found.iter()).any(|m| {
                    m.file == name && (m.kind == kind.record_type() || ContentKind::from_record_type(&m.kind).is_none())
                });
                if !known {
                    found.push(generate_record(&path, kind.record_type(), true));
                }
            }
        }
        if found.is_empty() {
            return false;
        }
        if self.options.identify_files {
            self.identify(&mut found).await;
        }
        let mut all = records;
        all.extend(found);
        self.instance.set_mod_records(&all);
        let _ = self.save();
        true
    }

    /// Asks Modrinth what files are, so they get a proper name, a picture and update checks.
    async fn identify(&mut self, records: &mut [ModRecord]) {
        let mc = self.minecraft_version();
        let mut by_hash: HashMap<String, usize> = HashMap::new();
        for (i, m) in records.iter().enumerate() {
            if let Some(path) = record_current_file(&self.instance.root, m, &mc).filter(|p| p.is_file()) {
                if let Ok(bytes) = std::fs::read(&path) {
                    by_hash.insert(http::sha1_hex(&bytes), i);
                }
            }
        }
        if by_hash.is_empty() {
            return;
        }
        let hashes: Vec<String> = by_hash.keys().cloned().collect();
        let Ok(versions) = self.api.versions_from_sha1(&hashes).await else { return };
        if versions.is_empty() {
            return;
        }
        let ids: Vec<String> = versions
            .values()
            .map(|v| v.project_id.clone())
            .filter(|p| !p.is_empty())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let Ok(projects) = self.api.projects(&ids).await else { return };
        let projects: HashMap<String, Project> = projects.into_iter().map(|p| (p.id.clone(), p)).collect();
        for (hash, version) in versions {
            let (Some(&i), Some(project)) = (by_hash.get(&hash), projects.get(&version.project_id)) else { continue };
            let m = &mut records[i];
            m.modrinth_project = Some(project.record_value());
            m.modrinth_version = Some(version.record_value());
            m.name = project.title.clone();
            m.version = version.name.clone();
            m.description = project.description.clone();
        }
    }

    // ------------------------------------------------------------------------------------------ shaders and the performance pack

    pub fn shader_setup(&self) -> ShaderSetup {
        let mods: Vec<ModRecord> = self.installed(ContentKind::Mods).into_iter().filter(|m| !m.disabled).collect();
        let files: Vec<String> = mods.iter().map(|m| m.file.clone()).collect();
        let ids: Vec<String> = mods
            .iter()
            .filter(|m| from_modrinth(m))
            .filter_map(|m| m.modrinth_project_id().map(str::to_string))
            .collect();
        loader_support::shader_setup(self.loader_key(), loader_support::has_shader_loader(&ids, &files))
    }

    /// The project that runs shaders on this instance (Iris or Oculus), or `None` if there is none.
    pub fn shader_loader_project(&self) -> Option<&'static str> {
        match loader_support::shader_setup(self.loader_key(), false) {
            ShaderSetup::AddIris => Some(loader_support::IRIS),
            ShaderSetup::AddOculus => Some(loader_support::OCULUS),
            _ => None,
        }
    }

    /// Installs Iris (with Sodium) or Oculus, whichever fits the loader.
    pub async fn install_shader_loader(&mut self, progress: &Arc<Progress>) -> Result<InstallResult> {
        let Some(loader) = self.shader_loader_project() else {
            return Err(NoCompatibleVersion("Shaders need a mod loader such as Fabric".to_string()).into());
        };
        let mut result = self.install(loader, ContentKind::Mods, progress).await?;
        let iris = loader == loader_support::IRIS;
        if iris
            && !self.installed_project_ids().contains(loader_support::SODIUM)
            && self.loader_key() != Some(loader_support::FORGE)
            && self.is_available(loader_support::SODIUM, ContentKind::Mods).await
        {
            let sodium = self.install(loader_support::SODIUM, ContentKind::Mods, progress).await?;
            result.installed.extend(sodium.installed);
        }
        Ok(result)
    }

    /// Installs the performance mods that are available for this instance and not there yet. A mod that can't be
    /// added is reported in `warnings` and `skipped`; it never stops the rest.
    pub async fn install_performance_pack(&mut self, progress: &Arc<Progress>) -> InstallResult {
        let mut result = InstallResult::default();
        let slots = performance_pack::slots_for(self.loader_key());
        progress.stage("Checking what's available");
        let installed: Vec<String> = self.installed_project_ids().into_iter().collect();
        let mut available: HashSet<String> = HashSet::new();
        for slot in &slots {
            for id in &slot.project_ids {
                if installed.iter().any(|i| i == id) || available.contains(*id) {
                    continue;
                }
                if self.is_available(id, ContentKind::Mods).await {
                    available.insert(id.to_string());
                }
            }
        }
        let ids = performance_pack::choose(&slots, |id| available.contains(id), &installed);
        for slot in &slots {
            let covered =
                slot.project_ids.iter().any(|id| ids.iter().any(|i| i == id) || installed.iter().any(|i| i == id));
            if !covered {
                result.skipped.push(performance_pack::name_of(slot.project_ids[0]));
            }
        }
        for id in ids {
            match self.install(&id, ContentKind::Mods, progress).await {
                Ok(one) => {
                    result.installed.push(performance_pack::name_of(&id));
                    result.warnings.extend(one.warnings);
                }
                Err(e) => {
                    result.warnings.push(format!("{e}"));
                    result.skipped.push(performance_pack::name_of(&id));
                }
            }
        }
        result
    }
}

/// A newer version of an installed item.
#[derive(Clone, Debug)]
pub struct Update {
    pub record: ModRecord,
    pub newest: Version,
}

/// Adds the performance mods to an instance that was just made, as the New instance page does when the option is
/// on. Returns the instance as it is afterwards, and what was added.
pub async fn install_performance_pack(
    dirs: &Dirs,
    settings: &Settings,
    instance: Instance,
    progress: &Arc<Progress>,
) -> (Instance, InstallResult) {
    let mut service = ContentService::new(dirs, instance, ServiceOptions::from_settings(settings));
    let result = service.install_performance_pack(progress).await;
    (service.into_instance(), result)
}

/// What a new instance with this loader gets when the player ticks "add performance mods".
pub fn wants_performance_mods(add_performance_mods: bool, loader_key: Option<&str>) -> bool {
    add_performance_mods && performance_pack::offered_for(loader_key)
}

/// The type a downloaded file is recorded as (`getTypeFromModrinthMod` in the Java edition).
fn record_type_for(project: &Project, version: &Version, kind: ContentKind, placement: &Placement) -> &'static str {
    if matches!(placement, Placement::DataPacks(_)) {
        return "datapack";
    }
    // a project filed as a data pack, or a version for nothing but data packs
    let only_datapack = !version.loaders.is_empty() && version.loaders.iter().all(|l| l == "datapack");
    if project.project_type == "datapack" || only_datapack {
        return "datapack";
    }
    match project.project_type.as_str() {
        "mod" => "mods",
        "shader" => "shaderpack",
        "resourcepack" => "resourcepack",
        _ => kind.record_type(),
    }
}

pub fn accepted_mod_file(name: &str) -> bool {
    name.ends_with(".jar") || name.ends_with(".zip") || name.ends_with(".litemod")
}

/// A record for a file that is already in place: named after what is inside it, or after the file.
pub fn generate_record(file: &Path, record_type: &str, enabled: bool) -> ModRecord {
    let file_name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut record = ModRecord::new(&file_name, &file_name, record_type);
    record.disabled = !enabled;
    record.user_added = true;
    record.was_selected = true;
    record.optional = true;
    record.version = "Unknown".to_string();
    if let Some((name, version, description)) = read_mod_info(file) {
        record.name = name.filter(|n| !n.is_empty()).unwrap_or_else(|| file_name.clone());
        record.version = version.filter(|v| !v.is_empty()).unwrap_or_else(|| "Unknown".to_string());
        record.description = description.unwrap_or_default();
    }
    record
}

/// The name, version and description a mod states about itself in `mcmod.info` or `fabric.mod.json`.
fn read_mod_info(file: &Path) -> Option<(Option<String>, Option<String>, Option<String>)> {
    let f = std::fs::File::open(file).ok()?;
    let mut archive = zip::ZipArchive::new(f).ok()?;
    let text = |archive: &mut zip::ZipArchive<std::fs::File>, name: &str| -> Option<String> {
        use std::io::Read;
        let entry = archive.by_name(name).ok()?;
        if entry.size() > dropped::MAX_TEXT {
            return None;
        }
        let mut bytes = Vec::new();
        entry.take(dropped::MAX_TEXT + 1).read_to_end(&mut bytes).ok()?;
        Some(String::from_utf8_lossy(&bytes).to_string())
    };
    let pick = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    if let Some(Value::Array(list)) = dropped::parse_lenient(text(&mut archive, "mcmod.info").as_deref()) {
        if let Some(first) = list.first().filter(|f| f.is_object()) {
            return Some((pick(first, "name"), pick(first, "version"), pick(first, "description")));
        }
    }
    if let Some(v) = dropped::parse_lenient(text(&mut archive, "fabric.mod.json").as_deref()).filter(Value::is_object) {
        return Some((pick(&v, "name"), pick(&v, "version"), pick(&v, "description")));
    }
    None
}

// ---------------------------------------------------------------------------------------------------------
// File helpers

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

fn remove_any(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        std::fs::remove_dir_all(path)?
    } else {
        std::fs::remove_file(path)?
    }
    Ok(())
}

/// Moves a file; across two disks that is a copy and a delete.
fn move_file(from: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        std::fs::remove_file(to)?;
    }
    if std::fs::rename(from, to).is_err() {
        std::fs::copy(from, to)?;
        std::fs::remove_file(from)?;
    }
    Ok(())
}

fn copy_any(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_any(&e.path(), &to.join(e.file_name()))?;
        }
    } else {
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::copy(from, to)?;
    }
    Ok(())
}

/// Unpacks a zipped world into `dest`. A zip with the world in one folder (the usual kind) loses that folder.
fn extract_world(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    // the folder that holds level.dat, or nothing when it's at the top
    let mut strip = String::new();
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i)?;
        let Ok(name) = entry.name().map(|n| n.replace('\\', "/")) else { continue };
        if name == "level.dat" {
            strip.clear();
            break;
        }
        if let Some(dir) = name.strip_suffix("/level.dat") {
            if !dir.contains('/') {
                strip = format!("{dir}/");
            }
        }
    }
    std::fs::create_dir_all(dest)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else { continue };
        let rel_text = rel.to_string_lossy().replace('\\', "/");
        let Some(inner) = rel_text.strip_prefix(strip.as_str()).filter(|r| !r.is_empty()) else { continue };
        let out = dest.join(inner);
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::dropped::detect;
    use std::io::Write;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("blockforge-service-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An instance folder with an `instance.json` the way the launcher writes one.
    fn make_instance(dirs: &Dirs, name: &str, mc: &str, loader: Option<&str>) -> Instance {
        let root = dirs.instance_dir(name);
        std::fs::create_dir_all(root.join("mods")).unwrap();
        let loader_version = match loader {
            Some(kind) => {
                serde_json::json!({"version":"0.16.0","rawVersion":"0.16.0","recommended":false,"type":kind,"downloadables":{}})
            }
            None => Value::Null,
        };
        let json = serde_json::json!({
            "uuid": "00000000-0000-0000-0000-000000000000",
            "launcher": {"name": name, "pack": "Minecraft", "loaderVersion": loader_version, "mods": []},
            "id": mc, "mainClass": "net.minecraft.client.main.Main", "libraries": []
        });
        std::fs::write(root.join("instance.json"), serde_json::to_vec_pretty(&json).unwrap()).unwrap();
        crate::instances::load_one(&root).unwrap()
    }

    fn zip(dir: &Path, name: &str, entries: &[(&str, &str)]) -> PathBuf {
        let file = dir.join(name);
        let mut z = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (entry, body) in entries {
            z.start_file(*entry, options).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
        file
    }

    fn offline() -> ServiceOptions {
        ServiceOptions { identify_files: false, ..Default::default() }
    }

    fn version(project: &str, id: &str, deps: &[(&str, &str, Option<&str>)]) -> Version {
        Version {
            id: id.into(),
            project_id: project.into(),
            name: format!("{project} {id}"),
            game_versions: vec!["1.21.1".into()],
            loaders: vec!["fabric".into()],
            dependencies: deps
                .iter()
                .map(|(p, t, v)| mr::Dependency {
                    project_id: Some(p.to_string()),
                    dependency_type: t.to_string(),
                    version_id: v.map(str::to_string),
                    file_name: None,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn keep_both_names_count_up() {
        assert_eq!(unique_name("sodium.jar", |_| false), "sodium (2).jar");
        assert_eq!(unique_name("sodium.jar", |n| n == "sodium (2).jar"), "sodium (3).jar");
        assert_eq!(unique_name("README", |_| false), "README (2)");
        assert_eq!(unique_name(".hidden", |_| false), ".hidden (2)");
    }

    #[test]
    fn file_names_cant_point_outside() {
        assert!(safe_file_name("sodium.jar").is_ok());
        assert!(safe_file_name("../evil.jar").is_err());
        assert!(safe_file_name("a/b.jar").is_err());
        assert!(safe_file_name("..").is_err());
    }

    #[test]
    fn restriction_and_keys_come_from_the_settings_extras() {
        let mut s = Settings::default();
        assert_eq!(ServiceOptions::from_settings(&s).restriction, AddModRestriction::Strict);
        s.extras.insert("addModRestriction".into(), Value::String("LAX".into()));
        s.extras.insert("modrinthApiKey".into(), Value::String("k".into()));
        let o = ServiceOptions::from_settings(&s);
        assert_eq!((o.restriction, o.modrinth_api_key.as_deref()), (AddModRestriction::Lax, Some("k")));
        let mut old = Settings::default();
        old.extras.insert("disableAddModRestrictions".into(), Value::Bool(true));
        assert_eq!(ServiceOptions::from_settings(&old).restriction, AddModRestriction::None);
    }

    #[test]
    fn nearby_versions_are_the_same_major_release() {
        let known: Vec<(String, String)> = [
            ("1.21.4", "release"),
            ("1.21.1", "release"),
            ("1.21", "release"),
            ("1.20.6", "release"),
            ("24w14a", "snapshot"),
            ("1.21.2-pre1", "snapshot"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let mut near = major_versions("1.21.1", &known);
        near.sort();
        assert_eq!(near, vec!["1.21", "1.21.1", "1.21.4"]);
        assert_eq!(major_versions("24w14a", &known), vec!["24w14a"]);
        assert!(major_versions("0.0", &known).is_empty());
    }

    #[test]
    fn record_files_follow_the_java_rules() {
        let root = Path::new("/inst");
        let mut r = ModRecord::new("x", "a.jar", "mods");
        assert_eq!(record_file(root, &r, "1.21.1").unwrap(), Path::new("/inst/mods/a.jar"));
        r.kind = "shaderpack".into();
        assert_eq!(record_file(root, &r, "1.21.1").unwrap(), Path::new("/inst/shaderpacks/a.jar"));
        r.kind = "dependency".into();
        assert_eq!(record_file(root, &r, "1.21.1").unwrap(), Path::new("/inst/mods/1.21.1/a.jar"));
        r.kind = "datapack".into();
        r.path = Some("saves/W/datapacks".into());
        assert_eq!(record_file(root, &r, "1.21.1").unwrap(), Path::new("/inst/saves/W/datapacks/a.jar"));
        r.kind = "worlds".into();
        r.path = None;
        assert!(record_file(root, &r, "1.21.1").is_none());
        assert_eq!(record_disabled_file(root, &r), Path::new("/inst/disabledmods/a.jar"));
    }

    #[test]
    fn dependencies_come_first_and_the_installed_ones_are_left_alone() {
        let wanted = version(
            "iris",
            "i1",
            &[("sodium", "required", None), ("api", "required", None), ("opt", "optional", None)],
        );
        let mut catalog = HashMap::new();
        catalog.insert("sodium".to_string(), vec![version("sodium", "s1", &[("api", "required", None)])]);
        catalog.insert("api".to_string(), vec![version("api", "a1", &[])]);
        let plan = plan_dependencies(&wanted, "1.21.1", &HashMap::new(), &HashMap::new(), &catalog);
        let order: Vec<&str> = plan.adds.iter().map(|v| v.project_id.as_str()).collect();
        assert_eq!(order, vec!["api", "sodium", "iris"]);
        assert!(plan.problems.is_empty() && plan.conflicts.is_empty());
        assert_eq!(plan.suggestions.len(), 1);

        // the Fabric API is already there
        let mut installed = HashMap::new();
        installed.insert("api".to_string(), "a0".to_string());
        let plan = plan_dependencies(&wanted, "1.21.1", &installed, &HashMap::new(), &catalog);
        let order: Vec<&str> = plan.adds.iter().map(|v| v.project_id.as_str()).collect();
        assert_eq!(order, vec!["sodium", "iris"]);
        assert_eq!(plan.already_installed.len(), 2);
    }

    #[test]
    fn missing_pinned_and_incompatible_dependencies_are_reported() {
        let wanted = version(
            "iris",
            "i1",
            &[("sodium", "required", Some("s9")), ("gone", "required", None), ("rival", "incompatible", None)],
        );
        let mut catalog = HashMap::new();
        catalog.insert("sodium".to_string(), vec![version("sodium", "s1", &[])]);
        let mut installed = HashMap::new();
        installed.insert("rival".to_string(), "r1".to_string());
        let plan = plan_dependencies(&wanted, "1.21.1", &installed, &HashMap::new(), &catalog);
        assert!(plan.problems.iter().any(|p| p.contains("version s9 isn't available")), "{:?}", plan.problems);
        assert!(plan.problems.iter().any(|p| p.contains("gone") && p.contains("couldn't be found")));
        assert!(plan.conflicts.iter().any(|c| c.contains("rival") && c.contains("which you have installed")));

        // an installed version of another mod pins this one
        let mut installed = HashMap::new();
        installed.insert("sodium".to_string(), "s0".to_string());
        let plan = plan_dependencies(&wanted, "1.21.1", &installed, &HashMap::new(), &catalog);
        assert!(plan.conflicts.iter().any(|c| c.contains("needs version s9 of sodium but s0 is installed")));
    }

    #[test]
    fn something_installed_can_rule_out_what_is_added() {
        let wanted = version("optifine", "o1", &[]);
        let have = version("sodium", "s1", &[("optifine", "incompatible", None)]);
        let mut installed = HashMap::new();
        installed.insert("sodium".to_string(), "s1".to_string());
        let mut have_versions = HashMap::new();
        have_versions.insert("sodium".to_string(), have);
        let plan = plan_dependencies(&wanted, "1.21.1", &installed, &have_versions, &HashMap::new());
        assert_eq!(plan.conflicts.len(), 1);
        assert!(plan.conflicts[0].contains("(installed) doesn't work together"));
    }

    #[test]
    fn mods_installed_from_modrinth_and_their_stand_ins() {
        let dirs = Dirs::at(temp("ids"));
        let instance = make_instance(&dirs, "Q", "1.21.1", Some("Quilt"));
        let mut svc = ContentService::new(&dirs, instance, offline());
        let mut qsl = ModRecord::new("QSL", "qsl.jar", "mods");
        qsl.modrinth_project = Some(serde_json::json!({"id": loader_support::QSL, "title": "QSL"}));
        qsl.modrinth_version = Some(serde_json::json!({"id": "v"}));
        let mut cf = ModRecord::new("Fabric API", "fa.jar", "mods");
        cf.curse_forge_project_id = Some(loader_support::CURSEFORGE_FABRIC_API);
        cf.curse_forge_file_id = Some(1);
        svc.instance.set_mod_records(&[qsl, cf]);
        let ids = svc.installed_project_ids();
        assert!(ids.contains(loader_support::QSL) && ids.contains(loader_support::FABRIC_API));
        assert_eq!(svc.loader_key(), Some("quilt"));
        assert_eq!(svc.accepted_loaders(), vec!["quilt".to_string(), "fabric".to_string()]);
        assert_eq!(svc.installed(ContentKind::Mods).len(), 2);
        assert_eq!(name_of(&svc.installed(ContentKind::Mods)[1]), "QSL");
    }

    #[tokio::test]
    async fn dropped_files_go_where_they_belong_and_are_recorded() {
        let dirs = Dirs::at(temp("drop"));
        let source = temp("drop-src");
        let instance = make_instance(&dirs, "Drop", "1.21.1", Some("Fabric"));
        std::fs::create_dir_all(instance.root.join("saves/Overworld")).unwrap();
        std::fs::write(instance.root.join("saves/Overworld/level.dat"), "x").unwrap();
        let mut svc = ContentService::new(&dirs, instance, offline());

        let jar = zip(
            &source,
            "cool.jar",
            &[(
                "fabric.mod.json",
                r#"{"name":"Cool Mod","version":"2.1","description":"Does cool things","depends":{"minecraft":"1.21.x"}}"#,
            )],
        );
        let pack = zip(
            &source,
            "Faithful.zip",
            &[("pack.mcmeta", r#"{"pack":{"description":"hi"}}"#), ("assets/minecraft/a.png", "")],
        );
        let shader = zip(&source, "BSL.zip", &[("shaders/final.fsh", "")]);
        let data = zip(&source, "Terra.zip", &[("pack.mcmeta", "{}"), ("data/t/x.json", "{}")]);
        let modpack = zip(&source, "p.mrpack", &[("modrinth.index.json", "{}")]);
        let files: Vec<DroppedFile> = [&jar, &pack, &shader, &data, &modpack].iter().map(|p| detect(p)).collect();

        let result = svc.add_dropped(&files, &["Overworld".to_string()], None).await;
        let root = svc.instance().root.clone();
        assert!(root.join("mods/cool.jar").is_file());
        assert!(root.join("resourcepacks/Faithful.zip").is_file());
        assert!(root.join("shaderpacks/BSL.zip").is_file());
        assert!(root.join("saves/Overworld/datapacks/Terra.zip").is_file());
        assert_eq!(result.data_packs, vec!["Terra.zip (Overworld)".to_string()]);
        assert_eq!(result.rejected, vec!["p.mrpack".to_string()]);
        assert_eq!(result.added_count(), 4);

        // the record is what the Java edition would write
        let on_disk = crate::instances::load_one(&root).unwrap();
        let mods = on_disk.mod_records();
        assert_eq!(mods.len(), 3);
        let cool = mods.iter().find(|m| m.file == "cool.jar").unwrap();
        assert_eq!((cool.name.as_str(), cool.version.as_str(), cool.kind.as_str()), ("Cool Mod", "2.1", "mods"));
        assert!(cool.user_added && cool.was_selected && !cool.disabled);
        assert_eq!(cool.description, "Does cool things");
        assert_eq!(on_disk.file.launcher.extras["mods"][0]["type"], "mods");

        // dropping the same file again: skipped without a handler
        let again = svc.add_dropped(&files[..1], &[], None).await;
        assert_eq!(again.duplicates, vec!["cool.jar".to_string()]);
        assert_eq!(again.added_count(), 0);

        // keep both
        let mut keep = |_: &DroppedFile, _: &str| Conflict::KeepBoth;
        let both = svc.add_dropped(&files[..1], &[], Some(&mut keep)).await;
        assert!(root.join("mods/cool (2).jar").is_file());
        assert_eq!(both.added_count(), 1);

        // replace
        let mut replace = |_: &DroppedFile, _: &str| Conflict::Replace;
        svc.add_dropped(&files[..1], &[], Some(&mut replace)).await;
        assert_eq!(svc.instance().mod_records().iter().filter(|m| m.file == "cool.jar").count(), 1);

        // dragging a file out of the mods folder itself is not an overwrite
        let inside = detect(&root.join("mods/cool.jar"));
        let same = svc.add_dropped(&[inside], &[], Some(&mut replace)).await;
        assert_eq!(same.duplicates.len(), 1);
        assert!(root.join("mods/cool.jar").is_file());
    }

    #[tokio::test]
    async fn worlds_are_unpacked_into_saves() {
        let dirs = Dirs::at(temp("world"));
        let source = temp("world-src");
        let instance = make_instance(&dirs, "W", "1.21.1", Some("Fabric"));
        let mut svc = ContentService::new(&dirs, instance, offline());
        let zipped = zip(&source, "My World.zip", &[("My World/level.dat", "d"), ("My World/region/r.0.0.mca", "r")]);
        let folder = source.join("Folder World");
        std::fs::create_dir_all(folder.join("region")).unwrap();
        std::fs::write(folder.join("level.dat"), "d").unwrap();
        let files = vec![detect(&zipped), detect(&folder)];
        let result = svc.add_dropped(&files, &[], None).await;
        assert_eq!(result.worlds, vec!["My World".to_string(), "Folder World".to_string()]);
        let root = svc.instance().root.clone();
        assert!(root.join("saves/My World/level.dat").is_file());
        assert!(root.join("saves/My World/region/r.0.0.mca").is_file());
        assert!(root.join("saves/Folder World/region").is_dir());
        assert_eq!(svc.worlds(), vec!["Folder World".to_string(), "My World".to_string()]);
    }

    #[tokio::test]
    async fn switching_off_and_removing_move_and_delete_the_file() {
        let dirs = Dirs::at(temp("toggle"));
        let source = temp("toggle-src");
        let instance = make_instance(&dirs, "T", "1.21.1", Some("Fabric"));
        let mut svc = ContentService::new(&dirs, instance, offline());
        let jar = zip(&source, "a.jar", &[("fabric.mod.json", r#"{"name":"A"}"#)]);
        svc.add_dropped(&[detect(&jar)], &[], None).await;
        let root = svc.instance().root.clone();
        let rec = svc.installed(ContentKind::Mods).remove(0);

        svc.set_enabled(&rec, false).unwrap();
        assert!(!root.join("mods/a.jar").exists() && root.join("disabledmods/a.jar").is_file());
        assert!(crate::instances::load_one(&root).unwrap().mod_records()[0].disabled);

        let rec = svc.installed(ContentKind::Mods).remove(0);
        svc.set_enabled(&rec, true).unwrap();
        assert!(root.join("mods/a.jar").is_file() && !root.join("disabledmods/a.jar").exists());

        let rec = svc.installed(ContentKind::Mods).remove(0);
        svc.remove(&rec).unwrap();
        assert!(!root.join("mods/a.jar").exists());
        assert!(svc.instance().mod_records().is_empty());
    }

    #[tokio::test]
    async fn files_put_in_the_folders_by_hand_are_picked_up_once() {
        let dirs = Dirs::at(temp("scan"));
        let instance = make_instance(&dirs, "S", "1.21.1", Some("Fabric"));
        let mut svc = ContentService::new(&dirs, instance, offline());
        let root = svc.instance().root.clone();
        zip(&root.join("mods"), "hand.jar", &[("fabric.mod.json", r#"{"name":"By Hand","version":"1"}"#)]);
        std::fs::write(root.join("mods/readme.txt"), "x").unwrap();
        std::fs::create_dir_all(root.join("shaderpacks")).unwrap();
        zip(&root.join("shaderpacks"), "sh.zip", &[("shaders/a.fsh", "")]);
        assert!(svc.scan_untracked().await);
        assert_eq!(svc.installed(ContentKind::Mods)[0].name, "By Hand");
        assert_eq!(svc.installed(ContentKind::Shaders).len(), 1);
        assert!(!svc.scan_untracked().await);
    }

    #[tokio::test]
    async fn a_missing_curseforge_side_says_so() {
        let dirs = Dirs::at(temp("cf"));
        let instance = make_instance(&dirs, "C", "1.21.1", Some("Fabric"));
        let mut svc = ContentService::new(&dirs, instance, offline());
        let err = svc
            .install_from_curseforge(
                CurseInstall { project_id: 1, file_id: None, kind: CurseKind::Mod },
                &Progress::new(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("CurseForge"));

        struct Fake;
        impl CurseForgeSide for Fake {
            fn install<'a>(
                &'a self,
                _: &'a Instance,
                r: CurseInstall,
                _: &'a Arc<Progress>,
            ) -> BoxFuture<'a, Result<Vec<ModRecord>>> {
                Box::pin(async move {
                    let mut m = ModRecord::new("Curse Thing", "ct.jar", "mods");
                    m.curse_forge_project_id = Some(r.project_id);
                    m.curse_forge_file_id = Some(7);
                    Ok(vec![m])
                })
            }
        }
        let mut svc = svc.with_curseforge(Arc::new(Fake));
        let done = svc
            .install_from_curseforge(
                CurseInstall { project_id: 42, file_id: None, kind: CurseKind::Mod },
                &Progress::new(),
            )
            .await
            .unwrap();
        assert_eq!(done.installed, vec!["Curse Thing".to_string()]);
        assert_eq!(svc.instance().mod_records()[0].curse_forge_project_id, Some(42));
    }

    /// Live check against the real Modrinth: `cargo test --lib live_ -- --ignored --nocapture`.
    /// Builds a Fabric 1.21.1 instance in `<crate>/data` and installs the performance pack, then Iris (which
    /// needs Sodium and the Fabric API) to see dependencies handled.
    #[tokio::test]
    #[ignore]
    async fn live_performance_pack_on_fabric_1_21_1() {
        let dirs = Dirs::at(Path::new(env!("CARGO_MANIFEST_DIR")).join("data"));
        let root = dirs.instance_dir("LivePerf");
        let _ = std::fs::remove_dir_all(&root);
        let instance = make_instance(&dirs, "LivePerf", "1.21.1", Some("Fabric"));
        let mut svc =
            ContentService::new(&dirs, instance, ServiceOptions { identify_files: true, ..Default::default() });
        let progress = Progress::new();

        for id in [
            loader_support::SODIUM,
            performance_pack::LITHIUM,
            performance_pack::FERRITE_CORE,
            performance_pack::MODERN_FIX,
        ] {
            let ok = svc.is_available(id, ContentKind::Mods).await;
            let versions = svc.versions(id).await.unwrap();
            let picked = svc.pick(ContentKind::Mods, &versions).map(|v| v.name.clone());
            println!("{} available={ok} picked={picked:?}", performance_pack::name_of(id));
            assert!(ok);
        }
        let result = svc.install_performance_pack(&progress).await;
        println!("installed={:?} skipped={:?} warnings={:?}", result.installed, result.skipped, result.warnings);
        assert_eq!(result.installed.len(), 4, "{result:?}");
        for e in std::fs::read_dir(root.join("mods")).unwrap().flatten() {
            println!("  mods/{}  ({} bytes)", e.file_name().to_string_lossy(), e.metadata().unwrap().len());
        }
        // run again: everything is installed, nothing happens
        let again = svc.install_performance_pack(&progress).await;
        assert!(again.installed.is_empty() && again.skipped.is_empty(), "{again:?}");

        // dependencies: Iris needs Sodium; Fabric API comes with some of the above or with Iris
        let iris = svc.install(loader_support::IRIS, ContentKind::Mods, &progress).await.unwrap();
        println!("iris installed={:?} warnings={:?}", iris.installed, iris.warnings);
        println!("shader setup: {:?}", svc.shader_setup());

        // a shader pack and a resource pack through the same path
        let rp = svc.search(ContentKind::ResourcePacks, "faithful", 0, "downloads").await.unwrap();
        println!("resource pack search: {} hits, first {:?}", rp.total_hits, rp.hits.first().map(|h| h.title.clone()));
        let updates = svc.find_updates(ContentKind::Mods).await.unwrap();
        println!("updates available: {}", updates.len());

        let reread = crate::instances::load_one(&root).unwrap();
        for m in reread.mod_records() {
            println!("  record {} / {} / {} / project {:?}", m.name, m.version, m.file, m.modrinth_project_id());
        }
        assert!(reread.mod_records().len() >= 5);
    }

    /// Live: installing Iris on a bare instance brings Sodium with it.
    #[tokio::test]
    #[ignore]
    async fn live_iris_brings_its_dependencies() {
        let dirs = Dirs::at(Path::new(env!("CARGO_MANIFEST_DIR")).join("data"));
        let _ = std::fs::remove_dir_all(dirs.instance_dir("LiveIris"));
        let instance = make_instance(&dirs, "LiveIris", "1.21.1", Some("Fabric"));
        let mut svc = ContentService::new(&dirs, instance, offline());
        let r = svc.install_shader_loader(&Progress::new()).await.unwrap();
        println!("installed={:?} warnings={:?}", r.installed, r.warnings);
        assert!(r.installed.iter().any(|n| n == "Sodium") && r.installed.iter().any(|n| n.contains("Iris")));
        assert_eq!(svc.shader_setup(), ShaderSetup::Ready);
    }
}
