//! BlockForge updating itself: the background check, the window that pops up to offer a new version with what is new
//! in it ("Update now" downloads it with a progress bar and restarts), and what the About page shows about updates.

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use egui::{CornerRadius, RichText, Stroke, Ui};

use super::discover::{close_button, dialog_footer};
use super::theme::*;
use super::widgets::*;
use super::{App, Msg};
use crate::update::{self, Asset, Found, Installed, Kind, Offers, Release, UpdateStatus};

/// Messages the update code sends to the window.
pub enum UpdateMsg {
    /// Time for the next background check.
    Tick,
    /// A check finished. `manual` is true when the player asked for it.
    Checked { manual: bool, result: Result<Found, String> },
    /// The download and install finished (or stopped).
    Finished(Result<Installed, String>),
    /// The player cancelled the download.
    Cancelled,
}

/// Where the banner is in its life.
enum Stage {
    /// Asking whether to update.
    Offer,
    Downloading {
        progress: Arc<update::DownloadProgress>,
        cancel: Arc<AtomicBool>,
    },
    /// Downloaded and checked, now being put in place.
    Installing,
    Restarting,
    /// The update needs the player to do something; maybe with a command to copy.
    Note {
        message: String,
        command: Option<String>,
    },
    Failed(String),
}

pub struct UpdateUi {
    status: UpdateStatus,
    offers: Offers,
    /// The newer release found, once there is one.
    latest: Option<Release>,
    /// Whether the banner is showing.
    shown: bool,
    kind: Kind,
    asset: Option<Asset>,
    stage: Stage,
}

impl Default for UpdateUi {
    fn default() -> Self {
        UpdateUi {
            status: UpdateStatus::NotChecked,
            offers: Offers::default(),
            latest: None,
            shown: false,
            kind: Kind::PageOnly,
            asset: None,
            stage: Stage::Offer,
        }
    }
}

impl App {
    /// Starts the background checks: the first a few seconds after start, then every hour. Also removes what an
    /// earlier update left behind.
    pub(super) fn start_update_checks(&mut self) {
        let temp = self.dirs.temp.clone();
        self.rt.spawn_blocking(move || update::apply::cleanup_leftovers(&temp));
        let n = self.notifier();
        self.rt.spawn(async move {
            tokio::time::sleep(update::check::FIRST_CHECK_DELAY).await;
            loop {
                n.send(Msg::Update(UpdateMsg::Tick));
                tokio::time::sleep(update::check::CHECK_EVERY).await;
            }
        });
    }

    /// What the About page shows about updates.
    pub fn update_status(&self) -> &UpdateStatus {
        &self.upd.status
    }

    /// Looks for a new version now, whatever the "check for launcher updates" setting says. The result shows in
    /// [`App::update_status`] and, when there is a newer version, in the banner.
    pub fn check_for_updates_now(&mut self) {
        self.run_update_check(true);
    }

    /// Whether an update is being downloaded or installed (the About page can disable its button).
    pub fn update_busy(&self) -> bool {
        self.upd.status.is_checking()
            || (self.upd.shown && matches!(self.upd.stage, Stage::Downloading { .. } | Stage::Installing))
    }

    /// Shows the banner again for the newer version found earlier (after "Later" or "Skip this version").
    pub fn show_update_offer(&mut self) {
        if self.upd.latest.is_some() && !self.upd.shown {
            self.upd.stage = Stage::Offer;
            self.upd.shown = true;
        }
    }

    fn run_update_check(&mut self, manual: bool) {
        if self.upd.status.is_checking() || (self.upd.shown && !matches!(self.upd.stage, Stage::Offer)) {
            return;
        }
        self.upd.status = UpdateStatus::Checking;
        let n = self.notifier();
        self.rt.spawn(async move {
            let result = update::check::check().await.map_err(|e| format!("{e:#}"));
            n.send(Msg::Update(UpdateMsg::Checked { manual, result }));
        });
    }

    pub(super) fn handle_update(&mut self, msg: UpdateMsg) {
        match msg {
            UpdateMsg::Tick => {
                if self.settings.check_for_launcher_updates {
                    self.run_update_check(false);
                }
            }
            UpdateMsg::Checked { manual, result } => self.update_checked(manual, result),
            UpdateMsg::Finished(result) => self.update_finished(result),
            UpdateMsg::Cancelled => {
                self.upd.stage = Stage::Offer;
            }
        }
    }

    fn update_checked(&mut self, manual: bool, result: Result<Found, String>) {
        match result {
            Ok(Found::Newer(release)) => {
                let tag = release.tag_name.clone();
                self.upd.status = UpdateStatus::Available(update::display_version(&tag).to_string());
                let offer = if manual {
                    self.upd.offers.mark_offered(&tag);
                    true
                } else {
                    self.upd.offers.should_offer(&tag, self.settings.skipped_launcher_update.as_deref())
                };
                self.remember_release(*release);
                if offer && matches!(self.upd.stage, Stage::Offer) {
                    self.upd.shown = true;
                }
            }
            Ok(Found::UpToDate | Found::NoRelease) => {
                self.upd.status = UpdateStatus::UpToDate;
            }
            Err(e) => {
                crate::logs::debug(format!("Update check failed: {e}"));
                self.upd.status = UpdateStatus::Error(e);
            }
        }
    }

    /// Keeps the newer release and works out which of its files fits this install.
    fn remember_release(&mut self, release: Release) {
        // a download in progress belongs to the release that started it
        if self.upd.shown && !matches!(self.upd.stage, Stage::Offer) {
            return;
        }
        let kind = super::about::install_kind();
        self.upd.asset = update::pick_asset(&release, kind, std::env::consts::ARCH).cloned();
        self.upd.kind = kind;
        self.upd.latest = Some(release);
    }

    fn can_install_update(&self) -> bool {
        self.upd.asset.is_some() && self.upd.kind.can_install()
    }

    /// Downloads the update, checks it, puts it in place and restarts.
    fn start_update(&mut self) {
        let (Some(release), Some(asset)) = (self.upd.latest.clone(), self.upd.asset.clone()) else { return };
        let kind = self.upd.kind;
        let progress = Arc::new(update::DownloadProgress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        self.upd.stage = Stage::Downloading { progress: progress.clone(), cancel: cancel.clone() };
        let (folder, n) = (update::apply::update_folder(&self.dirs.temp), self.notifier());
        self.rt.spawn(async move {
            let outcome: anyhow::Result<Installed> = async {
                let expected = update::download::expected_sha256(&release, &asset).await?;
                if expected.is_none() {
                    crate::logs::warn(format!(
                        "The release has no checksum for {}; only its size is checked",
                        asset.name
                    ));
                }
                let file = update::download::download(&asset, expected.as_deref(), &folder, &progress, &cancel).await?;
                n.send(Msg::Update(UpdateMsg::Finished(Ok(Installed::Stay { message: String::new(), command: None }))));
                tokio::task::spawn_blocking(move || update::apply::install(&file, kind)).await?
            }
            .await;
            match outcome {
                Err(e) if update::download::was_cancelled(&e) => n.send(Msg::Update(UpdateMsg::Cancelled)),
                other => n.send(Msg::Update(UpdateMsg::Finished(other.map_err(|e| format!("{e:#}"))))),
            }
        });
    }

    fn update_finished(&mut self, result: Result<Installed, String>) {
        match result {
            // the empty note marks "downloaded and checked, installing now"
            Ok(Installed::Stay { message, .. }) if message.is_empty() => self.upd.stage = Stage::Installing,
            Ok(Installed::Exit) => {
                self.upd.stage = Stage::Restarting;
                // the launcher closes itself to finish the update: no "Quit anyway?" for that
                self.quit_anyway = true;
                update::apply::exit_soon();
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Ok(Installed::Stay { message, command }) => self.upd.stage = Stage::Note { message, command },
            Err(e) => {
                crate::logs::warn(format!("The launcher update failed: {e}"));
                self.upd.stage = Stage::Failed(e);
            }
        }
    }

    /// The window that offers a new BlockForge, with what is new in it. It opens by itself when a check finds a
    /// version the player hasn't skipped; nothing is installed until "Update now" is pressed.
    pub(super) fn update_dialog(&mut self, ctx: &egui::Context) {
        if !self.upd.shown {
            return;
        }
        let Some(release) = self.upd.latest.clone() else { return };
        let version = update::display_version(&release.tag_name).to_string();
        // installing ends with a restart: jobs in progress would stop halfway, and a game that runs on would be lost
        // to the launcher (no log, no Stop, no backup after it)
        let wait = if self.games.values().any(super::Game::active) {
            Some("Close Minecraft first. The launcher restarts to finish the update.")
        } else if !self.tasks.is_empty() {
            Some("Wait for the jobs in progress to finish. The launcher restarts to finish the update.")
        } else {
            None
        };
        let can_install = self.can_install_update();
        // what is being done can't be walked away from; what is only being offered or said can
        let busy = matches!(self.upd.stage, Stage::Downloading { .. } | Stage::Installing | Stage::Restarting);
        let action: Cell<Option<Action>> = Cell::new(None);
        let (width, notes_height) = super::curseforge::modal_size(ctx, 560.0, 300.0);
        let logo = self.logo.clone();
        let modal =
            egui::Modal::new(egui::Id::unique("update-dialog")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(width);
                match &self.upd.stage {
                    Stage::Offer => {
                        let title = format!("BlockForge {version} is available");
                        if update_head(ui, &logo, &title, Some(&format!("You have {}", crate::VERSION)), true) {
                            action.set(Some(Action::Later));
                        }
                        ui.add_space(14.0);
                        if !release.body.trim().is_empty() {
                            egui::Frame::new()
                                .fill(c_card())
                                .stroke(Stroke::new(1.0, c_border()))
                                .corner_radius(CornerRadius::same(R_CARD))
                                .inner_margin(egui::Margin::symmetric(16, 14))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    egui::ScrollArea::vertical()
                                        .id_salt("update-notes")
                                        .max_height(notes_height.min(320.0))
                                        .show(ui, |ui| release_notes(ui, &release.body));
                                });
                            ui.add_space(14.0);
                        }
                        if let Some(wait) = wait.filter(|_| can_install) {
                            notice(ui, NoticeKind::Warn, wait, None);
                            ui.add_space(12.0);
                        }
                        dialog_footer(
                            ui,
                            "update-offer",
                            |ui| {
                                if ghost(ui, "Skip this version").clicked() {
                                    action.set(Some(Action::Skip));
                                }
                                if can_install && ghost(ui, "Release page").clicked() {
                                    action.set(Some(Action::OpenPage));
                                }
                            },
                            |ui| {
                                if secondary(ui, "Later").clicked() {
                                    action.set(Some(Action::Later));
                                }
                                if can_install {
                                    let now = ui.add_enabled_ui(wait.is_none(), |ui| primary(ui, "Update now")).inner;
                                    if now.on_disabled_hover_text(wait.unwrap_or_default()).clicked() {
                                        action.set(Some(Action::Install));
                                    }
                                } else if primary(ui, "Get the update").clicked() {
                                    action.set(Some(Action::OpenPage));
                                }
                            },
                        );
                    }
                    Stage::Downloading { progress, .. } => {
                        let (done, total) =
                            (progress.done.load(Ordering::Relaxed), progress.total.load(Ordering::Relaxed));
                        update_head(ui, &logo, &format!("Downloading BlockForge {version}"), None, false);
                        ui.add_space(16.0);
                        progress_bar(ui, (total > 0).then(|| progress.fraction()), 8.0);
                        ui.add_space(6.0);
                        let text = if total > 0 {
                            format!("{} of {}", human_size(done), human_size(total))
                        } else {
                            human_size(done)
                        };
                        ui.horizontal(|ui| {
                            small_muted(ui, text);
                            if total > 0 {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let percent = format!("{:.0}%", progress.fraction() * 100.0);
                                    ui.label(RichText::new(percent).size(FS_META).font(super::fonts::medium(FS_META)));
                                });
                            }
                        });
                        ui.add_space(14.0);
                        dialog_footer(
                            ui,
                            "update-downloading",
                            |_| {},
                            |ui| {
                                if secondary(ui, "Cancel").clicked() {
                                    action.set(Some(Action::Cancel));
                                }
                            },
                        );
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                    }
                    Stage::Installing => {
                        update_head(ui, &logo, &installing_text(self.upd.kind, &version), None, false);
                        ui.add_space(16.0);
                        progress_bar(ui, None, 8.0);
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                    }
                    Stage::Restarting => {
                        update_head(ui, &logo, &closing_text(self.upd.kind, &version), None, false);
                    }
                    Stage::Note { message, command } => {
                        if update_head(ui, &logo, &format!("BlockForge {version}"), None, true) {
                            action.set(Some(Action::Later));
                        }
                        ui.add_space(12.0);
                        ui.add(egui::Label::new(RichText::new(message)).wrap());
                        if let Some(command) = command {
                            ui.add_space(10.0);
                            // the command on a well of its own, so it reads as something to copy
                            egui::Frame::new()
                                .fill(c_card())
                                .stroke(Stroke::new(1.0, c_border()))
                                .corner_radius(CornerRadius::same(R_CONTROL))
                                .inner_margin(egui::Margin::symmetric(12, 10))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.add(egui::Label::new(RichText::new(command).monospace().color(c_text())).wrap());
                                });
                        }
                        ui.add_space(16.0);
                        dialog_footer(
                            ui,
                            "update-note",
                            |_| {},
                            |ui| {
                                if secondary(ui, "Close").clicked() {
                                    action.set(Some(Action::Later));
                                }
                                if let Some(command) = command {
                                    if primary(ui, "Copy command").clicked() {
                                        action.set(Some(Action::Copy(command.clone())));
                                    }
                                }
                            },
                        );
                    }
                    Stage::Failed(error) => {
                        if update_head(ui, &logo, &format!("Couldn't update to BlockForge {version}"), None, true) {
                            action.set(Some(Action::Later));
                        }
                        ui.add_space(12.0);
                        notice(ui, NoticeKind::Error, error, None);
                        if let Some(wait) = wait {
                            ui.add_space(8.0);
                            small_muted(ui, wait);
                        }
                        ui.add_space(16.0);
                        dialog_footer(
                            ui,
                            "update-failed",
                            |ui| {
                                if ghost(ui, "Download from the website").clicked() {
                                    action.set(Some(Action::OpenPage));
                                }
                            },
                            |ui| {
                                if secondary(ui, "Later").clicked() {
                                    action.set(Some(Action::Later));
                                }
                                let again = ui.add_enabled_ui(wait.is_none(), |ui| primary(ui, "Try again")).inner;
                                if again.on_disabled_hover_text(wait.unwrap_or_default()).clicked() {
                                    action.set(Some(Action::Install));
                                }
                            },
                        );
                    }
                }
            });
        // Escape or a click beside the window is "Later", except while the update is being fetched or put in place
        let mut action = action.into_inner();
        if modal.should_close() && !busy && action.is_none() {
            action = Some(Action::Later);
        }
        match action {
            Some(Action::Install) => self.start_update(),
            Some(Action::OpenPage) => {
                let url = if release.html_url.is_empty() {
                    "https://github.com/riyadmondol2006/BlockForge/releases".to_string()
                } else {
                    release.html_url.clone()
                };
                let _ = open::that_detached(url);
            }
            Some(Action::Skip) => {
                self.settings.skipped_launcher_update = Some(release.tag_name.clone());
                self.save_settings();
                self.upd.shown = false;
                self.upd.stage = Stage::Offer;
            }
            Some(Action::Later) => {
                self.upd.shown = false;
                self.upd.stage = Stage::Offer;
            }
            Some(Action::Cancel) => {
                if let Stage::Downloading { cancel, .. } = &self.upd.stage {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            Some(Action::Copy(command)) => {
                self.ctx.copy_text(command);
                self.toast("Command copied");
            }
            None => {}
        }
    }
}

/// One piece of release notes.
#[derive(Debug, PartialEq)]
enum Note {
    Heading(String),
    Bullet(String),
    Text(String),
}

/// Release notes as GitHub has them (the version's section of CHANGELOG.md) in pieces to show: headings, bullets and
/// paragraphs, with the lines a bullet or paragraph runs on over joined, and the Markdown marks taken out.
fn notes(body: &str) -> Vec<Note> {
    let mut out: Vec<Note> = Vec::new();
    // whether the next plain line goes on with the last bullet or paragraph
    let mut open = false;
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() {
            open = false;
        } else if let Some(h) = t.strip_prefix('#') {
            out.push(Note::Heading(plain(h.trim_start_matches('#').trim())));
            open = false;
        } else if let Some(b) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            out.push(Note::Bullet(plain(b)));
            open = true;
        } else if let (true, Some(Note::Bullet(prev) | Note::Text(prev))) = (open, out.last_mut()) {
            prev.push(' ');
            prev.push_str(&plain(t));
        } else {
            out.push(Note::Text(plain(t)));
            open = true;
        }
    }
    out
}

/// A line of Markdown as plain text: `**bold**`, `` `code` `` and `[links](to)` keep only their words.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find("](").and_then(|j| after[j..].find(')').map(|k| (j, j + k))) {
            Some((j, end)) => {
                out.push_str(&after[..j]);
                rest = &after[end + 1..];
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace("__", "").replace('`', "")
}

fn release_notes(ui: &mut Ui, body: &str) {
    ui.spacing_mut().item_spacing.y = 4.0;
    for (n, note) in notes(body).into_iter().enumerate() {
        match note {
            Note::Heading(text) => {
                if n > 0 {
                    ui.add_space(6.0);
                }
                ui.label(RichText::new(text).semibold());
            }
            Note::Bullet(text) => {
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new("\u{2022}").color(c_muted()));
                    ui.add(egui::Label::new(RichText::new(text).color(c_text())).wrap());
                });
            }
            Note::Text(text) => {
                ui.add(egui::Label::new(RichText::new(text).color(c_muted())).wrap());
            }
        }
    }
}

/// The head of the update window: the logo glowing softly, the title (wrapped when long) and an optional line under
/// it, and at the right the close button when the window can be closed. Returns whether that was pressed.
fn update_head(ui: &mut Ui, logo: &egui::TextureHandle, title: &str, sub: Option<&str>, closable: bool) -> bool {
    let mut closed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
        super::about::brand_glow(ui.painter(), rect.center(), 32.0, 0.6);
        egui::Image::from_texture(logo).paint_at(ui, rect);
        let close_w = if closable { 28.0 + ui.spacing().item_spacing.x } else { 0.0 };
        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - close_w).max(120.0));
            ui.spacing_mut().item_spacing.y = 2.0;
            if sub.is_none() {
                ui.add_space(9.0);
            }
            ui.add(egui::Label::new(RichText::new(title).size(FS_SECTION).semibold()).wrap());
            if let Some(sub) = sub {
                small_muted(ui, sub);
            }
        });
        if closable {
            closed =
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| close_button(ui).clicked()).inner;
        }
    });
    closed
}

/// The window's line while the downloaded update is being put in place.
fn installing_text(kind: Kind, version: &str) -> String {
    match kind {
        // the package manager asks for the password in a window of its own
        Kind::Deb | Kind::Rpm => {
            format!("Installing BlockForge {version}. Enter your password if you are asked for it")
        }
        Kind::WindowsInstaller => format!("Starting the BlockForge {version} installer\u{2026}"),
        Kind::MacApp => format!("Opening the BlockForge {version} disk image\u{2026}"),
        _ => format!("Installing BlockForge {version}\u{2026}"),
    }
}

/// The window's line just before this copy closes for the update.
fn closing_text(kind: Kind, version: &str) -> String {
    match kind {
        Kind::WindowsInstaller => {
            format!("The BlockForge {version} installer is starting. BlockForge closes so it can be updated")
        }
        Kind::MacApp => {
            "The disk image is open. Drag BlockForge into Applications, replace the old copy, then open it again"
                .to_string()
        }
        _ => format!("BlockForge {version} is ready. Restarting\u{2026}"),
    }
}

enum Action {
    Install,
    OpenPage,
    Skip,
    Later,
    Cancel,
    Copy(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_notes_read_as_headings_bullets_and_paragraphs() {
        let body = "Quicker downloads, and a [fix](https://example.com/1).\n\n### Fixed\n\n- **The bar.** It no longer\n  covers `Tools`.\n- Stalls retry.\n\n## 4.8.0\nOlder text\nthat goes on.";
        assert_eq!(
            notes(body),
            vec![
                Note::Text("Quicker downloads, and a fix.".into()),
                Note::Heading("Fixed".into()),
                Note::Bullet("The bar. It no longer covers Tools.".into()),
                Note::Bullet("Stalls retry.".into()),
                Note::Heading("4.8.0".into()),
                Note::Text("Older text that goes on.".into()),
            ]
        );
        assert_eq!(plain("[a](b) and [not a link"), "a and [not a link");
    }

    #[test]
    fn the_banner_says_what_happens_for_each_kind_of_install() {
        assert!(installing_text(Kind::Deb, "4.2.0").contains("password"));
        assert!(installing_text(Kind::Rpm, "4.2.0").contains("password"));
        assert!(installing_text(Kind::WindowsInstaller, "4.2.0").contains("installer"));
        assert!(closing_text(Kind::MacApp, "4.2.0").contains("Applications"), "a disk image doesn't restart by itself");
        assert!(closing_text(Kind::WindowsInstaller, "4.2.0").contains("installer"));
        for kind in [Kind::AppImage, Kind::WindowsPortable, Kind::Deb, Kind::Rpm] {
            assert!(closing_text(kind, "4.2.0").contains("Restarting"), "{kind:?}");
            assert!(installing_text(kind, "4.2.0").contains("4.2.0"), "{kind:?}");
        }
    }
}
