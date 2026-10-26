//! The pack installer: the Java edition's `InstanceInstaller` for ATLauncher and Technic packs.
//!
//! One run does, in this order: read the pack version's definition, show the pack's install/update message,
//! decide the mods, back up what a reinstall must keep, clear what the old install put there, install the
//! loader, download the game (assets, libraries, game jar, Mojang's Java), download and place the mods, run the
//! pack's actions, unpack its configs, and write `instance.json`. A new install that fails is removed; a
//! reinstall that fails leaves the instance marked as not playable, as the Java edition does.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::atlauncher::{self, Pack, PackVersion};
use super::choose::{ModChoices, Remembered};
use super::dl::{self, Dl};
use super::json::{DownloadKind, PackJson, PackLibrary, PackMod};
use super::loader::{self, Kind};
use super::place::{self, Site};
use super::server;
use super::technic::{self, Modpack, SolderManifest};
use super::{DOWNLOAD_SERVER, fsx, safe_name};
use crate::config::{Settings, write_json_atomic};
use crate::install::{self, Progress};
use crate::instances::{Instance, InstanceFile, LauncherBlock, LoaderVersion};
use crate::model::{Arg, Arguments, Artifact, LibDownloads, Library, VersionJson};
use crate::modrecord::ModRecord;
use crate::paths::Dirs;

const LEGACY_JAVA_FIXER_URL: &str = "https://cdn.atlcdn.net/legacyjavafixer-1.0.jar";
const LEGACY_JAVA_FIXER_MD5: &str = "12c337cb2445b56b097e7c25a5642710";

/// Things an install needs to know about the launcher's settings.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub dirs: Dirs,
    /// Download Mojang's Java for the Minecraft version (`useJavaProvidedByMinecraft`, on by default).
    pub use_mojang_java: bool,
    /// Where the player's browser saves files, when the settings name a folder (`customDownloadsPath`).
    pub custom_downloads_path: Option<String>,
}

impl Ctx {
    pub fn new(dirs: Dirs) -> Ctx {
        Ctx { dirs, use_mojang_java: true, custom_downloads_path: None }
    }

    pub fn from_settings(dirs: &Dirs, settings: &Settings) -> Ctx {
        Ctx {
            dirs: dirs.clone(),
            use_mojang_java: settings.extras.get("useJavaProvidedByMinecraft").and_then(Value::as_bool).unwrap_or(true),
            custom_downloads_path: settings
                .extras
                .get("customDownloadsPath")
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    }
}

/// Which pack is being installed.
#[derive(Clone, Debug)]
pub enum Source {
    Atl(Pack),
    Technic(Modpack),
}

impl Source {
    pub fn pack_name(&self) -> String {
        match self {
            Source::Atl(p) => p.name.clone(),
            Source::Technic(m) => m.title().to_string(),
        }
    }

    pub fn pack_id(&self) -> i64 {
        match self {
            Source::Atl(p) => p.id,
            Source::Technic(_) => 0,
        }
    }

    pub fn external_id(&self) -> i64 {
        match self {
            Source::Atl(p) => p.external_id,
            Source::Technic(m) => m.id,
        }
    }

    pub fn description(&self) -> String {
        match self {
            Source::Atl(p) => p.description.clone(),
            Source::Technic(m) => m.description.clone().unwrap_or_default(),
        }
    }
}

/// The version being installed.
#[derive(Clone, Debug, Default)]
pub struct VersionInfo {
    pub version: String,
    pub hash: Option<String>,
    pub is_dev: bool,
}

impl From<&PackVersion> for VersionInfo {
    fn from(v: &PackVersion) -> Self {
        VersionInfo { version: v.version.clone(), hash: v.hash.clone(), is_dev: v.is_dev() }
    }
}

/// Something the player has to do by hand: save a file from a web page.
#[derive(Clone, Debug)]
pub struct BrowserRequest {
    pub name: String,
    pub file: String,
    pub url: String,
    /// Where the file will be looked for.
    pub watch: Vec<PathBuf>,
    pub expected_size: i64,
    /// This is the `n`th of `of` browser downloads.
    pub n: usize,
    pub of: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserAnswer {
    /// The file is in one of the watched folders.
    Downloaded,
    /// Leave this mod out (the pack may not work without it).
    Skip,
    /// Stop the whole install.
    Cancel,
}

/// How an install talks to the player. All calls come from a worker thread and may block until the player has
/// answered. [`Quiet`] is the version for programs without a window.
pub trait Interaction: Send + Sync {
    /// The pack's install or update message; `false` cancels the install.
    fn message(&self, _text: &str, _is_update: bool) -> bool {
        true
    }

    /// A mod that can only be downloaded in a browser.
    fn browser_download(&self, request: &BrowserRequest) -> BrowserAnswer;
}

/// The no-window version: opens the page in the browser and waits for the file to appear in the downloads
/// folder, for up to ten minutes, then skips the mod. Stops waiting when the install is cancelled.
pub struct Quiet {
    pub progress: Arc<Progress>,
    pub wait: Duration,
}

impl Quiet {
    pub fn new(progress: Arc<Progress>) -> Quiet {
        Quiet { progress, wait: Duration::from_secs(600) }
    }
}

impl Interaction for Quiet {
    fn browser_download(&self, r: &BrowserRequest) -> BrowserAnswer {
        let _ = open::that(&r.url);
        let started = Instant::now();
        loop {
            if self.progress.cancelled.load(Ordering::Relaxed) {
                return BrowserAnswer::Cancel;
            }
            if r.watch.iter().any(|dir| dir.join(&r.file).is_file() && !dir.join(format!("{}.part", r.file)).exists()) {
                // give the browser a moment to finish writing
                std::thread::sleep(Duration::from_secs(1));
                return BrowserAnswer::Downloaded;
            }
            if started.elapsed() > self.wait {
                return BrowserAnswer::Skip;
            }
            std::thread::sleep(Duration::from_millis(700));
        }
    }
}

/// Everything one install or reinstall needs.
pub struct Request {
    /// The instance name the player typed. Ignored for a reinstall, which keeps the instance's name.
    pub name: String,
    pub source: Source,
    pub version: VersionInfo,
    pub is_reinstall: bool,
    pub is_server: bool,
    /// On an update that changes the Minecraft version: keep the mods the player added.
    pub save_mods: bool,
    /// The instance being reinstalled or updated.
    pub existing: Option<Instance>,
    /// A loader version picked instead of the pack's own.
    pub loader_choice: Option<loader::Choice>,
    /// The optional mods as the player chose them. `None` uses the defaults (or, on a reinstall, what the
    /// instance had).
    pub choices: Option<ModChoices>,
    /// For a Solder pack: the build's manifest, when it has been fetched already.
    pub solder: Option<SolderManifest>,
    pub ui: Arc<dyn Interaction>,
    /// Save a snapshot of the instance first (updates and reinstalls).
    pub snapshot: bool,
    /// Set the moment the install starts changing the instance's files: only a failure after that leaves it in a
    /// state that can't be trusted.
    files_touched: bool,
}

impl Request {
    pub fn new(name: &str, source: Source, version: VersionInfo, ui: Arc<dyn Interaction>) -> Request {
        Request {
            name: name.to_string(),
            source,
            version,
            is_reinstall: false,
            is_server: false,
            save_mods: false,
            existing: None,
            loader_choice: None,
            choices: None,
            solder: None,
            ui,
            snapshot: true,
            files_touched: false,
        }
    }
}

/// What is known about a version before installing it, so a window can show the optional mods, the loader
/// choice, the install message and the Java and memory the pack wants.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub json: PackJson,
    pub choices: ModChoices,
    /// Set when the pack lets the player pick the loader version.
    pub loader_kind: Option<Kind>,
    pub solder: Option<SolderManifest>,
}

/// Read a version's definition. For a plain-zip Technic pack nothing is downloaded here (the zip itself is
/// only fetched by the install), so its definition is just the Minecraft version.
pub async fn prepare(
    ctx: &Ctx,
    source: &Source,
    version: &VersionInfo,
    is_server: bool,
    existing: Option<&Instance>,
) -> Result<Prepared> {
    let (json, solder) = match source {
        Source::Atl(pack) => {
            (atlauncher::fetch_pack_json(&ctx.dirs, &pack.name, &version.version, false).await?.0, None)
        }
        Source::Technic(m) => technic_definition(ctx, m, &version.version, None).await?,
    };
    let remembered = existing.map(|i| Remembered::from_records(&i.mod_records()));
    let mods: Vec<PackMod> = json.mods.iter().filter(|m| m.installs_on(is_server)).cloned().collect();
    let choices = ModChoices::new(mods, is_server, remembered.as_ref())?;
    let loader_kind = json.loader.as_ref().and_then(|l| l.class_name.as_deref()).and_then(Kind::from_class);
    Ok(Prepared { json, choices, loader_kind, solder })
}

/// The pack definition for a Technic pack: for Solder from the build's manifest, for a zip just the Minecraft
/// version (the rest comes out of the zip during the install).
async fn technic_definition(
    ctx: &Ctx,
    m: &Modpack,
    build: &str,
    manifest: Option<SolderManifest>,
) -> Result<(PackJson, Option<SolderManifest>)> {
    let mut json = PackJson {
        version: build.to_string(),
        enable_curse_forge_integration: true,
        enable_editing_mods: true,
        ..Default::default()
    };
    if let Some(solder) = m.solder.as_deref().filter(|s| !s.is_empty()) {
        let manifest = match manifest {
            Some(x) => x,
            None => technic::solder_manifest(&ctx.dirs, solder, &m.name, build).await?,
        };
        json.minecraft = manifest
            .minecraft
            .clone()
            .or_else(|| m.minecraft.clone())
            .context("Minecraft version couldn't be found.")?;
        if let Some(mem) = technic::solder_memory(&manifest) {
            json.memory = mem;
        }
        Ok((json, Some(manifest)))
    } else {
        json.minecraft =
            m.minecraft.clone().filter(|v| !v.is_empty()).context("Minecraft version couldn't be found.")?;
        Ok((json, None))
    }
}

fn target_root(ctx: &Ctx, req: &Request) -> Result<PathBuf> {
    if let Some(e) = &req.existing {
        if req.is_reinstall {
            return Ok(e.root.clone());
        }
    }
    let safe = safe_name(&req.name);
    if safe.is_empty() {
        bail!(
            "{} name is invalid. It must contain at least 1 letter or number.",
            if req.is_server { "Server" } else { "Instance" }
        );
    }
    let base = if req.is_server { super::servers_dir(&ctx.dirs) } else { ctx.dirs.instances.clone() };
    let root = base.join(&safe);
    if root.exists() {
        bail!(
            "{} already exists with that name. Rename it and try again.",
            if req.is_server { "A server" } else { "An instance" }
        );
    }
    Ok(root)
}

/// Install a pack version (or reinstall / update an instance to it). Returns the finished instance.
pub async fn install(ctx: &Ctx, mut req: Request, progress: &Arc<Progress>) -> Result<Instance> {
    // a plain-zip Technic pack has one version: the one Technic says is current
    if let Source::Technic(m) = &req.source {
        if !m.is_solder() && req.version.version.is_empty() {
            req.version.version = m.version.clone().unwrap_or_default();
        }
    }
    let root = target_root(ctx, &req)?;
    let existed_before = root.exists();
    let result = run(ctx, &mut req, &root, progress).await;
    let temp = temp_dir(ctx, &req);
    fsx::delete_path(&temp);
    match result {
        Ok(instance) => Ok(instance),
        Err(e) => {
            if req.is_reinstall {
                // from the moment the old files were touched, the instance cannot be trusted; a reinstall that was
                // cancelled or failed before that (offline, the player said no to the pack's message) changed
                // nothing, and the instance stays playable
                if req.files_touched {
                    if let Some(mut inst) = crate::instances::load_one(&root) {
                        inst.file.launcher.extras.insert("isPlayable".into(), Value::Bool(false));
                        let _ = inst.save();
                    }
                }
            } else if !existed_before && root.exists() {
                fsx::delete_path(&root);
            }
            Err(e)
        }
    }
}

fn temp_dir(ctx: &Ctx, req: &Request) -> PathBuf {
    ctx.dirs.temp.join(format!("{}_{}", safe_name(&req.source.pack_name()), safe_name(&req.version.version)))
}

struct Downloaded {
    json: PackJson,
    solder: Option<SolderManifest>,
    /// The folder a Technic zip was unpacked to.
    technic_zip: Option<PathBuf>,
    /// The loader found inside a Technic pack (it is not in the definition).
    technic_loader: Option<super::json::PackLoader>,
}

async fn definition(ctx: &Ctx, req: &Request, progress: &Arc<Progress>) -> Result<Downloaded> {
    progress.stage(format!(
        "Generating pack version from {}",
        match req.source {
            Source::Atl(_) => "ATLauncher",
            Source::Technic(ref m) if m.is_solder() => "Technic Solder",
            _ => "Technic Zip",
        }
    ));
    match &req.source {
        Source::Atl(pack) => {
            let (json, _) = atlauncher::fetch_pack_json(&ctx.dirs, &pack.name, &req.version.version, true)
                .await
                .context("Failed to download pack version definition")?;
            Ok(Downloaded { json, solder: None, technic_zip: None, technic_loader: None })
        }
        Source::Technic(m) => {
            let (mut json, solder) = technic_definition(ctx, m, &req.version.version, req.solder.clone()).await?;
            if solder.is_some() {
                return Ok(Downloaded { json, solder, technic_zip: None, technic_loader: None });
            }
            // a plain zip: fetch it now, the loader and the mods are inside
            let url = m.url.clone().filter(|u| !u.is_empty()).context("this Technic pack has no download address")?;
            let zip = ctx.dirs.temp.join(format!("technic-{}-modpack.zip", m.name));
            let out = ctx.dirs.temp.join(format!("technic-{}", m.name));
            fsx::delete_path(&out);
            progress.stage("Downloading modpack.zip");
            dl::download_all(vec![Dl::new(&url, &zip)], progress).await.context("downloading the modpack")?;
            progress.stage("Unpacking modpack.zip");
            let (zip2, out2) = (zip.clone(), out.clone());
            tokio::task::spawn_blocking(move || fsx::unzip(&zip2, &out2)).await??;
            let _ = std::fs::remove_file(&zip);
            let loader = technic_loader(ctx, &out, &json.minecraft).await?;
            json.loader = loader.clone();
            Ok(Downloaded { json, solder: None, technic_zip: Some(out), technic_loader: loader })
        }
    }
}

/// Look in a Technic pack's `bin/` for the game description that names its loader (Forge, Fabric, NeoForge).
/// The description (`bin/version.json`, or the one inside `bin/modpack.jar`) is removed; it is not needed again.
async fn technic_loader(ctx: &Ctx, base: &Path, mc: &str) -> Result<Option<super::json::PackLoader>> {
    let version_json = base.join("bin").join("version.json");
    let modpack_jar = base.join("bin").join("modpack.jar");
    let mut found: Option<VersionJson> = None;
    if version_json.is_file() {
        let bytes = std::fs::read(&version_json).context("reading bin/version.json")?;
        found = Some(serde_json::from_slice(&bytes).context("Error reading in version.json")?);
        fsx::delete_path(&version_json);
    }
    if modpack_jar.is_file() && fsx::zip_contains(&modpack_jar, "version.json") {
        let bytes =
            fsx::read_zip_entry(&modpack_jar, "version.json").context("reading version.json from modpack.jar")?;
        found = Some(serde_json::from_slice(&bytes).context("Error reading in version.json from modpack.jar")?);
        fsx::delete_path(&modpack_jar);
    }
    match found {
        Some(v) if !v.libraries.is_empty() || v.id.starts_with("neoforge-") => {
            loader::loader_from_version_json(&ctx.dirs, &v, mc).await
        }
        _ => Ok(None),
    }
}

async fn run(ctx: &Ctx, req: &mut Request, root: &Path, progress: &Arc<Progress>) -> Result<Instance> {
    let dirs = &ctx.dirs;
    dirs.ensure()?;
    let kind_label = if req.is_server { "server" } else { "instance" };
    progress.stage("Reading the pack");
    let temp = temp_dir(ctx, req);
    fsx::delete_path(&temp);
    std::fs::create_dir_all(&temp)?;

    // 1. the pack version
    let Downloaded { json, solder, technic_zip, technic_loader } = definition(ctx, req, progress).await?;
    let mc = json.minecraft.clone();
    if mc.is_empty() {
        bail!("The pack version doesn't say which Minecraft version it is for");
    }
    if progress.cancelled.load(Ordering::Relaxed) {
        bail!("cancelled");
    }

    // 2. Minecraft's own description
    progress.stage(format!("Downloading Minecraft {mc}"));
    let vanilla = install::vanilla_json(dirs, &mc).await.with_context(|| format!("getting Minecraft {mc}"))?;

    // 3. the pack's message
    if let Some(m) = &json.messages {
        let text = if req.is_reinstall { m.update.clone() } else { m.install.clone() };
        if let Some(text) = text {
            let ui = req.ui.clone();
            let is_update = req.is_reinstall;
            let ok = tokio::task::spawn_blocking(move || ui.message(&text, is_update)).await?;
            if !ok {
                bail!("Install cancelled after viewing message!");
            }
        }
    }

    // 4. which mods
    let existing_records: Vec<ModRecord> = req.existing.as_ref().map(|i| i.mod_records()).unwrap_or_default();
    let mut choices = match req.choices.take() {
        Some(c) => c,
        None => {
            let mods = json_mods(&json, req.is_server, &existing_records, req.is_reinstall);
            let remembered = if req.is_reinstall { Some(Remembered::from_records(&existing_records)) } else { None };
            ModChoices::new(mods, req.is_server, remembered.as_ref())?
        }
    };
    // bring in what a previous install found out about its mods (CurseForge details)
    if req.is_reinstall {
        for c in &mut choices.choices {
            if let Some(r) = existing_records.iter().find(|r| r.file == c.pack_mod.file) {
                if r.curse_forge_project.is_some() && r.curse_forge_file.is_some() {
                    c.pack_mod.curse_forge_project = r.curse_forge_project.clone();
                    c.pack_mod.curse_forge_file = r.curse_forge_file.clone();
                }
            }
        }
    }
    let selected = choices.selected();

    // 5. keep custom mods through an update, or drop them when the Minecraft version changed and the player said so
    let mut records: Vec<ModRecord> = selected.iter().map(|m| record_for(&json, m)).collect();
    let mut mc_changed = false;
    if let Some(e) = req.existing.as_ref().filter(|_| req.is_reinstall) {
        mc_changed = !e.mc_version().eq_ignore_ascii_case(&mc);
        let custom: Vec<ModRecord> = existing_records.iter().filter(|r| r.user_added).cloned().collect();
        if !custom.is_empty() {
            if req.save_mods || !mc_changed {
                records.extend(custom.iter().cloned());
            } else {
                for r in &custom {
                    delete_record_file(&e.root, r, e.mc_version());
                }
            }
        }
    }

    // 6. a snapshot, so a bad update can be undone
    if req.is_reinstall && req.snapshot {
        if let Some(e) = &req.existing {
            progress.stage("Saving a snapshot of the instance");
            if let Err(err) = super::snapshot::save_before(
                dirs,
                e,
                if req.version.is_dev || e.mc_version() == mc { "before-update" } else { "before-update" },
            ) {
                progress.stage(format!("Couldn't save a snapshot ({err:#}); going on"));
            }
        }
    }

    let site = Site {
        root: root.to_path_buf(),
        temp: temp.clone(),
        downloads: super::downloads_dir(dirs),
        mc: mc.clone(),
        is_server: req.is_server,
    };
    let override_paths: Vec<String> = req
        .existing
        .as_ref()
        .and_then(|e| e.file.launcher.extras.get("overridePaths"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();

    // 7. keep what must survive, clear what the old install wrote
    req.files_touched = true;
    let saved = place::backup_select_files(&site, req.is_reinstall, json.keeps.as_ref());
    place::prepare_filesystem(&site, req.is_reinstall, &existing_records, &override_paths, json.deletes.as_ref());
    std::fs::create_dir_all(super::downloads_dir(dirs))?;

    // from here on the instance is changed
    progress.stage("Preparing");

    // 8. Technic Solder: the mods are zips that unpack into the instance, and they carry the loader information
    let mut technic_solder_mods: Vec<PackMod> = Vec::new();
    let mut loader_def = json.loader.clone().or(technic_loader);
    if let (Some(manifest), Source::Technic(m)) = (&solder, &req.source) {
        technic_solder_mods = manifest.mods.iter().map(|x| x.to_pack_mod()).collect();
        progress.stage("Downloading mods");
        let dir = super::technic_downloads_dir(dirs).join(&m.name);
        let jobs: Vec<Dl> = technic_solder_mods
            .iter()
            .map(|pm| {
                let mut d = Dl::new(pm.download_url(), dir.join(pm.file_name())).size(pm.filesize).unzip_to(root);
                d = d.hash(pm.md5.as_deref()).keep_on_mismatch();
                d
            })
            .collect();
        dl::download_all(jobs, progress).await.context("downloading the pack's mods")?;
        loader_def = technic_loader_from_root(ctx, root, &mc, loader_def).await?;
        for (dir_rel, kind) in [("mods", "mods"), (&format!("mods/{mc}"), "dependency"), ("mods/ic2", "ic2lib")] {
            records.extend(records_from_folder(&root.join(dir_rel), kind));
        }
    }

    // 9. the loader
    let mut loader_installed: Option<loader::Installed> = None;
    if let Some(def) = loader_def.as_ref().filter(|d| d.class_name.is_some()) {
        let class = def.class_name.clone().unwrap_or_default();
        let kind = Kind::from_class(&class).with_context(|| format!("Loader class {class} is unknown."))?;
        if !kind.for_clients() && !req.is_server {
            bail!("{} can only be used for servers", kind.type_name());
        }
        if req.is_server
            && matches!(
                kind,
                Kind::Forge113 | Kind::NeoForge | Kind::Quilt | Kind::Paper | Kind::Purpur | Kind::LegacyFabric
            )
        {
            bail!(
                "Making a server with {} isn't supported yet. Use the Java edition for this pack's server.",
                kind.type_name()
            );
        }
        progress.stage(format!("Installing {} (this can take a minute)", kind.type_name()));
        loader_installed = Some(
            loader::install(dirs, kind, &def.metadata, &mc, req.loader_choice.as_ref(), &vanilla, root, progress)
                .await?,
        );
    }

    // 10. the game description of the finished instance
    let selected_names: Vec<&PackMod> = selected.iter().collect();
    let mut game: VersionJson = match &loader_installed {
        Some(l) => l.merged.clone(),
        None => vanilla.clone(),
    };
    game.id = mc.clone();
    game.inherits_from = None;
    if !req.is_server {
        for lib in &mut game.libraries {
            fix_log4j(lib);
        }
    }
    let main_class = determine_main_class(&json, &selected_names, loader_installed.as_ref(), &game.main_class);
    game.main_class = main_class;
    apply_arguments(&mut game, &json, &selected_names);

    // 11. the game's files
    let mut assets_map = false;
    if !req.is_server {
        if let Some(jv) = &game.java_version {
            if ctx.use_mojang_java && !jv.component.is_empty() {
                if let Err(e) = crate::java::ensure_runtime(dirs, &jv.component, progress).await {
                    progress.stage(format!("Couldn't get Mojang's Java ({e:#}); the instance will use another Java"));
                }
            }
        }
        progress.stage("Downloading game files");
        install::ensure_game_files(dirs, &game, progress, false).await.context("downloading Minecraft")?;
        assets_map = assets_map_to_resources(dirs, &game);
        if assets_map || game.asset_index.as_ref().is_some_and(|a| a.id.eq_ignore_ascii_case("legacy")) {
            let _ = install::map_legacy_assets(dirs, &game, root);
        }
    }

    let mut server_jar_file: Option<String> = None;
    if req.is_server {
        server_jar_file = server_files(ctx, &site, &vanilla, &game, loader_installed.as_ref(), progress).await?;
    }

    // 12. the pack's own libraries
    let pack_libs = pack_libraries(&json, &selected_names, req.is_server);
    if !pack_libs.is_empty() {
        progress.stage("Downloading libraries");
        let jobs: Vec<Dl> = pack_libs
            .iter()
            .filter_map(|l| {
                let a = l.downloads.as_ref()?.artifact.as_ref()?;
                Some(
                    Dl::new(&a.url, dirs.libraries.join(a.path.as_deref()?))
                        .hash(a.sha1.as_deref())
                        .size(a.size.unwrap_or(0)),
                )
            })
            .collect();
        dl::download_all(jobs, progress).await.context("downloading the pack's libraries")?;
    }
    let mut libraries = pack_libs;
    libraries.extend(std::mem::take(&mut game.libraries));
    game.libraries = libraries;

    // 13. mods: download, then place
    let downloadable: Vec<&PackMod> = selected.iter().filter(|m| m.download != Some(DownloadKind::Browser)).collect();
    if !downloadable.is_empty() {
        progress.stage("Downloading mods");
        let jobs: Vec<Dl> = downloadable
            .iter()
            .map(|m| {
                let server_file = m.uses_server_file(req.is_server);
                let url = if server_file {
                    m.server_url.clone().unwrap_or_default().replace(' ', "%20")
                } else {
                    m.download_url()
                };
                let mut d =
                    Dl::new(url, site.downloads.join(place::downloaded_name(m, req.is_server))).size(m.filesize);
                d = d.hash(if server_file {
                    m.server_md5.as_deref()
                } else {
                    m.md5.as_deref().or(m.sha1.as_deref()).or(m.sha512.as_deref())
                });
                if m.ignore_failures {
                    d = d.ignore_failures();
                }
                d.keep_on_mismatch()
            })
            .collect();
        dl::download_all(jobs, progress).await.context("downloading the pack's mods")?;
    }
    let browser_mods: Vec<&PackMod> = selected.iter().filter(|m| m.download == Some(DownloadKind::Browser)).collect();
    let mut skipped: HashSet<String> = HashSet::new();
    // a mod found by a pattern (`filePattern`) is saved under the name the player's browser gave it, and installed
    // under that name: its `file` is the pattern, not a file that exists
    let mut found_as: HashMap<String, String> = HashMap::new();
    for (i, m) in browser_mods.iter().enumerate() {
        if progress.cancelled.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        progress.stage(format!("Waiting for you to download {} ({}/{})", m.name, i + 1, browser_mods.len()));
        match fetch_browser_mod(ctx, req.ui.clone(), &site, m, i + 1, browser_mods.len()).await? {
            Some(actual) => {
                if m.file_pattern {
                    if let Some(rec) = records.iter_mut().find(|r| r.file == cased_for_record(&json, m)) {
                        rec.file = actual.clone();
                    }
                    found_as.insert(m.name.clone(), actual);
                }
            }
            None => {
                skipped.insert(m.file_name());
                if let Some(rec) = records.iter_mut().find(|r| r.file == cased_for_record(&json, m)) {
                    rec.skipped = true;
                }
            }
        }
    }

    progress.stage("Installing mods");
    {
        let chosen: Vec<PackMod> = selected
            .iter()
            .filter(|m| !skipped.contains(&m.file_name()))
            .cloned()
            .map(|mut m| {
                if let Some(actual) = found_as.get(&m.name).filter(|_| m.file_pattern) {
                    m.file = actual.clone();
                    m.file_prefix = None;
                    m.file_pattern = false;
                }
                m
            })
            .collect();
        let site2 = site.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            for m in &chosen {
                place::install_mod(&site2, m)?;
            }
            Ok(())
        })
        .await??;
    }

    // 14. the little extras the Java edition adds for very old Forge
    if let Some(l) = loader_installed.as_ref().filter(|l| l.info.kind.eq_ignore_ascii_case("Forge")) {
        if !req.is_server && (loader::match_version(&mc, "1.6", true, true) || mc == "1.7.2") {
            let _ = l;
            if let Some(rec) = legacy_java_fixer(ctx, root, progress).await {
                records.push(rec);
            }
        }
    }

    // 15. case, actions, configs
    place::convert_case(&site, &json, &selected);
    place::run_actions(
        &site,
        &json,
        &selected.iter().filter(|m| !skipped.contains(&m.file_name())).cloned().collect::<Vec<_>>(),
    )?;
    let mut new_override_paths: Vec<String> = Vec::new();
    install_configs(ctx, req, &json, &site, technic_zip.as_deref(), &mut new_override_paths, progress).await?;
    if let Source::Technic(m) = &req.source {
        technic_image(m, root).await;
    }
    if matches!(req.source, Source::Technic(_)) {
        let bin = root.join("bin");
        if bin.is_dir() && std::fs::read_dir(&bin).map(|mut r| r.next().is_none()).unwrap_or(false) {
            fsx::delete_path(&bin);
        }
    }
    let common = super::common_dir(dirs);
    if common.is_dir() && std::fs::read_dir(&common).map(|mut r| r.next().is_some()).unwrap_or(false) {
        fsx::copy_dir(&common, root, false)?;
    }
    place::restore_select_files(&site, &saved, req.is_reinstall, json.keeps.as_ref());

    // 16. mods that ended up in a different case need their record to say so, and skipped ones are marked
    for r in &mut records {
        if skipped.contains(&r.file) {
            r.skipped = true;
        }
    }

    // 17. instance.json (server.json and the start scripts for a server)
    progress.stage(format!("Saving the {kind_label}"));
    if req.is_server {
        let loader_info = loader_installed.as_ref().map(|l| l.info.clone());
        let jar = server_jar_file.unwrap_or_else(|| {
            let lk = loader_installed.as_ref().map(|l| {
                (
                    Kind::from_class(loader_def.as_ref().and_then(|d| d.class_name.as_deref()).unwrap_or(""))
                        .unwrap_or(Kind::Forge),
                    &l.info,
                )
            });
            server::server_jar_name(&mc, lk, &selected, None)
        });
        server::write_scripts(root, &json, &mc, &jar)?;
        let previous: Option<Value> =
            std::fs::read(root.join("server.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
        server::write_server_json(
            root,
            &req.name,
            &req.source.pack_name(),
            req.source.pack_id(),
            req.source.external_id(),
            &mc,
            &json.version,
            req.version.hash.as_deref(),
            req.version.is_dev,
            loader_info.as_ref(),
            vanilla.java_version.as_ref(),
            &records,
            previous.as_ref(),
        )?;
        let mut file = InstanceFile::default();
        file.launcher.name = req.name.clone();
        file.launcher.pack = req.source.pack_name();
        file.version.id = mc.clone();
        let folder = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        return Ok(Instance { folder, mod_count: 0, root: root.to_path_buf(), file });
    }
    let instance = write_instance(
        ctx,
        req,
        &json,
        game,
        loader_installed.as_ref().map(|l| l.info.clone()),
        records,
        &mc,
        assets_map,
        new_override_paths,
        root,
    )?;
    let _ = (technic_solder_mods, mc_changed);
    Ok(instance)
}

/// The files a server needs besides mods: Mojang's server jar, the loader's libraries and, for Forge and Fabric,
/// the loader's own server jar. Returns the jar name the start script should run when the loader decides it.
async fn server_files(
    ctx: &Ctx,
    site: &Site,
    vanilla: &VersionJson,
    game: &VersionJson,
    loader: Option<&loader::Installed>,
    progress: &Arc<Progress>,
) -> Result<Option<String>> {
    let dirs = &ctx.dirs;
    let root = &site.root;
    progress.stage("Downloading the Minecraft server");
    let download =
        server::server_download(game).with_context(|| format!("Minecraft {} has no server to download", site.mc))?;
    let shared = server::server_jar_library(dirs, &site.mc);
    let job = install::Job::new(&download.url, &shared).checked(download.sha1.as_deref(), download.size);
    install::download_all(vec![job], progress, false).await.context("downloading the Minecraft server")?;
    fsx::copy_file(&shared, &root.join(format!("minecraft_server.{}.jar", site.mc)), true)?;
    let Some(l) = loader else { return Ok(None) };
    let libs = server::loader_only_libraries(&l.merged, vanilla);
    let jobs: Vec<install::Job> = libs
        .iter()
        .filter_map(|lib| {
            let a = lib.artifact()?;
            let path = a.path.clone()?;
            Some(install::Job::new(&a.url, dirs.libraries.join(path)).checked(a.sha1.as_deref(), a.size))
        })
        .collect();
    install::download_all(jobs, progress, false).await.context("downloading the loader's libraries")?;
    server::copy_libraries(dirs, root, &libs)?;
    let mut jar = None;
    match l.info.kind.as_str() {
        "Fabric" => {
            let files: Vec<PathBuf> =
                libs.iter().filter_map(|x| x.artifact()?.path).map(|p| root.join("libraries").join(p)).collect();
            server::make_fabric_launch_jar(
                root,
                &files,
                &l.merged.main_class,
                &l.info.version,
                &format!("minecraft_server.{}.jar", site.mc),
            )?;
            jar = Some("fabric-server-launch.jar".to_string());
        }
        "Forge" => {
            // old Forge is a replacement server jar, copied next to the scripts
            if let Some(forge) =
                libs.iter().find(|x| x.name.starts_with("net.minecraftforge:forge:")).and_then(|x| x.artifact()?.path)
            {
                let from = dirs.libraries.join(&forge);
                let name = forge.rsplit('/').next().unwrap_or(&forge).to_string();
                fsx::copy_file(&from, &root.join(&name), true)?;
                jar = Some(name);
            }
        }
        _ => {}
    }
    Ok(jar)
}

fn json_mods(json: &PackJson, is_server: bool, _existing: &[ModRecord], _reinstall: bool) -> Vec<PackMod> {
    json.mods.iter().filter(|m| m.installs_on(is_server)).cloned().collect()
}

fn delete_record_file(root: &Path, r: &ModRecord, mc: &str) {
    let file = record_path(root, r, mc, r.disabled);
    if let Some(f) = file {
        fsx::delete_path(&f);
    }
}

/// Where a recorded mod's file lives (the Java edition's `DisableableMod.getFile`).
pub fn record_path(root: &Path, r: &ModRecord, mc: &str, disabled: bool) -> Option<PathBuf> {
    if disabled {
        return Some(root.join("disabledmods").join(&r.file));
    }
    let dir = if let Some(p) = r.path.as_deref().filter(|p| !p.is_empty()) {
        root.join(p)
    } else {
        match r.kind.as_str() {
            "jar" | "forge" | "mcpc" => root.join("jarmods"),
            "texturepack" => root.join("texturepacks"),
            "resourcepack" => root.join("resourcepacks"),
            "mods" => root.join("mods"),
            "plugins" => root.join("plugins"),
            "ic2lib" => root.join("mods").join("ic2"),
            "denlib" => root.join("mods").join("denlib"),
            "coremods" => root.join("coremods"),
            "shaderpack" => root.join("shaderpacks"),
            "datapack" => root.join("datapacks"),
            "dependency" => root.join("mods").join(mc),
            _ => return None,
        }
    };
    Some(dir.join(&r.file))
}

fn cased_for_record(json: &PackJson, m: &PackMod) -> String {
    place::cased_file_name(json, m)
}

/// The Java edition's `DisableableMod` for a mod that is about to be installed.
pub fn record_for(json: &PackJson, m: &PackMod) -> ModRecord {
    let mut r =
        ModRecord::new(&m.name, &place::cased_file_name(json, m), m.kind.map(|k| k.record_name()).unwrap_or("mods"));
    r.version = m.version.clone();
    r.optional = m.optional;
    r.path = m.path.clone();
    r.description = m.description.clone().unwrap_or_default();
    r.disabled = false;
    r.user_added = false;
    r.was_selected = true;
    r.skipped = false;
    r.curse_forge_project_id = m.curse_forge_project_id;
    r.curse_forge_file_id = m.curse_forge_file_id;
    r.curse_forge_project = m.curse_forge_project.clone();
    r.curse_forge_file = m.curse_forge_file.clone();
    r.modrinth_project = m.modrinth_project.clone();
    r.modrinth_version = m.modrinth_version.clone();
    if let Some((red, green, blue)) = m.colour.as_deref().and_then(|c| json.colour_rgb(c)) {
        let argb = ((0xFFu32 << 24) | ((red as u32) << 16) | ((green as u32) << 8) | blue as u32) as i32;
        r.extras.insert("colour".into(), serde_json::json!({"value": argb, "falpha": 0.0}));
    }
    r
}

/// Records for the files a Solder pack unpacked into `dir`, named from what the mod itself says about itself.
fn records_from_folder(dir: &Path, kind: &str) -> Vec<ModRecord> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<_> = rd.flatten().filter(|e| e.path().is_file()).collect();
    files.sort_by_key(|e| e.file_name());
    files
        .into_iter()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            n.ends_with(".jar") || n.ends_with(".zip")
        })
        .map(|e| {
            let file = e.file_name().to_string_lossy().to_string();
            let mut r = ModRecord::new(&file, &file, kind);
            r.version = "Unknown".into();
            r.optional = true;
            if let Some((name, version, description)) = mod_info(&e.path()) {
                r.name = name.unwrap_or_else(|| file.clone());
                r.version = version.unwrap_or_else(|| "Unknown".into());
                r.description = description.unwrap_or_default();
            }
            r.was_selected = true;
            r
        })
        .collect()
}

/// A mod's own name, version and description from `mcmod.info` (Forge) or `fabric.mod.json` (Fabric).
pub fn mod_info(path: &Path) -> Option<(Option<String>, Option<String>, Option<String>)> {
    if let Some(bytes) = fsx::read_zip_entry(path, "mcmod.info") {
        if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
            let first = match &v {
                Value::Array(a) => a.first().cloned(),
                Value::Object(o) => {
                    o.get("modList").and_then(Value::as_array).and_then(|a| a.first().cloned()).or(Some(v.clone()))
                }
                _ => None,
            };
            if let Some(m) = first {
                let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
                return Some((s("name"), s("version"), s("description")));
            }
        }
    }
    if let Some(bytes) = fsx::read_zip_entry(path, "fabric.mod.json") {
        if let Ok(m) = serde_json::from_slice::<Value>(&bytes) {
            let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
            return Some((s("name"), s("version"), s("description")));
        }
    }
    None
}

async fn technic_loader_from_root(
    ctx: &Ctx,
    root: &Path,
    mc: &str,
    current: Option<super::json::PackLoader>,
) -> Result<Option<super::json::PackLoader>> {
    let found = technic_loader(ctx, root, mc).await?;
    Ok(found.or(current))
}

async fn technic_image(m: &Modpack, root: &Path) {
    let Some(logo) = m.logo.as_ref() else { return };
    let Some(url) = logo.url.as_deref().filter(|u| !u.is_empty()) else { return };
    let job = Dl::new(url, root.join("instance.png")).hash(logo.md5.as_deref()).ignore_failures();
    let _ = dl::download_all(vec![job], &Progress::new()).await;
}

async fn legacy_java_fixer(ctx: &Ctx, root: &Path, progress: &Arc<Progress>) -> Option<ModRecord> {
    progress.stage("Installing Legacy Java Fixer");
    let file = super::downloads_dir(&ctx.dirs).join("legacyjavafixer-1.0.jar");
    let job = Dl::new(LEGACY_JAVA_FIXER_URL, &file).hash(Some(LEGACY_JAVA_FIXER_MD5));
    if dl::download_all(vec![job], &Progress::new()).await.is_err() {
        return None;
    }
    fsx::copy_file(&file, &root.join("mods").join("legacyjavafixer-1.0.jar"), true).ok()?;
    let mut r = ModRecord::new("Legacy Java Fixer", "legacyjavafixer-1.0.jar", "mods");
    r.version = "1.0".into();
    r.optional = false;
    r.description = "Fixes issues with newer Java versions on Minecraft 1.6 and below".into();
    Some(r)
}

// ---------------------------------------------------------------------------------------------------------
// the game description

/// Fix the log4j libraries that have the Log4Shell hole: from 2.0-beta9 up to 2.15 the ATLauncher servers hold a
/// patched copy.
pub fn fix_log4j(lib: &mut Library) {
    if !lib.name.starts_with("org.apache.logging.log4j") {
        return;
    }
    let parts: Vec<&str> = lib.name.split(':').collect();
    if parts.len() < 3 {
        return;
    }
    let version_parts: Vec<&str> = parts[2].split('.').collect();
    if let (Some(Ok(major)), minor) =
        (version_parts.first().map(|p| p.parse::<i64>()), version_parts.get(1).and_then(|p| p.parse::<i64>().ok()))
    {
        if major > 2 || (major == 2 && minor.unwrap_or(0) >= 16) {
            return;
        }
    }
    let (name, path, sha1, size): (&str, &str, &str, i64) = match parts[1] {
        "log4j-api" if parts[2].starts_with("2.0-beta9") => (
            "org.apache.logging.log4j:log4j-api:2.0-beta9-fixed",
            "org/apache/logging/log4j/log4j-api/2.0-beta9-fixed/log4j-api-2.0-beta9-fixed.jar",
            "b61eaf2e64d8b0277e188262a8b771bbfa1502b3",
            107347,
        ),
        "log4j-api" => (
            "org.apache.logging.log4j:log4j-api:2.16.0",
            "org/apache/logging/log4j/log4j-api/2.16.0/log4j-api-2.16.0.jar",
            "f821a18687126c2e2f227038f540e7953ad2cc8c",
            301892,
        ),
        "log4j-core" if parts[2].starts_with("2.0-beta9") => (
            "org.apache.logging.log4j:log4j-core:2.0-beta9-fixed",
            "org/apache/logging/log4j/log4j-core/2.0-beta9-fixed/log4j-core-2.0-beta9-fixed.jar",
            "677991ea2d7426f76309a73739cecf609679492c",
            677588,
        ),
        "log4j-core" => (
            "org.apache.logging.log4j:log4j-core:2.16.0",
            "org/apache/logging/log4j/log4j-core/2.16.0/log4j-core-2.16.0.jar",
            "539a445388aee52108700f26d9644989e7916e7c",
            1789565,
        ),
        "log4j-slf4j18-impl" => (
            "org.apache.logging.log4j:log4j-slf4j18-impl:2.16.0",
            "org/apache/logging/log4j/log4j-slf4j18-impl/2.16.0/log4j-slf4j18-impl-2.16.0.jar",
            "0c880a059056df5725f5d8d1035276d9749eba6d",
            21249,
        ),
        _ => return,
    };
    lib.name = name.to_string();
    lib.downloads = Some(LibDownloads {
        artifact: Some(Artifact {
            path: Some(path.to_string()),
            url: format!("{DOWNLOAD_SERVER}/maven/{path}"),
            sha1: Some(sha1.to_string()),
            size: Some(size),
        }),
        classifiers: lib.downloads.as_ref().and_then(|d| d.classifiers.clone()),
    });
}

fn determine_main_class(
    json: &PackJson,
    selected: &[&PackMod],
    loader: Option<&loader::Installed>,
    minecraft_main: &str,
) -> String {
    let mut main: Option<String> = None;
    if let Some(mc) = &json.main_class {
        let wanted = mc.main_class.clone();
        if mc.depends.is_none() && mc.depends_group.is_none() {
            main = wanted;
        } else if let Some(dep) = &mc.depends {
            if selected.iter().any(|m| m.name.eq_ignore_ascii_case(dep)) {
                main = wanted;
            }
        } else if let Some(group) = &mc.depends_group {
            if selected.iter().any(|m| m.group.as_deref().is_some_and(|g| g.eq_ignore_ascii_case(group))) {
                main = wanted;
            }
        }
    }
    // the loader's own main class wins; very old Forge has none and the game's is used
    if let Some(l) = loader {
        if !l.joins_game_jar && !l.merged.main_class.is_empty() {
            main = Some(l.merged.main_class.clone());
        } else {
            main = None;
        }
    }
    main.filter(|m| !m.is_empty()).unwrap_or_else(|| minecraft_main.to_string())
}

/// Put the game arguments into the form the Java edition saves (`arguments.game` as a list) and add the pack's
/// own arguments.
fn apply_arguments(game: &mut VersionJson, json: &PackJson, selected: &[&PackMod]) {
    let mut args = game.arguments.take().unwrap_or_default();
    if let Some(legacy) = game.minecraft_arguments.take() {
        // older Minecraft versions give their arguments as one line; they come first
        let mut game_args: Vec<Arg> =
            legacy.split(' ').filter(|s| !s.is_empty()).map(|s| Arg::Plain(s.to_string())).collect();
        game_args.extend(std::mem::take(&mut args.game));
        args.game = game_args;
    }
    if let Some(extra) = &json.extra_arguments {
        let add = match (&extra.depends, &extra.depends_group) {
            (None, None) => true,
            (Some(dep), _) => selected.iter().any(|m| m.name.eq_ignore_ascii_case(dep)),
            (None, Some(group)) => {
                selected.iter().any(|m| m.group.as_deref().is_some_and(|g| g.eq_ignore_ascii_case(group)))
            }
        };
        if add {
            if let Some(a) = &extra.arguments {
                args.game.extend(a.split(' ').filter(|s| !s.is_empty()).map(|s| Arg::Plain(s.to_string())));
            }
        }
    }
    game.arguments = Some(Arguments { game: args.game, jvm: args.jvm });
}

fn assets_map_to_resources(dirs: &Dirs, game: &VersionJson) -> bool {
    game.asset_index
        .as_ref()
        .and_then(|a| install::read_asset_index(dirs, &a.id).ok())
        .map(|i| i.map_to_resources)
        .unwrap_or(false)
}

/// The libraries the pack brings, as the Java edition lists them in `instance.json` (the MD5 sits in the `sha1`
/// field, the way it always has; checksums are told apart by their length).
pub fn pack_libraries(json: &PackJson, selected: &[&PackMod], is_server: bool) -> Vec<Library> {
    let mut out = Vec::new();
    for l in &json.libraries {
        if is_server && l.server.is_none() {
            continue;
        }
        if let Some(dep) = &l.depends {
            if !selected.iter().any(|m| m.name.eq_ignore_ascii_case(dep)) {
                continue;
            }
        } else if let Some(group) = &l.depends_group {
            if !selected.iter().any(|m| m.group.as_deref().is_some_and(|g| g.eq_ignore_ascii_case(group))) {
                continue;
            }
        }
        out.push(library_of(l));
    }
    out
}

fn library_of(l: &PackLibrary) -> Library {
    let path = l.path.clone().or_else(|| l.server.clone()).unwrap_or_else(|| l.file.clone());
    Library {
        name: l.file.clone(),
        downloads: Some(LibDownloads {
            artifact: Some(Artifact {
                path: Some(path),
                url: format!("{DOWNLOAD_SERVER}/{}", l.url),
                sha1: l.md5.clone(),
                size: Some(l.filesize),
            }),
            classifiers: None,
        }),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------------------------------------
// browser downloads

/// Get a mod the player has to download in a browser into `downloads/`. Returns false when the player skipped it.
async fn fetch_browser_mod(
    ctx: &Ctx,
    ui: Arc<dyn Interaction>,
    site: &Site,
    m: &PackMod,
    n: usize,
    of: usize,
) -> Result<Option<String>> {
    let downloads = site.downloads.clone();
    let user_dir = super::user_downloads_dir(ctx.custom_downloads_path.as_deref());
    let m = m.clone();
    let is_server = site.is_server;
    tokio::task::spawn_blocking(move || -> Result<Option<String>> {
        let name_wanted = downloaded_browser_name(&m, is_server);
        // the name the file has in the downloads folder, once it is there and checked
        let saved = |location: &Path| location.file_name().map(|n| n.to_string_lossy().to_string());
        let (url, hash) = if m.uses_server_file(is_server) {
            (m.server_url.clone().unwrap_or_default(), m.server_md5.clone())
        } else {
            (m.clean_url(), m.md5.clone())
        };
        let mut location = downloads.join(&name_wanted);
        for _attempt in 0..5 {
            // already there from earlier?
            if location.is_file() {
                if m.force {
                    fsx::delete_path(&location);
                } else if hash.as_deref().is_none_or(|h| h.is_empty()) {
                    if std::fs::metadata(&location).map(|x| x.len() > 0).unwrap_or(false) {
                        return Ok(saved(&location));
                    }
                } else if dl::hash_file(&location, hash.as_deref().unwrap_or(""))
                    .is_some_and(|got| got.eq_ignore_ascii_case(hash.as_deref().unwrap_or("")))
                {
                    return Ok(saved(&location));
                } else {
                    fsx::delete_path(&location);
                }
            }
            // a file the browser saved in the user's downloads folder
            let mut watch = vec![downloads.clone()];
            if let Some(u) = &user_dir {
                watch.push(u.clone());
            }
            let find = |watch: &[PathBuf]| -> Option<PathBuf> { find_browser_file(&m, &name_wanted, watch) };
            if let Some(found) = find(&watch) {
                location = if m.file_pattern {
                    downloads.join(found.file_name().unwrap_or_default())
                } else {
                    downloads.join(&name_wanted)
                };
                if found != location {
                    fsx::copy_file(&found, &location, true)?;
                    fsx::delete_path(&found);
                }
            } else {
                let request = BrowserRequest {
                    name: m.name.clone(),
                    file: if m.file_pattern { String::new() } else { name_wanted.clone() },
                    url: url.clone(),
                    watch: watch.clone(),
                    expected_size: m.filesize,
                    n,
                    of,
                };
                loop {
                    match ui.browser_download(&request) {
                        BrowserAnswer::Cancel => bail!("cancelled"),
                        BrowserAnswer::Skip => return Ok(None),
                        BrowserAnswer::Downloaded => {}
                    }
                    if let Some(found) = find(&watch) {
                        location = downloads.join(found.file_name().unwrap_or_default());
                        if found != location {
                            fsx::copy_file(&found, &location, true)?;
                            fsx::delete_path(&found);
                        }
                        break;
                    }
                }
            }
            // check it
            match hash.as_deref().filter(|h| !h.is_empty()) {
                None => return Ok(saved(&location)),
                Some(h) => {
                    if dl::hash_file(&location, h).is_some_and(|got| got.eq_ignore_ascii_case(h)) {
                        return Ok(saved(&location));
                    }
                    fsx::delete_path(&location);
                }
            }
        }
        bail!("Cannot download {}. Aborting install!", name_wanted)
    })
    .await?
}

fn downloaded_browser_name(m: &PackMod, is_server: bool) -> String {
    place::downloaded_name(m, is_server)
}

/// Look for a mod's file in the watched folders; a pattern (`filePattern`) is a regular expression for the whole
/// name, and `filePreference` picks the first or last when several fit.
fn find_browser_file(m: &PackMod, name: &str, watch: &[PathBuf]) -> Option<PathBuf> {
    if m.file_pattern {
        let re = regex::Regex::new(&format!("^(?:{})$", m.file)).ok()?;
        for dir in watch {
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            let mut matches: Vec<PathBuf> = rd
                .flatten()
                .filter(|e| e.path().is_file() && re.is_match(&e.file_name().to_string_lossy()))
                .map(|e| e.path())
                .collect();
            matches.sort();
            if matches.len() == 1 {
                return matches.pop();
            }
            if matches.len() > 1 {
                return if m.file_preference.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("last")) {
                    matches.pop()
                } else {
                    matches.into_iter().next()
                };
            }
        }
        return None;
    }
    let candidates = [name.to_string(), name.replace(' ', "+"), format!("{name}.zip")];
    for dir in watch {
        for c in &candidates {
            let p = dir.join(c);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------------------
// configs

async fn install_configs(
    ctx: &Ctx,
    req: &Request,
    json: &PackJson,
    site: &Site,
    technic_zip: Option<&Path>,
    override_paths: &mut Vec<String>,
    progress: &Arc<Progress>,
) -> Result<()> {
    if json.no_configs {
        return Ok(());
    }
    match &req.source {
        Source::Technic(_) => {
            if let Some(dir) = technic_zip {
                progress.stage("Copying modpack.zip files");
                let (dir, root) = (dir.to_path_buf(), site.root.clone());
                tokio::task::spawn_blocking(move || fsx::copy_dir(&dir, &root, false)).await??;
            }
        }
        Source::Atl(pack) => {
            let Some(cfg) = &json.configs else { return Ok(()) };
            progress.stage("Downloading configs");
            let zip = site.temp.join("Configs.zip");
            let url = pack.configs_zip_url(&req.version.version);
            dl::download_all(vec![Dl::new(&url, &zip).hash(cfg.sha1.as_deref()).size(cfg.filesize)], progress)
                .await
                .context("Failed to download configs for pack!")?;
            if !zip.exists() {
                bail!("Failed to download configs for pack!");
            }
            // an empty file means there is nothing to unpack
            if std::fs::metadata(&zip).map(|m| m.len()).unwrap_or(0) == 0 {
                return Ok(());
            }
            progress.stage("Extracting configs");
            let (zip2, root, scratch) = (zip.clone(), site.root.clone(), site.temp.join("configs"));
            let files = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
                fsx::unzip(&zip2, &root)?;
                fsx::delete_path(&scratch);
                fsx::unzip(&zip2, &scratch)?;
                let files = place::relative_files(&scratch);
                fsx::delete_path(&scratch);
                Ok(files)
            })
            .await??;
            *override_paths = files;
            let _ = std::fs::remove_file(&zip);
        }
    }
    let _ = ctx;
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// instance.json

fn required_memory(root: &Path, default: i64) -> i64 {
    // some packs ship a file that says how much memory they need
    let file = root.join("config").join("memorysettings.json");
    if let Ok(bytes) = std::fs::read(file) {
        if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
            if let Some(n) = v.get("minimumClient").and_then(|m| m.get("minimumClient")).and_then(Value::as_i64) {
                return n;
            }
        }
    }
    default
}

#[allow(clippy::too_many_arguments)]
fn write_instance(
    _ctx: &Ctx,
    req: &Request,
    json: &PackJson,
    game: VersionJson,
    loader: Option<LoaderVersion>,
    records: Vec<ModRecord>,
    mc: &str,
    assets_map: bool,
    override_paths: Vec<String>,
    root: &Path,
) -> Result<Instance> {
    let (uuid, mut launcher) = match req.existing.as_ref().filter(|_| req.is_reinstall) {
        Some(e) => (
            if e.file.uuid.is_empty() { crate::instances::uuid_v4() } else { e.file.uuid.clone() },
            e.file.launcher.clone(),
        ),
        // a new instance starts "never played", the Java edition's defaults
        None => (
            crate::instances::uuid_v4(),
            LauncherBlock { enable_editing_mods: true, last_played: Some(0), num_plays: Some(0), ..Default::default() },
        ),
    };
    let new = !req.is_reinstall;
    launcher.loader_version = loader;
    if new {
        launcher.name = req.name.clone();
    }
    launcher.pack = req.source.pack_name();
    launcher.description = req.source.description();
    launcher.pack_id = req.source.pack_id();
    launcher.external_pack_id = req.source.external_id();
    launcher.version = if json.version.is_empty() { req.version.version.clone() } else { json.version.clone() };
    launcher.enable_curse_forge_integration = json.enable_curse_forge_integration;
    launcher.enable_editing_mods = json.enable_editing_mods;
    launcher.required_memory = required_memory(root, json.memory);
    launcher.required_perm_gen = json.perm_gen;
    launcher.vanilla_instance = false;
    let x = &mut launcher.extras;
    match &json.java {
        Some(j) => {
            x.insert("java".into(), serde_json::to_value(j)?);
        }
        None => {
            x.remove("java");
        }
    }
    x.insert("isDev".into(), Value::Bool(req.version.is_dev));
    x.insert("isPlayable".into(), Value::Bool(true));
    x.insert("assetsMapToResources".into(), Value::Bool(assets_map));
    x.insert("overridePaths".into(), json!(override_paths));
    match &req.version.hash {
        Some(h) if req.version.is_dev => {
            x.insert("hash".into(), Value::String(h.clone()));
        }
        _ => {
            if new {
                x.remove("hash");
            } else if !req.version.is_dev {
                x.remove("hash");
            }
        }
    }
    if let Source::Technic(m) = &req.source {
        x.insert("technicModpack".into(), serde_json::to_value(m)?);
    } else {
        x.remove("technicModpack");
    }
    if new {
        x.entry("checkForUpdates").or_insert(Value::Bool(true));
        x.entry("ignoredUpdates").or_insert(json!([]));
        x.entry("ignoreAllUpdates").or_insert(Value::Bool(false));
        x.entry("quickPlay").or_insert(json!({}));
    }
    // CurseForge's manifest is dropped, as the Java edition does
    x.remove("curseForgeManifest");
    let mut file = InstanceFile { uuid, launcher, version: game };
    file.version.id = mc.to_string();
    let folder = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut instance = Instance { folder, mod_count: 0, root: root.to_path_buf(), file };
    instance.set_mod_records(&records);
    instance.file.launcher.extras.insert("mods".into(), serde_json::to_value(&records)?);
    write_json_atomic(&instance.instance_file(), &instance.file)?;
    instance.mod_count = instance.enabled_mod_count();
    Ok(instance)
}

// ---------------------------------------------------------------------------------------------------------
// what a window needs around an install

/// An instance that can be reinstalled or updated through this module, with the source that goes with it.
#[derive(Clone, Debug)]
pub struct Reinstall {
    pub source: Source,
    pub version: VersionInfo,
}

/// Work out the pack behind an ATLauncher-list instance (for the Update and Reinstall buttons). `version` is the
/// version to go to; `None` reinstalls the installed one.
pub fn reinstall_atl(
    list: &atlauncher::PackList,
    instance: &Instance,
    version: Option<&PackVersion>,
) -> Option<Reinstall> {
    let pack = atlauncher::pack_of(list, instance)?;
    let v = match version {
        Some(v) => v.clone(),
        None => atlauncher::installed_version(list, instance).cloned().or_else(|| {
            // the version has left the list (a removed pack version): reinstall it by name all the same
            Some(PackVersion {
                version: instance.file.launcher.version.clone(),
                minecraft: instance.mc_version().to_string(),
                ..Default::default()
            })
        })?,
    };
    Some(Reinstall { source: Source::Atl(pack.clone()), version: VersionInfo::from(&v) })
}

/// The same for a Technic instance: `modpack` is the pack as Technic describes it now (see
/// [`technic::modpack`]) and `build` the version to go to (`None` keeps the installed one).
pub fn reinstall_technic(instance: &Instance, modpack: Modpack, build: Option<&str>) -> Reinstall {
    let version = build.map(str::to_string).unwrap_or_else(|| instance.file.launcher.version.clone());
    Reinstall { source: Source::Technic(modpack), version: VersionInfo { version, hash: None, is_dev: false } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::json::ModKind;

    #[test]
    fn log4shell_libraries_are_swapped_for_the_patched_ones() {
        let mut lib: Library = serde_json::from_str(
            r#"{"name":"org.apache.logging.log4j:log4j-core:2.0-beta9","downloads":{"artifact":{"path":"org/apache/logging/log4j/log4j-core/2.0-beta9/log4j-core-2.0-beta9.jar","url":"https://libraries.minecraft.net/x","sha1":"aa","size":1}}}"#,
        )
        .unwrap();
        fix_log4j(&mut lib);
        assert_eq!(lib.name, "org.apache.logging.log4j:log4j-core:2.0-beta9-fixed");
        let a = lib.artifact().unwrap();
        assert_eq!(a.sha1.as_deref(), Some("677991ea2d7426f76309a73739cecf609679492c"));
        assert_eq!(
            a.url,
            "https://download.nodecdn.net/containers/atl/maven/org/apache/logging/log4j/log4j-core/2.0-beta9-fixed/log4j-core-2.0-beta9-fixed.jar"
        );
        let mut api: Library = serde_json::from_str(r#"{"name":"org.apache.logging.log4j:log4j-api:2.14.1","downloads":{"artifact":{"path":"p","url":"u","sha1":"s","size":1}}}"#).unwrap();
        fix_log4j(&mut api);
        assert_eq!(api.name, "org.apache.logging.log4j:log4j-api:2.16.0");
        // new enough, or not log4j: untouched
        let mut ok: Library = serde_json::from_str(r#"{"name":"org.apache.logging.log4j:log4j-core:2.19.0","downloads":{"artifact":{"path":"p","url":"u","sha1":"s","size":1}}}"#).unwrap();
        fix_log4j(&mut ok);
        assert_eq!(ok.name, "org.apache.logging.log4j:log4j-core:2.19.0");
        let mut other: Library = serde_json::from_str(r#"{"name":"com.google.guava:guava:21.0"}"#).unwrap();
        fix_log4j(&mut other);
        assert_eq!(other.name, "com.google.guava:guava:21.0");
    }

    #[test]
    fn pack_libraries_follow_the_java_layout() {
        let json = PackJson::parse(
            br#"{"libraries":[
              {"url":"packs/X/files/libraries/launchwrapper-1.8.jar","file":"launchwrapper-1.8.jar","server":"net/minecraft/launchwrapper/1.8/launchwrapper-1.8.jar","md5":"460ffeedae268dc70c8210ce179fa44a","download":"server","filesize":28927,"force":false},
              {"url":"packs/X/files/libraries/only-with-mod.jar","file":"only-with-mod.jar","md5":"d21c2a06a4e6b175aa01e328f38a1182","filesize":1,"depends":"Optional Mod"},
              {"url":"packs/X/files/libraries/pathy.jar","file":"pathy.jar","path":"custom/pathy.jar","filesize":2}
            ]}"#,
        )
        .unwrap();
        let libs = pack_libraries(&json, &[], false);
        assert_eq!(libs.len(), 2, "a library that depends on a mod that isn't chosen is left out");
        let a = libs[0].artifact().unwrap();
        assert_eq!(a.path.as_deref(), Some("net/minecraft/launchwrapper/1.8/launchwrapper-1.8.jar"));
        assert_eq!(a.url, "https://download.nodecdn.net/containers/atl/packs/X/files/libraries/launchwrapper-1.8.jar");
        assert_eq!(
            a.sha1.as_deref(),
            Some("460ffeedae268dc70c8210ce179fa44a"),
            "the MD5 is saved in the sha1 field like the Java edition"
        );
        assert_eq!(libs[1].artifact().unwrap().path.as_deref(), Some("custom/pathy.jar"));
        // servers only get libraries with a server path
        assert_eq!(pack_libraries(&json, &[], true).len(), 1);
        let dep = PackMod { name: "optional mod".into(), ..Default::default() };
        assert_eq!(pack_libraries(&json, &[&dep], false).len(), 3);
    }

    #[test]
    fn main_class_and_arguments_follow_the_pack() {
        let json = PackJson::parse(br#"{"mainClass":{"mainClass":"net.minecraft.launchwrapper.Launch"},"extraArguments":{"arguments":"--tweakClass=cpw.mods.fml.common.launcher.FMLTweaker"}}"#).unwrap();
        assert_eq!(
            determine_main_class(&json, &[], None, "net.minecraft.client.main.Main"),
            "net.minecraft.launchwrapper.Launch"
        );
        let none = PackJson::default();
        assert_eq!(
            determine_main_class(&none, &[], None, "net.minecraft.client.main.Main"),
            "net.minecraft.client.main.Main"
        );
        // a loader's main class beats the pack's
        let l = loader::Installed {
            merged: VersionJson {
                main_class: "net.fabricmc.loader.impl.launch.knot.KnotClient".into(),
                ..Default::default()
            },
            info: LoaderVersion::default(),
            joins_game_jar: false,
        };
        assert_eq!(determine_main_class(&json, &[], Some(&l), "x"), "net.fabricmc.loader.impl.launch.knot.KnotClient");
        let mut game = VersionJson {
            minecraft_arguments: Some("--username ${auth_player_name} --version ${version_name}".into()),
            ..Default::default()
        };
        apply_arguments(&mut game, &json, &[]);
        assert!(game.minecraft_arguments.is_none());
        let g = &game.arguments.as_ref().unwrap().game;
        assert_eq!(g.len(), 5);
        assert!(matches!(&g[4], Arg::Plain(s) if s == "--tweakClass=cpw.mods.fml.common.launcher.FMLTweaker"));
        // an extra argument that depends on a mod is only added with that mod
        let dep = PackJson::parse(br#"{"extraArguments":{"arguments":"--x","depends":"Some Mod"}}"#).unwrap();
        let mut g2 = VersionJson::default();
        apply_arguments(&mut g2, &dep, &[]);
        assert!(g2.arguments.unwrap().game.is_empty());
        let m = PackMod { name: "some mod".into(), ..Default::default() };
        let mut g3 = VersionJson::default();
        apply_arguments(&mut g3, &dep, &[&m]);
        assert_eq!(g3.arguments.unwrap().game.len(), 1);
    }

    #[test]
    fn a_record_for_a_mod_has_the_java_shape() {
        let json = PackJson::parse(br#"{"colours":{"Green":"cef4cb"},"caseAllFiles":"lower"}"#).unwrap();
        let m = PackMod {
            name: "Cardinal".into(),
            version: "7.3.0".into(),
            file: "Cardinal-API.jar".into(),
            kind: Some(ModKind::Mods),
            optional: true,
            colour: Some("Green".into()),
            curse_forge_project_id: Some(318449),
            curse_forge_file_id: Some(7315135),
            description: Some("d".into()),
            ..Default::default()
        };
        let r = record_for(&json, &m);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["name"], "Cardinal");
        assert_eq!(v["file"], "cardinal-api.jar", "case conversion changes only the part before the extension");
        assert_eq!(v["type"], "mods");
        assert_eq!(v["optional"], true);
        assert_eq!(v["wasSelected"], true);
        assert_eq!(v["userAdded"], false);
        assert_eq!(v["curseForgeProjectId"], 318449);
        assert_eq!(v["curseForgeFileId"], 7315135);
        assert_eq!(v["colour"]["value"], ((0xFFu32 << 24) | 0xcef4cb) as i32);
        assert!(v.get("path").is_none());
    }

    #[test]
    fn recorded_files_are_found_where_the_java_edition_looks() {
        let root = Path::new("/i");
        let mut r = ModRecord::new("A", "a.jar", "dependency");
        assert_eq!(record_path(root, &r, "1.7.10", false).unwrap(), Path::new("/i/mods/1.7.10/a.jar"));
        assert_eq!(record_path(root, &r, "1.7.10", true).unwrap(), Path::new("/i/disabledmods/a.jar"));
        r.kind = "forge".into();
        assert_eq!(record_path(root, &r, "x", false).unwrap(), Path::new("/i/jarmods/a.jar"));
        r.path = Some("mods/special".into());
        assert_eq!(record_path(root, &r, "x", false).unwrap(), Path::new("/i/mods/special/a.jar"));
    }

    #[test]
    fn browser_files_are_found_by_name_or_pattern() {
        let base = std::env::temp_dir().join(format!("bf-browser-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("My+Mod.zip"), "x").unwrap();
        std::fs::write(base.join("pattern-1.0.jar"), "x").unwrap();
        std::fs::write(base.join("pattern-2.0.jar"), "x").unwrap();
        let m = PackMod { file: "My Mod.zip".into(), ..Default::default() };
        // browsers turn spaces into plus signs
        assert_eq!(find_browser_file(&m, "My Mod.zip", std::slice::from_ref(&base)).unwrap(), base.join("My+Mod.zip"));
        let mut p = PackMod {
            file: "pattern-.*\\.jar".into(),
            file_pattern: true,
            file_preference: Some("last".into()),
            ..Default::default()
        };
        assert_eq!(find_browser_file(&p, "", std::slice::from_ref(&base)).unwrap(), base.join("pattern-2.0.jar"));
        p.file_preference = Some("first".into());
        assert_eq!(find_browser_file(&p, "", std::slice::from_ref(&base)).unwrap(), base.join("pattern-1.0.jar"));
        assert!(
            find_browser_file(
                &PackMod { file: "nope.jar".into(), ..Default::default() },
                "nope.jar",
                std::slice::from_ref(&base)
            )
            .is_none()
        );
        let _ = std::fs::remove_dir_all(base);
    }
}
