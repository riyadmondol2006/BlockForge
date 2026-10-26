//! BlockForge updating itself: the background check, the banner that offers a new version ("Update now" downloads it
//! with a progress bar and restarts), and what the About page shows about updates.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use egui::{CornerRadius, RichText, Stroke, Ui};

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
    /// Starts the background checks: the first a few seconds after start, then every few hours. Also removes what an
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

    /// The banner that offers a new BlockForge. Nothing is installed until "Update now" is pressed.
    pub(super) fn update_banner(&mut self, ui: &mut Ui) {
        if !self.upd.shown {
            return;
        }
        let Some(release) = self.upd.latest.clone() else { return };
        let version = update::display_version(&release.tag_name).to_string();
        let mut action = None;
        egui::Frame::new()
            .fill(c_card())
            .stroke(Stroke::new(1.0, c_border()))
            .corner_radius(CornerRadius::same(R_CARD))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                match &self.upd.stage {
                    Stage::Offer => {
                        let can_install = self.can_install_update();
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("BlockForge {version} is available")).semibold());
                            small_muted(ui, format!("You have {}", crate::VERSION));
                        });
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            if can_install {
                                if primary(ui, "Update now").clicked() {
                                    action = Some(Action::Install);
                                }
                                if secondary(ui, "See what's new").clicked() {
                                    action = Some(Action::OpenPage);
                                }
                            } else if primary(ui, "Get the update").clicked() {
                                action = Some(Action::OpenPage);
                            }
                            if ghost(ui, "Skip this version").clicked() {
                                action = Some(Action::Skip);
                            }
                            if ghost(ui, "Later").clicked() {
                                action = Some(Action::Later);
                            }
                        });
                    }
                    Stage::Downloading { progress, .. } => {
                        let (done, total) =
                            (progress.done.load(Ordering::Relaxed), progress.total.load(Ordering::Relaxed));
                        ui.label(RichText::new(format!("Downloading BlockForge {version}")).semibold());
                        ui.add_space(4.0);
                        progress_bar(ui, (total > 0).then(|| progress.fraction()), 6.0);
                        ui.horizontal(|ui| {
                            let text = if total > 0 {
                                format!("{} of {}", human_size(done), human_size(total))
                            } else {
                                human_size(done)
                            };
                            small_muted(ui, text);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ghost(ui, "Cancel").clicked() {
                                    action = Some(Action::Cancel);
                                }
                            });
                        });
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                    }
                    Stage::Installing => {
                        ui.label(RichText::new(installing_text(self.upd.kind, &version)).semibold());
                        ui.add_space(4.0);
                        progress_bar(ui, None, 6.0);
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                    }
                    Stage::Restarting => {
                        ui.label(RichText::new(closing_text(self.upd.kind, &version)).semibold());
                    }
                    Stage::Note { message, command } => {
                        ui.label(RichText::new(message).semibold());
                        if let Some(command) = command {
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(RichText::new(command).monospace().color(c_text())).wrap());
                        }
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            if let Some(command) = command {
                                if primary(ui, "Copy command").clicked() {
                                    action = Some(Action::Copy(command.clone()));
                                }
                            }
                            if secondary(ui, "Close").clicked() {
                                action = Some(Action::Later);
                            }
                        });
                    }
                    Stage::Failed(error) => {
                        ui.label(
                            RichText::new(format!("Couldn't update to BlockForge {version}"))
                                .semibold()
                                .color(c_danger()),
                        );
                        ui.add(egui::Label::new(RichText::new(error).color(c_muted()).size(FS_META)).wrap());
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            if primary(ui, "Try again").clicked() {
                                action = Some(Action::Install);
                            }
                            if secondary(ui, "Download from the website").clicked() {
                                action = Some(Action::OpenPage);
                            }
                            if ghost(ui, "Later").clicked() {
                                action = Some(Action::Later);
                            }
                        });
                    }
                }
            });
        ui.add_space(8.0);
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

/// The banner's line while the downloaded update is being put in place.
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

/// The banner's line just before this copy closes for the update.
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
