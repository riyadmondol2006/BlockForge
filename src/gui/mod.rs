//! The launcher window, built with egui. Everything slow happens on a background runtime and reports back
//! through a channel, so the window never waits on the network or the disk.

mod about;
mod accounts;
mod console;
#[cfg(feature = "converter")]
mod convert;
#[cfg(feature = "converter")]
mod convert_view;
mod craft;
mod craft_fmt;
mod craft_pick;
mod craft_view;
mod create;
mod curseforge;
mod discover;
mod export;
mod fonts;
mod ftb;
mod icons;
mod import;
mod instance_icon;
mod instance_menu;
mod instance_settings;
mod library;
mod mods;
mod packs;
mod performance;
mod pictures;
mod settings;
mod startup;
mod theme;
mod tray;
mod updates;
mod whoplays;
mod widgets;

// services: tools, servers, news, skins, backups
mod backups;
mod news;
mod servers;
mod services;
mod skins;
mod tools;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use egui::{Align, CornerRadius, Layout, RichText, Stroke, Ui, vec2};

use crate::accounts::Account;
use crate::config::Settings;
use crate::install::{Manifest, Progress};
use crate::instances::{self, Instance};
use crate::java::JavaInstall;
use crate::launch::{self, Level, LogAssembler, LogLine};
use crate::loaders::Loader;
use crate::modrinth::Hit;
use crate::msauth::DeviceCode;
use crate::paths::Dirs;

use icons::Icon;
use theme::*;
use widgets::*;

const LOGO: &[u8] = include_bytes!("../../assets/icon.png");
const MAX_LOG_LINES: usize = 6000;
/// The smallest window, in points: every page is laid out to work at this size.
const MIN_WINDOW: egui::Vec2 = egui::vec2(860.0, 560.0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Home,
    Library,
    Instance,
    Discover,
    Packs,
    Ftb,
    Create,
    Accounts,
    Settings,
    Tools,
    Craft,
    /// The world converter; reached from the Tools page and an instance, so it has no item of its own.
    #[cfg(feature = "converter")]
    Convert,
    Servers,
    About,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InstTab {
    Overview,
    Mods,
    Logs,
    Settings,
}

/// What the background work tells the window. `Deleted` ends the background delete of an instance folder (whether
/// it worked is told by its task); `InstanceImage` says whether a picture chosen for an instance was saved as its
/// `instance.png`; `Device` and `SignedIn` carry the number of the Microsoft sign-in they belong to, so the answer of
/// one that was cancelled is not taken for a newer one's.
pub enum Msg {
    Reload,
    Deleted(String),
    InstanceImage(String, Result<(), String>),
    Manifest(Result<Manifest, String>),
    LoaderVersions {
        loader: Loader,
        mc: String,
        result: Result<Vec<String>, String>,
    },
    Search {
        seq: u64,
        result: Result<(Vec<Hit>, u64), String>,
    },
    Icon {
        url: String,
        bytes: Option<Vec<u8>>,
    },
    TaskDone {
        id: u64,
        result: Result<String, String>,
    },
    GameRunning(String),
    GameLog(String, LogLine),
    GameExit(String, Option<i32>),
    GameFailed(String, String),
    Device(u64, Result<DeviceCode, String>),
    SignedIn(u64, Result<Account, String>),
    Java(Vec<JavaInstall>),
    Gpus(Vec<crate::gpu::Gpu>),
    Runtimes(Vec<(String, String)>),
    LogUploaded(Result<String, String>),
    Picked(instance_menu::PickPurpose, Option<std::path::PathBuf>),
    DialogLoaderVersions {
        loader: Loader,
        mc: String,
        result: Result<Vec<String>, String>,
    },
    Update(updates::UpdateMsg),
    Tray(tray::TrayCmd),
    AccountRefreshed(Result<Account, String>),
    Toast(String, bool),
    Svc(services::SvcMsg),
    Mods(mods::ModsMsg),
    Packs(packs::PacksMsg),
    Craft(craft::CraftMsg),
    #[cfg(feature = "converter")]
    Convert(convert::ConvertMsg),
}

#[derive(Clone)]
pub struct Notifier {
    tx: Sender<Msg>,
    ctx: egui::Context,
}

impl Notifier {
    pub fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
        self.ctx.request_repaint();
    }
}

pub struct Task {
    id: u64,
    label: String,
    progress: Arc<Progress>,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Phase {
    Preparing,
    Running,
    Exited(Option<i32>),
    Failed,
}

pub struct Game {
    /// The instance's folder (the key in `games` is this for the first session and carries a number after it for
    /// more sessions of the same instance playing at the same time).
    folder: String,
    /// Who is playing in this session.
    player: String,
    /// Whether this start used the fast-start helper (to say so if the game fails early).
    fast_start: bool,
    /// Whether this start used the smoothness helper, the same way.
    lite: bool,
    /// The "Starting ..." progress card of this session, taken away once the game is up.
    task: u64,
    lines: Vec<LogLine>,
    phase: Phase,
    kill: Option<tokio::sync::oneshot::Sender<()>>,
    started: Instant,
}

impl Game {
    fn active(&self) -> bool {
        matches!(self.phase, Phase::Preparing | Phase::Running)
    }
}

struct Toast {
    text: String,
    error: bool,
    until: Instant,
}

pub struct App {
    pub dirs: Dirs,
    rt: Arc<tokio::runtime::Runtime>,
    ctx: egui::Context,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub settings: Settings,
    pub accounts: Vec<Account>,
    pub instances: Vec<Instance>,
    page: Page,
    selected: Option<String>,
    tab: InstTab,
    library_search: String,
    toasts: Vec<Toast>,
    tasks: Vec<Task>,
    next_task: u64,
    games: HashMap<String, Game>,
    manifest: Option<Arc<Manifest>>,
    manifest_error: Option<String>,
    icons: HashMap<String, IconState>,
    local_pictures: instance_icon::LocalPictures,
    java_list: Vec<JavaInstall>,
    /// The graphics cards of this computer, once looked for.
    gpus: Option<Vec<crate::gpu::Gpu>>,
    logo: egui::TextureHandle,
    confirm_delete: Option<String>,
    /// Instances whose folders are being deleted in the background; they are left out of the lists meanwhile.
    deleting: Vec<String>,
    quick_server: String,
    rename_target: Option<(String, String)>,
    inst_draft: Option<instance_settings::InstDraft>,
    runtimes: Vec<(String, String)>,
    console_lines: Vec<LogLine>,
    console_uploading: bool,
    console_confirm_kill: bool,
    who: Option<whoplays::WhoState>,
    /// The instance whose game closed early with a helper on: the player is offered to turn the helpers off.
    boost_blame: Option<String>,
    /// Which of an instance's running games the Logs tab shows, when there are several.
    log_session: Option<String>,
    /// The instance whose Play was pressed with no account: it starts once the player has chosen one.
    who_then_play: Option<String>,
    startup_warnings: Vec<startup::Warning>,
    desc_edit: Option<(String, String)>,
    offline_play: Option<(String, String, bool)>,
    loader_dialog: Option<instance_menu::LoaderDialog>,
    tray: Option<tray::Tray>,
    window_visible: bool,
    upd: updates::UpdateUi,
    last_window: Option<(crate::config::Size, crate::config::Pos)>,
    last_window_save: Instant,
    /// When settings changed by [`App::save_settings_soon`] are to be written.
    settings_save_due: Option<Instant>,
    /// The zoom the smallest window size was last set for.
    zoom_seen: f32,
    mods_cache: Option<(String, Vec<crate::instances::ModFile>)>,
    log_filter: String,
    create: create::CreateState,
    discover: discover::DiscoverState,
    pub import: import::ImportState,
    pub export: export::ExportState,
    pub ftb: ftb::FtbState,
    cf: curseforge::CfState,
    acct: accounts::AcctState,
    svc: services::SvcState,
    mods: mods::ModsState,
    packs: packs::PacksState,
    /// The crafting calculator, made when its page is first opened.
    craft: Option<Box<craft::CraftState>>,
    /// The world converter, made when its page is first opened.
    #[cfg(feature = "converter")]
    convert: Option<Box<convert::ConvertState>>,
    shot: Option<Shot>,
    autoplay: Option<AutoPlay>,
    frames: u64,
    /// Files the program was started with, handed to the import flow once the window is up.
    pending_open: Vec<std::path::PathBuf>,
    last_account_check: Instant,
}

pub fn run(dirs: Dirs, open_files: Vec<std::path::PathBuf>) -> anyhow::Result<()> {
    let icon = image::load_from_memory(LOGO).ok().map(|i| {
        let rgba = i.to_rgba8();
        egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
    });
    let saved = Settings::load(&dirs);
    let size = saved
        .launcher_size
        .filter(|_| saved.remember_window_size_position)
        .unwrap_or(crate::config::Size { width: 1200, height: 700 });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(crate::NAME)
        .with_app_id("blockforge")
        .with_inner_size([(size.width as f32).max(MIN_WINDOW.x), (size.height as f32).max(MIN_WINDOW.y)])
        .with_min_inner_size(MIN_WINDOW);
    if let Some(p) = saved.launcher_position.filter(|_| saved.remember_window_size_position) {
        viewport = viewport.with_position([p.x as f32, p.y as f32]);
    }
    if let Some(icon) = icon {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let rt = Arc::new(tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build()?);
    eframe::run_native(
        crate::NAME,
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, dirs, rt, open_files)) as Box<dyn eframe::App>)),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        dirs: Dirs,
        rt: Arc<tokio::runtime::Runtime>,
        open_files: Vec<std::path::PathBuf>,
    ) -> App {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let logo = image::load_from_memory(LOGO)
            .map(|i| {
                let rgba = i.to_rgba8();
                egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
            })
            .unwrap_or_else(|_| egui::ColorImage::filled([1, 1], c_accent()));
        let logo = cc.egui_ctx.load_texture("logo", logo, egui::TextureOptions::LINEAR);
        let (tx, rx) = std::sync::mpsc::channel();
        let settings = Settings::load(&dirs);
        fonts::install(&cc.egui_ctx);
        theme::set_dark(theme::theme_is_dark(&settings.theme));
        theme::apply(&cc.egui_ctx, settings.light_mode.unwrap_or_else(low_end_pc));
        let accounts = crate::accounts::load(&dirs);
        let instances = instances::load_all(&dirs);
        launch::clean_stale_temp(&dirs);
        {
            let dirs = dirs.clone();
            std::thread::spawn(move || pictures::prune(&dirs));
        }
        let mut app = App {
            dirs,
            rt,
            ctx: cc.egui_ctx.clone(),
            tx,
            rx,
            settings,
            accounts,
            instances,
            page: Page::Home,
            selected: None,
            tab: InstTab::Overview,
            library_search: String::new(),
            toasts: Vec::new(),
            tasks: Vec::new(),
            next_task: 1,
            games: HashMap::new(),
            manifest: None,
            manifest_error: None,
            icons: HashMap::new(),
            local_pictures: Default::default(),
            java_list: Vec::new(),
            gpus: None,
            logo,
            confirm_delete: None,
            deleting: Vec::new(),
            quick_server: String::new(),
            rename_target: None,
            inst_draft: None,
            runtimes: Vec::new(),
            console_lines: Vec::new(),
            console_uploading: false,
            console_confirm_kill: false,
            who: None,
            boost_blame: None,
            log_session: None,
            who_then_play: None,
            startup_warnings: Vec::new(),
            desc_edit: None,
            offline_play: None,
            loader_dialog: None,
            tray: None,
            window_visible: true,
            upd: updates::UpdateUi::default(),
            last_window: None,
            last_window_save: Instant::now(),
            settings_save_due: None,
            zoom_seen: 1.0,
            mods_cache: None,
            log_filter: String::new(),
            create: create::CreateState::default(),
            discover: discover::DiscoverState::default(),
            import: import::ImportState::default(),
            export: export::ExportState::default(),
            ftb: ftb::FtbState::default(),
            svc: services::SvcState::default(),
            mods: mods::ModsState::default(),
            packs: packs::PacksState::default(),
            craft: None,
            #[cfg(feature = "converter")]
            convert: None,
            cf: curseforge::CfState::default(),
            acct: accounts::AcctState::default(),
            shot: Shot::from_env(),
            autoplay: AutoPlay::from_env(),
            frames: 0,
            pending_open: open_files,
            last_account_check: Instant::now() - Duration::from_secs(3600),
        };
        app.refresh_manifest();
        app.ftb_check_updates();
        if app.shot.is_none() {
            app.svc_idle_start();
        }
        app.load_runtimes();
        let interactive =
            app.shot.is_none() && app.autoplay.is_none() && std::env::var_os("BLOCKFORGE_BENCH").is_none();
        if interactive {
            // an AppImage puts its own menu entry and icons in the user's folders (a no-op for the packages)
            crate::desktop_integration::setup(&app.settings);
            app.startup_warnings = startup::collect(&app);
        }
        if interactive {
            app.sync_tray();
        }
        if interactive {
            app.start_update_checks();
        }
        let n = app.notifier();
        app.rt.spawn_blocking(move || n.send(Msg::Java(crate::java::detect_installed())));
        if crate::gpu::supported() {
            let n = app.notifier();
            app.rt.spawn_blocking(move || n.send(Msg::Gpus(crate::gpu::detect())));
        }
        if app.shot.is_none() {
            let n = app.notifier();
            app.rt.spawn(async move {
                let _ = n; // reserved for background update checks
            });
        }
        app
    }

    pub fn notifier(&self) -> Notifier {
        Notifier { tx: self.tx.clone(), ctx: self.ctx.clone() }
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        let text: String = text.into();
        self.console_push(Level::Info, &format!("[BlockForge] {text}"));
        self.toasts.push(Toast { text, error: false, until: Instant::now() + Duration::from_secs(4) });
    }

    pub fn toast_err(&mut self, text: impl Into<String>) {
        let text: String = text.into();
        self.console_push(Level::Error, &format!("[BlockForge] {text}"));
        self.toasts.push(Toast { text, error: true, until: Instant::now() + Duration::from_secs(8) });
    }

    pub fn reload_instances(&mut self) {
        self.instances = instances::load_all(&self.dirs);
        if !self.deleting.is_empty() {
            self.instances.retain(|i| !self.deleting.contains(&i.folder));
        }
    }

    pub fn refresh_manifest(&mut self) {
        let (dirs, n) = (self.dirs.clone(), self.notifier());
        self.rt.spawn(async move {
            let r = crate::install::manifest(&dirs).await.map_err(|e| format!("{e:#}"));
            n.send(Msg::Manifest(r));
        });
    }

    /// Renew the selected Microsoft account's sign-in in the background when it runs out within two hours, so that
    /// pressing Play doesn't wait several seconds for it.
    pub fn renew_account_early(&mut self) {
        let Some(account) =
            self.current_account().filter(|a| !a.must_login && a.needs_refresh_within(2 * 3600)).cloned()
        else {
            return;
        };
        let (dirs, n) = (self.dirs.clone(), self.notifier());
        self.rt.spawn(async move {
            let result = launch::refresh_if_expiring(&dirs, &account, 2 * 3600).await.map_err(|e| format!("{e:#}"));
            n.send(Msg::AccountRefreshed(result));
        });
    }

    pub fn current_account(&self) -> Option<&Account> {
        self.settings
            .last_account
            .as_ref()
            .and_then(|id| self.accounts.iter().find(|a| &a.uuid == id))
            .or_else(|| self.accounts.first())
    }

    /// Show or remove the tray icon to match the setting.
    pub fn sync_tray(&mut self) {
        if self.settings.enable_tray_menu && self.tray.is_none() {
            self.tray = Some(tray::Tray::start(&self.rt, self.notifier()));
        } else if !self.settings.enable_tray_menu {
            if let Some(t) = self.tray.take() {
                t.stop();
            }
        }
    }

    pub fn save_settings(&mut self) {
        self.settings_save_due = None;
        if let Err(e) = self.settings.save(&self.dirs) {
            self.toast_err(format!("Couldn't save settings: {e:#}"));
        }
    }

    /// Save the settings a moment from now: for values that change on every frame while a window is dragged, so the
    /// file is written once the move is over instead of sixty times a second.
    pub fn save_settings_soon(&mut self) {
        self.settings_save_due.get_or_insert_with(|| Instant::now() + Duration::from_secs(1));
    }

    fn flush_settings(&mut self, ctx: &egui::Context) {
        let Some(due) = self.settings_save_due else { return };
        match due.checked_duration_since(Instant::now()) {
            Some(left) if !left.is_zero() => ctx.request_repaint_after(left),
            _ => self.save_settings(),
        }
    }

    /// Ctrl and + (or -) changes the size of everything. The smallest window grows with it, so the pages keep the room
    /// they are laid out for, and zooming in stops where such a window would no longer fit on the screen.
    fn follow_zoom(&mut self, ctx: &egui::Context) {
        let zoom = ctx.zoom_factor();
        if zoom == self.zoom_seen {
            return;
        }
        // both in points at the current zoom
        let (monitor, inner) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().inner_rect));
        if let Some(smaller) = monitor.and_then(|m| zoom_that_fits(zoom, m)) {
            ctx.set_zoom_factor(smaller);
            return; // looked at again with the new zoom on the next frame
        }
        self.zoom_seen = zoom;
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(MIN_WINDOW));
        if let Some(r) = inner.filter(|r| r.width() < MIN_WINDOW.x || r.height() < MIN_WINDOW.y) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(r.size().max(MIN_WINDOW)));
        }
    }

    /// Open a folder or file with the system's own program, and say so when that doesn't work.
    pub fn open_path(&mut self, path: &std::path::Path) {
        if let Err(e) = open::that(path) {
            self.toast_err(format!("Couldn't open {}: {e}", path.display()));
        }
    }

    pub fn save_accounts(&mut self) {
        if let Err(e) = crate::accounts::save(&self.dirs, &self.accounts) {
            self.toast_err(format!("Couldn't save accounts: {e:#}"));
        }
    }

    pub fn instance(&self, folder: &str) -> Option<&Instance> {
        self.instances.iter().find(|i| i.folder == folder)
    }

    pub fn open_instance(&mut self, folder: &str) {
        self.svc_warm(folder);
        self.selected = Some(folder.to_string());
        self.page = Page::Instance;
        self.tab = InstTab::Overview;
        self.inst_draft = None;
    }

    /// Run `work` in the background with a progress bar in the sidebar. A `Some` message is shown when it ends.
    pub fn start_task<F, Fut>(&mut self, label: impl Into<String>, work: F)
    where
        F: FnOnce(Arc<Progress>, Notifier) -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<String>> + Send + 'static,
    {
        let id = self.next_task;
        self.next_task += 1;
        let progress = Progress::new();
        self.tasks.push(Task { id, label: label.into(), progress: progress.clone() });
        let n = self.notifier();
        self.rt.spawn(async move {
            let result = work(progress, n.clone()).await.map_err(|e| format!("{e:#}"));
            n.send(Msg::TaskDone { id, result });
        });
    }

    pub fn is_playing(&self, folder: &str) -> bool {
        self.games.values().any(|g| g.folder == folder && g.active())
    }

    /// The keys of the sessions of one instance (the first one, then those started while it was running).
    pub fn sessions_of(&self, folder: &str) -> Vec<String> {
        let mut keys: Vec<String> =
            self.games.iter().filter(|(_, g)| g.folder == folder).map(|(k, _)| k.clone()).collect();
        keys.sort_by_key(|k| (k != folder, k.clone()));
        keys
    }

    /// The key for a new session: the instance's folder, or when that session is still running, the folder with a
    /// number after it.
    fn new_session_key(&self, folder: &str) -> String {
        let free = |k: &str| !self.games.get(k).is_some_and(Game::active);
        if free(folder) {
            return folder.to_string();
        }
        (2..).map(|n| format!("{folder}\u{1f}{n}")).find(|k| free(k)).expect("there is always a free number")
    }

    /// The instance folder a session key belongs to.
    pub fn folder_of_session(&self, key: &str) -> String {
        self.games
            .get(key)
            .map(|g| g.folder.clone())
            .unwrap_or_else(|| key.split('\u{1f}').next().unwrap_or(key).to_string())
    }

    pub fn play(&mut self, folder: &str, options: launch::LaunchOptions) {
        self.play_as(folder, options, None);
    }

    /// Start an instance. `account` overrides the account to play as (for "play offline with another name").
    pub fn play_as(&mut self, folder: &str, options: launch::LaunchOptions, account: Option<Account>) {
        crate::warmer::stop_for_launch();
        let Some(instance) = self.instance(folder).cloned() else { return };
        if instance.file.launcher.extras.get("isPlayable").and_then(|v| v.as_bool()) == Some(false) {
            self.toast_err("Cannot play instance as it's corrupted. Please reinstall, update or delete it.");
            return;
        }
        // an instance can name the account it wants (stored by username, like the Java edition)
        let own = instance
            .file
            .launcher
            .extras
            .get("account")
            .and_then(|v| v.as_str())
            .and_then(|u| self.accounts.iter().find(|a| a.username.eq_ignore_ascii_case(u)))
            .cloned();
        let Some(account) = account.or(own).or_else(|| self.current_account().cloned()) else {
            // nobody to play as yet: ask once, then the game starts by itself
            self.who_then_play = Some(folder.to_string());
            self.who = Some(whoplays::WhoState::default());
            return;
        };
        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel();
        let key = self.new_session_key(folder);
        self.games.insert(
            key.clone(),
            Game {
                folder: folder.to_string(),
                player: account.name().to_string(),
                fast_start: launch::fast_start_enabled(&self.settings, &instance),
                lite: launch::lite_enabled(&self.settings, &instance),
                task: self.next_task,
                lines: Vec::new(),
                phase: Phase::Preparing,
                kill: Some(kill_tx),
                started: Instant::now(),
            },
        );
        let (dirs, settings, folder_s) = (self.dirs.clone(), self.settings.clone(), key);
        let label = format!("Starting {} as {}", instance.name(), account.name());
        if !self.settings.keep_launcher_open {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        self.start_task(label, move |progress, n| async move {
            let outcome: anyhow::Result<()> = async {
                let account = launch::fresh_account(&dirs, &account).await?;
                let prepared = launch::prepare(&dirs, &settings, &instance, &account, &progress, &options).await?;
                for note in &prepared.notes {
                    n.send(Msg::GameLog(
                        folder_s.clone(),
                        LogLine { text: format!("[BlockForge] {note}"), level: Level::Info },
                    ));
                }
                // settings changed in the player's own file are worth saying out loud, not only in the log
                if prepared.notes.iter().any(|n| n.starts_with(launch::lowend::LOWERED_NOTE)) {
                    n.send(Msg::Toast(
                        "Low-end mode lowered some graphics settings for smoother play. Settings > Performance \
                         switches it off and puts them back."
                            .to_string(),
                        false,
                    ));
                }
                n.send(Msg::GameLog(
                    folder_s.clone(),
                    LogLine { text: format!("[BlockForge] {}", prepared.display()), level: Level::Debug },
                ));
                let mut running = launch::spawn(&prepared)?;
                n.send(Msg::GameRunning(folder_s.clone()));
                let mut instance = instance.clone();
                let _ = instance.mark_played();
                n.send(Msg::Reload);
                let mut assembler = LogAssembler::default();
                let mut kill_rx = kill_rx;
                loop {
                    tokio::select! {
                        line = running.lines.recv() => match line {
                            Some(l) => if let Some(entry) = assembler.push(&l) {
                                n.send(Msg::GameLog(folder_s.clone(), entry));
                            },
                            None => break,
                        },
                        _ = &mut kill_rx => {
                            running.kill_tree().await;
                            break;
                        }
                    }
                }
                let status = running.child.wait().await?;
                prepared.cleanup();
                n.send(Msg::GameExit(folder_s.clone(), status.code()));
                Ok(())
            }
            .await;
            if let Err(e) = outcome {
                n.send(Msg::GameFailed(folder_s, format!("{e:#}")));
            }
            Ok(String::new())
        });
    }

    /// Stop every session of this instance.
    pub fn stop(&mut self, folder: &str) {
        for g in self.games.values_mut().filter(|g| g.folder == folder) {
            if let Some(k) = g.kill.take() {
                let _ = k.send(());
            }
        }
    }

    /// Stop one session, by its key in `games`.
    /// Switch the fast-start helper on or off for one instance (kept in the instance's own file).
    pub fn set_fast_start(&mut self, folder: &str, on: bool) {
        let Some(inst) = self.instances.iter_mut().find(|i| i.folder == folder) else { return };
        inst.file.launcher.extras.insert("fastStart".into(), serde_json::json!(on));
        if let Err(e) = inst.save() {
            self.toast_err(format!("Couldn't save the setting: {e}"));
        }
    }

    pub fn stop_session(&mut self, key: &str) {
        if let Some(k) = self.games.get_mut(key).and_then(|g| g.kill.take()) {
            let _ = k.send(());
        }
    }

    /// Show a picture from the web, fetching it in the background the first time it is asked for. A picture that
    /// couldn't be fetched (no connection yet, a busy server) is tried again a few times, a little later each time;
    /// until it arrives, and when it never does, a coloured letter stands in.
    pub fn remote_image(&mut self, ui: &mut Ui, url: &str, size: f32, fallback: &str) {
        let radius = CornerRadius::same((size * 0.22) as u8);
        // a long list (hundreds of packs) only fetches the pictures that scroll into view
        let visible = ui.is_rect_visible(egui::Rect::from_min_size(ui.cursor().min, vec2(size, size)));
        match self.icons.get(url).copied() {
            Some(IconState::Ready) => {
                ui.add(egui::Image::new(icon_uri(url)).fit_to_exact_size(vec2(size, size)).corner_radius(radius));
                return;
            }
            _ if !visible => {}
            Some(IconState::Failed { at, tries }) if tries < ICON_TRIES => {
                let wait = Duration::from_secs(10 * u64::from(tries) * u64::from(tries));
                match wait.checked_sub(at.elapsed()) {
                    Some(left) => ui.ctx().request_repaint_after(left),
                    None => self.fetch_icon(url, tries),
                }
            }
            Some(_) => {}
            None => self.fetch_icon(url, 0),
        }
        avatar(ui, fallback, size);
    }

    fn fetch_icon(&mut self, url: &str, tries: u8) {
        self.icons.insert(url.to_string(), IconState::Loading { tries });
        let (u, n, dirs) = (url.to_string(), self.notifier(), self.dirs.clone());
        self.rt.spawn(async move {
            let bytes = pictures::fetch(&dirs, &u).await;
            n.send(Msg::Icon { url: u, bytes });
        });
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Reload => self.reload_instances(),
            Msg::Deleted(folder) => self.deleting.retain(|f| *f != folder),
            Msg::InstanceImage(folder, result) => self.instance_image_saved(&folder, result),
            Msg::Manifest(r) => match r {
                Ok(m) => {
                    if self.create.mc.is_empty() {
                        self.create.mc = m.latest.get("release").cloned().unwrap_or_default();
                    }
                    self.manifest = Some(Arc::new(m));
                    self.manifest_error = None;
                    self.request_loader_versions();
                }
                Err(e) => self.manifest_error = Some(e),
            },
            Msg::LoaderVersions { loader, mc, result } => self.create.loader_versions_arrived(loader, &mc, result),
            Msg::Search { seq, result } => self.discover.arrived(seq, result),
            Msg::Icon { url, bytes } => match bytes {
                Some(b) => {
                    self.ctx.include_bytes(icon_uri(&url), b);
                    self.icons.insert(url, IconState::Ready);
                    self.ctx.request_repaint();
                }
                None => {
                    let tries = match self.icons.get(&url) {
                        Some(IconState::Loading { tries }) => tries + 1,
                        _ => 1,
                    };
                    self.icons.insert(url, IconState::Failed { at: Instant::now(), tries });
                }
            },
            Msg::TaskDone { id, result } => {
                self.tasks.retain(|t| t.id != id);
                self.mods_cache = None;
                match result {
                    Ok(m) if !m.is_empty() => self.toast(m),
                    Ok(_) => {}
                    Err(e) => self.toast_err(e),
                }
                self.reload_instances();
            }
            Msg::GameRunning(f) => {
                if std::env::var_os("BLOCKFORGE_AUTOPLAY").is_some() {
                    if let Some(g) = self.games.get(&f) {
                        eprintln!("play pressed -> game process started: {} ms", g.started.elapsed().as_millis());
                    }
                }
                if let Some(g) = self.games.get_mut(&f) {
                    // the "Starting ..." progress card is done once the game is up (the task itself goes on until
                    // the game closes)
                    let task = g.task;
                    self.tasks.retain(|t| t.id != task);
                    g.phase = Phase::Running;
                    g.started = Instant::now();
                }
            }
            Msg::GameLog(f, line) => {
                self.console_push(line.level, &line.text);
                if let Some(g) = self.games.get_mut(&f) {
                    for part in line.text.split('\n') {
                        g.lines.push(LogLine {
                            text: part.trim_end_matches('\r').replace('\t', "    "),
                            level: line.level,
                        });
                    }
                    if g.lines.len() > MAX_LOG_LINES {
                        g.lines.drain(..MAX_LOG_LINES / 10);
                    }
                }
            }
            Msg::GameExit(f, code) => {
                if let Some(g) = self.games.get_mut(&f) {
                    g.phase = Phase::Exited(code);
                    g.kill = None;
                }
                self.helpers_after_exit(&f, code);
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                let folder = self.folder_of_session(&f);
                self.svc_after_game_exit(&folder);
                if let Some(c) = code.filter(|c| *c != 0) {
                    self.toast_err(format!("Minecraft closed with an error (code {c}). The Logs tab has details."));
                }
                self.reload_instances();
            }
            Msg::GameFailed(f, e) => {
                if let Some(g) = self.games.get_mut(&f) {
                    g.phase = Phase::Failed;
                    g.kill = None;
                    g.lines.push(LogLine { text: format!("[BlockForge] {e}"), level: Level::Error });
                }
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                self.toast_err(e);
            }
            // a sign-in that was cancelled, or replaced by a newer one, is left alone
            Msg::Device(attempt, _) | Msg::SignedIn(attempt, _)
                if !self.acct.signing_in || attempt != self.acct.attempt => {}
            Msg::Device(attempt, r) => match r {
                Ok(code) => {
                    let (c2, n) = (code.clone(), self.notifier());
                    self.rt.spawn(async move {
                        let r = crate::msauth::finish_device_login(&c2).await.map_err(|e| format!("{e:#}"));
                        n.send(Msg::SignedIn(attempt, r));
                    });
                    let _ = open::that(&code.verification_uri);
                    self.acct.device = Some(code);
                }
                Err(e) => {
                    self.acct.signing_in = false;
                    self.toast_err(e);
                }
            },
            Msg::SignedIn(_, r) => {
                self.acct.signing_in = false;
                self.acct.device = None;
                match r {
                    Ok(acc) => {
                        self.accounts.retain(|a| a.uuid != acc.uuid);
                        self.toast(format!("Signed in as {}", acc.name()));
                        self.settings.last_account = Some(acc.uuid.clone());
                        self.accounts.push(acc);
                        self.save_accounts();
                        self.save_settings();
                    }
                    Err(e) => self.toast_err(e),
                }
            }
            Msg::Java(list) => self.java_list = list,
            Msg::Gpus(list) => self.gpus = Some(list),
            Msg::Runtimes(list) => self.runtimes = list,
            Msg::AccountRefreshed(result) => match result {
                Ok(fresh) => {
                    if let Some(slot) = self.accounts.iter_mut().find(|a| a.uuid == fresh.uuid) {
                        *slot = fresh;
                    }
                }
                // not worth a message now; pressing Play tries again and says what is wrong
                Err(e) => self
                    .console_push(Level::Warn, &format!("[BlockForge] Couldn't renew the sign-in ahead of time: {e}")),
            },
            Msg::Svc(m) => self.svc_msg(m),
            Msg::Mods(m) => self.mods_handle(m),
            Msg::Packs(m) => self.packs_msg(m),
            Msg::Craft(m) => self.craft_msg(m),
            #[cfg(feature = "converter")]
            Msg::Convert(m) => self.convert_msg(m),
            Msg::Picked(p, path) => self.picked(p, path),
            Msg::DialogLoaderVersions { loader, mc, result } => self.dialog_loader_versions(loader, &mc, result),
            Msg::Tray(cmd) => match cmd {
                tray::TrayCmd::Toggle => {
                    self.window_visible = !self.window_visible;
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Visible(self.window_visible));
                    if self.window_visible {
                        self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                }
                tray::TrayCmd::KillMinecraft => {
                    let folders: Vec<String> =
                        self.games.iter().filter(|(_, g)| g.active()).map(|(f, _)| f.clone()).collect();
                    for f in folders {
                        self.stop_session(&f);
                    }
                }
                tray::TrayCmd::ToggleConsole => {
                    self.settings.enable_console = !self.settings.enable_console;
                    self.save_settings();
                }
                tray::TrayCmd::OpenFolder => {
                    let _ = open::that(&self.dirs.base);
                }
                tray::TrayCmd::Quit => self.ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            },
            Msg::LogUploaded(r) => {
                self.console_uploading = false;
                match r {
                    Ok(url) => {
                        self.ctx.copy_text(url.clone());
                        self.toast(format!("Log uploaded and link copied to clipboard: {url}"));
                    }
                    Err(e) => self.toast_err(format!("Log failed to upload: {e}")),
                }
            }
            Msg::Update(m) => self.handle_update(m),
            Msg::Toast(t, err) => {
                if err {
                    self.toast_err(t)
                } else {
                    self.toast(t)
                }
            }
        }
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add(egui::Image::from_texture(&self.logo).fit_to_exact_size(vec2(38.0, 38.0)));
            ui.vertical(|ui| {
                ui.add_space(1.0);
                ui.label(RichText::new(crate::NAME).size(19.0).strong());
                small_muted(ui, format!("v{}", crate::VERSION));
            });
        });
        ui.add_space(14.0);
        let items = [
            (Page::Home, "Home", Icon::Home),
            (Page::Library, "Library", Icon::Library),
            (Page::Discover, "Discover", Icon::Search),
            (Page::Packs, "Packs", Icon::Library),
            (Page::Ftb, "FTB", Icon::Library),
            (Page::Create, "Create", Icon::Plus),
            (Page::Accounts, "Accounts", Icon::User),
            (Page::Tools, "Tools", Icon::Sliders),
            (Page::Craft, "Crafting", Icon::Craft),
            (Page::Servers, "Servers", Icon::Folder),
            (Page::Settings, "Settings", Icon::Sliders),
            (Page::About, "About", Icon::Info),
        ];
        // the account card and the running downloads need their room at the bottom; the list scrolls if the window
        // is too short for all the items
        let reserve = 84.0 + self.tasks.len().min(3) as f32 * 100.0;
        egui::ScrollArea::vertical()
            .id_salt("nav")
            .max_height((ui.available_height() - reserve).max(120.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                for (page, label, icon) in items {
                    let active = self.page == page || (page == Page::Library && self.page == Page::Instance);
                    #[cfg(feature = "converter")]
                    let active = active || (page == self.convert_nav_page() && self.page == Page::Convert);
                    if self.nav_button(ui, icon, label, active).clicked() {
                        self.page = page;
                    }
                }
            });

        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            // who is playing
            let account = self.current_account().map(|a| (a.name().to_string(), a.kind(), a.uuid.clone(), a.offline));
            let resp = egui::Frame::new()
                .fill(c_card())
                .stroke(Stroke::new(1.0, c_border()))
                .corner_radius(CornerRadius::same(14))
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| match &account {
                        Some((name, kind, uuid, offline)) => {
                            self.account_avatar(ui, uuid, name, *offline, 34.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(name).strong());
                                small_muted(ui, *kind);
                            });
                        }
                        None => {
                            avatar(ui, "?", 34.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new("No account").strong());
                                small_muted(ui, "Click to add one");
                            });
                        }
                    });
                });
            if resp.response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.page = Page::Accounts;
            }
            ui.add_space(6.0);
            // running downloads
            for t in self.tasks.iter().take(3) {
                card(ui, |ui| {
                    ui.label(RichText::new(&t.label).strong().size(13.0));
                    let stage = t.progress.stage_text();
                    if !stage.is_empty() {
                        small_muted(ui, stage);
                    }
                    let (d, total) =
                        (t.progress.files_done.load(Ordering::Relaxed), t.progress.files_total.load(Ordering::Relaxed));
                    let bar = egui::ProgressBar::new(t.progress.fraction())
                        .desired_height(8.0)
                        .corner_radius(4)
                        .fill(c_accent());
                    if total > 0 {
                        ui.add(bar);
                        small_muted(
                            ui,
                            format!(
                                "{d} / {total} files  \u{00B7}  {}",
                                human_size(t.progress.bytes_done.load(Ordering::Relaxed))
                            ),
                        );
                    } else {
                        ui.add(bar.animate(true));
                    }
                });
                ui.add_space(6.0);
            }
        });
    }

    fn nav_button(&self, ui: &mut Ui, icon: Icon, label: &str, active: bool) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), egui::Sense::click());
        let bg = if active {
            c_accent_soft()
        } else if resp.hovered() {
            c_card()
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(11), bg);
        if active {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.left_top() + vec2(0.0, 10.0), vec2(3.0, 22.0)),
                CornerRadius::same(2),
                c_accent(),
            );
        }
        let color = if active { c_accent() } else { c_text() };
        icons::paint(ui.painter(), icon, rect.left_center() + vec2(28.0, 0.0), 18.0, color);
        ui.painter().text(
            rect.left_center() + vec2(46.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(15.0),
            color,
        );
        focus_ring(ui, &resp, rect, 11);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, active, label));
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    fn toasts_ui(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.toasts.retain(|t| t.until > now);
        if self.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(500));
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                for t in self.toasts.iter().rev().take(3) {
                    let (bg, fg) = if t.error { (c_danger_bg(), c_danger()) } else { (c_card_hover(), c_text()) };
                    egui::Frame::new()
                        .fill(bg)
                        .stroke(Stroke::new(1.0, if t.error { c_danger().gamma_multiply(0.5) } else { c_border() }))
                        .corner_radius(CornerRadius::same(12))
                        .inner_margin(egui::Margin::symmetric(16, 10))
                        .shadow(egui::Shadow {
                            offset: [0, 6],
                            blur: 18,
                            spread: 0,
                            color: egui::Color32::from_black_alpha(110),
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(560.0);
                            ui.label(RichText::new(&t.text).color(fg));
                        });
                    ui.add_space(6.0);
                }
            });
    }

    fn confirm_ui(&mut self, ctx: &egui::Context) {
        let Some(folder) = self.confirm_delete.clone() else { return };
        let Some(name) = self.instance(&folder).map(|i| i.name().to_string()) else {
            self.confirm_delete = None;
            return;
        };
        let playing = self.is_playing(&folder);
        let mut close = false;
        let mut delete = false;
        let modal = egui::Modal::new(egui::Id::unique("confirm-delete")).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(format!("Delete \"{name}\"?")).size(19.0).strong());
            ui.add_space(4.0);
            if playing {
                ui.label(
                    RichText::new("This instance is being played. Close the game first, then delete it.")
                        .color(c_warn()),
                );
            } else {
                muted(ui, "Its worlds, mods and settings will be removed from this computer. This can't be undone.");
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if danger_enabled(ui, "Delete", !playing).clicked() {
                    delete = true;
                }
                if secondary(ui, "Cancel").clicked() {
                    close = true;
                }
            });
        });
        if modal.should_close() {
            close = true;
        }
        if delete && !playing {
            if let Some(inst) = self.instance(&folder).cloned() {
                if self.selected.as_deref() == Some(&folder) {
                    self.page = Page::Library;
                    self.selected = None;
                }
                // a big instance takes a while to remove: it disappears from the lists now, and the folder goes in
                // the background (a failure brings it back with the next reload and says why)
                self.deleting.push(folder.clone());
                self.instances.retain(|i| i.folder != folder);
                self.start_task(format!("Deleting {name}"), move |_, n| async move {
                    let result = tokio::task::spawn_blocking(move || instances::delete(&inst)).await;
                    n.send(Msg::Deleted(folder));
                    result??;
                    Ok(format!("Deleted {name}"))
                });
            }
            close = true;
        }
        if close {
            self.confirm_delete = None;
        }
    }
}

/// The zoom to go back to when the smallest window at `zoom` would not fit on a screen of `monitor` points (measured at
/// that zoom), or `None` when it fits. Zooming out (below 1) is always allowed.
fn zoom_that_fits(zoom: f32, monitor: egui::Vec2) -> Option<f32> {
    let fits = zoom * (0.95 * monitor.x / MIN_WINDOW.x).min(0.95 * monitor.y / MIN_WINDOW.y);
    (zoom > 1.0 && zoom > fits + 0.01).then(|| fits.max(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zooming_in_stops_where_the_smallest_window_still_fits() {
        // a 1920 x 1080 screen at zoom 1: 2x needs a 1720 x 1120 window, too tall
        let at = |zoom: f32| vec2(1920.0, 1080.0) / zoom;
        let back = zoom_that_fits(2.0, at(2.0)).unwrap();
        assert!((back - 1.832).abs() < 0.01, "{back}");
        // once there, it stays (no small steps frame after frame)
        assert_eq!(zoom_that_fits(back, at(back)), None);
        assert_eq!(zoom_that_fits(1.5, at(1.5)), None);
        // smaller is always fine, and a tiny screen never forces the zoom below 1
        assert_eq!(zoom_that_fits(0.75, at(0.75)), None);
        assert_eq!(zoom_that_fits(1.3, vec2(800.0, 500.0) / 1.3), Some(1.0));
    }
}

/// Computers with 4 GB of memory or less get the lighter look by default, like the Java edition.
pub fn low_end_pc() -> bool {
    // asked on every frame of the Settings page; the memory a computer has doesn't change, so it is read once
    static LOW: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LOW.get_or_init(|| {
        let total = crate::platform::memory_mb().0;
        total > 0 && total <= 4096
    })
}

/// The frame every dialog uses: the window look with room around the content.
pub fn modal_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(24))
}

/// How far a picture from the web has got.
#[derive(Clone, Copy)]
enum IconState {
    Loading { tries: u8 },
    Ready,
    Failed { at: Instant, tries: u8 },
}

/// How often a picture that doesn't arrive is asked for before giving up until the next start.
const ICON_TRIES: u8 = 4;

pub fn icon_uri(url: &str) -> String {
    format!("bytes://remote/{}", crate::http::sha1_hex(url.as_bytes()))
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(m) = self.rx.try_recv() {
            self.handle(m);
        }
        // modpacks and instance folders dropped on the window open the import window
        let dropped: Vec<std::path::PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        // a world dropped on the Convert page is the world to convert
        #[cfg(feature = "converter")]
        let dropped: Vec<std::path::PathBuf> = dropped.into_iter().filter(|p| !self.convert_handle_drop(p)).collect();
        // a PNG dropped on the skin window is the new skin; everything else goes to the import flow
        let dropped: Vec<std::path::PathBuf> =
            dropped.into_iter().filter(|p| !self.svc_handle_drop(p) && !self.mods_accept_drop(p)).collect();
        if !dropped.is_empty() {
            self.handle_dropped_files(dropped);
        }
        self.svc_frame();
        self.follow_zoom(ctx);
        // progress bars read counters that send no message, so they are looked at a few times a second. A running
        // game needs nothing of the sort: its log lines wake the window themselves, so a long session costs no CPU.
        if !self.tasks.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        self.flush_settings(ctx);
        self.craft_tick(ctx);
        self.frames += 1;
        if self.last_account_check.elapsed() > Duration::from_secs(600)
            && self.shot.is_none()
            && self.autoplay.is_none()
        {
            self.last_account_check = Instant::now();
            self.renew_account_early();
        }
        // wake up now and then even when nothing is happening, to look at the sign-in again
        ctx.request_repaint_after(Duration::from_secs(600));
        if self.frames == 3 && !self.pending_open.is_empty() {
            let files = std::mem::take(&mut self.pending_open);
            self.handle_dropped_files(files);
        }
        if self.frames == 2 && std::env::var_os("BLOCKFORGE_BENCH").is_some() {
            if let Some(t) = crate::START.get() {
                eprintln!("first frame drawn {:.0} ms after start", t.elapsed().as_secs_f64() * 1000.0);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.settings.remember_window_size_position && self.last_window_save.elapsed() > Duration::from_secs(2) {
            let geom = ctx.input(|i| {
                let v = i.viewport();
                match (v.inner_rect, v.outer_rect) {
                    (Some(inner), Some(outer)) => Some((
                        crate::config::Size { width: inner.width() as i32, height: inner.height() as i32 },
                        crate::config::Pos { x: outer.min.x as i32, y: outer.min.y as i32 },
                    )),
                    _ => None,
                }
            });
            if geom.is_some() && geom != self.last_window {
                self.last_window = geom;
                self.last_window_save = Instant::now();
                if let Some((size, pos)) = geom {
                    self.settings.launcher_size = Some(size);
                    self.settings.launcher_position = Some(pos);
                    self.save_settings();
                }
            }
        }
        if self.autoplay.is_some() {
            self.drive_autoplay(ctx);
        } else {
            self.drive_shot(ctx);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Panel::left("nav")
            .exact_size(236.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(c_surface())
                    .inner_margin(egui::Margin::same(16))
                    .stroke(Stroke::new(1.0, c_border())),
            )
            .show(ui, |ui| self.sidebar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(c_bg()).inner_margin(egui::Margin::symmetric(30, 24)))
            .show(ui, |ui| {
                self.update_banner(ui);
                match self.page {
                    Page::Home => self.home_ui(ui),
                    Page::Library => self.library_ui(ui),
                    Page::Instance => self.instance_ui(ui),
                    Page::Discover => self.discover_ui(ui),
                    Page::Packs => self.packs_ui(ui),
                    Page::Ftb => self.ftb_ui(ui),
                    Page::Create => self.create_ui(ui),
                    Page::Accounts => self.accounts_ui(ui),
                    Page::Settings => self.settings_ui(ui),
                    Page::Tools => self.tools_ui(ui),
                    Page::Craft => self.craft_ui(ui),
                    #[cfg(feature = "converter")]
                    Page::Convert => self.convert_ui(ui),
                    Page::Servers => self.servers_ui(ui),
                    Page::About => self.about_ui(ui),
                }
            });
        self.import_modal(&ctx);
        self.export_modal(&ctx);
        self.toasts_ui(&ctx);
        self.confirm_ui(&ctx);
        self.svc_overlays(&ctx);
        self.cf_windows(&ctx);
        self.poll_ftb();
        self.rename_ui(&ctx);
        self.instance_dialogs_ui(&ctx);
        self.startup_ui(&ctx);
        if self.startup_warnings.is_empty() {
            self.who_ui(&ctx);
        }
        self.console_ui(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.craft_flush_now();
        #[cfg(feature = "converter")]
        self.convert_on_exit();
        // a window moved just before closing still has its new place written
        if self.settings_save_due.is_some() {
            let _ = self.settings.save(&self.dirs);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// Screenshots: `BLOCKFORGE_SHOT=<folder>` renders every page to a PNG and quits. Used to check the look
// without clicking through the window by hand.

struct Shot {
    dir: std::path::PathBuf,
    /// The pages to picture: `BLOCKFORGE_SHOT_PAGES=home,library-light` picks some (a `-light` ending shows the
    /// page in the light look), otherwise all of [`SHOT_PAGES`].
    pages: Vec<String>,
    /// `BLOCKFORGE_SHOT_ZOOM=1.5`: the size of everything, like Ctrl and + does.
    zoom: Option<f32>,
    /// `BLOCKFORGE_SHOT_SIZE=860x560`: the window's size.
    size: Option<egui::Vec2>,
    step: usize,
    frames: u32,
    waiting: Arc<std::sync::atomic::AtomicBool>,
}

const SHOT_PAGES: [&str; 34] = [
    "home",
    "library",
    "instance",
    "instance-mods",
    "instance-logs",
    "instance-settings",
    "instance-settings-java",
    "instance-settings-cmd",
    "discover",
    "create",
    "accounts",
    "settings",
    "create-fabric",
    "tools",
    "servers",
    "craft",
    "craft-steps",
    "craft-tree",
    "craft-storage",
    "craft-stock",
    "craft-get-light",
    "convert",
    "convert-progress",
    "convert-done",
    "convert-error",
    "convert-instance",
    "convert-instance-done",
    "packs",
    "ftb",
    "about",
    "who",
    "settings-light",
    "home-light",
    "mods-light",
];

impl Shot {
    fn from_env() -> Option<Shot> {
        let dir = std::env::var_os("BLOCKFORGE_SHOT")?;
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let pages = match std::env::var("BLOCKFORGE_SHOT_PAGES") {
            Ok(list) => list.split(',').map(str::trim).filter(|p| !p.is_empty()).map(String::from).collect(),
            Err(_) => SHOT_PAGES.iter().map(|p| p.to_string()).collect(),
        };
        let zoom = std::env::var("BLOCKFORGE_SHOT_ZOOM").ok().and_then(|z| z.parse().ok());
        let size = std::env::var("BLOCKFORGE_SHOT_SIZE").ok().and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some(vec2(w.parse().ok()?, h.parse().ok()?))
        });
        Some(Shot {
            dir,
            pages,
            zoom,
            size,
            step: 0,
            frames: 0,
            waiting: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
}

impl App {
    fn drive_shot(&mut self, ctx: &egui::Context) {
        let Some(shot) = self.shot.as_mut() else { return };
        ctx.request_repaint();
        if shot.waiting.load(Ordering::Relaxed) {
            return;
        }
        shot.frames += 1;
        let step = shot.step;
        let Some(full_name) = shot.pages.get(step).cloned() else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        };
        if step == 0 && shot.frames == 1 {
            if let Some(z) = shot.zoom {
                ctx.set_zoom_factor(z);
            }
            if let Some(size) = shot.size {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
        }
        let name = full_name.as_str();
        if shot.frames == 1 {
            let first = self.instances.first().map(|i| i.folder.clone());
            if name != "who" {
                self.who = None;
            }
            self.confirm_delete = None;
            self.rename_target = None;
            self.desc_edit = None;
            self.offline_play = None;
            self.loader_dialog = None;
            self.acct.confirm_remove = None;
            // a page name ending in "-light" is pictured in the light look, any other in the dark one
            let light = name.ends_with("-light");
            if light != !theme::is_dark() {
                theme::set_dark(!light);
                theme::apply(ctx, false);
            }
            match name.trim_end_matches("-light") {
                "home" => self.page = Page::Home,
                "library" => self.page = Page::Library,
                "instance"
                | "instance-mods"
                | "instance-logs"
                | "instance-settings"
                | "instance-settings-java"
                | "instance-settings-cmd" => {
                    if let Some(f) = first {
                        self.open_instance(&f);
                        self.tab = match name {
                            "instance-mods" => InstTab::Mods,
                            "instance-logs" => InstTab::Logs,
                            "instance-settings" | "instance-settings-java" | "instance-settings-cmd" => {
                                InstTab::Settings
                            }
                            _ => InstTab::Overview,
                        };
                        match name {
                            "instance-settings" => self.shot_instance_settings_tab(&f, 0),
                            "instance-settings-java" => self.shot_instance_settings_tab(&f, 1),
                            "instance-settings-cmd" => self.shot_instance_settings_tab(&f, 2),
                            _ => {}
                        }
                        if name == "instance-logs" {
                            self.games.insert(
                                f.clone(),
                                Game {
                                    folder: f,
                                    player: "Player".into(),
                                    fast_start: false,
                                    lite: false,
                                    task: 0,
                                    lines: sample_log(),
                                    phase: Phase::Exited(Some(0)),
                                    kill: None,
                                    started: Instant::now(),
                                },
                            );
                        }
                    }
                }
                "discover" => {
                    self.page = Page::Discover;
                    self.discover.query = "sodium".into();
                    self.discover_search(true);
                }
                "create" => self.page = Page::Create,
                "create-fabric" => {
                    self.page = Page::Create;
                    self.create.name = "My Fabric world".into();
                    self.create.loader = Loader::Fabric;
                    self.request_loader_versions();
                }
                "accounts" => self.page = Page::Accounts,
                "settings" => self.page = Page::Settings,
                "who" => {
                    self.page = Page::Home;
                    theme::set_dark(true);
                    theme::apply(ctx, false);
                    self.settings.theme = crate::config::DEFAULT_THEME.into();
                    self.who = Some(whoplays::WhoState::default());
                }
                "tools" => self.page = Page::Tools,
                "servers" => self.page = Page::Servers,
                v if v.starts_with("craft") => self.shot_craft(v.trim_end_matches("-light")),
                #[cfg(feature = "converter")]
                v if v.starts_with("convert") => self.shot_convert(v),
                "packs" => self.page = Page::Packs,
                "ftb" => self.page = Page::Ftb,
                "about" => self.page = Page::About,
                "mods" => {
                    if let Some(f) = first {
                        self.open_instance(&f);
                        self.tab = InstTab::Mods;
                    }
                }
                // the dialogs
                "delete" => {
                    self.page = Page::Library;
                    self.confirm_delete = first;
                }
                "rename" => {
                    self.page = Page::Library;
                    self.rename_target = first.map(|f| (f, "A new name".into()));
                }
                "description" => {
                    self.page = Page::Library;
                    self.desc_edit = first.map(|f| (f, "A few words about this instance.".into()));
                }
                "offline" => {
                    self.page = Page::Library;
                    self.offline_play = first.map(|f| (f, "ab".into(), true));
                }
                "loader" => {
                    self.page = Page::Library;
                    if let Some(f) = first {
                        self.apply(instance_menu::Act::ChangeLoader(f));
                    }
                }
                "who-offline" => {
                    self.page = Page::Home;
                    self.who = Some(whoplays::WhoState::offline_step());
                }
                "accounts-remove" => {
                    self.page = Page::Accounts;
                    self.acct.confirm_remove = self.accounts.first().map(|a| a.uuid.clone());
                }
                "library-empty" => {
                    self.page = Page::Library;
                    self.library_search = "no instance is called this".into();
                }
                // keys pressed by the tour itself (below): the picture shows what they did
                "tab-focus" => self.page = Page::Home,
                "rename-esc" => {
                    self.page = Page::Library;
                    self.rename_target = first.map(|f| (f, "A new name".into()));
                }
                "delete-esc" => {
                    self.page = Page::Library;
                    self.confirm_delete = first;
                }
                "offline-enter" => {
                    self.page = Page::Library;
                    self.offline_play = first.map(|f| (f, "ab".into(), false));
                }
                _ => {}
            }
        }
        let frame = self.shot.as_ref().map_or(0, |s| s.frames);
        let press = |key: egui::Key| {
            let event = egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            };
            ctx.input_mut(|i| i.events.push(event));
        };
        match name.trim_end_matches("-light") {
            "tab-focus" if matches!(frame, 5 | 8 | 11) => ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::Next)),
            "rename-esc" | "delete-esc" if frame == 10 => press(egui::Key::Escape),
            "offline-enter" if frame == 10 => press(egui::Key::Enter),
            _ => {}
        }
        let wait = if matches!(
            name.trim_end_matches("-light"),
            "discover" | "create-fabric" | "instance-mods" | "mods" | "packs" | "ftb" | "about" | "loader"
        ) || name.trim_end_matches("-light").starts_with("craft")
        {
            180
        } else {
            30
        };
        let shot = self.shot.as_mut().unwrap();
        if shot.frames == wait {
            let path = shot.dir.join(format!("{name}.png"));
            let flag = shot.waiting.clone();
            flag.store(true, Ordering::Relaxed);
            ctx.request_screenshot(move |img| {
                let mut raw = Vec::with_capacity(img.pixels.len() * 4);
                for p in &img.pixels {
                    raw.extend_from_slice(&p.to_array());
                }
                let _ =
                    image::save_buffer(&path, &raw, img.size[0] as u32, img.size[1] as u32, image::ColorType::Rgba8);
                flag.store(false, Ordering::Relaxed);
            });
            shot.step += 1;
            shot.frames = 0;
        }
    }
}

fn sample_log() -> Vec<LogLine> {
    let l = |level, t: &str| LogLine { level, text: t.to_string() };
    vec![
        l(
            Level::Info,
            "[BlockForge] Java: runtimes/minecraft/java-runtime-delta/linux/java-runtime-delta (version 21)",
        ),
        l(Level::Info, "[BlockForge] Memory: 4096 MB, picked automatically (this computer has 29914 MB, 9227 MB free)"),
        l(
            Level::Info,
            "[12:04:31] [main/INFO] (FabricLoader/GameProvider): Loading Minecraft 1.21.1 with Fabric Loader 0.20.0",
        ),
        l(Level::Info, "[12:04:31] [main/INFO] (FabricLoader): Loading 12 mods: lithium 0.15.4, sodium 0.8.13"),
        l(
            Level::Warn,
            "[12:04:33] [Render thread/c_warn()] (Sodium): Sodium has applied one or more workarounds to prevent crashes",
        ),
        l(Level::Info, "[12:04:35] [Render thread/INFO] (Minecraft): Backend library: LWJGL version 3.3.3-snapshot"),
        l(
            Level::Error,
            "[12:04:36] [Render thread/ERROR] (Minecraft): Failed to load sound event minecraft:block.amethyst_block.step",
        ),
        l(Level::Info, "[12:04:38] [Render thread/INFO] (Minecraft): Sound engine started"),
    ]
}

// Press Play on one instance through the same code the Play button uses, wait for the game to talk, take a
// picture of the log, then stop it. `BLOCKFORGE_AUTOPLAY=<instance folder>` with `BLOCKFORGE_SHOT=<folder>`.

struct AutoPlay {
    folder: String,
    frame: u32,
    asked_stop: bool,
    shot_taken: Arc<std::sync::atomic::AtomicBool>,
}

impl AutoPlay {
    fn from_env() -> Option<AutoPlay> {
        let folder = std::env::var("BLOCKFORGE_AUTOPLAY").ok()?;
        Some(AutoPlay {
            folder,
            frame: 0,
            asked_stop: false,
            shot_taken: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
}

impl App {
    fn drive_autoplay(&mut self, ctx: &egui::Context) {
        ctx.request_repaint_after(Duration::from_millis(100));
        let (folder, frame) = {
            let a = self.autoplay.as_mut().unwrap();
            a.frame += 1;
            (a.folder.clone(), a.frame)
        };
        if frame == 3 {
            self.open_instance(&folder);
            self.tab = InstTab::Logs;
            // `BLOCKFORGE_AUTOPLAY_PLAYERS=Alpha,Beta` starts the instance once for each offline name, all together
            match std::env::var("BLOCKFORGE_AUTOPLAY_PLAYERS") {
                Ok(names) => {
                    for name in names.split(',').filter(|n| !n.is_empty()) {
                        self.play_as(&folder, launch::LaunchOptions::default(), Some(Account::offline(name)));
                    }
                }
                Err(_) => self.play(&folder, launch::LaunchOptions::default()),
            }
        }
        let (lines, phase) = match self.games.get(&folder) {
            Some(g) => (g.lines.len(), g.phase),
            None => return,
        };
        let a = self.autoplay.as_mut().unwrap();
        let dir = std::env::var_os("BLOCKFORGE_SHOT").map(std::path::PathBuf::from);
        // enough output (or a long wait): picture, then stop the game
        if !a.asked_stop
            && ((lines >= 50
                && phase == Phase::Running
                && (frame > 450 || std::env::var_os("BLOCKFORGE_AUTOPLAY_PLAYERS").is_none()))
                || frame > 900
                || matches!(phase, Phase::Exited(_) | Phase::Failed))
        {
            if let Some(dir) = dir {
                let _ = std::fs::create_dir_all(&dir);
                let path = dir.join("autoplay.png");
                let flag = a.shot_taken.clone();
                ctx.request_screenshot(move |img| {
                    let mut raw = Vec::with_capacity(img.pixels.len() * 4);
                    for p in &img.pixels {
                        raw.extend_from_slice(&p.to_array());
                    }
                    let _ = image::save_buffer(
                        &path,
                        &raw,
                        img.size[0] as u32,
                        img.size[1] as u32,
                        image::ColorType::Rgba8,
                    );
                    flag.store(true, Ordering::Relaxed);
                });
            } else {
                a.shot_taken.store(true, Ordering::Relaxed);
            }
            a.asked_stop = true;
            eprintln!(
                "autoplay: {lines} log lines, phase {}",
                match phase {
                    Phase::Preparing => "preparing".to_string(),
                    Phase::Running => "running".to_string(),
                    Phase::Exited(c) => format!("exited {c:?}"),
                    Phase::Failed => "failed".to_string(),
                }
            );
        }
        if a.asked_stop && a.shot_taken.load(Ordering::Relaxed) {
            if matches!(phase, Phase::Exited(_) | Phase::Failed) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                self.stop(&folder);
            }
        }
    }
}
