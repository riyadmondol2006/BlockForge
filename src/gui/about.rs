//! The About page: what BlockForge is, which version this is and how it was installed, the update check, credits and
//! links, and a "Copy details" button for bug reports.

use egui::{Align, Layout, RichText, ScrollArea, Ui, vec2};

use super::icons::Icon;
use super::theme::*;
use super::widgets::*;
use super::{App, Page};
use crate::update::{self, Kind};

const REPO: &str = "https://github.com/riyadmondol2006/BlockForge";

/// How this copy was installed, in words for the page.
fn install_label(kind: Kind) -> &'static str {
    match kind {
        Kind::AppImage => "AppImage",
        Kind::Deb => "Debian package (.deb)",
        Kind::Rpm => "RPM package (.rpm)",
        Kind::WindowsInstaller => "Windows installer",
        Kind::WindowsPortable => "Portable program (.exe)",
        Kind::MacApp => "macOS app",
        Kind::PageOnly => "From source or a system package",
    }
}

/// The version with the commit it was built from when CI recorded one: `4.0.0 (a1b2c3d)`.
pub fn version_text() -> String {
    match option_env!("GITHUB_SHA").filter(|s| s.len() >= 7) {
        Some(sha) => format!("{} ({})", crate::VERSION, &sha[..7]),
        None => crate::VERSION.to_string(),
    }
}

/// The operating system in words, like "Fedora Linux 45" or "macOS 15.1".
fn os_text() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(text) = std::fs::read_to_string("/etc/os-release") {
            if let Some(line) = text.lines().find(|l| l.starts_with("PRETTY_NAME=")) {
                return line.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_string();
            }
        }
        "Linux".to_string()
    }
    #[cfg(target_os = "macos")]
    {
        let out = crate::platform::command("sw_vers").arg("-productVersion").output().ok();
        match out.and_then(|o| String::from_utf8(o.stdout).ok()) {
            Some(v) if !v.trim().is_empty() => format!("macOS {}", v.trim()),
            _ => "macOS".to_string(),
        }
    }
    #[cfg(target_os = "windows")]
    {
        let out = crate::platform::command("cmd").args(["/C", "ver"]).output().ok();
        match out.and_then(|o| String::from_utf8(o.stdout).ok()) {
            Some(v) if !v.trim().is_empty() => v.trim().to_string(),
            _ => "Windows".to_string(),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        std::env::consts::OS.to_string()
    }
}

/// The folder with the user's name replaced, so the details can be pasted into a public issue.
fn shortened(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    match std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        Ok(home) if !home.is_empty() => text.replacen(&home, "~", 1),
        _ => text,
    }
}

impl App {
    /// The plain-text details people paste into a bug report.
    pub(super) fn about_details(&self) -> String {
        let (total, free) = crate::platform::memory_mb();
        format!(
            "BlockForge {}\nInstalled as: {}\nSystem: {} ({}), {} MB memory, {} MB free\nData folder: {}",
            version_text(),
            install_label(update::install_kind()),
            os_text(),
            std::env::consts::ARCH,
            total,
            free,
            shortened(&self.dirs.base),
        )
    }

    pub(super) fn about_ui(&mut self, ui: &mut Ui) {
        let mut copy = false;
        let mut open_data = false;
        let mut check = false;
        let mut show_offer = false;
        let busy = self.update_busy();
        let status_text = self.update_status().text();
        let available = self.update_status().available().map(str::to_string);
        let is_error = self.update_status().is_error();
        let kind = update::install_kind();
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(780.0);
            ui.horizontal(|ui| {
                ui.add(egui::Image::from_texture(&self.logo).fit_to_exact_size(vec2(84.0, 84.0)));
                ui.vertical(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new(crate::NAME).size(30.0).strong());
                    muted(ui, "A fast, modern Minecraft launcher, written in Rust.");
                    ui.horizontal(|ui| {
                        tag(ui, &format!("Version {}", version_text()), c_accent());
                        tag(ui, install_label(kind), c_muted());
                    });
                });
            });
            ui.add_space(8.0);

            card(ui, |ui| {
                ui.label(RichText::new("UPDATES").size(11.0).strong().color(c_accent()));
                ui.horizontal(|ui| {
                    if busy {
                        ui.spinner();
                    }
                    let color = if is_error { c_danger() } else { c_text() };
                    ui.label(RichText::new(&status_text).color(color));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if available.is_some() {
                            if primary(ui, "Update now").clicked() {
                                show_offer = true;
                            }
                        } else if primary_enabled(ui, "Check for updates", !busy).clicked() {
                            check = true;
                        }
                    });
                });
                if !self.settings.check_for_launcher_updates {
                    small_muted(ui, "Automatic update checks are off in Settings.");
                }
            });
            ui.add_space(6.0);

            card(ui, |ui| {
                ui.label(RichText::new("BUILT ON ATLAUNCHER").size(11.0).strong().color(c_accent()));
                ui.label(
                    "BlockForge is a fork of ATLauncher, rewritten in Rust. The way instances, accounts and modpacks \
                     are stored, and how games are launched, follow ATLauncher's, so your existing instances keep \
                     working. ATLauncher's authors and contributors made that foundation, and their copyright \
                     notices are kept.",
                );
                small_muted(ui, "BlockForge is free software under the GNU General Public License v3.");
                ui.horizontal(|ui| {
                    if secondary(ui, "ATLauncher on GitHub").clicked() {
                        let _ = open::that("https://github.com/ATLauncher/ATLauncher");
                    }
                    if secondary(ui, "License").clicked() {
                        let _ = open::that(format!("{REPO}/blob/main/LICENSE"));
                    }
                    if secondary(ui, "Notices").clicked() {
                        let _ = open::that(format!("{REPO}/blob/main/NOTICE.md"));
                    }
                });
            });
            ui.add_space(6.0);

            card(ui, |ui| {
                ui.label(RichText::new("WHAT IT DOES").size(11.0).strong().color(c_accent()));
                for line in [
                    "Play offline with any name, or sign in with Microsoft.",
                    "ATLauncher, CurseForge, Modrinth, FTB and Technic modpacks in one place, or any version with \
                     Fabric, Forge, NeoForge or Quilt.",
                    "Mods, resource packs and shaders with updates, dependencies and a safety scan.",
                    "Brings in instances from Prism, MultiMC, CurseForge and more, and exports them again.",
                    "Fast to open, with memory picked for your PC and light mode for older computers.",
                    "Private: no statistics and no crash reports are ever sent.",
                ] {
                    ui.horizontal_top(|ui| {
                        ui.label(RichText::new("\u{2022}").color(c_accent()));
                        ui.add(egui::Label::new(line).wrap());
                    });
                }
            });
            ui.add_space(6.0);

            card(ui, |ui| {
                ui.label(RichText::new("THIS COPY").size(11.0).strong().color(c_accent()));
                let (total, _) = crate::platform::memory_mb();
                egui::Grid::new("about-grid").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
                    for (k, v) in [
                        ("Installed as", install_label(kind).to_string()),
                        ("System", format!("{} ({})", os_text(), std::env::consts::ARCH)),
                        ("Memory", format!("{total} MB")),
                    ] {
                        muted(ui, k);
                        ui.label(v);
                        ui.end_row();
                    }
                });
                muted(ui, "Data folder");
                ui.add(egui::Label::new(self.dirs.base.display().to_string()).truncate());
                ui.horizontal(|ui| {
                    if secondary_icon(ui, Icon::Folder, "Open data folder").clicked() {
                        open_data = true;
                    }
                    if secondary(ui, "Copy details").clicked() {
                        copy = true;
                    }
                });
            });
            ui.add_space(6.0);

            card(ui, |ui| {
                ui.label(RichText::new("HELP").size(11.0).strong().color(c_accent()));
                ui.horizontal(|ui| {
                    if secondary(ui, "GitHub").clicked() {
                        let _ = open::that(REPO);
                    }
                    if secondary(ui, "Report a problem").clicked() {
                        let _ = open::that(format!("{REPO}/issues/new/choose"));
                    }
                    if secondary(ui, "Releases").clicked() {
                        let _ = open::that(format!("{REPO}/releases"));
                    }
                    if secondary(ui, "Privacy").clicked() {
                        let _ = open::that(format!("{REPO}/blob/main/PRIVACY.md"));
                    }
                });
                ui.horizontal(|ui| {
                    small_muted(ui, "Game crashed?");
                    if ui.link("Make a support bundle").clicked() {
                        self.page = Page::Tools;
                    }
                });
            });
            ui.add_space(8.0);
            small_muted(
                ui,
                "\"Minecraft\" is a trademark of Mojang AB. BlockForge is not made by or approved by Mojang or Microsoft.",
            );
            ui.add_space(16.0);
        });

        if copy {
            ui.ctx().copy_text(self.about_details());
            self.toast("Copied the details");
        }
        if open_data {
            let _ = open::that(&self.dirs.base);
        }
        if check {
            self.check_for_updates_now();
        }
        if show_offer {
            self.show_update_offer();
            self.page = Page::Home;
        }
    }
}
