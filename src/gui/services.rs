//! Shared state and plumbing for the pages of this area: Tools, Servers, news, skins and backups.
//!
//! Everything slow runs on the background runtime and reports back through [`SvcMsg`], which rides inside the
//! window's own `Msg` channel (`Msg::Svc`). Windows that sit on top of a page (confirmations, the skin dialog,
//! the news article, the network report) are drawn by [`App::svc_overlays`], called once per frame.

use std::collections::{HashMap, HashSet};

use egui::{Context, RichText, Ui};

use super::backups::BackupsState;
use super::news::NewsState;
use super::skins::{HeadTexture, SkinDialog};
use super::theme::*;
use super::widgets::*;
use super::{App, Msg};
use crate::accounts::Account;
use crate::servers::Server;
use crate::svcsettings::ServiceSettings;

/// A finished network check, shown on this computer, and how far sharing it got.
pub struct NetReport {
    pub text: String,
    pub share: Share,
}

/// Sharing a network report: only ever started by the player, after the report was shown and they agreed.
#[derive(Clone, PartialEq, Debug)]
pub enum Share {
    /// Not shared: the report is only on this computer.
    Local,
    /// "Share report" was pressed: the window says what is uploaded where and asks to go ahead.
    Asking,
    Sending,
    /// Where the shared report can be read.
    Shared(String),
    /// Why it could not be shared.
    Failed(String),
}

/// Results coming back from the background.
pub enum SvcMsg {
    News(Result<Vec<crate::news::NewsItem>, String>),
    NetworkReport(String),
    /// The answer of the paste service to a report the player chose to share: its link, or why it failed.
    NetworkShared(Result<String, String>),
    /// The skin was changed; the account may hold renewed sign-in tokens.
    SkinChanged(Result<Account, String>),
    /// A skin was downloaded for this id; its picture should be read again.
    SkinFetched(String),
    ProxyTest(Result<(), String>),
    /// A server was deleted (or deleting it stopped half way): read the servers folder again.
    ServersChanged,
    /// The file chooser of the skin window closed (`None`: cancelled).
    SkinPicked(Option<std::path::PathBuf>),
}

/// How a tool's job ended: well or not, what it said, and when.
pub struct ToolResult {
    pub ok: bool,
    pub text: String,
    pub at: std::time::Instant,
}

#[derive(Default)]
pub struct SvcState {
    // Tools
    /// The tool waiting for the player to confirm it.
    pub tool_confirm: Option<&'static str>,
    pub skip_speed_test: bool,
    pub net_report: Option<NetReport>,
    /// How each tool's job ended last, by the job's label, so its row can say so.
    pub tool_results: HashMap<String, ToolResult>,
    // News
    pub news: NewsState,
    // Skins
    pub heads: HashMap<String, HeadTexture>,
    pub skin_requested: HashSet<String>,
    pub skin_dialog: Option<SkinDialog>,
    // Servers
    pub servers: Vec<Server>,
    /// What the cards show about each of `servers`, worked out when the list is read rather than on every frame.
    pub server_rows: Vec<super::servers::ServerRow>,
    pub servers_loaded: bool,
    pub server_confirm_remove: Option<String>,
    pub server_confirm_launch: Option<String>,
    /// The server whose "Reinstall" window is open.
    pub server_repair: Option<String>,
    // Backups
    pub backups: BackupsState,
    // Settings
    pub edit: Option<ServiceSettings>,
    pub proxy_testing: bool,
    /// Set every frame: a game, an install or a download is running, so reading files ahead must wait.
    pub busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The width for a window drawn over the page: `wanted`, or less when the launcher window is narrow (small window or
/// a large zoom), so the window never runs off the screen.
pub fn modal_width(ctx: &Context, wanted: f32) -> f32 {
    fit_width(wanted, ctx.content_rect().width())
}

fn fit_width(wanted: f32, screen: f32) -> f32 {
    // the modal's own frame and a margin to the window's edge
    wanted.min(screen - 96.0).max(200.0)
}

/// The height a scrolling part of such a window may take: `wanted`, or less on a short window. `rest` is the height
/// of everything else in the window (title, buttons).
pub fn modal_scroll_height(ctx: &Context, wanted: f32, rest: f32) -> f32 {
    wanted.min(ctx.content_rect().height() - rest - 96.0).max(120.0)
}

/// A yes/no window. `Some(true)` when confirmed, `Some(false)` when cancelled, `None` while it is open.
pub fn confirm_modal(ctx: &Context, id: &str, title: &str, body: &str, confirm: &str, dangerous: bool) -> Option<bool> {
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::unique(id)).frame(super::modal_frame(ctx)).show(ctx, |ui| {
        ui.set_width(modal_width(ctx, 420.0));
        ui.label(RichText::new(title).size(FS_SECTION).semibold());
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(body).color(c_muted())).wrap());
        ui.add_space(20.0);
        answer = confirm_buttons(ui, id, "Cancel", confirm, dangerous, true);
    });
    // Escape answers the window on top only: one press never closes two stacked windows
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// The buttons that finish a window asking to confirm something, in every such window the same way: at its right,
/// the safe answer (`cancel`) first and the answer asked for last, 8 points apart, red when `dangerous` (it can't be
/// undone). The safe answer has the keyboard focus when the window opens, so Enter or Space never confirms by
/// accident; Escape (the window's own) is the safe answer too. `enabled` greys out the answer asked for while it
/// can't be given. `Some(true)` when confirmed, `Some(false)` when cancelled.
pub fn confirm_buttons(
    ui: &mut Ui,
    key: &str,
    cancel: &str,
    confirm: &str,
    dangerous: bool,
    enabled: bool,
) -> Option<bool> {
    confirm_buttons_with(ui, key, |_| {}, cancel, confirm, dangerous, enabled)
}

/// [`confirm_buttons`] with quiet things at the left of the row (drawn by `left`).
pub fn confirm_buttons_with(
    ui: &mut Ui,
    key: &str,
    left: impl FnOnce(&mut Ui),
    cancel: &str,
    confirm: &str,
    dangerous: bool,
    enabled: bool,
) -> Option<bool> {
    let mut answer = None;
    super::discover::dialog_footer(ui, key, left, |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let safe = secondary(ui, cancel);
        focus_when_shown(ui, &safe);
        if safe.clicked() {
            answer = Some(false);
        }
        let asked = match (dangerous, enabled) {
            (true, _) => danger_enabled(ui, confirm, enabled),
            (false, true) => primary(ui, confirm),
            (false, false) => primary_enabled(ui, confirm, false),
        };
        if asked.clicked() && enabled {
            answer = Some(true);
        }
    });
    answer
}

impl App {
    /// Opens a folder in the system's file manager (Explorer, Finder, or what the desktop uses on Linux), making it
    /// first if it isn't there yet. Doesn't wait for the file manager.
    pub(super) fn open_folder(&mut self, path: &std::path::Path) {
        let _ = std::fs::create_dir_all(path);
        if let Err(e) = open::that_detached(path) {
            crate::logs::warn(format!("Couldn't open {}: {e}", path.display()));
            self.toast_err(format!("Couldn't open the folder {}", path.display()));
        }
    }

    /// Opens a web page in the browser without waiting for it.
    pub(super) fn open_link(&mut self, url: &str) {
        if let Err(e) = open::that_detached(url) {
            crate::logs::warn(format!("Couldn't open {url}: {e}"));
            self.toast_err(format!("Couldn't open the browser. The address is {url}"));
        }
    }

    /// Results from the background reach the window here (called from `App::handle`).
    pub(super) fn svc_msg(&mut self, msg: SvcMsg) {
        match msg {
            SvcMsg::News(result) => self.svc.news.arrived(result),
            SvcMsg::NetworkReport(text) => {
                self.toast("Network check finished");
                self.svc.net_report = Some(NetReport { text, share: Share::Local });
            }
            SvcMsg::NetworkShared(result) => {
                let share = match result {
                    Ok(link) => Share::Shared(link),
                    Err(e) => Share::Failed(e),
                };
                match self.svc.net_report.as_mut() {
                    Some(report) => report.share = share,
                    // the window was closed while it was sent: the toast keeps the link
                    None => match share {
                        Share::Shared(link) => self.toast(format!("Network report shared: {link}")),
                        Share::Failed(e) => self.toast_err(format!("The network report could not be shared: {e}")),
                        _ => {}
                    },
                }
            }
            SvcMsg::SkinPicked(path) => {
                if let Some(d) = self.svc.skin_dialog.as_mut() {
                    d.picked(path.as_deref());
                }
            }
            SvcMsg::SkinChanged(result) => match result {
                Ok(account) => {
                    if let Some(slot) = self.accounts.iter_mut().find(|a| a.uuid == account.uuid) {
                        *slot = account.clone();
                    }
                    self.save_accounts();
                    self.svc.heads.remove(&account.uuid);
                    if let Some(d) = self.svc.skin_dialog.as_mut() {
                        d.finished_ok();
                    }
                    self.svc.skin_dialog = None;
                    self.toast("Your skin was changed");
                }
                Err(e) => {
                    if let Some(d) = self.svc.skin_dialog.as_mut() {
                        d.failed(e);
                    } else {
                        self.toast_err(e);
                    }
                }
            },
            SvcMsg::SkinFetched(uuid) => {
                self.svc.heads.remove(&uuid);
                self.ctx.request_repaint();
            }
            SvcMsg::ProxyTest(result) => {
                self.svc.proxy_testing = false;
                match result {
                    Ok(()) => self.toast("The proxy works"),
                    Err(e) => self.toast_err(e),
                }
            }
            SvcMsg::ServersChanged => self.reload_servers(),
        }
    }

    /// Everything that is drawn on top of a page, once per frame.
    pub(super) fn svc_overlays(&mut self, ctx: &Context) {
        self.tools_overlays(ctx);
        self.news_overlay(ctx);
        self.skin_dialog_ui(ctx);
        self.backups_overlays(ctx);
        self.servers_overlays(ctx);
    }

    /// Once per frame: note whether the disk is busy, for reading game files ahead of Play.
    pub(super) fn svc_frame(&self) {
        let busy = !self.tasks.is_empty() || self.games.values().any(super::Game::active);
        self.svc.busy.store(busy, std::sync::atomic::Ordering::Relaxed);
    }

    fn busy_fn(&self) -> crate::warmer::BusyFn {
        let flag = self.svc.busy.clone();
        std::sync::Arc::new(move || flag.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// A few seconds after the window opens, read the most recently played game's files ahead of Play.
    pub(super) fn svc_idle_start(&self) {
        let (dirs, settings, instances, busy) =
            (self.dirs.clone(), self.settings.clone(), self.instances.clone(), self.busy_fn());
        let _ = std::thread::Builder::new().name("warm-up-start".into()).spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            crate::warmer::launcher_idle(&dirs, &settings, &instances, busy);
        });
    }

    /// The player opened an instance: read its files ahead if that hasn't been done lately.
    pub(super) fn svc_warm(&self, folder: &str) {
        let Some(inst) = self.instance(folder) else { return };
        let latest = self.instances.iter().max_by_key(|i| i.last_played()).map(|i| i.folder.clone());
        crate::warmer::warm_soon(&self.dirs, &self.settings, inst, latest.as_deref(), self.busy_fn());
    }

    /// A game has closed: build its class cache if one was being recorded, and make the automatic backup if that
    /// is switched on. Call it from the `GameExit` message.
    pub(super) fn svc_after_game_exit(&mut self, folder: &str) {
        let Some(inst) = self.instance(folder).cloned() else { return };
        let safe = crate::backups::safe_name(inst.name());
        crate::classcache::game_exited(&self.dirs, &safe);
        let svc = ServiceSettings::from_settings(&self.settings);
        if svc.enable_automatic_backup_after_launch {
            let dirs = self.dirs.clone();
            self.svc.backups.mark_stale();
            self.start_task(format!("Backing up {}", inst.name()), move |_p, _n| async move {
                let name = inst.name().to_string();
                let root = inst.root.clone();
                let made = tokio::task::spawn_blocking(move || {
                    crate::backups::create(&dirs, &svc, &root, &name, svc.backup_mode)
                })
                .await??;
                Ok(format!(
                    "Backup is complete ({})",
                    made.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                ))
            });
        }
    }

    /// Read the settings of this area into the working copy the first time it is needed. The switches the rest of the
    /// Settings page shows (class cache, reading ahead, the two privacy ones) are taken from the settings every time,
    /// so the working copy never writes back an old value of theirs.
    fn svc_edit(&mut self) -> ServiceSettings {
        let mut s = match self.svc.edit.take() {
            Some(s) => s,
            None => {
                let (s, warnings) = ServiceSettings::read(&self.settings);
                for w in warnings {
                    crate::logs::warn(w);
                }
                s
            }
        };
        sync_shared(&mut s, &self.settings);
        self.svc.edit = Some(s.clone());
        s
    }

    /// The Backups and the Network groups of the Settings page: the network, folder and backup settings that only
    /// exist in the Java edition's settings file. Changes are saved as they are made.
    pub(super) fn services_settings_ui(
        &mut self,
        ui: &mut Ui,
        view: &super::settings::View,
        nav: &super::settings::Nav,
    ) {
        use super::settings::{COMBO_W, CONTROL_W, Group, Part, section};
        use crate::backups::BackupMode;
        use crate::svcsettings::{DEFAULT_CONCURRENT_CONNECTIONS, DEFAULT_CONNECTION_TIMEOUT};
        let mut s = self.svc_edit();
        let before = s.clone();
        // a number field as wide in every row, so they line up
        let number = [96.0, 30.0];
        let mut typed_downloads = None;
        let mut typed_backups = None;
        let mut reset_downloads = false;
        let mut reset_backups = false;
        let own_backups = s.backups_path.is_some() || s.unavailable_backups_path.is_some();
        section(ui, view, nav, Group::Backups, usize::from(own_backups), |ui, c, part| match part {
            Part::Main => {
                c.switch(ui, "auto-backup", &mut s.enable_automatic_backup_after_launch, false);
                let reset = (s.backups_to_keep != 0).then_some("All");
                if c.row(ui, "backups-keep", number[0], reset, |ui| {
                    ui.add_sized(
                        number,
                        egui::DragValue::new(&mut s.backups_to_keep)
                            .range(0..=crate::svcsettings::MAX_BACKUPS_TO_KEEP)
                            .custom_formatter(|n, _| if n == 0.0 { "All".to_string() } else { format!("{n}") }),
                    );
                }) {
                    s.backups_to_keep = 0;
                }
                let reset = (s.backup_mode != BackupMode::default()).then(|| BackupMode::default().label());
                if c.row(ui, "backup-contents", CONTROL_W, reset, |ui| {
                    egui::ComboBox::from_id_salt("backup-mode")
                        .selected_text(s.backup_mode.label())
                        .width(COMBO_W)
                        .show_ui(ui, |ui| {
                            for m in BackupMode::ALL {
                                ui.selectable_value(&mut s.backup_mode, m, m.label());
                            }
                        });
                }) {
                    s.backup_mode = BackupMode::default();
                }
            }
            Part::Advanced => {
                let backups = s.backups_path.clone().or_else(|| s.unavailable_backups_path.clone()).unwrap_or_default();
                let (typed, reset) =
                    c.text(ui, "backups-folder", "Default", &backups, own_backups.then_some("the default folder"));
                typed_backups = typed;
                reset_backups = reset;
                if c.shows("backups-folder") {
                    folder_error_line(ui, "backups");
                }
            }
        });
        let own_downloads = s.custom_downloads_path.is_some() || s.unavailable_downloads_path.is_some();
        let net_changed = usize::from(own_downloads) + usize::from(s.modrinth_api_key.is_some());
        section(ui, view, nav, Group::Network, net_changed, |ui, c, part| match part {
            Part::Main => {
                let reset = (s.concurrent_connections != DEFAULT_CONCURRENT_CONNECTIONS)
                    .then(|| DEFAULT_CONCURRENT_CONNECTIONS.to_string());
                if c.row(ui, "connections", number[0], reset.as_deref(), |ui| {
                    ui.add_sized(number, egui::DragValue::new(&mut s.concurrent_connections).range(1..=100));
                }) {
                    s.concurrent_connections = DEFAULT_CONCURRENT_CONNECTIONS;
                }
                let reset = (s.connection_timeout != DEFAULT_CONNECTION_TIMEOUT)
                    .then(|| format!("{DEFAULT_CONNECTION_TIMEOUT} s"));
                if c.row(ui, "timeout", number[0], reset.as_deref(), |ui| {
                    ui.add_sized(number, egui::DragValue::new(&mut s.connection_timeout).range(1..=600).suffix(" s"));
                }) {
                    s.connection_timeout = DEFAULT_CONNECTION_TIMEOUT;
                }
                c.switch(ui, "proxy", &mut s.enable_proxy, false);
                if s.enable_proxy && c.shows("proxy") {
                    // the proxy itself, under the switch that uses it
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        egui::ComboBox::from_id_salt("proxy-type")
                            .selected_text(s.proxy_type.key())
                            .width(100.0)
                            .show_ui(ui, |ui| {
                                for t in crate::net::ProxyType::ALL {
                                    ui.selectable_value(&mut s.proxy_type, t, t.key());
                                }
                            });
                        ui.add_sized(
                            [220.0, 32.0],
                            egui::TextEdit::singleline(&mut s.proxy_host)
                                .hint_text("Proxy address")
                                .margin(egui::Margin::symmetric(10, 7))
                                .background_color(c_field()),
                        );
                        ui.add_sized(number, egui::DragValue::new(&mut s.proxy_port).range(1..=65535).prefix("Port "));
                        let testing = self.svc.proxy_testing;
                        if secondary(ui, if testing { "Testing\u{2026}" } else { "Test" }).clicked() && !testing {
                            if let Some(proxy) = crate::net::NetConfig::from_settings(&s).proxy {
                                self.svc.proxy_testing = true;
                                let n = self.notifier();
                                self.rt.spawn(async move {
                                    let r = crate::net::test_proxy(
                                        &proxy,
                                        &format!("{}/ping", crate::tools::DOWNLOAD_SERVER),
                                    )
                                    .await
                                    .map_err(|e| format!("{e:#}"));
                                    n.send(Msg::Svc(SvcMsg::ProxyTest(r)));
                                });
                            } else {
                                self.toast_err("Enter the proxy's host and port first");
                            }
                        }
                    });
                }
            }
            Part::Advanced => {
                let mut key = s.modrinth_api_key.clone().unwrap_or_default();
                let reset = s.modrinth_api_key.is_some().then_some("none");
                if c.row(ui, "modrinth-key", CONTROL_W, reset, |ui| {
                    let width = ui.available_width().clamp(160.0, CONTROL_W);
                    ui.add_sized(
                        [width, 32.0],
                        egui::TextEdit::singleline(&mut key)
                            .password(true)
                            .margin(egui::Margin::symmetric(10, 7))
                            .background_color(c_field()),
                    );
                }) {
                    key.clear();
                }
                s.modrinth_api_key = Some(key).filter(|k| !k.trim().is_empty());
                let downloads = s
                    .custom_downloads_path
                    .clone()
                    .or_else(|| s.unavailable_downloads_path.clone())
                    .unwrap_or_default();
                let (typed, reset) = c.text(
                    ui,
                    "downloads-folder",
                    "Default",
                    &downloads,
                    own_downloads.then_some("your Downloads folder"),
                );
                typed_downloads = typed;
                reset_downloads = reset;
                if c.shows("downloads-folder") {
                    folder_error_line(ui, "downloads");
                }
            }
        });
        // a typed path is only taken once it names a folder that exists, as the Java edition does; when it doesn't,
        // that is said under the field. Reset goes back to the default folder.
        for (typed, reset, slot, unavailable, what) in [
            (
                typed_downloads,
                reset_downloads,
                &mut s.custom_downloads_path,
                &mut s.unavailable_downloads_path,
                "downloads",
            ),
            (typed_backups, reset_backups, &mut s.backups_path, &mut s.unavailable_backups_path, "backups"),
        ] {
            let key = folder_error_id(what);
            let Some(text) = typed.or_else(|| reset.then(String::new)) else { continue };
            match folder_setting(&text) {
                Ok(value) => {
                    *slot = value;
                    // a folder typed by the player replaces one that wasn't there
                    *unavailable = None;
                    ui.data_mut(|d| d.remove::<String>(key));
                }
                Err(()) => {
                    let error = format!("The {what} folder \"{}\" doesn't exist", text.trim());
                    ui.data_mut(|d| d.insert_temp(key, error));
                    ui.ctx().request_repaint();
                }
            }
        }
        if s != before {
            s.write_to(&mut self.settings);
            self.save_settings();
            super::settings::mark_saved(ui.ctx());
            self.svc.edit = Some(s);
        }
    }
}

/// Where the answer for a folder field that was typed wrongly is kept until a good one is typed.
fn folder_error_id(what: &str) -> egui::Id {
    egui::Id::unique(("settings-folder-error", what))
}

/// The reason a folder field's text was not taken, under the field.
fn folder_error_line(ui: &mut Ui, what: &str) {
    if let Some(error) = ui.data(|d| d.get_temp::<String>(folder_error_id(what))) {
        ui.add(egui::Label::new(RichText::new(error).color(c_danger()).size(FS_META)).wrap());
    }
}

/// The switches the main part of the Settings page owns, copied into the working copy of this area's settings.
fn sync_shared(s: &mut ServiceSettings, settings: &crate::config::Settings) {
    s.class_data_cache = settings.class_data_cache;
    s.enable_logs = settings.enable_logs;
    s.enable_analytics = settings.enable_analytics;
    s.warm_up_game_files = settings.extras.get("warmUpGameFiles").and_then(|v| v.as_bool()).unwrap_or(true);
}

/// A typed folder setting: `Ok(None)` for empty (the default folder), `Ok(Some(path))` for a folder that exists,
/// `Err` otherwise.
fn folder_setting(text: &str) -> Result<Option<String>, ()> {
    let trimmed = text.trim().trim_matches('"');
    if trimmed.is_empty() {
        Ok(None)
    } else if std::path::Path::new(trimmed).is_dir() {
        Ok(Some(trimmed.to_string()))
    } else {
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of a confirmation window with `events`: its answer, and which widget has the focus afterwards.
    fn confirm_frame(ctx: &Context, events: Vec<egui::Event>) -> (Option<bool>, Option<egui::Id>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut answer = None;
        let mut full = ctx.run_ui(input, |ui| {
            answer = confirm_modal(ui.ctx(), "test-confirm", "Delete it?", "It can't be undone.", "Delete", true);
        });
        full.textures_delta.clear();
        (answer, ctx.memory(|m| m.focused()))
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
    }

    #[test]
    fn a_confirmation_opens_on_its_safe_answer_and_enter_or_escape_never_confirm() {
        let ctx = Context::default();
        for _ in 0..3 {
            assert_eq!(confirm_frame(&ctx, vec![]).0, None);
        }
        assert!(confirm_frame(&ctx, vec![]).1.is_some(), "a button has the focus when the window opens");
        // Enter presses the focused button: the safe one
        let (answer, _) = confirm_frame(&ctx, vec![key(egui::Key::Enter)]);
        assert_eq!(answer, Some(false));

        let ctx = Context::default();
        for _ in 0..3 {
            confirm_frame(&ctx, vec![]);
        }
        assert_eq!(confirm_frame(&ctx, vec![key(egui::Key::Escape)]).0, Some(false), "Escape is the safe answer");
    }

    #[test]
    fn windows_fit_a_narrow_launcher() {
        assert_eq!(fit_width(420.0, 1280.0), 420.0);
        assert_eq!(fit_width(620.0, 500.0), 404.0);
        assert_eq!(fit_width(620.0, 100.0), 200.0, "never narrower than a button row");
    }

    #[test]
    fn the_main_page_switches_win_over_an_old_working_copy() {
        let mut settings = crate::config::Settings::default();
        let mut copy = ServiceSettings::from_settings(&settings);
        // the main Settings page turns these over after the working copy was read
        settings.class_data_cache = false;
        settings.enable_logs = true;
        settings.enable_analytics = true;
        settings.extras.insert("warmUpGameFiles".into(), false.into());
        sync_shared(&mut copy, &settings);
        // a change in the folded section then writes them back unchanged
        copy.connection_timeout = 30;
        copy.write_to(&mut settings);
        assert!(!settings.class_data_cache && settings.enable_logs && settings.enable_analytics);
        assert_eq!(settings.extras["warmUpGameFiles"], false);
        assert_eq!(settings.extras["connectionTimeout"], 30);
    }

    #[test]
    fn typed_folders_must_exist() {
        let dir = std::env::temp_dir();
        assert_eq!(folder_setting("  "), Ok(None));
        assert_eq!(folder_setting(&format!("\"{}\"", dir.display())), Ok(Some(dir.display().to_string())));
        assert_eq!(folder_setting("/no/such/folder/for/blockforge"), Err(()));
    }
}
