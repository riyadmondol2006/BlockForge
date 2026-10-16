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

/// A finished network check.
pub struct NetReport {
    pub text: String,
    /// Where the shared report can be read.
    pub link: Option<String>,
    /// Why it could not be shared, if that went wrong.
    pub error: Option<String>,
}

/// Results coming back from the background.
pub enum SvcMsg {
    News(Result<Vec<crate::news::NewsItem>, String>),
    NetworkReport(NetReport),
    /// The skin was changed; the account may hold renewed sign-in tokens.
    SkinChanged(Result<Account, String>),
    /// A skin was downloaded for this id; its picture should be read again.
    SkinFetched(String),
    ProxyTest(Result<(), String>),
    /// A server was deleted (or deleting it stopped half way): read the servers folder again.
    ServersChanged,
}

#[derive(Default)]
pub struct SvcState {
    // Tools
    /// The tool waiting for the player to confirm it.
    pub tool_confirm: Option<&'static str>,
    pub skip_speed_test: bool,
    pub net_report: Option<NetReport>,
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
    egui::Modal::new(egui::Id::unique(id)).frame(super::modal_frame(ctx)).show(ctx, |ui| {
        ui.set_width(modal_width(ctx, 420.0));
        ui.label(RichText::new(title).size(19.0).strong());
        ui.add_space(4.0);
        muted(ui, body);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let clicked = if dangerous { danger(ui, confirm).clicked() } else { primary(ui, confirm).clicked() };
            if clicked {
                answer = Some(true);
            }
            if secondary(ui, "Cancel").clicked() {
                answer = Some(false);
            }
        });
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && answer.is_none() {
        answer = Some(false);
    }
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
            SvcMsg::NetworkReport(report) => {
                match (&report.link, &report.error) {
                    (Some(_), _) => self.toast("Network check finished and shared"),
                    (None, Some(e)) => self.toast_err(format!("Network check finished, but sharing it failed: {e}")),
                    _ => self.toast("Network check finished"),
                }
                self.svc.net_report = Some(report);
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

    /// A folded section for the Settings page with the network, folder and backup settings that only exist in the
    /// Java edition's settings file. Changes are saved as they are made.
    pub(super) fn services_settings_ui(&mut self, ui: &mut Ui) {
        let mut s = self.svc_edit();
        let before = s.clone();
        let mut folder_error: Option<String> = None;
        ui.add_space(8.0);
        card(ui, |ui| {
            egui::CollapsingHeader::new(RichText::new("Network, folders and backups").strong().size(16.0))
                .default_open(false)
                .show(ui, |ui| {
                    small_muted(ui, "Connection changes apply the next time BlockForge starts.");
                    ui.add_space(4.0);
                    setting_row(
                        ui,
                        "Files downloaded at once",
                        "More is faster on a good connection (1 to 100)",
                        |ui| {
                            ui.add(egui::DragValue::new(&mut s.concurrent_connections).range(1..=100));
                        },
                    );
                    setting_row(ui, "Connection timeout", "Seconds to wait for a server (1 to 600)", |ui| {
                        ui.add(egui::DragValue::new(&mut s.connection_timeout).range(1..=600).suffix(" s"));
                    });
                    let mut key = s.modrinth_api_key.clone().unwrap_or_default();
                    super::settings::wide_row(
                        ui,
                        "Modrinth API key",
                        "Optional. Raises the limits Modrinth puts on requests",
                        270.0,
                        |ui| {
                            ui.add_sized(
                                [260.0, 30.0],
                                egui::TextEdit::singleline(&mut key)
                                    .password(true)
                                    .margin(egui::Margin::symmetric(10, 6))
                                    .background_color(c_field()),
                            );
                        },
                    );
                    s.modrinth_api_key = Some(key).filter(|k| !k.trim().is_empty());
                    setting_row(ui, "Use a proxy", "Send downloads through a proxy server", |ui| {
                        toggle(ui, &mut s.enable_proxy);
                    });
                    if s.enable_proxy {
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
                                [220.0, 30.0],
                                egui::TextEdit::singleline(&mut s.proxy_host)
                                    .hint_text("Proxy address")
                                    .margin(egui::Margin::symmetric(10, 6))
                                    .background_color(c_field()),
                            );
                            ui.add(egui::DragValue::new(&mut s.proxy_port).range(1..=65535));
                            let testing = self.svc.proxy_testing;
                            if secondary(ui, if testing { "Testing..." } else { "Test" }).clicked() && !testing {
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
                    ui.add_space(4.0);
                    let width = ui.available_width().min(420.0);
                    let margin = egui::Margin::symmetric(10, 6);
                    ui.label("Downloads folder");
                    small_muted(ui, "Where support bundles are saved. Empty uses your Downloads folder.");
                    let downloads = s.custom_downloads_path.clone().unwrap_or_default();
                    let typed_downloads = super::settings::draft_text(
                        ui,
                        "downloads-folder",
                        &downloads,
                        "Default",
                        [width, 30.0],
                        margin,
                    );
                    ui.label("Backups folder");
                    small_muted(
                        ui,
                        "Where instance backups are saved. Empty uses the backups folder in the data folder.",
                    );
                    let backups = s.backups_path.clone().unwrap_or_default();
                    let typed_backups =
                        super::settings::draft_text(ui, "backups-folder", &backups, "Default", [width, 30.0], margin);
                    // a typed path is only taken once it names a folder that exists, as the Java edition does
                    for (typed, slot, what) in [
                        (typed_downloads, &mut s.custom_downloads_path, "downloads"),
                        (typed_backups, &mut s.backups_path, "backups"),
                    ] {
                        let Some(text) = typed else { continue };
                        match folder_setting(&text) {
                            Ok(value) => *slot = value,
                            Err(()) => {
                                folder_error = Some(format!("The {what} folder \"{}\" doesn't exist", text.trim()))
                            }
                        }
                    }
                    ui.add_space(4.0);
                    setting_row(ui, "Back up after playing", "Makes a backup every time a game closes", |ui| {
                        toggle(ui, &mut s.enable_automatic_backup_after_launch);
                    });
                    setting_row(ui, "Backup contents", "What the automatic backup holds", |ui| {
                        egui::ComboBox::from_id_salt("backup-mode")
                            .selected_text(s.backup_mode.label())
                            .width(260.0)
                            .show_ui(ui, |ui| {
                                for m in crate::backups::BackupMode::ALL {
                                    ui.selectable_value(&mut s.backup_mode, m, m.label());
                                }
                            });
                    });
                });
        });
        if let Some(e) = folder_error {
            self.toast_err(e);
        }
        if s != before {
            s.write_to(&mut self.settings);
            self.save_settings();
            self.svc.edit = Some(s);
        }
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
