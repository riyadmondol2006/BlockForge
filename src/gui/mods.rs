//! The Mods tab of an instance: a full manager for its mods, resource packs and shader packs. Each item shows its
//! picture, name, version and description, can be switched on and off, updated, rolled back, inspected and removed,
//! and the whole list can be searched, checked for updates and updated in one go. New files dropped on the window
//! are recorded, and "Add" opens Discover on the right kind.
//!
//! Everything slow (the network, reading jars, copying) runs in the background and reports back with a
//! [`ModsMsg`]; the screen only reads the cached lists.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use egui::{Align, CornerRadius, Layout, RichText, ScrollArea, Stroke, Ui};

use super::icons::Icon;
use super::instance_menu::{ghost_button, secondary_menu, split_or_stack};
use super::theme::*;
use super::widgets::*;
use super::{App, InstTab, Msg, Page};
use crate::instances::{self, Instance};
use crate::modrinth::Kind;
use crate::mods::api::MrVersion;
use crate::mods::files::{self, ModEntry, ModKey, ScanScope, SyncReport};
use crate::mods::metadata::{self, ContentKind, ModInfo};
use crate::mods::scan::ScanReport;
use crate::mods::updates::{self, UpdateOffer};
use crate::mods::{ModContext, ModSettings, install, performance, versions};
use crate::snapshots;

/// How tall a row of the list is, and how big the picture in it.
const ROW_H: f32 = 56.0;
const ICON: f32 = 36.0;

/// Between the parts of the grey line after a mod's name (its version, where it came from).
const META_SEP: &str = " \u{00B7} ";

/// Which items the list shows.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Show {
    #[default]
    All,
    On,
    Off,
    Updates,
}

/// What was read from inside a jar, kept for the list and the details.
#[derive(Clone)]
pub struct Details {
    pub info: ModInfo,
    /// Why this might not work in the instance, if it looks that way.
    pub problem: Option<String>,
}

/// News from background work for the Mods tab.
pub enum ModsMsg {
    /// The folders were compared with the mod list.
    Synced {
        folder: String,
        result: Result<SyncReport, String>,
    },
    /// Pictures and metadata read from jars.
    Details {
        folder: String,
        batch: Vec<(ModKey, Details, Option<egui::ColorImage>)>,
    },
    Updates {
        folder: String,
        result: Result<Vec<UpdateOffer>, String>,
    },
    /// A quick change finished (switching on or off, removing, refreshing).
    Done {
        folder: String,
        result: Result<String, String>,
    },
    Versions {
        folder: String,
        key: ModKey,
        result: Result<Vec<MrVersion>, String>,
    },
    Scan {
        folder: String,
        report: ScanReport,
    },
}

struct VersionsDialog {
    key: ModKey,
    name: String,
    /// `None` while loading.
    list: Option<Result<Vec<MrVersion>, String>>,
    installed_id: Option<String>,
}

#[derive(Default)]
pub struct ModsState {
    folder: String,
    signature: Option<SystemTime>,
    kind: Option<ContentKind>,
    entries: HashMap<ContentKind, Vec<ModEntry>>,
    textures: HashMap<ModKey, egui::TextureHandle>,
    details: HashMap<ModKey, Details>,
    asked_details: HashSet<ModKey>,
    updates: HashMap<ModKey, UpdateOffer>,
    checking: bool,
    checked: bool,
    recheck: bool,
    synced: bool,
    filter: String,
    show: Show,
    select_mode: bool,
    selected: HashSet<ModKey>,
    expanded: Option<ModKey>,
    busy: Option<String>,
    /// Quick changes still running for this instance; `busy` clears when the last one ends.
    jobs: usize,
    /// What the instance is (loader, Minecraft version) and what shaders need, worked out when the instance changes
    /// rather than on every frame (both read files).
    target: Option<crate::mods::InstanceTarget>,
    shader_setup: Option<versions::ShaderSetup>,
    confirm_remove: Vec<ModKey>,
    confirm_restore: Option<PathBuf>,
    confirm_pack: bool,
    versions: Option<VersionsDialog>,
    scan: Option<ScanReport>,
    scanning: bool,
    last_report: Option<SyncReport>,
    /// Why looking through the folders failed, shown in place of the list until it works.
    sync_error: Option<String>,
}

impl ModsState {
    fn kind(&self) -> ContentKind {
        self.kind.unwrap_or(ContentKind::Mods)
    }

    fn reset_for(&mut self, folder: &str) {
        let kind = self.kind;
        *self = ModsState::default();
        self.kind = kind;
        self.folder = folder.to_string();
    }
}

/// What the player asked for, collected while drawing and carried out afterwards.
enum Act {
    Kind(ContentKind),
    Toggle(ModKey, bool),
    Remove(Vec<ModKey>),
    Update(ModKey),
    UpdateAll,
    CheckUpdates,
    CheckOne(ModKey),
    Versions(ModKey),
    Reinstall(ModKey),
    Refresh(Vec<ModKey>),
    Rescan,
    Details(ModKey),
    ShowInFolder(ModKey),
    Url(String),
    Add,
    OpenFolder,
    Scan,
    Snapshot,
    Restore(PathBuf),
    Pack,
    ShaderLoader,
    EnableAll(Vec<ModKey>, bool),
    Select(ModKey, bool),
    ClearFilter,
    /// Open the mod loader dialog of the instance.
    Loader,
    DismissScan,
}

/// The platforms besides Modrinth that can offer updates. The CurseForge module adds its source here.
fn external_sources() -> Vec<Box<dyn updates::ExternalSource>> {
    Vec::new()
}

fn discover_kind(kind: ContentKind) -> Kind {
    match kind {
        ContentKind::Mods => Kind::Mod,
        ContentKind::ResourcePacks => Kind::ResourcePack,
        ContentKind::Shaders => Kind::Shader,
    }
}

/// Reads the pictures and details of some jars. This opens each file, so it is meant for a background thread.
pub fn read_details(
    items: Vec<(ModKey, PathBuf)>,
    loader_key: Option<&str>,
    mc: &str,
) -> Vec<(ModKey, Details, Option<egui::ColorImage>)> {
    items
        .into_iter()
        .map(|(key, path)| {
            let d = metadata::detect(&path);
            let problem = if key.kind == "mods" { d.problem_for(loader_key, mc) } else { None };
            let image = metadata::read_icon(&path, &d.info).and_then(|bytes| thumbnail(&bytes));
            (key, Details { info: d.info, problem }, image)
        })
        .collect()
}

/// A picture shrunk to what a list row shows, so hundreds of mods don't hold hundreds of big textures.
fn thumbnail(bytes: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?.thumbnail(72, 72).to_rgba8();
    // a wide or tall logo is centred on a square, so the row shows it undistorted
    let side = img.width().max(img.height());
    let mut square = image::RgbaImage::new(side, side);
    image::imageops::overlay(
        &mut square,
        &img,
        i64::from((side - img.width()) / 2),
        i64::from((side - img.height()) / 2),
    );
    Some(egui::ColorImage::from_rgba_unmultiplied([side as usize, side as usize], square.as_raw()))
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// A long list in a scroll area where only the rows in view (and a little around them) are laid out; the others
/// take the height they had when last drawn, or `guess`. Hundreds of rows then cost what a screenful costs.
pub(super) fn lazy_list<T>(ui: &mut Ui, id: egui::Id, items: &[T], guess: f32, mut row: impl FnMut(&mut Ui, &T)) {
    let mut heights: Vec<f32> = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    heights.resize(items.len(), guess);
    ScrollArea::vertical().id_salt(id).auto_shrink(false).show_viewport(ui, |ui, viewport| {
        let origin = ui.max_rect().top();
        let margin = 120.0;
        for (i, item) in items.iter().enumerate() {
            let top = ui.cursor().min.y - origin;
            if top + heights[i] < viewport.min.y - margin || top > viewport.max.y + margin {
                ui.add_space(heights[i]);
                continue;
            }
            row(ui, item);
            // what the row took, gaps included, so skipping it later moves the same distance
            heights[i] = ui.cursor().min.y - origin - top;
        }
    });
    ui.data_mut(|d| d.insert_temp(id, heights));
}

/// One lock per instance. Changes read `instance.json`, change the mod list and save it; two at once (switching
/// two mods off quickly, or an update while one is removed) would each save their own copy and lose the other's.
pub(super) fn instance_lock(root: &Path) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(root.to_path_buf()).or_default().clone()
}

fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

impl App {
    // -----------------------------------------------------------------------------------------------------
    // Background work

    /// Runs a change to one instance in the background with a fresh copy of it, then reloads the screen.
    fn mods_job<F, Fut>(&mut self, folder: &str, label: &str, work: F)
    where
        F: FnOnce(Instance, ModContext) -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<String>> + Send + 'static,
    {
        let Some(root) = self.instance(folder).map(|i| i.root.clone()) else { return };
        self.mods.busy = Some(label.to_string());
        self.mods.jobs += 1;
        let (dirs, settings, n, folder) =
            (self.dirs.clone(), self.settings.clone(), self.notifier(), folder.to_string());
        let lock = instance_lock(&root);
        self.rt.spawn(async move {
            let result = async {
                let _turn = lock.lock().await;
                let inst = instances::load_one(&root)
                    .ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
                let ctx = ModContext::build(&dirs, &inst, &settings).await;
                work(inst, ctx).await
            }
            .await
            .map_err(|e| format!("{e:#}"));
            n.send(Msg::Mods(ModsMsg::Done { folder, result }));
        });
    }

    /// Like [`Self::mods_job`] but shown in the sidebar with a progress bar, for downloads.
    fn mods_task<F, Fut>(&mut self, folder: &str, label: String, work: F)
    where
        F: FnOnce(Instance, ModContext, std::sync::Arc<crate::install::Progress>) -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<String>> + Send + 'static,
    {
        let Some(root) = self.instance(folder).map(|i| i.root.clone()) else { return };
        let (dirs, settings, lock) = (self.dirs.clone(), self.settings.clone(), instance_lock(&root));
        self.start_task(label, move |progress, _| async move {
            let _turn = lock.lock().await;
            let inst =
                instances::load_one(&root).ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
            let ctx = ModContext::build(&dirs, &inst, &settings).await;
            work(inst, ctx, progress).await
        });
        self.mods.recheck = true;
    }

    /// Compares the folders with the mod list, then reads what is in the jars and looks for updates.
    fn mods_sync(&mut self, inst: &Instance) {
        let (root, settings, n, folder) =
            (inst.root.clone(), ModSettings::from_settings(&self.settings), self.notifier(), inst.folder.clone());
        self.mods.synced = true;
        let lock = instance_lock(&root);
        self.rt.spawn(async move {
            let result = async {
                let _turn = lock.lock().await;
                let mut inst = instances::load_one(&root)
                    .ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
                files::sync(&mut inst, &settings, ScanScope::Full).await
            }
            .await
            .map_err(|e| format!("{e:#}"));
            n.send(Msg::Mods(ModsMsg::Synced { folder, result }));
        });
    }

    fn mods_load_details(&mut self, inst: &Instance) {
        let loader = inst.loader().and_then(|(k, _)| versions::loader_key(k));
        let mc = inst.mc_version().to_string();
        let mut wanted: Vec<(ModKey, PathBuf)> = Vec::new();
        for list in self.mods.entries.values() {
            for e in list {
                if let Some(p) = e.path.clone().filter(|_| e.exists) {
                    if self.mods.asked_details.insert(e.key()) {
                        wanted.push((e.key(), p));
                    }
                }
            }
        }
        if wanted.is_empty() {
            return;
        }
        let (n, folder) = (self.notifier(), inst.folder.clone());
        self.rt.spawn_blocking(move || {
            for chunk in wanted.chunks(8) {
                let batch = read_details(chunk.to_vec(), loader, &mc);
                n.send(Msg::Mods(ModsMsg::Details { folder: folder.clone(), batch }));
            }
        });
    }

    fn mods_check_updates(&mut self, inst: &Instance) {
        if self.mods.checking {
            return;
        }
        self.mods.checking = true;
        let (root, dirs, settings, n, folder) =
            (inst.root.clone(), self.dirs.clone(), self.settings.clone(), self.notifier(), inst.folder.clone());
        self.rt.spawn(async move {
            let result = async {
                let inst = instances::load_one(&root)
                    .ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
                let ctx = ModContext::build(&dirs, &inst, &settings).await;
                let sources = external_sources();
                let refs: Vec<&dyn updates::ExternalSource> = sources.iter().map(|b| b.as_ref()).collect();
                let mut all = Vec::new();
                for kind in ContentKind::ALL {
                    all.extend(updates::find_updates_with(&inst, &ctx, kind, &refs).await?);
                }
                Ok(all)
            }
            .await
            .map_err(|e: anyhow::Error| format!("{e:#}"));
            n.send(Msg::Mods(ModsMsg::Updates { folder, result }));
        });
    }

    /// Brings the cached lists up to date when the instance, or its `instance.json`, changed.
    fn mods_prepare(&mut self, inst: &Instance) {
        let first_time = self.mods.folder != inst.folder;
        if first_time {
            self.mods.reset_for(&inst.folder);
        }
        let stamp = modified(&inst.instance_file());
        if first_time || stamp != self.mods.signature {
            self.mods.signature = stamp;
            let target = crate::mods::InstanceTarget::of(inst, &crate::mods::RemoteConfig::load(&self.dirs));
            self.mods.shader_setup = Some(install::shader_setup(inst, &target));
            self.mods.target = Some(target);
            self.mods.entries = ContentKind::ALL.iter().map(|k| (*k, files::entries(inst, *k))).collect();
            let keys: HashSet<ModKey> = self.mods.entries.values().flatten().map(|e| e.key()).collect();
            self.mods.selected.retain(|k| keys.contains(k));
            self.mods.updates.retain(|k, _| keys.contains(k));
            // a file replaced by an update has a new name, so its picture is read again
            self.mods.textures.retain(|k, _| keys.contains(k));
            self.mods.details.retain(|k, _| keys.contains(k));
            self.mods.asked_details.retain(|k| keys.contains(k));
            self.mods_load_details(inst);
            if self.mods.checked && self.mods.recheck {
                self.mods.recheck = false;
                self.mods_check_updates(inst);
            }
        }
        if !self.mods.synced {
            self.mods_sync(inst);
        }
    }

    pub(super) fn mods_handle(&mut self, msg: ModsMsg) {
        match msg {
            ModsMsg::Synced { folder, result } => {
                if folder != self.mods.folder {
                    return;
                }
                match result {
                    Ok(report) => {
                        if report.changed() {
                            if !report.added.is_empty() {
                                self.toast(format!("Found {} new file(s) in the folders", report.added.len()));
                            }
                            self.mods.signature = None;
                            self.reload_instances();
                        }
                        self.mods.last_report = Some(report);
                        self.mods.sync_error = None;
                    }
                    Err(e) => {
                        self.toast_err(format!("Couldn't look through the mod folders: {e}"));
                        self.mods.sync_error = Some(e);
                    }
                }
                if !self.mods.checked {
                    self.mods.checked = true;
                    if let Some(inst) = self.instance(&folder).cloned() {
                        self.mods_check_updates(&inst);
                    }
                }
            }
            ModsMsg::Details { folder, batch } => {
                if folder != self.mods.folder {
                    return;
                }
                for (key, details, image) in batch {
                    if let Some(img) = image {
                        let name = format!("modicon/{folder}/{}/{}", key.kind, key.file);
                        self.mods
                            .textures
                            .insert(key.clone(), self.ctx.load_texture(name, img, egui::TextureOptions::LINEAR));
                    }
                    self.mods.details.insert(key, details);
                }
            }
            ModsMsg::Updates { folder, result } => {
                if folder != self.mods.folder {
                    return;
                }
                self.mods.checking = false;
                match result {
                    Ok(list) => self.mods.updates = list.into_iter().map(|o| (o.key.clone(), o)).collect(),
                    Err(e) => self.toast_err(format!("Couldn't check for updates: {e}")),
                }
            }
            ModsMsg::Done { folder, result } => {
                if folder == self.mods.folder {
                    self.mods.jobs = self.mods.jobs.saturating_sub(1);
                    if self.mods.jobs == 0 {
                        self.mods.busy = None;
                    }
                }
                match result {
                    Ok(m) if !m.is_empty() => self.toast(m),
                    Ok(_) => {}
                    Err(e) => self.toast_err(e),
                }
                self.mods.signature = None;
                if folder == self.mods.folder {
                    self.mods.recheck = self.mods.checked;
                }
                self.reload_instances();
            }
            ModsMsg::Versions { folder, key, result } => {
                if folder != self.mods.folder {
                    return;
                }
                if let Some(d) = self.mods.versions.as_mut().filter(|d| d.key == key) {
                    d.list = Some(result);
                }
            }
            ModsMsg::Scan { folder, report } => {
                if folder != self.mods.folder {
                    return;
                }
                self.mods.scanning = false;
                self.mods.scan = Some(report);
            }
        }
    }

    /// A file dropped on the window while the Mods tab is open is added to the instance. Returns whether it was
    /// taken, so the caller knows not to complain about it.
    pub(super) fn mods_accept_drop(&mut self, path: &Path) -> bool {
        if self.page != Page::Instance || self.tab != InstTab::Mods {
            return false;
        }
        let Some(folder) = self.selected.clone() else { return false };
        let detected = metadata::detect(path);
        // a modpack becomes an instance of its own: the window's usual drop handling imports it
        if detected.kind.is_modpack() {
            return false;
        }
        let record_type = match detected.kind {
            metadata::FileKind::DataPack => "datapack",
            k => match k.content_kind() {
                Some(c) => c.record_type(),
                None => {
                    self.toast_err(format!("{} is not a mod, resource pack or shader pack", detected.file_name));
                    return true;
                }
            },
        };
        let source = path.to_path_buf();
        let settings = ModSettings::from_settings(&self.settings);
        let name = detected.display_name().to_string();
        self.mods_job(&folder, "Adding", move |mut inst, ctx| async move {
            if let Some(problem) = detected.problem_for(ctx.target.loader_key, &ctx.target.minecraft_version) {
                // still added: the player decides, the list shows the warning
                let _ = problem;
            }
            match files::add_file_identified(&mut inst, &source, record_type, &settings).await? {
                files::AddOutcome::Added(_) => Ok(format!("Added {name}")),
                files::AddOutcome::AlreadyThere => Ok(format!("{name} is already in this instance")),
            }
        });
        true
    }

    // -----------------------------------------------------------------------------------------------------
    // Carrying out what was asked

    fn mods_apply(&mut self, inst: &Instance, act: Act) {
        let folder = inst.folder.clone();
        let settings = ModSettings::from_settings(&self.settings);
        match act {
            Act::Kind(k) => {
                self.mods.kind = Some(k);
                self.mods.expanded = None;
                self.mods.selected.clear();
            }
            Act::Toggle(key, on) => {
                self.mods_job(&folder, "Switching", move |mut inst, _| async move {
                    tokio::task::spawn_blocking(move || files::set_enabled(&mut inst, &key, on).map(|_| String::new()))
                        .await?
                });
            }
            Act::EnableAll(keys, on) => {
                self.mods_job(&folder, "Switching", move |mut inst, _| async move {
                    tokio::task::spawn_blocking(move || {
                        let mut failed = Vec::new();
                        for k in &keys {
                            if let Err(e) = files::set_enabled(&mut inst, k, on) {
                                failed.push(format!("{}: {e:#}", k.file));
                            }
                        }
                        if failed.is_empty() { Ok(String::new()) } else { Err(anyhow::anyhow!(failed.join("; "))) }
                    })
                    .await?
                });
            }
            Act::ClearFilter => {
                self.mods.filter.clear();
                self.mods.show = Show::All;
            }
            Act::Loader => self.apply(super::instance_menu::Act::ChangeLoader(folder)),
            Act::DismissScan => self.mods.scan = None,
            Act::Select(key, on) => {
                if on {
                    self.mods.selected.insert(key);
                } else {
                    self.mods.selected.remove(&key);
                }
            }
            Act::Remove(keys) => self.mods.confirm_remove = keys,
            Act::Update(key) => {
                if let Some(offer) = self.mods.updates.get(&key).cloned() {
                    let label = format!("Updating {}", offer.name);
                    self.mods_task(&folder, label, move |mut inst, ctx, progress| async move {
                        let sources = external_sources();
                        let refs: Vec<&dyn updates::ExternalSource> = sources.iter().map(|b| b.as_ref()).collect();
                        updates::apply_update_with(&mut inst, &ctx, &offer, &progress, &refs).await?;
                        Ok(format!("Updated {} to {}", offer.name, offer.newer))
                    });
                }
            }
            Act::UpdateAll => {
                let kind = self.mods.kind();
                let offers: Vec<UpdateOffer> = self.mods.updates.values().filter(|o| o.kind == kind).cloned().collect();
                if !offers.is_empty() {
                    let what = if offers.len() == 1 { "item".to_string() } else { kind.label().to_lowercase() };
                    self.mods_task(
                        &folder,
                        format!("Updating {} {what}", offers.len()),
                        move |mut inst, ctx, progress| async move {
                            let sources = external_sources();
                            let refs: Vec<&dyn updates::ExternalSource> = sources.iter().map(|b| b.as_ref()).collect();
                            let report = updates::update_all(&mut inst, &ctx, &offers, &progress, &refs).await;
                            if report.failed.is_empty() {
                                Ok(format!("Updated {} item(s)", report.updated.len()))
                            } else {
                                let names: Vec<String> =
                                    report.failed.iter().map(|(n, e)| format!("{n} ({e})")).collect();
                                Err(anyhow::anyhow!(
                                    "Updated {}, but couldn't update: {}",
                                    report.updated.len(),
                                    names.join(", ")
                                ))
                            }
                        },
                    );
                }
            }
            Act::CheckUpdates => {
                self.mods_check_updates(inst);
                self.mods.checked = true;
            }
            Act::CheckOne(key) => {
                let name =
                    self.mods.entries.values().flatten().find(|e| e.key() == key).map(|e| e.name()).unwrap_or_default();
                self.mods_job(&folder, "Checking", move |inst, ctx| async move {
                    Ok(match updates::check_single(&inst, &ctx, &key).await? {
                        Some(v) => {
                            format!("{name}: a newer version is available, {}. Open Versions to install it.", v.label())
                        }
                        None => format!("{name} is up to date"),
                    })
                });
            }
            Act::Versions(key) => {
                let name =
                    self.mods.entries.values().flatten().find(|e| e.key() == key).map(|e| e.name()).unwrap_or_default();
                let installed_id = inst
                    .mod_records()
                    .iter()
                    .find(|m| ModKey::of(m) == key)
                    .and_then(|m| m.modrinth_version_id().map(str::to_string));
                self.mods.versions = Some(VersionsDialog { key: key.clone(), name, list: None, installed_id });
                let (root, dirs, s, n) = (inst.root.clone(), self.dirs.clone(), self.settings.clone(), self.notifier());
                self.rt.spawn(async move {
                    let result = async {
                        let inst = instances::load_one(&root)
                            .ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
                        let ctx = ModContext::build(&dirs, &inst, &s).await;
                        updates::available_versions(&inst, &ctx, &key).await
                    }
                    .await
                    .map_err(|e| format!("{e:#}"));
                    n.send(Msg::Mods(ModsMsg::Versions { folder, key, result }));
                });
            }
            Act::Reinstall(key) => {
                self.mods_task(
                    &folder,
                    format!("Reinstalling {}", key.file),
                    move |mut inst, ctx, progress| async move {
                        updates::reinstall(&mut inst, &ctx, &key, &progress).await?;
                        Ok(format!("Reinstalled {}", key.file))
                    },
                );
            }
            Act::Refresh(keys) => {
                self.mods_job(&folder, "Refreshing", move |mut inst, ctx| async move {
                    let n = crate::mods::identify::refresh_metadata(&mut inst, &keys, &ctx.settings).await?;
                    Ok(format!("{n} of {} recognised on Modrinth", keys.len()))
                });
            }
            Act::Rescan => {
                self.mods.synced = false;
                self.mods.sync_error = None;
            }
            Act::Details(key) => {
                self.mods.expanded = if self.mods.expanded.as_ref() == Some(&key) { None } else { Some(key) };
            }
            Act::ShowInFolder(key) => {
                if let Some(p) =
                    self.mods.entries.values().flatten().find(|e| e.key() == key).and_then(|e| e.path.clone())
                {
                    let _ = open::that(p.parent().unwrap_or(&inst.root));
                }
            }
            Act::Url(u) => {
                let _ = open::that(u);
            }
            Act::Add => {
                self.discover.kind = discover_kind(self.mods.kind());
                self.discover.target = Some(folder);
                self.page = Page::Discover;
                self.discover_search(true);
            }
            Act::OpenFolder => {
                let dir = inst.root.join(self.mods.kind().folder());
                let _ = std::fs::create_dir_all(&dir);
                let _ = open::that(dir);
            }
            Act::Scan => {
                self.mods.scanning = true;
                let (dirs, inst2, n) = (self.dirs.clone(), inst.clone(), self.notifier());
                // a manual scan always runs, whatever scanModsOnLaunch says
                let mut s = settings;
                s.scan_mods_on_launch = true;
                self.rt.spawn_blocking(move || {
                    let report = crate::mods::scan::scan_instance(&dirs, &inst2, &s);
                    n.send(Msg::Mods(ModsMsg::Scan { folder, report }));
                });
            }
            Act::Snapshot => {
                let dirs = self.dirs.clone();
                self.mods_job(&folder, "Saving a snapshot", move |inst, _| async move {
                    tokio::task::spawn_blocking(move || {
                        snapshots::save_before(&dirs, &inst, "manual").map(|_| "Snapshot saved".to_string())
                    })
                    .await?
                });
            }
            Act::Restore(file) => self.mods.confirm_restore = Some(file),
            Act::Pack => self.mods.confirm_pack = true,
            Act::ShaderLoader => {
                self.mods_task(&folder, "Adding a shader loader".into(), move |mut inst, ctx, progress| async move {
                    let r = install::install_shader_loader(&mut inst, &ctx, &progress).await?;
                    Ok(format!("Added {}", r.installed.join(", ")))
                });
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // Drawing

    /// The Mods tab.
    pub(super) fn mods_tab(&mut self, ui: &mut Ui, inst: &Instance) {
        self.mods_prepare(inst);
        let mut acts: Vec<Act> = Vec::new();
        let kind = self.mods.kind();
        let busy = self.mods.busy.is_some()
            || self.tasks.iter().any(|t| {
                t.label.starts_with("Updating") || t.label.starts_with("Reinstalling") || t.label.starts_with("Adding")
            });
        let target = self
            .mods
            .target
            .clone()
            .unwrap_or_else(|| crate::mods::InstanceTarget::of(inst, &crate::mods::RemoteConfig::load(&self.dirs)));

        // kinds on the left, the folder and the main action on the right; then the search and the filters on the left
        // and the tools on the right. Each pair shares a line, or the right part goes on the next line in a narrow window.
        let total = self.mods.entries.get(&kind).map_or(0, Vec::len);
        let offers_n = self.mods.updates.values().filter(|o| o.kind == kind).count();
        let kinds_id = ui.scope_id().with("mods-kinds");
        let tools_id = ui.scope_id().with("mods-tools");
        let mut st = (&mut *self, &mut acts);
        split_or_stack(
            ui,
            kinds_id,
            34.0,
            &mut st,
            |ui, (this, acts)| {
                ui.spacing_mut().item_spacing.x = 22.0;
                for k in ContentKind::ALL {
                    let count = this.mods.entries.get(&k).map_or(0, Vec::len);
                    let label = if count > 0 { format!("{} {count}", k.label()) } else { k.label().to_string() };
                    if tab(ui, &label, kind == k).clicked() && kind != k {
                        acts.push(Act::Kind(k));
                    }
                }
            },
            |ui, (_, acts)| {
                if secondary_icon(ui, Icon::Folder, "Open folder").clicked() {
                    acts.push(Act::OpenFolder);
                }
                if primary_icon(ui, Icon::Plus, "Add").clicked() {
                    acts.push(Act::Add);
                }
            },
        );
        ui.add_space(6.0);
        split_or_stack(
            ui,
            tools_id,
            32.0,
            &mut st,
            |ui, (this, _)| {
                let mut f = std::mem::take(&mut this.mods.filter);
                let width = 200.0f32.min(ui.available_width());
                search_field(ui, &mut f, &format!("Filter {}", kind.label().to_lowercase()), width);
                this.mods.filter = f;
                let updates = if offers_n > 0 { format!("Updates {offers_n}") } else { "Updates".to_string() };
                let choices =
                    [(Show::All, "All"), (Show::On, "On"), (Show::Off, "Off"), (Show::Updates, updates.as_str())];
                let labels: Vec<&str> = choices.iter().map(|c| c.1).collect();
                let chosen = choices.iter().position(|c| c.0 == this.mods.show).unwrap_or(0);
                if let Some(i) = segmented(ui, &labels, chosen) {
                    this.mods.show = choices[i].0;
                }
            },
            |ui, (this, acts)| {
                if let Some(b) = &this.mods.busy {
                    muted(ui, format!("{b}\u{2026}"));
                    ui.add(egui::Spinner::new().size(14.0));
                }
                if this.mods.checking {
                    muted(ui, "Checking for updates");
                    ui.add(egui::Spinner::new().size(14.0));
                } else if secondary(ui, "Check for updates").clicked() {
                    acts.push(Act::CheckUpdates);
                }
                if offers_n > 0 {
                    let all = format!("Update all ({offers_n})");
                    if ui.add_enabled_ui(!busy, |ui| secondary(ui, &all)).inner.clicked() {
                        acts.push(Act::UpdateAll);
                    }
                }
                let select = if this.mods.select_mode { "Done selecting" } else { "Select" };
                if ghost(ui, select).clicked() {
                    this.mods.select_mode = !this.mods.select_mode;
                    this.mods.selected.clear();
                }
                secondary_menu(ui, "More", |ui| {
                    if ui.button("Look for new and missing files").clicked() {
                        acts.push(Act::Rescan);
                        ui.close();
                    }
                    if ui.add_enabled(!this.mods.scanning, egui::Button::new("Scan for malware")).clicked() {
                        acts.push(Act::Scan);
                        ui.close();
                    }
                    if kind == ContentKind::Mods
                        && performance::offered_for(target.loader_key)
                        && ui.add_enabled(!busy, egui::Button::new("Make Minecraft run faster")).clicked()
                    {
                        acts.push(Act::Pack);
                        ui.close();
                    }
                    if ui.button("Refresh names from Modrinth").clicked() {
                        let keys: Vec<ModKey> =
                            this.mods.entries.get(&kind).into_iter().flatten().map(|e| e.key()).collect();
                        acts.push(Act::Refresh(keys));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Save a snapshot").clicked() {
                        acts.push(Act::Snapshot);
                        ui.close();
                    }
                    let saved = snapshots::latest(&snapshots::folder_among(&this.dirs, inst, &this.instances), 8);
                    if saved.is_empty() {
                        ui.add_enabled(false, egui::Button::new("No snapshots yet"));
                    }
                    for s in saved {
                        let when = time_ago(s.created_at);
                        if ui.button(format!("Restore: {} ({when})", s.label())).clicked() {
                            acts.push(Act::Restore(s.file.clone()));
                            ui.close();
                        }
                    }
                });
            },
        );

        if self.mods.select_mode {
            let chosen: Vec<ModKey> = self.mods.selected.iter().cloned().collect();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                small_muted(ui, format!("{} selected", chosen.len()));
                let any = !chosen.is_empty();
                ui.add_enabled_ui(any, |ui| {
                    if secondary(ui, "Switch on").clicked() {
                        acts.push(Act::EnableAll(chosen.clone(), true));
                    }
                    if secondary(ui, "Switch off").clicked() {
                        acts.push(Act::EnableAll(chosen.clone(), false));
                    }
                    if secondary(ui, "Refresh names").clicked() {
                        acts.push(Act::Refresh(chosen.clone()));
                    }
                    if danger(ui, "Remove").clicked() {
                        acts.push(Act::Remove(chosen.clone()));
                    }
                });
            });
        }
        ui.add_space(6.0);

        // things worth knowing before the list
        self.mods_banners(ui, inst, &target, kind, &mut acts);

        // the list
        let filter = self.mods.filter.trim().to_lowercase();
        let rows: Vec<&ModEntry> = self
            .mods
            .entries
            .get(&kind)
            .into_iter()
            .flatten()
            .filter(|e| {
                let matches = filter.is_empty()
                    || e.name().to_lowercase().contains(&filter)
                    || e.record.file.to_lowercase().contains(&filter)
                    || e.record.description.to_lowercase().contains(&filter);
                let shown = match self.mods.show {
                    Show::All => true,
                    Show::On => !e.record.disabled,
                    Show::Off => e.record.disabled,
                    Show::Updates => self.mods.updates.contains_key(&e.key()),
                };
                matches && shown
            })
            .collect();
        if total == 0 {
            let loading = self.mods.last_report.is_none() && self.mods.sync_error.is_none();
            if let Some(e) = &self.mods.sync_error {
                let text = format!("Couldn't look through the folders: {}", short(e, 200));
                if notice(ui, NoticeKind::Error, &text, Some("Try again")).is_some_and(|r| r.clicked()) {
                    acts.push(Act::Rescan);
                }
            } else if loading {
                skeleton_rows(ui, 6, ROW_H);
            } else if kind == ContentKind::Mods && target.loader_key.is_none() {
                let text =
                    "No mod loader yet. This instance runs plain Minecraft, and mods need a mod loader such as Fabric.";
                if notice(ui, NoticeKind::Info, text, Some("Add a mod loader")).is_some_and(|r| r.clicked()) {
                    acts.push(Act::Loader);
                }
            } else {
                let text = match kind {
                    ContentKind::Mods => "No mods yet. Add some from Modrinth, or drop .jar files onto this window.",
                    ContentKind::ResourcePacks => {
                        "No resource packs yet. Browse Modrinth, or drop .zip files onto this window."
                    }
                    ContentKind::Shaders => "No shaders yet. Browse Modrinth, or drop .zip files onto this window.",
                };
                empty_row(ui, text, None);
            }
        } else if rows.is_empty() {
            let what = kind.label().to_lowercase();
            if empty_row(ui, &format!("No {what} match your filter."), Some("Clear filter"))
                .is_some_and(|r| r.clicked())
            {
                acts.push(Act::ClearFilter);
            }
        }
        lazy_list(ui, egui::Id::unique(("mod-list", kind)), &rows, ROW_H + 1.0, |ui, e| {
            ui.spacing_mut().item_spacing.y = 0.0;
            self.mod_row(ui, e, busy, &mut acts);
        });

        let ctx = ui.ctx().clone();
        self.mods_dialogs(&ctx, inst);
        for a in acts {
            self.mods_apply(inst, a);
        }
    }

    /// Notes above the list: the malware scan result, shader setup, a missing loader.
    fn mods_banners(
        &mut self,
        ui: &mut Ui,
        inst: &Instance,
        target: &crate::mods::InstanceTarget,
        kind: ContentKind,
        acts: &mut Vec<Act>,
    ) {
        if let Some(report) = &self.mods.scan {
            let (level, mut text) = match report.warning() {
                Some(w) => (NoticeKind::Error, w.to_string()),
                None => (
                    NoticeKind::Info,
                    format!(
                        "No malware found ({} files read, {} already known to be fine)",
                        report.scanned, report.known_clean
                    ),
                ),
            };
            if !report.unreadable.is_empty() {
                text.push_str(&format!(". {} file(s) couldn't be opened as a zip", report.unreadable.len()));
            }
            if notice(ui, level, &text, Some("Dismiss")).is_some_and(|r| r.clicked()) {
                acts.push(Act::DismissScan);
            }
            ui.add_space(6.0);
        } else if self.mods.scanning {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                muted(ui, "Scanning for malware");
            });
        }
        if kind == ContentKind::Shaders {
            let setup = self.mods.shader_setup.unwrap_or_else(|| install::shader_setup(inst, target));
            let (text, action) = match setup {
                versions::ShaderSetup::AddIris => (
                    "Shaders need Iris to work. Iris and Sodium will be added from Modrinth; they make shaders run fast."
                        .to_string(),
                    Some("Add Iris"),
                ),
                versions::ShaderSetup::AddOculus => (
                    format!(
                        "Shaders on Forge need Oculus or OptiFine. Oculus can be added from Modrinth if it's made for Minecraft {}.",
                        inst.mc_version()
                    ),
                    Some("Add Oculus"),
                ),
                versions::ShaderSetup::NeedsModLoader => (
                    "Shaders need a mod loader. Plain Minecraft can't show shaders; they work once the instance has Fabric and Iris."
                        .to_string(),
                    None,
                ),
                versions::ShaderSetup::Ready => return,
            };
            if notice(ui, NoticeKind::Info, &text, action).is_some_and(|r| r.clicked()) {
                acts.push(Act::ShaderLoader);
            }
            ui.add_space(6.0);
        }
    }

    /// One mod as a row: its picture, its name with the version and where it came from, a line about it, and on the
    /// right its actions (shown while the row is pointed at) and its switch.
    fn mod_row(&self, ui: &mut Ui, e: &ModEntry, busy: bool, acts: &mut Vec<Act>) {
        let key = e.key();
        ui.push_id(&key.file, |ui| self.mod_row_inner(ui, e, &key, busy, acts));
    }

    fn mod_row_inner(&self, ui: &mut Ui, e: &ModEntry, key: &ModKey, busy: bool, acts: &mut Vec<Act>) {
        let rec = &e.record;
        let offer = self.mods.updates.get(key);
        let details = self.mods.details.get(key);
        let expanded = self.mods.expanded.as_ref() == Some(key);
        let name = e.name();
        let off = rec.disabled;

        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), egui::Sense::hover());
        let hovered = ui.rect_contains_pointer(rect);
        if hovered || expanded {
            ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), c_hover());
        }
        let inner = rect.shrink2(egui::vec2(8.0, 0.0));
        let mut left = inner.left();
        if self.mods.select_mode {
            let mut on = self.mods.selected.contains(key);
            let r = egui::Rect::from_center_size(egui::pos2(left + 10.0, rect.center().y), egui::vec2(20.0, 20.0));
            if ui.new_child(egui::UiBuilder::new().max_rect(r)).add(egui::Checkbox::new(&mut on, "")).changed() {
                acts.push(Act::Select(key.clone(), on));
            }
            left += 30.0;
        }
        // the picture, dimmed while the mod is off
        let icon = egui::Rect::from_min_size(egui::pos2(left, rect.center().y - ICON / 2.0), egui::vec2(ICON, ICON));
        let mut picture = ui.new_child(egui::UiBuilder::new().max_rect(icon));
        if off {
            picture.set_opacity(0.45);
        }
        match self.mods.textures.get(key) {
            Some(tex) => {
                picture.add(
                    egui::Image::from_texture(tex)
                        .fit_to_exact_size(egui::vec2(ICON, ICON))
                        .corner_radius(CornerRadius::same(R_CONTROL)),
                );
            }
            None => paint_avatar(&picture, icon, &name),
        }
        left += ICON + 12.0;
        // the switch, at the right edge
        let mut on = !off;
        let switch =
            egui::Rect::from_center_size(egui::pos2(inner.right() - 18.0, rect.center().y), egui::vec2(36.0, 20.0));
        let switched = toggle(&mut ui.new_child(egui::UiBuilder::new().max_rect(switch)), &mut on);
        if switched.on_hover_text(if on { "Switched on" } else { "Switched off" }).changed() {
            acts.push(Act::Toggle(key.clone(), on));
        }
        // what can be done with it, left of the switch
        let update_label = offer.map(|o| format!("Update: {}", short(&o.newer, 18)));
        let text_w = |ui: &Ui, t: &str, size: f32| {
            ui.painter().layout_no_wrap(t.to_string(), super::fonts::medium(size), c_text()).size().x
        };
        let actions_w = 2.0 * 8.0
            + text_w(ui, "Remove", FS_BODY)
            + 20.0
            + text_w(ui, "Details", FS_BODY)
            + 20.0
            + update_label.as_deref().map_or(0.0, |t| text_w(ui, t, FS_BODY) + 20.0 + 4.0);
        let actions = egui::Rect::from_min_max(
            egui::pos2((switch.left() - 12.0 - actions_w).max(left + 120.0), rect.top()),
            egui::pos2(switch.left() - 12.0, rect.bottom()),
        );
        let mut ac =
            ui.new_child(egui::UiBuilder::new().max_rect(actions).layout(Layout::right_to_left(Align::Center)));
        ac.spacing_mut().item_spacing.x = 0.0;
        if ghost_button(&mut ac, "Remove", None, Some(c_danger()), hovered).clicked() {
            acts.push(Act::Remove(vec![key.clone()]));
        }
        if ghost_button(&mut ac, if expanded { "Hide" } else { "Details" }, None, None, hovered || expanded).clicked() {
            acts.push(Act::Details(key.clone()));
        }
        if let (Some(o), Some(label)) = (offer, update_label) {
            // plain text in the text colour: green is kept for starting the game
            let r = ac
                .add_enabled_ui(!busy, |ui| ghost_button(ui, &label, Some(c_text()), Some(c_text()), true))
                .inner
                .on_hover_text(format!("{} \u{2192} {} on {}", o.current, o.newer, o.platform.label()));
            if r.clicked() {
                acts.push(Act::Update(key.clone()));
            }
        }
        // the name and what is known about it
        let text = egui::Rect::from_min_max(
            egui::pos2(left, rect.top() + 8.0),
            egui::pos2(actions.left() - 8.0, rect.bottom() - 8.0),
        );
        let mut tc = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(Layout::top_down(Align::Min)));
        tc.set_clip_rect(tc.clip_rect().intersect(text));
        tc.spacing_mut().item_spacing = egui::vec2(6.0, 2.0);
        tc.spacing_mut().interact_size.y = 20.0;
        let version =
            if rec.version.is_empty() || rec.version == "Unknown" { String::new() } else { short(&rec.version, 40) };
        // a tag is for what needs a look; where the mod came from is plain grey text at the end of the version
        let mut tags: Vec<(&str, egui::Color32)> = Vec::new();
        if !e.exists {
            tags.push(("File missing", c_danger()));
        }
        let tags_w: f32 = tags.iter().map(|(t, _)| text_w(&tc, t, FS_META) + 12.0 + 6.0).sum();
        let name_text = RichText::new(&name).semibold().color(if off { c_muted() } else { c_text() });
        let name_nat = tc.painter().layout_no_wrap(name.clone(), super::fonts::semibold(FS_BODY), c_text()).size().x;
        let name_w = name_nat.min((text.width() - tags_w - 6.0).max(text.width() * 0.4));
        // the version and the rest take what is left on the line: a part that does not fit is left out whole, and
        // all of it is left to the details when there is no room
        let left_over = text.width() - name_w - tags_w - 12.0;
        let mut meta: Vec<&str> = Vec::new();
        if !version.is_empty() {
            meta.push(&version);
        }
        if files::is_from_modrinth(rec) {
            meta.push("Modrinth");
        }
        if files::is_from_curseforge(rec) {
            meta.push("CurseForge");
        }
        if !rec.user_added {
            meta.push("Part of the modpack");
        }
        let meta_w = |parts: &[&str]| {
            let font = egui::FontId::proportional(FS_META);
            tc.painter().layout_no_wrap(parts.join(META_SEP), font, c_muted()).size().x
        };
        while meta.len() > 1 && meta_w(&meta) > left_over {
            meta.pop();
        }
        let show_meta = !meta.is_empty() && meta_w(&meta) <= left_over;
        tc.horizontal(|ui| {
            ui.scope(|ui| {
                ui.set_max_width(name_w);
                ui.add(egui::Label::new(name_text).truncate());
            });
            if show_meta {
                ui.scope(|ui| {
                    ui.set_max_width(left_over);
                    ui.add(
                        egui::Label::new(RichText::new(meta.join(META_SEP)).color(c_muted()).size(FS_META)).truncate(),
                    );
                });
            }
            for (t, c) in &tags {
                tag(ui, t, *c);
            }
        });
        if let Some(p) = details.and_then(|d| d.problem.as_ref()) {
            tc.add(
                egui::Label::new(RichText::new(format!("May not work here: {p}")).color(c_warn()).size(FS_META))
                    .truncate(),
            );
        } else if !rec.description.is_empty() {
            tc.add(
                egui::Label::new(RichText::new(short(&rec.description, 160)).color(c_muted()).size(FS_META)).truncate(),
            );
        }
        if expanded {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(left - rect.left() + 8.0 - ui.spacing().item_spacing.x);
                ui.vertical(|ui| {
                    ui.spacing_mut().interact_size.y = 20.0;
                    ui.spacing_mut().item_spacing.y = 4.0;
                    self.mod_details(ui, e, details, offer, busy, acts)
                });
            });
            ui.add_space(8.0);
        }
        // one hairline between rows
        let y = if expanded { ui.cursor().top() } else { rect.bottom() - 0.5 };
        ui.painter().hline(rect.left()..=rect.right(), y, Stroke::new(1.0, c_rule()));
    }

    /// The details of one mod: what the file says about itself, links, and the actions that need a closer look.
    fn mod_details(
        &self,
        ui: &mut Ui,
        e: &ModEntry,
        details: Option<&Details>,
        offer: Option<&UpdateOffer>,
        busy: bool,
        acts: &mut Vec<Act>,
    ) {
        let rec = &e.record;
        let key = e.key();
        let info = details.map(|d| &d.info);
        egui::Grid::new(("mod-details", &key.file)).num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
            let row = |ui: &mut Ui, k: &str, v: String| {
                if !v.is_empty() {
                    small_muted(ui, k);
                    ui.add(egui::Label::new(v).wrap());
                    ui.end_row();
                }
            };
            row(ui, "File", format!("{}  ({})", rec.file, human_size(e.size)));
            if let Some(i) = info {
                row(ui, "Version", i.version.clone().unwrap_or_default());
                row(ui, "Authors", i.authors.join(", "));
                row(ui, "License", i.license.clone().unwrap_or_default());
                row(
                    ui,
                    "Loader",
                    i.loaders.iter().map(|l| versions::display_name(Some(l))).collect::<Vec<_>>().join(", "),
                );
                row(ui, "Minecraft", i.minecraft_versions.join(" or "));
                row(
                    ui,
                    "Runs on",
                    i.environment
                        .clone()
                        .map(|e| match e.as_str() {
                            "*" => "Client and server".to_string(),
                            "client" => "Client only".to_string(),
                            "server" => "Server only".to_string(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default(),
                );
                row(ui, "Needs", i.depends.join(", "));
            }
            if let Some(o) = offer {
                row(ui, "Update", format!("{} \u{2192} {} ({})", o.current, o.newer, o.platform.label()));
            }
        });
        let text = info
            .and_then(|i| i.description.clone())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| rec.description.clone());
        if !text.is_empty() {
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(text).color(c_muted())).wrap());
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if let Some(p) = &rec.modrinth_project {
                let s = |k: &str| p.get(k).and_then(|v| v.as_str()).filter(|v| !v.is_empty()).map(str::to_string);
                if let Some(slug) = s("slug") {
                    let t = s("project_type").unwrap_or_else(|| "mod".into());
                    if secondary(ui, "Open on Modrinth").clicked() {
                        acts.push(Act::Url(format!("https://modrinth.com/{t}/{slug}")));
                    }
                }
                for (k, label) in [
                    ("source_url", "Source"),
                    ("issues_url", "Issues"),
                    ("wiki_url", "Wiki"),
                    ("discord_url", "Discord"),
                ] {
                    if let Some(u) = s(k) {
                        if secondary(ui, label).clicked() {
                            acts.push(Act::Url(u));
                        }
                    }
                }
                for d in p.get("donation_urls").and_then(|d| d.as_array()).into_iter().flatten() {
                    let (platform, url) =
                        (d.get("platform").and_then(|v| v.as_str()), d.get("url").and_then(|v| v.as_str()));
                    if let (Some(platform), Some(url)) = (platform, url) {
                        if secondary(ui, &format!("Donate ({platform})")).clicked() {
                            acts.push(Act::Url(url.to_string()));
                        }
                    }
                }
            } else if let Some(h) = info.and_then(|i| i.homepage.clone()) {
                if secondary(ui, "Website").clicked() {
                    acts.push(Act::Url(h));
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if secondary(ui, "Show in folder").clicked() {
                acts.push(Act::ShowInFolder(key.clone()));
            }
            if files::is_from_modrinth(rec) {
                if ui.add_enabled_ui(!busy, |ui| secondary(ui, "Check this one")).inner.clicked() {
                    acts.push(Act::CheckOne(key.clone()));
                }
                if ui
                    .add_enabled_ui(!busy, |ui| secondary(ui, "Versions\u{2026}"))
                    .inner
                    .on_hover_text("Pick another version, or go back to an older one")
                    .clicked()
                {
                    acts.push(Act::Versions(key.clone()));
                }
                if ui.add_enabled_ui(!busy, |ui| secondary(ui, "Reinstall")).inner.clicked() {
                    acts.push(Act::Reinstall(key.clone()));
                }
            }
            if ui.add_enabled_ui(!busy, |ui| secondary(ui, "Refresh from Modrinth")).inner.clicked() {
                acts.push(Act::Refresh(vec![key.clone()]));
            }
        });
    }

    /// The confirmations and the version list.
    fn mods_dialogs(&mut self, ctx: &egui::Context, inst: &Instance) {
        // remove
        if !self.mods.confirm_remove.is_empty() {
            let keys = self.mods.confirm_remove.clone();
            let label = if keys.len() == 1 { format!("\"{}\"", keys[0].file) } else { format!("{} items", keys.len()) };
            let (mut yes, mut no) = (false, false);
            egui::Modal::new(egui::Id::unique("mods-confirm-remove")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new(format!("Remove {label}?")).size(FS_SECTION).semibold());
                ui.add_space(4.0);
                muted(ui, "The file is deleted from this instance. You can add it again at any time.");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    yes = danger(ui, "Remove").clicked();
                    no = secondary(ui, "Cancel").clicked();
                });
            });
            if yes || no {
                self.mods.confirm_remove.clear();
            }
            if yes {
                let s = ModSettings::from_settings(&self.settings);
                self.mods_job(&inst.folder, "Removing", move |mut inst, _| async move {
                    tokio::task::spawn_blocking(move || {
                        let mut failed = Vec::new();
                        for k in &keys {
                            if let Err(e) = files::remove(&mut inst, k, &s) {
                                failed.push(format!("{}: {e:#}", k.file));
                            }
                        }
                        if failed.is_empty() {
                            Ok(format!("Removed {label}"))
                        } else {
                            Err(anyhow::anyhow!(failed.join("; ")))
                        }
                    })
                    .await?
                });
                self.mods.selected.clear();
            }
        }
        // restore a snapshot
        if let Some(file) = self.mods.confirm_restore.clone() {
            let (mut yes, mut no) = (false, false);
            egui::Modal::new(egui::Id::unique("mods-confirm-restore")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(RichText::new("Restore this snapshot?").size(FS_SECTION).semibold());
                ui.add_space(4.0);
                muted(ui, format!("Puts the mods and configs of \"{}\" back to how they were. Your worlds are not changed, and the current state is saved first so you can undo this.", inst.name()));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    yes = primary(ui, "Restore").clicked();
                    no = secondary(ui, "Cancel").clicked();
                });
            });
            if yes || no {
                self.mods.confirm_restore = None;
            }
            if yes {
                let dirs = self.dirs.clone();
                self.mods_job(&inst.folder, "Restoring", move |inst, _| async move {
                    tokio::task::spawn_blocking(move || {
                        let dir = snapshots::folder_for(&dirs, &inst);
                        let snapshot = snapshots::list(&dir)
                            .into_iter()
                            .find(|s| s.file == file)
                            .ok_or_else(|| anyhow::anyhow!("that snapshot is gone"))?;
                        snapshots::restore(&snapshot, &inst.root, &dir).map(|_| "Snapshot restored".to_string())
                    })
                    .await?
                });
            }
        }
        // performance pack
        if self.mods.confirm_pack {
            let target =
                self.mods.target.clone().unwrap_or_else(|| {
                    crate::mods::InstanceTarget::of(inst, &crate::mods::RemoteConfig::load(&self.dirs))
                });
            let (mut yes, mut no) = (false, false);
            egui::Modal::new(egui::Id::unique("mods-confirm-pack")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(RichText::new("Make Minecraft run faster").size(FS_SECTION).semibold());
                ui.add_space(4.0);
                muted(
                    ui,
                    format!(
                        "Adds {} from Modrinth. They make the game smoother and load faster, especially on older computers, and don't change how it plays. Anything not made for Minecraft {} yet is skipped.",
                        performance::describe(target.loader_key),
                        inst.mc_version()
                    ),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    yes = primary(ui, "Add them").clicked();
                    no = secondary(ui, "Cancel").clicked();
                });
            });
            if yes || no {
                self.mods.confirm_pack = false;
            }
            if yes {
                self.mods_task(
                    &inst.folder,
                    "Adding performance mods".into(),
                    move |mut inst, ctx, progress| async move {
                        let r = install::install_performance_pack(&mut inst, &ctx, &progress).await;
                        Ok(if r.installed.is_empty() {
                            if r.skipped.is_empty() {
                                "Performance mods are already installed".to_string()
                            } else {
                                format!(
                                    "No performance mods are available for Minecraft {} yet",
                                    ctx.target.minecraft_version
                                )
                            }
                        } else {
                            format!("Added {}", r.installed.join(", "))
                        })
                    },
                );
            }
        }
        // versions of one mod
        let mut install_version: Option<(ModKey, String)> = None;
        let mut close = false;
        if let Some(d) = &self.mods.versions {
            egui::Modal::new(egui::Id::unique("mods-versions")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(560.0);
                ui.label(RichText::new(format!("Versions of {}", d.name)).size(FS_SECTION).semibold());
                small_muted(ui, "Pick one to install it in place of the current one. Older versions are fine if a newer one gives you trouble.");
                ui.add_space(8.0);
                match &d.list {
                    None => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(14.0));
                            muted(ui, "Asking Modrinth");
                        });
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).color(c_danger()));
                    }
                    Some(Ok(list)) if list.is_empty() => {
                        muted(ui, "No versions are made for this instance.");
                    }
                    Some(Ok(list)) => {
                        ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            for v in list {
                                let current = d.installed_id.as_deref() == Some(v.id.as_str());
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.set_max_width(380.0);
                                        ui.add(egui::Label::new(RichText::new(v.label()).semibold()).truncate());
                                        small_muted(ui, format!("{}  ·  {}", v.date_published.get(..10).unwrap_or(&v.date_published), v.game_versions.iter().take(4).cloned().collect::<Vec<_>>().join(", ")));
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if current {
                                            small_muted(ui, "\u{2713} Installed");
                                        } else if secondary(ui, "Install").clicked() {
                                            install_version = Some((d.key.clone(), v.id.clone()));
                                        }
                                    });
                                });
                                ui.separator();
                            }
                        });
                    }
                }
                ui.add_space(8.0);
                if secondary(ui, "Close").clicked() {
                    close = true;
                }
            });
        }
        if close || install_version.is_some() {
            self.mods.versions = None;
        }
        if let Some((key, version_id)) = install_version {
            self.mods_task(
                &inst.folder,
                format!("Installing {}", key.file),
                move |mut inst, ctx, progress| async move {
                    updates::install_specific(&mut inst, &ctx, &key, &version_id, &progress).await?;
                    Ok(format!("Installed {version_id} over {}", key.file))
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn details_and_a_small_picture_are_read_from_a_jar() {
        let dir = std::env::temp_dir().join(format!("blockforge-gui-mods-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = {
            let mut out = Vec::new();
            let img = image::RgbaImage::from_pixel(200, 200, image::Rgba([10, 200, 90, 255]));
            image::DynamicImage::ImageRgba8(img)
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .unwrap();
            out
        };
        let jar = dir.join("sodium.jar");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("fabric.mod.json", o).unwrap();
        z.write_all(br#"{"name":"Sodium","version":"1","icon":"icon.png","depends":{"minecraft":"1.20.1"}}"#).unwrap();
        z.start_file("icon.png", o).unwrap();
        z.write_all(&png).unwrap();
        z.finish().unwrap();

        let key = ModKey { file: "sodium.jar".into(), kind: "mods".into() };
        let out = read_details(vec![(key.clone(), jar.clone())], Some("fabric"), "1.21");
        let (k, details, image) = &out[0];
        assert_eq!(k, &key);
        assert_eq!(details.info.name.as_deref(), Some("Sodium"));
        assert!(details.problem.as_ref().unwrap().contains("Minecraft"));
        let img = image.as_ref().unwrap();
        assert!(img.size[0] <= 72 && img.size[1] <= 72);
        // fine for the version it asks for
        assert!(read_details(vec![(key, jar)], Some("fabric"), "1.20.1")[0].1.problem.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_list_only_lays_out_what_is_in_view() {
        let ctx = egui::Context::default();
        let items: Vec<u32> = (0..300).collect();
        let mut drawn: Vec<u32> = Vec::new();
        for _ in 0..3 {
            drawn.clear();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                lazy_list(ui, egui::Id::unique("test-list"), &items, 40.0, |ui, i| {
                    drawn.push(*i);
                    ui.allocate_space(egui::vec2(100.0, 40.0));
                });
            });
            // nothing paints here, so the font pictures are thrown away
            out.textures_delta.clear();
        }
        // a screenful and a margin around it, not all 300
        assert_eq!(drawn.first(), Some(&0));
        assert!(drawn.len() > 10 && drawn.len() < 30, "{} rows laid out", drawn.len());
    }

    #[test]
    fn changes_to_one_instance_take_turns() {
        let a = instance_lock(Path::new("/instances/a"));
        assert!(Arc::ptr_eq(&a, &instance_lock(Path::new("/instances/a"))));
        assert!(!Arc::ptr_eq(&a, &instance_lock(Path::new("/instances/b"))));
        let held = a.try_lock().unwrap();
        assert!(instance_lock(Path::new("/instances/a")).try_lock().is_err());
        assert!(instance_lock(Path::new("/instances/b")).try_lock().is_ok());
        drop(held);
        assert!(a.try_lock().is_ok());
    }

    #[test]
    fn long_text_is_cut() {
        assert_eq!(short("abcdef", 4), "abcd…");
        assert_eq!(short("abc", 4), "abc");
    }
}
