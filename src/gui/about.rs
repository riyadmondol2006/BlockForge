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

/// How this copy was installed. Finding out may ask the package manager (`dpkg -S`, `rpm -qf`), so it is done once
/// and remembered; it doesn't change while the launcher runs.
pub fn install_kind() -> Kind {
    static KIND: std::sync::OnceLock<Kind> = std::sync::OnceLock::new();
    *KIND.get_or_init(update::install_kind)
}

/// [`os_text`], worked out once (it reads a file or runs a program).
fn os_text_cached() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(os_text)
}

/// The version with the commit it was built from when CI recorded one: `4.0.0 (a1b2c3d)`.
pub fn version_text() -> String {
    version_with_commit(crate::VERSION, option_env!("GITHUB_SHA"))
}

fn version_with_commit(version: &str, sha: Option<&str>) -> String {
    match sha.map(str::trim).and_then(|s| s.get(..7)).filter(|s| s.chars().all(|c| c.is_ascii_hexdigit())) {
        Some(short) => format!("{version} ({short})"),
        None => version.to_string(),
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
            install_label(install_kind()),
            os_text_cached(),
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
        let kind = install_kind();
        let can_install = kind.can_install();
        let (total_mb, _) = super::settings::memory_now();
        let mut link: Option<String> = None;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // at most 760 wide, but never wider than the window (set_max_width alone would widen a narrow page)
            ui.set_max_width((ui.available_width() - 14.0).min(760.0));
            ui.horizontal(|ui| {
                ui.add(egui::Image::from_texture(&self.logo).fit_to_exact_size(vec2(64.0, 64.0)));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new(crate::NAME).size(24.0).semibold());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(RichText::new(format!("Version {}", version_text())).semibold());
                        small_muted(ui, "\u{00B7}");
                        muted(ui, install_label(kind));
                    });
                    muted(ui, "A fast, modern Minecraft launcher, written in Rust.");
                });
            });

            ui.add_space(16.0);
            section(ui, "Updates");
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if available.is_some() {
                            // the banner's words: a copy that can't update itself gets the download page
                            if primary(ui, if can_install { "Update now" } else { "Get the update" }).clicked() {
                                show_offer = true;
                            }
                        } else if busy {
                            danger_enabled(ui, "Check for updates", false);
                        } else if secondary(ui, "Check for updates").clicked() {
                            check = true;
                        }
                        // one line of status; a failed check can say a lot, so it wraps beside the button
                        ui.with_layout(Layout::left_to_right(Align::Center).with_main_wrap(true), |ui| {
                            if busy {
                                ui.spinner();
                            }
                            let color = if is_error { c_danger() } else { c_text() };
                            ui.add(egui::Label::new(RichText::new(&status_text).color(color)).wrap());
                        });
                    });
                });
                if !self.settings.check_for_launcher_updates {
                    small_muted(ui, "Automatic update checks are off in Settings.");
                }
            });

            ui.add_space(12.0);
            section(ui, "Built on ATLauncher");
            card(ui, |ui| {
                ui.label(
                    "BlockForge is a fork of ATLauncher, rewritten in Rust. The way instances, accounts and modpacks \
                     are stored, and how games are launched, follow ATLauncher's, so your existing instances keep \
                     working. ATLauncher's authors and contributors made that foundation, and their copyright \
                     notices are kept.",
                );
                small_muted(ui, "BlockForge is free software under the GNU General Public License v3.");
                ui.add_space(4.0);
                if let Some(url) = links(
                    ui,
                    &[
                        ("ATLauncher on GitHub", "https://github.com/ATLauncher/ATLauncher".to_string()),
                        ("License", format!("{REPO}/blob/main/LICENSE")),
                        ("Notices", format!("{REPO}/blob/main/NOTICE.md")),
                    ],
                ) {
                    link = Some(url);
                }
            });

            // the translation key is upper case; without a translation it is shown in sentence case
            let credits = crate::tr!("CREDITS");
            ui.add_space(12.0);
            section(ui, if credits == "CREDITS" { "Credits" } else { &credits });
            card(ui, |ui| {
                bullet(
                    ui,
                    &crate::tr!(
                        "ATLauncher, by the ATLauncher team and its contributors (GPL-3.0): the foundation BlockForge is built on."
                    ),
                );
                #[cfg(feature = "converter")]
                {
                    bullet(
                        ui,
                        &crate::tr!("Chunker, by Hive Games (MIT License): the world converter is a Rust port of it."),
                    );
                    small_muted(
                        ui,
                        crate::tr!("BlockForge is not affiliated with or endorsed by Hive Games, the makers of Chunker."),
                    );
                    ui.add_space(4.0);
                    if let Some(url) = links(
                        ui,
                        &[
                            (&crate::tr!("Chunker on GitHub"), "https://github.com/HiveGamesOSS/Chunker".to_string()),
                            (&crate::tr!("Chunker license"), format!("{REPO}/blob/main/NOTICE.md")),
                        ],
                    ) {
                        link = Some(url);
                    }
                }
            });

            ui.add_space(12.0);
            section(ui, "What it does");
            card(ui, |ui| {
                for line in [
                    "Play offline with any name, or sign in with Microsoft.",
                    "ATLauncher, CurseForge, Modrinth, FTB and Technic modpacks in one place, or any version with \
                     Fabric, Forge, NeoForge or Quilt.",
                    "Mods, resource packs and shaders with updates, dependencies and a safety scan.",
                    "Brings in instances from Prism, MultiMC, CurseForge and more, and exports them again.",
                    "Fast to open, with memory picked for your PC and light mode for older computers.",
                    "Private: no statistics and no crash reports are sent unless you turn that on in Settings.",
                ] {
                    bullet(ui, line);
                }
            });

            ui.add_space(12.0);
            section(ui, "This copy");
            card(ui, |ui| {
                egui::Grid::new("about-grid").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                    for (k, v) in [
                        ("Installed as", install_label(kind).to_string()),
                        ("System", format!("{} ({})", os_text_cached(), std::env::consts::ARCH)),
                        ("Memory", format!("{total_mb} MB")),
                        ("Data folder", self.dirs.base.display().to_string()),
                    ] {
                        muted(ui, k);
                        ui.add(egui::Label::new(v).truncate());
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if secondary_icon(ui, Icon::Folder, "Open data folder").clicked() {
                        open_data = true;
                    }
                    if secondary(ui, "Copy details").clicked() {
                        copy = true;
                    }
                });
            });

            ui.add_space(12.0);
            section(ui, "Help");
            card(ui, |ui| {
                if let Some(url) = links(
                    ui,
                    &[
                        ("GitHub", REPO.to_string()),
                        ("Report a problem", format!("{REPO}/issues/new/choose")),
                        ("Releases", format!("{REPO}/releases")),
                        ("Privacy", format!("{REPO}/blob/main/PRIVACY.md")),
                    ],
                ) {
                    link = Some(url);
                }
                ui.horizontal(|ui| {
                    small_muted(ui, "Game crashed?");
                    if ui.link(RichText::new("Make a support bundle").underline().color(c_text())).clicked() {
                        self.page = Page::Tools;
                    }
                });
            });
            ui.add_space(12.0);
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
            self.open_folder(&self.dirs.base.clone());
        }
        if let Some(url) = link {
            self.open_link(&url);
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

/// A row of text links. Returns the address of the one that was clicked.
fn links(ui: &mut Ui, items: &[(&str, String)]) -> Option<String> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        // text links in the text colour, underlined, so a page of links is not a page of green
        ui.visuals_mut().hyperlink_color = c_text();
        for (label, url) in items {
            if ui.link(RichText::new(*label).underline()).clicked() {
                clicked = Some(url.clone());
            }
        }
    });
    clicked
}

/// One line of a quiet list: a small dash and the text, wrapped.
fn bullet(ui: &mut Ui, text: &str) {
    ui.horizontal_top(|ui| {
        ui.label(RichText::new("\u{2013}").color(c_faint()));
        ui.add(egui::Label::new(text).wrap());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commit_is_shown_only_when_it_looks_like_one() {
        assert_eq!(version_with_commit("4.1.0", Some("a1b2c3d4e5f6")), "4.1.0 (a1b2c3d)");
        assert_eq!(version_with_commit("4.1.0", None), "4.1.0");
        assert_eq!(version_with_commit("4.1.0", Some("abc")), "4.1.0", "too short");
        // a byte-offset slice would have cut this one inside a letter
        assert_eq!(version_with_commit("4.1.0", Some("\u{00e9}\u{00e9}\u{00e9}\u{00e9}x")), "4.1.0");
    }
}
