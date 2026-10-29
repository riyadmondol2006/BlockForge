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
mod discover_both;
mod export;
mod fonts;
mod ftb;
mod github_add;
mod icons;
mod import;
mod instance_icon;
mod instance_menu;
mod instance_settings;
mod library;
mod mods;
mod motion;
mod packs;
mod performance;
mod pictures;
mod settings;
mod startup;
mod strip;
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

/// What "Stop Minecraft?" says, wherever it is asked (the instance page, the sidebar, the log window).
pub(super) const STOP_WARNING: &str = "Minecraft is stopped at once. A world that wasn't saved can be damaged. To close \
                                        it safely, use Save and Quit in the game.";

/// The smallest and largest interface size kept in the settings (a value outside, typed into the file, is ignored).
const MIN_ZOOM: f32 = 0.5;
const MAX_ZOOM: f32 = 3.0;

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
    /// CurseForge's half of a search on both sites.
    SearchCf {
        seq: u64,
        reset: bool,
        result: Result<crate::curseforge::api::SearchPage, String>,
    },
    /// Smithed's part of a search for data packs on every site.
    SearchSmithed {
        seq: u64,
        reset: bool,
        result: Result<(Vec<crate::smithed::Pack>, u64), String>,
    },
    /// A GitHub repository's releases, for "Add from GitHub".
    GithubReleases {
        seq: u64,
        result: Result<Vec<crate::github::Release>, String>,
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
    /// The desktop has no tray (GNOME without an extension): why, for the log.
    NoTray(String),
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

/// How soon the window shows what came in by [`Notifier::send_soon`].
const SOON: Duration = Duration::from_millis(100);

impl Notifier {
    pub fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
        self.ctx.request_repaint();
    }

    /// Like [`Notifier::send`], for what comes in streams (a game's log lines): the window shows it within [`SOON`]
    /// instead of at once. A modpack that logs thousands of lines while it loads kept the launcher drawing at the
    /// screen's full rate, taking processor time from the game exactly when it needed it most.
    pub fn send_soon(&self, m: Msg) {
        let _ = self.tx.send(m);
        self.ctx.request_repaint_after(SOON);
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

/// How the game was asked to start, kept while "Save the settings first?" is up.
#[derive(Clone)]
pub enum PendingPlay {
    Play,
    As(Option<Account>),
    Join(String),
}

struct Toast {
    text: String,
    error: bool,
    until: Instant,
    /// An error that came while the player was away from the window: it waits for them, however long.
    held: bool,
    /// How many times this same message came in a row ("×3" after it).
    count: u32,
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
    /// How tall the sidebar's bottom part (account, Settings, downloads, games) was last frame, so the list above
    /// stops before it.
    sidebar_bottom: f32,
    tasks: Vec<Task>,
    next_task: u64,
    games: HashMap<String, Game>,
    manifest: Option<Arc<Manifest>>,
    manifest_error: Option<String>,
    icons: HashMap<String, IconState>,
    local_pictures: instance_icon::LocalPictures,
    play_cache: strip::PlayCache,
    java_list: Vec<JavaInstall>,
    /// The graphics cards of this computer, once looked for.
    gpus: Option<Vec<crate::gpu::Gpu>>,
    logo: egui::TextureHandle,
    confirm_delete: Option<String>,
    /// A Stop waiting to be confirmed: an instance folder, or a session key when `.1` is true.
    confirm_stop: Option<(String, bool)>,
    /// The window was asked to close while games or jobs were running: "Quit anyway?" is showing.
    confirm_quit: bool,
    /// The player said to quit anyway (or the launcher closes itself to finish an update): close without asking.
    quit_anyway: bool,
    /// Instances whose folders are being deleted in the background; they are left out of the lists meanwhile.
    deleting: Vec<String>,
    quick_server: String,
    rename_target: Option<(String, String)>,
    inst_draft: Option<instance_settings::InstDraft>,
    /// Instance folders whose instance.json can't be read; the Library says so.
    unreadable: Vec<std::path::PathBuf>,
    /// Settings drafts with changes that weren't saved, kept for their instance while another one is open.
    kept_drafts: HashMap<String, instance_settings::InstDraft>,
    /// Play was pressed for an instance whose settings were changed and not saved: "Save first?" is showing.
    confirm_play_unsaved: Option<(String, PendingPlay)>,
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
    /// A tray that can't start says so in the log only: at every start on a desktop without one, a message would
    /// only be noise. Turned off once the player switches the tray on in Settings, where an answer is expected.
    tray_quiet: bool,
    upd: updates::UpdateUi,
    gh: github_add::GhState,
    /// The page shown last frame and when it opened (input clock), for the page's way in.
    page_since: (Page, f64),
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
        // the size the window had last time (checked against the screen once the window is up, see `follow_zoom`)
        if let Some(z) = settings.ui_zoom.filter(|z| (MIN_ZOOM..=MAX_ZOOM).contains(z)) {
            cc.egui_ctx.set_zoom_factor(z);
        }
        let accounts = crate::accounts::load(&dirs);
        let (instances, unreadable) = instances::load_all_and_unreadable(&dirs);
        launch::clean_stale_temp(&dirs);
        {
            let dirs = dirs.clone();
            std::thread::spawn(move || pictures::prune(&dirs));
        }
        {
            // what interrupted work left behind (half-written backups, temporary copies of settings files)
            let dirs = dirs.clone();
            let backups =
                crate::backups::backups_dir(&dirs, &crate::svcsettings::ServiceSettings::from_settings(&settings));
            std::thread::spawn(move || crate::leftovers::sweep(&dirs, &backups, crate::leftovers::MIN_AGE));
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
            sidebar_bottom: 0.0,
            tasks: Vec::new(),
            next_task: 1,
            games: HashMap::new(),
            manifest: None,
            manifest_error: None,
            icons: HashMap::new(),
            local_pictures: Default::default(),
            play_cache: Default::default(),
            java_list: Vec::new(),
            gpus: None,
            logo,
            confirm_delete: None,
            confirm_stop: None,
            confirm_quit: false,
            quit_anyway: false,
            deleting: Vec::new(),
            quick_server: String::new(),
            rename_target: None,
            inst_draft: None,
            kept_drafts: HashMap::new(),
            unreadable,
            confirm_play_unsaved: None,
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
            tray_quiet: true,
            upd: updates::UpdateUi::default(),
            gh: Default::default(),
            page_since: (Page::Home, f64::NEG_INFINITY),
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
        self.push_toast(text, false, Duration::from_secs(4));
    }

    pub fn toast_err(&mut self, text: impl Into<String>) {
        let text: String = text.into();
        self.console_push(Level::Error, &format!("[BlockForge] {text}"));
        self.push_toast(text, true, Duration::from_secs(8));
    }

    /// The same message again (a failing check, a button pressed twice) counts up on the toast that shows it
    /// instead of pushing the others away.
    fn push_toast(&mut self, text: String, error: bool, show: Duration) {
        let until = Instant::now() + show;
        match self.toasts.last_mut() {
            Some(t) if t.text == text && t.error == error => {
                t.count += 1;
                t.until = t.until.max(until);
            }
            _ => self.toasts.push(Toast { text, error, until, count: 1, held: false }),
        }
    }

    pub fn reload_instances(&mut self) {
        (self.instances, self.unreadable) = instances::load_all_and_unreadable(&self.dirs);
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
        // kept for the next start (not the size a screenshot run asked for)
        if self.shot.is_none() {
            let keep = ((zoom - 1.0).abs() > 0.005).then_some(zoom);
            if self.settings.ui_zoom != keep {
                self.settings.ui_zoom = keep;
                self.save_settings_soon();
            }
        }
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
        // tokens renewed in the background since the window read the file are kept, not written over
        crate::accounts::merge_newer_tokens(&mut self.accounts, &crate::accounts::load(&self.dirs));
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
        // changes that weren't saved are kept for their instance (and come back with its Settings tab)
        self.keep_unsaved_draft();
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
            let result = work(progress.clone(), n.clone()).await.map_err(|e| format!("{e:#}"));
            // pack files kept although their checksum disagreed are said when the job ends
            let kept = progress.take_notes();
            if !kept.is_empty() {
                n.send(Msg::Toast(crate::install::kept_files_message(&kept), true));
            }
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
                let mut kill_rx = kill_rx;
                // Stop works while the game is still being got ready (downloads, a Microsoft sign-in): before, it
                // was only heard once the game had started, which then started and was killed at once
                let getting_ready = async {
                    let account = launch::fresh_account(&dirs, &account).await?;
                    launch::prepare(&dirs, &settings, &instance, &account, &progress, &options).await
                };
                let prepared = tokio::select! {
                    prepared = getting_ready => prepared?,
                    _ = &mut kill_rx => {
                        progress.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
                        n.send(Msg::GameExit(folder_s.clone(), None));
                        return Ok(());
                    }
                };
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
                let mut running = match launch::spawn(&prepared) {
                    Ok(r) => r,
                    Err(e) => {
                        // the natives folder, the class-cache claim and the combined jar are let go here too
                        prepared.cleanup();
                        return Err(e);
                    }
                };
                n.send(Msg::GameRunning(folder_s.clone()));
                // the instance as it is on disk now, not as it was when Play was pressed: a rename or a settings
                // change made while the game was getting ready must not be written over. Done on the side, so a mod
                // job holding the instance for minutes doesn't hold up the log or Stop
                {
                    let (root, fallback, n) = (instance.root.clone(), instance.clone(), n.clone());
                    tokio::spawn(async move {
                        let lock = mods::instance_lock(&root);
                        let _held = lock.lock().await;
                        let mut fresh = instances::load_one(&root).unwrap_or(fallback);
                        let _ = fresh.mark_played();
                        n.send(Msg::Reload);
                    });
                }
                let mut assembler = LogAssembler::default();
                let mut stopped = false;
                // the well known crashes (too little memory, the wrong Java) are spotted as the lines go by
                let mut watch = launch::gamelog::CrashWatch::new(instance.mc_version());
                loop {
                    tokio::select! {
                        line = running.lines.recv() => match line {
                            Some(l) => {
                                watch.line(&l);
                                if let Some(entry) = assembler.push(&l) {
                                    n.send_soon(Msg::GameLog(folder_s.clone(), entry));
                                }
                            }
                            None => break,
                        },
                        _ = &mut kill_rx => {
                            stopped = true;
                            running.kill_tree().await;
                            break;
                        }
                    }
                }
                let status = running.child.wait().await?;
                prepared.cleanup();
                // stopped by the player: not an error, whatever code the killed process left (1 on Windows)
                let code = if stopped { None } else { launch::exit_code(&status) };
                n.send(Msg::GameExit(folder_s.clone(), code));
                if let Some(advice) = watch.advice(code).filter(|_| !stopped) {
                    let text = advice.message.replace('\n', " ");
                    n.send(Msg::GameLog(
                        folder_s.clone(),
                        LogLine { text: format!("[BlockForge] {}: {text}", advice.title), level: Level::Error },
                    ));
                    n.send(Msg::Toast(first_sentence(&advice.message), true));
                }
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
    /// Stop pressed for an instance (or one session of it, when `session` is true). A game that is running is killed,
    /// which loses what wasn't saved, so that asks first; a start that is still being prepared is cancelled at once,
    /// unless the Play button turned into Stop a moment ago (a double click on Play).
    pub fn ask_stop(&mut self, key: &str, session: bool) {
        let games: Vec<&Game> = if session {
            self.games.get(key).into_iter().collect()
        } else {
            self.games.values().filter(|g| g.folder == key).collect()
        };
        if games.iter().any(|g| matches!(g.phase, Phase::Running)) {
            self.confirm_stop = Some((key.to_string(), session));
        } else if games.iter().any(|g| g.active() && g.started.elapsed() > Duration::from_millis(600)) {
            if session { self.stop_session(key) } else { self.stop(key) }
        }
    }

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
            Msg::SearchCf { seq, reset, result } => self.discover.cf_arrived(seq, reset, result),
            Msg::SearchSmithed { seq, reset, result } => self.discover.sm_arrived(seq, reset, result),
            Msg::GithubReleases { seq, result } => self.gh_arrived(seq, result),
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
                self.play_cache.forget();
                // stopped while it was still being got ready: the game never ran, so there is nothing to back up and
                // no early end to blame on the helpers
                let ran = self.games.get(&f).is_some_and(|g| g.phase != Phase::Preparing);
                if let Some(g) = self.games.get_mut(&f) {
                    g.phase = Phase::Exited(code);
                    g.kill = None;
                }
                if ran {
                    self.helpers_after_exit(&f, code);
                }
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                let folder = self.folder_of_session(&f);
                if ran {
                    self.svc_after_game_exit(&folder);
                }
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
                        let r = crate::msauth::finish_device_login(&c2).await.map_err(|e| whoplays::sign_in_error(&e));
                        n.send(Msg::SignedIn(attempt, r));
                    });
                    // the code is waiting on the clipboard when the browser page asks for it
                    self.ctx.copy_text(code.user_code.clone());
                    self.open_link(&code.verification_uri);
                    self.acct.device = Some(code);
                }
                Err(e) => {
                    self.acct.signing_in = false;
                    self.acct.last_error = Some(e.clone());
                    self.toast_err(e);
                }
            },
            Msg::SignedIn(_, r) => {
                self.acct.signing_in = false;
                self.acct.device = None;
                // the player finished in the browser: the launcher comes back to the front
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                match r {
                    Ok(acc) => {
                        self.acct.last_error = None;
                        self.accounts.retain(|a| a.uuid != acc.uuid);
                        self.toast(format!("Signed in as {}", acc.name()));
                        self.settings.last_account = Some(acc.uuid.clone());
                        self.accounts.push(acc);
                        self.save_accounts();
                        self.save_settings();
                    }
                    Err(e) => {
                        self.acct.last_error = Some(e.clone());
                        self.toast_err(e);
                    }
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
            Msg::NoTray(e) => {
                crate::logs::warn(format!("No tray available on this desktop: {e}"));
                if !self.tray_quiet {
                    self.toast(format!("No tray available on this desktop ({e})"));
                }
            }
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
        // in a short window the group names go, so every destination still fits without scrolling
        let compact = ui.available_height() < 610.0;
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let logo =
                ui.add(egui::Image::from_texture(&self.logo).fit_to_exact_size(vec2(32.0, 32.0)).corner_radius(9));
            // the logo sits on a little light of the brand colour
            let a = c_accent();
            ui.painter().add(
                egui::Shadow {
                    offset: [0, 4],
                    blur: 16,
                    spread: 0,
                    color: egui::Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), if is_dark() { 60 } else { 40 }),
                }
                .as_shape(logo.rect, CornerRadius::same(9)),
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(crate::NAME).size(15.0).semibold());
                ui.label(RichText::new(format!("v{}", crate::VERSION)).size(11.0).color(c_faint()));
            });
        });
        ui.add_space(18.0);
        // Three groups of destinations, then Settings and About pinned to the bottom with the account. Who plays is the
        // account row at the bottom, so Accounts is not repeated in the list.
        let groups: Vec<(&str, Vec<(Page, &str, Icon)>)> = vec![
            ("Play", vec![(Page::Home, "Home", Icon::Home), (Page::Library, "Library", Icon::Library)]),
            (
                "Mods and modpacks",
                vec![
                    (Page::Discover, "Discover", Icon::Search),
                    (Page::Packs, "ATLauncher & Technic", Icon::Package),
                    (Page::Ftb, "FTB", Icon::Layers),
                ],
            ),
            ("Utilities", {
                #[allow(unused_mut)]
                let mut tools = vec![(Page::Craft, "Crafting", Icon::Craft)];
                #[cfg(feature = "converter")]
                tools.push((Page::Convert, "World converter", Icon::Convert));
                tools.push((Page::Servers, "Servers", Icon::Server));
                tools.push((Page::Tools, "Tools", Icon::Wrench));
                tools
            }),
        ];
        // games that are starting or running, and what is being downloaded, show from every page
        let playing: Vec<(String, String, std::time::Duration)> = self
            .games
            .values()
            .filter(|g| g.active())
            .filter_map(|g| Some((g.folder.clone(), self.instance(&g.folder)?.name().to_string(), g.started.elapsed())))
            .collect();
        // the account row and the running downloads need their room at the bottom; the list scrolls if the window is
        // too short for all the items. A download's rows are as tall as last frame measured, or a fair guess when it
        // has just begun
        let reserve = (52.0
            + 2.0 * 34.0
            + 24.0
            + self.tasks.len().min(2) as f32 * TASK_ROW_HEIGHT
            + playing.len().min(2) as f32 * 30.0)
            .max(self.sidebar_bottom + 8.0);
        let mut go = None;
        egui::ScrollArea::vertical()
            .id_salt("nav")
            .max_height((ui.available_height() - reserve).max(120.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (n, (group, items)) in groups.into_iter().enumerate() {
                    if n > 0 {
                        ui.add_space(if compact { 6.0 } else { 10.0 });
                    }
                    if !compact {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new(group.to_uppercase())
                                    .size(10.5)
                                    .color(c_faint())
                                    .extra_letter_spacing(0.8),
                            );
                        });
                        ui.add_space(2.0);
                    }
                    for (page, label, icon) in items {
                        let active = self.page == page || (page == Page::Library && self.page == Page::Instance);
                        if self.nav_button(ui, icon, label, active).clicked() {
                            go = Some(page);
                        }
                    }
                }
            });
        match go {
            #[cfg(feature = "converter")]
            Some(Page::Convert) => self.open_convert(None),
            Some(page) => self.page = page,
            None => {}
        }

        let bottom = ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            // who is playing
            let account = self.current_account().map(|a| (a.name().to_string(), a.kind(), a.uuid.clone(), a.offline));
            let on_accounts = self.page == Page::Accounts;
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), egui::Sense::click());
            if on_accounts || resp.hovered() {
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::same(R_CONTROL),
                    if on_accounts { c_card_hover() } else { c_card() },
                );
            }
            let mut inner = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect.shrink2(vec2(8.0, 0.0)))
                    .layout(Layout::left_to_right(Align::Center)),
            );
            match &account {
                Some((name, kind, uuid, offline)) => {
                    self.account_avatar(&mut inner, uuid, name, *offline, 30.0);
                    inner.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add(egui::Label::new(RichText::new(name).semibold()).truncate());
                        ui.label(RichText::new(*kind).size(12.0).color(c_muted()));
                    });
                }
                None => {
                    avatar(&mut inner, "?", 30.0);
                    inner.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new("No account").semibold());
                        ui.label(RichText::new("Choose who plays").size(12.0).color(c_muted()));
                    });
                }
            }
            focus_ring(ui, &resp, rect, R_CONTROL);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Accounts"));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.page = Page::Accounts;
            }
            ui.add_space(6.0);
            for (page, label, icon) in [(Page::About, "About", Icon::Info), (Page::Settings, "Settings", Icon::Sliders)]
            {
                if self.nav_button(ui, icon, label, self.page == page).clicked() {
                    self.page = page;
                }
            }
            ui.add_space(8.0);
            // downloads in progress, as bare rows
            for t in self.tasks.iter().take(2) {
                task_row(ui, t);
            }
            if self.tasks.len() > 2 {
                // the others are a click away, each with its progress and its Cancel
                let more = egui::Button::new(
                    RichText::new(format!("+{} more", self.tasks.len() - 2)).size(FS_META).color(c_muted()),
                )
                .frame(false);
                let r = ui.add(more).on_hover_cursor(egui::CursorIcon::PointingHand);
                egui::Popup::menu(&r).show(|ui| {
                    ui.set_width(260.0);
                    for t in self.tasks.iter().skip(2) {
                        task_row(ui, t);
                    }
                });
            }
            // games that are running: one row each, a click opens the instance
            for (folder, name, t) in playing.iter().take(2) {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), egui::Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), c_hover());
                }
                ui.painter().circle_filled(rect.left_center() + vec2(14.0, 0.0), 3.5, c_accent());
                let when = elapsed_short(*t);
                let when_galley = ui.painter().layout_no_wrap(when, egui::FontId::proportional(FS_META), c_faint());
                let name_w = rect.width() - 28.0 - when_galley.size().x - 18.0;
                let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), fonts::medium(FS_META), c_text());
                job.wrap = egui::text::TextWrapping::truncate_at_width(name_w.max(20.0));
                let name_galley = ui.fonts_mut(|f| f.layout_job(job));
                ui.painter().with_clip_rect(rect).galley(
                    egui::pos2(rect.left() + 28.0, rect.center().y - name_galley.size().y / 2.0),
                    name_galley,
                    c_text(),
                );
                ui.painter().galley(
                    egui::pos2(
                        rect.right() - 10.0 - when_galley.size().x,
                        rect.center().y - when_galley.size().y / 2.0,
                    ),
                    when_galley,
                    c_faint(),
                );
                focus_ring(ui, &resp, rect, R_CONTROL);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Playing {name}")));
                if resp.on_hover_text(name.as_str()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    let f = folder.clone();
                    self.open_instance(&f);
                }
            }
            if playing.len() > 2 {
                ui.label(RichText::new(format!("+{} more playing", playing.len() - 2)).size(FS_META).color(c_faint()));
            }
        });
        self.sidebar_bottom = bottom.response.rect.height();
    }

    fn nav_button(&self, ui: &mut Ui, icon: Icon, label: &str, active: bool) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), egui::Sense::click());
        // the open page: a pill that fades in, with a short bar of the brand gradient at its left edge
        let on = motion::toward(ui.ctx(), resp.id.with("nav-on"), active, motion::MEDIUM);
        let hot = motion::hover(ui, resp.id, resp.hovered() && !active);
        if hot > 0.0 {
            ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), c_hover().gamma_multiply(hot));
        }
        if on > 0.0 {
            ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), c_active().gamma_multiply(on));
            let h = 18.0 * on;
            let bar = egui::Rect::from_center_size(rect.left_center() + vec2(1.5, 0.0), vec2(3.0, h));
            motion::gradient_rect(ui.painter(), bar, 2, c_accent(), c_accent2());
        }
        let color = motion::mix(c_muted(), c_text(), on.max(hot));
        let icon_color = motion::mix(color, c_accent(), on);
        icons::paint(ui.painter(), icon, rect.left_center() + vec2(20.0, 0.0), 16.0, icon_color);
        ui.painter().text(
            rect.left_center() + vec2(40.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            if active { fonts::medium(14.0) } else { egui::FontId::proportional(14.0) },
            color,
        );
        focus_ring(ui, &resp, rect, R_CONTROL);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, active, label));
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    fn toasts_ui(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        // an error that comes while the player is in the game waits for them to come back to the window, and then
        // shows for a few seconds more
        let away = !ctx.input(|i| i.viewport().focused.unwrap_or(true));
        for t in self.toasts.iter_mut().filter(|t| t.error) {
            if away {
                t.held = true;
            } else if std::mem::take(&mut t.held) {
                t.until = t.until.max(now + Duration::from_secs(5));
            }
        }
        self.toasts.retain(|t| t.until > now || t.held);
        if self.toasts.is_empty() {
            return;
        }
        // nothing to count down while away (coming back wakes the window): a long game costs no CPU
        if !away {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        let mut dismiss: Option<usize> = None;
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                let shown = self.toasts.len().saturating_sub(3);
                for (i, t) in self.toasts.iter_mut().enumerate().skip(shown).rev() {
                    let (bg, fg) = if t.error { (c_danger_bg(), c_danger()) } else { (c_raised(), c_text()) };
                    let r = egui::Frame::new()
                        .fill(bg)
                        .stroke(Stroke::new(1.0, if t.error { c_danger_line() } else { c_control() }))
                        .corner_radius(CornerRadius::same(R_CONTROL + 2))
                        .inner_margin(egui::Margin::symmetric(14, 9))
                        .shadow(egui::Shadow {
                            offset: [0, 6],
                            blur: 18,
                            spread: 0,
                            color: egui::Color32::from_black_alpha(if is_dark() { 110 } else { 30 }),
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(560.0);
                            let text =
                                if t.count > 1 { format!("{}  \u{00D7}{}", t.text, t.count) } else { t.text.clone() };
                            ui.label(RichText::new(text).color(fg));
                        })
                        .response
                        .interact(egui::Sense::click())
                        .on_hover_text("Click to close");
                    // one being read stays a little longer
                    if r.hovered() {
                        t.until = t.until.max(now + Duration::from_secs(3));
                    }
                    if r.clicked() {
                        dismiss = Some(i);
                    }
                    ui.add_space(6.0);
                }
            });
        if let Some(i) = dismiss {
            self.toasts.remove(i);
        }
    }

    /// "Stop Minecraft?" for a game that is running (see [`App::ask_stop`]).
    fn stop_confirm_ui(&mut self, ctx: &egui::Context) {
        let Some((key, session)) = self.confirm_stop.clone() else { return };
        let still_running =
            if session { self.games.get(&key).is_some_and(Game::active) } else { self.is_playing(&key) };
        if !still_running {
            // the game closed by itself meanwhile
            self.confirm_stop = None;
            return;
        }
        match services::confirm_modal(ctx, "confirm-stop", "Stop Minecraft?", STOP_WARNING, "Stop Minecraft", true) {
            Some(true) => {
                self.confirm_stop = None;
                if session { self.stop_session(&key) } else { self.stop(&key) }
            }
            Some(false) => self.confirm_stop = None,
            None => {}
        }
    }

    /// Start the game the way `how` says, after asking about settings that were changed and not saved. Every Play
    /// (the button, Play as, an offline name, Play and join) comes through here.
    pub fn play_checked(&mut self, folder: &str, how: PendingPlay) {
        if self.unsaved_draft(folder) && !self.is_playing(folder) {
            self.confirm_play_unsaved = Some((folder.to_string(), how));
        } else {
            self.start_pending(folder, how);
        }
    }

    fn start_pending(&mut self, folder: &str, how: PendingPlay) {
        match how {
            PendingPlay::Play => self.play(folder, launch::LaunchOptions::default()),
            PendingPlay::As(account) => self.play_as(folder, launch::LaunchOptions::default(), account),
            PendingPlay::Join(server) => self.play(folder, launch::LaunchOptions { server: Some(server), world: None }),
        }
    }

    /// "Save first?" when Play is pressed with changed settings that weren't saved.
    fn play_unsaved_ui(&mut self, ctx: &egui::Context) {
        let Some((folder, how)) = self.confirm_play_unsaved.clone() else { return };
        let name = self.instance(&folder).map(|i| i.name().to_string()).unwrap_or_default();
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::unique("confirm-play-unsaved")).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(services::modal_width(ctx, 420.0));
            ui.label(RichText::new("Save the settings first?").size(FS_SECTION).semibold());
            ui.add_space(4.0);
            muted(ui, format!("You changed settings of {name} and didn't save them. The game starts with the saved ones unless you save first."));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if primary(ui, "Save and play").clicked() {
                    answer = Some(Some(true));
                }
                if secondary(ui, "Play without saving").clicked() {
                    answer = Some(Some(false));
                }
                if secondary(ui, "Cancel").clicked() {
                    answer = Some(None);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(None);
        }
        let Some(choice) = answer else { return };
        self.confirm_play_unsaved = None;
        match choice {
            Some(true) => {
                let mut d = match self.inst_draft.take() {
                    Some(d) if d.folder == folder => d,
                    other => {
                        self.inst_draft = other;
                        match self.kept_drafts.remove(&folder) {
                            Some(d) => d,
                            None => return self.start_pending(&folder, how),
                        }
                    }
                };
                let saved = self.save_draft(&mut d);
                let error = d.error.clone();
                if self.inst_draft.as_ref().is_none_or(|o| o.folder == folder) {
                    self.inst_draft = Some(d);
                } else {
                    self.kept_drafts.insert(folder.clone(), d);
                }
                if saved {
                    self.start_pending(&folder, how);
                } else {
                    // the check that failed is shown where it can be fixed
                    self.open_instance(&folder);
                    self.tab = InstTab::Settings;
                    self.toast_err(error.unwrap_or_else(|| "The settings couldn't be saved.".into()));
                }
            }
            Some(false) => self.start_pending(&folder, how),
            None => {}
        }
    }

    /// What closing the launcher now would cut short: running games and jobs, one line each. Empty when nothing is.
    fn quit_blockers(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let mut playing: Vec<&str> = self
            .games
            .values()
            .filter(|g| g.active())
            .filter_map(|g| self.instance(&g.folder).map(|i| i.name()))
            .collect();
        playing.sort_unstable();
        playing.dedup();
        if !playing.is_empty() {
            lines.push(format!(
                "Minecraft is running ({}). It keeps running, but BlockForge won't see it finish: no log, no Stop button, no backup afterwards.",
                playing.join(", ")
            ));
        }
        for t in self.tasks.iter().take(3) {
            lines.push(format!("{} stops halfway.", t.label));
        }
        if self.tasks.len() > 3 {
            lines.push(format!("{} more jobs stop halfway.", self.tasks.len() - 3));
        }
        #[cfg(feature = "converter")]
        if self.convert_running() {
            lines.push("The world conversion stops, and what it wrote so far is deleted.".to_string());
        }
        lines
    }

    /// "Quit anyway?" when the window is closed while something is running.
    /// A close asked for while something is running is held back, and the window comes forward to ask. Called from
    /// `logic`, which runs also while the window is minimized or covered by the game (`ui` doesn't then).
    fn hold_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quit_anyway
            && self.shot.is_none()
            && self.autoplay.is_none()
            && !self.quit_blockers().is_empty()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_quit = true;
            // a minimized window, or one hidden in the tray, comes back to ask
            self.window_visible = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    /// "Quit anyway?", while a close is held back (see [`App::hold_close`]).
    fn quit_confirm_ui(&mut self, ctx: &egui::Context) {
        if !self.confirm_quit {
            return;
        }
        let lines = self.quit_blockers();
        if lines.is_empty() {
            // everything finished while the question was up
            self.confirm_quit = false;
            return;
        }
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::unique("confirm-quit")).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(services::modal_width(ctx, 440.0));
            ui.label(RichText::new("Quit BlockForge?").size(FS_SECTION).semibold());
            ui.add_space(4.0);
            for line in &lines {
                muted(ui, line);
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if danger(ui, "Quit anyway").clicked() {
                    answer = Some(true);
                }
                if secondary(ui, "Keep it open").clicked() {
                    answer = Some(false);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        match answer {
            Some(true) => {
                self.confirm_quit = false;
                self.quit_anyway = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(false) => self.confirm_quit = false,
            None => {}
        }
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
            ui.label(RichText::new(format!("Delete \"{name}\"?")).size(FS_SECTION).semibold());
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
                // its unsaved settings go too: a new instance can get the same folder name
                self.kept_drafts.remove(&folder);
                if self.inst_draft.as_ref().is_some_and(|d| d.folder == folder) {
                    self.inst_draft = None;
                }
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

/// "12 min" or "1 h 05": how long a game has been running, short enough for the sidebar.
fn elapsed_short(t: Duration) -> String {
    let mins = t.as_secs() / 60;
    if mins < 60 { format!("{mins} min") } else { format!("{} h {:02}", mins / 60, mins % 60) }
}

/// The zoom to go back to when the smallest window at `zoom` would not fit on a screen of `monitor` points (measured at
/// that zoom), or `None` when it fits. Zooming out (below 1) is always allowed.
fn zoom_that_fits(zoom: f32, monitor: egui::Vec2) -> Option<f32> {
    let fits = zoom * (0.95 * monitor.x / MIN_WINDOW.x).min(0.95 * monitor.y / MIN_WINDOW.y);
    (zoom > 1.0 && zoom > fits + 0.01).then(|| fits.max(1.0))
}

/// The first sentence of a longer explanation, for a toast; the whole of it goes to the log.
/// About how tall a [`task_row`] with sizes is: the name, the bar, two lines of numbers, Cancel and the gap.
const TASK_ROW_HEIGHT: f32 = 92.0;

/// One job in the sidebar (or in its "+N more" list): what it is, how far it got, and its Cancel.
fn task_row(ui: &mut egui::Ui, t: &Task) {
    enum Line {
        Text(RichText),
        Bar(Option<f32>),
        Cancel,
        Gap,
    }
    let meta = |text: String, color| Line::Text(RichText::new(text).size(FS_META).color(color));
    let mut lines = vec![Line::Text(RichText::new(&t.label).size(FS_META).medium_weight())];
    if let Some((sizes, files)) = t.progress.summary() {
        // how much of how much, in per cent, then the files and the speed; a moving bar while the size isn't known
        lines.extend([Line::Bar(t.progress.known_fraction()), meta(sizes, c_muted()), meta(files, c_faint())]);
    } else {
        lines.push(Line::Bar(None));
        let stage = t.progress.stage_text();
        if !stage.is_empty() {
            lines.push(meta(stage, c_faint()));
        }
    }
    lines.extend([Line::Cancel, Line::Gap]);
    // the sidebar's bottom part is laid out upwards: there the lines go in last first, so they still read downwards
    if ui.layout().main_dir() == egui::Direction::BottomUp {
        lines.reverse();
    }
    for line in lines {
        match line {
            Line::Text(text) => {
                ui.add(egui::Label::new(text).truncate());
            }
            Line::Bar(fraction) => {
                progress_bar(ui, fraction, 4.0);
            }
            Line::Cancel => {
                // a long download or install can be stopped; the job stops at its next step and says it was cancelled
                let cancelling = t.progress.cancelled.load(Ordering::Relaxed);
                let text = if cancelling { "Cancelling\u{2026}" } else { "Cancel" };
                let button = egui::Button::new(RichText::new(text).size(FS_META).color(c_faint())).frame(false);
                let resp = ui.add_enabled(!cancelling, button);
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, !cancelling, format!("Cancel {}", t.label))
                });
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    t.progress.cancelled.store(true, Ordering::Relaxed);
                }
            }
            Line::Gap => ui.add_space(8.0),
        }
    }
}

fn first_sentence(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    match line.find(". ") {
        Some(i) => line[..=i].to_string(),
        None => line.to_string(),
    }
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
        self.hold_close(ctx);
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
        // the ground with its soft glow, under the glass of the sidebar and the pages
        motion::backdrop(ui.painter(), ctx.content_rect());
        let glass = c_surface();
        let nav = egui::Panel::left("nav")
            .exact_size(232.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(glass.r(), glass.g(), glass.b(), 222))
                    .inner_margin(egui::Margin::symmetric(14, 18)),
            )
            .show(ui, |ui| self.sidebar(ui));
        // one line divides the sidebar from the page
        let edge = nav.response.rect.right();
        ui.painter().vline(edge - 0.5, nav.response.rect.y_range(), Stroke::new(1.0, c_border()));
        // a new page fades in and rises into place
        let now = ctx.input(|i| i.time);
        if self.page_since.0 != self.page {
            self.page_since = (self.page, if self.page_since.1.is_finite() { now } else { f64::NEG_INFINITY });
        }
        let arrived = motion::arrival(&ctx, self.page_since.1);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 36, right: 24, top: 28, bottom: 24 }))
            .show(ui, |ui| {
                if arrived < 1.0 {
                    ui.multiply_opacity(arrived);
                    ui.add_space((1.0 - arrived) * 14.0);
                }
                self.page_ui(ui)
            });
        self.windows_ui(&ctx);
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

impl App {
    fn page_ui(&mut self, ui: &mut Ui) {
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
    }

    /// The windows drawn over the page.
    fn windows_ui(&mut self, ctx: &egui::Context) {
        let ctx = ctx.clone();
        // below the other windows: the first start's questions come first
        self.update_dialog(&ctx);
        self.gh_dialog(&ctx);
        self.import_modal(&ctx);
        self.export_modal(&ctx);
        self.toasts_ui(&ctx);
        self.confirm_ui(&ctx);
        self.stop_confirm_ui(&ctx);
        self.play_unsaved_ui(&ctx);
        self.quit_confirm_ui(&ctx);
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

const SHOT_PAGES: [&str; 37] = [
    "home",
    "library",
    "instance",
    "instance-mods",
    "instance-logs",
    "instance-settings",
    "instance-settings-java",
    "instance-settings-cmd",
    "discover",
    "discover-both",
    "discover-datapacks",
    "github-add",
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
                        let name = name.trim_end_matches("-light");
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
                "discover-both" => {
                    self.page = Page::Discover;
                    self.cf.active = false;
                    self.discover.both = true;
                    self.discover.query = "sodium".into();
                    self.discover_search(true);
                }
                "discover-datapacks" => {
                    self.page = Page::Discover;
                    self.cf.active = false;
                    self.discover.both = true;
                    self.discover.kind = crate::modrinth::Kind::DataPack;
                    self.discover.query = "terralith".into();
                    self.discover_search(true);
                }
                "github-add" => {
                    self.page = Page::Discover;
                    self.cf.active = false;
                    self.discover.target = self.instances.first().map(|i| i.folder.clone());
                    self.gh_open();
                    self.gh.input = "https://github.com/IrisShaders/Iris".into();
                    self.gh_find();
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
        // both sites at once: the slower of the two decides
        let wait = if matches!(name.trim_end_matches("-light"), "discover-both" | "discover-datapacks" | "github-add") {
            400
        } else if matches!(
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

#[cfg(test)]
mod first_sentence_tests {
    use super::first_sentence;

    #[test]
    fn toasts_get_the_first_sentence() {
        assert_eq!(first_sentence("Too little memory. Raise it in Settings."), "Too little memory.");
        assert_eq!(first_sentence("\n\n  One line only  "), "One line only");
        assert_eq!(first_sentence(""), "");
        assert_eq!(first_sentence("Ünïcode first. Then more"), "Ünïcode first.");
    }
}
