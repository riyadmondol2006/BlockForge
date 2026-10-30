//! The About page: what BlockForge is, which version this is and how it was installed, the update check, credits and
//! links, and a "Copy details" button for bug reports.

use egui::{Align, Layout, RichText, ScrollArea, Ui, vec2};

use super::icons::Icon;
use super::motion;
use super::theme::*;
use super::widgets::*;
use super::{App, Page};
use crate::update::{self, Kind, UpdateStatus};

const REPO: &str = "https://github.com/riyadmondol2006/BlockForge";

/// The widest the page's content gets, and the widest a paragraph gets.
const COLUMN: f32 = 720.0;
const PROSE_WIDTH: f32 = 640.0;
/// The column of names in the "This copy" card.
const KEY_WIDTH: f32 = 120.0;

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
        self.account_corner(
            ui,
            super::Page::About,
            "About",
            "The version, updates, and where BlockForge keeps its files.",
            COLUMN,
            |app, ui| app.about_body(ui),
        );
    }

    /// The page itself, beside the list of the account corner.
    fn about_body(&mut self, ui: &mut Ui) {
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
            // at most 720 wide, but never wider than the window (set_max_width alone would widen a narrow page)
            ui.set_max_width((ui.available_width() - 14.0).min(COLUMN));
            // the hero: the logo glowing on a panel of glass, the name and version, the update button at the right
            // and its answer on the line under the version
            let up_to_date = matches!(self.update_status(), UpdateStatus::UpToDate);
            hero(ui, |ui| {
                let button_id = ui.scope_id().with("about-update-button");
                let button_w = remembered_width(ui, button_id, 150.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    let (logo, _) = ui.allocate_exact_size(vec2(64.0, 64.0), egui::Sense::hover());
                    brand_glow(ui.painter(), logo.center(), 96.0, 1.0);
                    egui::Image::from_texture(&self.logo).paint_at(ui, logo);
                    ui.vertical(|ui| {
                        // the version wraps rather than run under the button in a narrow window
                        ui.set_max_width((ui.available_width() - button_w - 16.0).max(140.0));
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.add_space(2.0);
                        ui.label(RichText::new(crate::NAME).size(FS_TITLE).semibold());
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(RichText::new(format!("v{}", version_text())).font(super::fonts::mono(FS_BODY)).color(c_text()));
                            muted(ui, format!("\u{00B7} {}", install_label(kind)));
                        });
                        ui.add_space(2.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if busy {
                                spinner_sized(ui, 14.0);
                            } else {
                                // a dot that says how it stands: the brand colour when all is well or a new version
                                // waits, red when the check failed
                                let color = if is_error {
                                    c_danger()
                                } else if available.is_some() || up_to_date {
                                    c_accent()
                                } else {
                                    c_faint()
                                };
                                let (dot, _) = ui.allocate_exact_size(vec2(8.0, 16.0), egui::Sense::hover());
                                ui.painter().circle_filled(dot.center(), 4.0, color);
                            }
                            let (color, size) = match (is_error, available.is_some()) {
                                (true, _) => (c_danger(), FS_META),
                                (false, true) => (c_text(), FS_BODY),
                                (false, false) => (c_muted(), FS_META),
                            };
                            let mut line = status_text.clone();
                            if !is_error && !self.settings.check_for_launcher_updates {
                                line.push_str(" \u{00B7} Automatic update checks are off in Settings.");
                            }
                            ui.add(egui::Label::new(RichText::new(line).color(color).size(size)).wrap());
                        });
                    });
                    let used = ui
                        .with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if available.is_some() {
                                // the banner's words: a copy that can't update itself gets the download page
                                if primary(ui, if can_install { "Update now" } else { "Get the update" }).clicked() {
                                    show_offer = true;
                                }
                            } else if secondary_enabled(ui, "Check for updates", !busy).clicked() {
                                check = true;
                            }
                            ui.min_rect().width()
                        })
                        .inner;
                    remember_width(ui, button_id, used);
                });
            });

            ui.add_space(26.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("This copy").size(FS_SECTION).semibold());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ghost(ui, "Copy details").clicked() {
                        copy = true;
                    }
                });
            });
            ui.add_space(2.0);
            card(ui, |ui| {
                // the name of a row sits close to its value
                ui.spacing_mut().item_spacing.y = 2.0;
                let mut rows = super::settings::Rows::new();
                for (k, v) in [
                    ("Installed as", install_label(kind).to_string()),
                    ("System", format!("{} ({})", os_text_cached(), std::env::consts::ARCH)),
                    // in GB, as Settings and the instance pages say it
                    ("Memory", super::settings::memory_text(total_mb.min(u64::from(u32::MAX)) as u32)),
                ] {
                    rows.gap(ui);
                    key_value(ui, k, |ui| {
                        // the facts of the computer in the monospace face, like every technical figure
                        ui.add(egui::Label::new(RichText::new(v).font(super::fonts::mono(FS_META + 1.0))).truncate());
                    });
                }
                rows.gap(ui);
                key_value(ui, "Data folder", |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if secondary_icon(ui, Icon::Folder, "Open").clicked() {
                            open_data = true;
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(self.dirs.base.display().to_string()).font(super::fonts::mono(FS_META)),
                                )
                                .truncate(),
                            );
                        });
                    });
                });
            });

            about_title(ui, "Help");
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
                ui.spacing_mut().item_spacing.x = 6.0;
                muted(ui, "Game crashed?");
                if ui.link(RichText::new("Make a support bundle").underline()).clicked() {
                    self.page = Page::Tools;
                }
            });
            });

            about_title(ui, "Built on ATLauncher");
            card(ui, |ui| {
            prose(ui, |ui| {
                ui.add(egui::Label::new(
                    "BlockForge is a fork of ATLauncher, rewritten in Rust. The way instances, accounts and modpacks \
                     are stored, and how games are launched, follow ATLauncher's, so your existing instances keep \
                     working. ATLauncher's authors and contributors made that foundation, and their copyright \
                     notices are kept.",
                ));
            });
            if let Some(url) = links(ui, &[("ATLauncher on GitHub", "https://github.com/ATLauncher/ATLauncher".to_string())]) {
                link = Some(url);
            }
            });

            // the translation key is upper case; without a translation it is shown in sentence case
            let credits = crate::tr!("CREDITS");
            about_title(ui, if credits == "CREDITS" { "Credits" } else { &credits });
            card(ui, |ui| {
            prose(ui, |ui| {
                bullet(
                    ui,
                    &crate::tr!(
                        "ATLauncher, by the ATLauncher team and its contributors (GPL-3.0): the foundation BlockForge is built on."
                    ),
                );
                bullet(
                    ui,
                    &crate::tr!("Inter, by The Inter Project Authors (SIL Open Font License 1.1): the typeface of the window."),
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
                }
            });
            #[cfg(feature = "converter")]
            if let Some(url) = links(
                ui,
                &[
                    (&crate::tr!("Chunker on GitHub"), "https://github.com/HiveGamesOSS/Chunker".to_string()),
                    (&crate::tr!("Chunker license"), format!("{REPO}/blob/main/NOTICE.md")),
                ],
            ) {
                link = Some(url);
            }
            });

            about_title(ui, "License");
            card(ui, |ui| {
            muted(ui, "BlockForge is free software under the GNU General Public License v3.");
            if let Some(url) = links(
                ui,
                &[("License", format!("{REPO}/blob/main/LICENSE")), ("Notices", format!("{REPO}/blob/main/NOTICE.md"))],
            ) {
                link = Some(url);
            }
            });

            ui.add_space(24.0);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "\"Minecraft\" is a trademark of Mojang AB. BlockForge is not made by or approved by Mojang or Microsoft.",
                    )
                    .size(FS_META)
                    .color(c_faint()),
                )
                .wrap(),
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

/// The title of a block of the page, with room above it.
fn about_title(ui: &mut Ui, text: &str) {
    ui.add_space(26.0);
    ui.label(RichText::new(text).size(FS_SECTION).semibold());
    ui.add_space(2.0);
}

/// A soft light in the two brand colours around `center`, reaching out to `radius`; `strength` from 0 to 1. On the
/// dark look the bright colours are laid on thinly. On the light look the glow is an opaque pale tint of them instead:
/// a thin layer of a colour over white turns grey at its edge.
pub(super) fn brand_glow(painter: &egui::Painter, center: egui::Pos2, radius: f32, strength: f32) {
    let bright = |c: u32| egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    let (green, cyan) = (bright(0x34D399), bright(0x22D3EE));
    let tint = |c: egui::Color32| {
        if is_dark() {
            let a = (150.0 * strength) as u8;
            egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
        } else {
            motion::mix(c_card(), c, 0.28 * strength)
        }
    };
    motion::glow(painter, center + vec2(-radius * 0.08, radius * 0.02), radius, tint(green));
    motion::glow(painter, center + vec2(radius * 0.15, radius * 0.06), radius * 0.83, tint(cyan));
}

/// The top of the page: a card of glass that holds the logo's glow inside its edges.
fn hero(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    const MARGIN: f32 = 24.0;
    let id = ui.scope_id().with("about-hero-size");
    // the glow is cut at the card's edge; how big the card is comes from the last frame
    let last = ui.data(|d| d.get_temp::<egui::Vec2>(id));
    let out = card_frame().inner_margin(egui::Margin::same(MARGIN as i8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if let Some(size) = last {
            let edge = egui::Rect::from_min_size(ui.max_rect().min - vec2(MARGIN, MARGIN), size).shrink(1.0);
            ui.set_clip_rect(ui.clip_rect().intersect(edge));
        }
        add(ui);
    });
    let rect = out.response.rect;
    glass_edge(ui.painter(), rect, R_CARD);
    if last.is_none_or(|l| (l - rect.size()).length() > 0.5) {
        ui.data_mut(|d| d.insert_temp(id, rect.size()));
        ui.ctx().request_discard("the size of the About card changed");
    }
}

/// Text that reads at a comfortable line length, not the whole width of the page.
fn prose(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.scope(|ui| {
        ui.set_max_width(ui.available_width().min(PROSE_WIDTH));
        add(ui);
    });
}

/// A row of the "This copy" card: the name of the fact in a column of its own, then its value.
fn key_value(ui: &mut Ui, key: &str, value: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(KEY_WIDTH, 32.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            key,
            egui::FontId::proportional(FS_BODY),
            c_muted(),
        );
        value(ui);
    });
}

/// A row of links, each a small pill with an arrow that says it opens in the browser. Returns the address of the one
/// that was clicked.
fn links(ui: &mut Ui, items: &[(&str, String)]) -> Option<String> {
    let mut clicked = None;
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for (label, url) in items {
            if link_pill(ui, label).on_hover_text(url).clicked() {
                clicked = Some(url.clone());
            }
        }
    });
    clicked
}

/// One link of [`links`]: a quiet pill that brightens under the pointer, the arrow in the brand colour.
fn link_pill(ui: &mut Ui, label: &str) -> egui::Response {
    let font = super::fonts::medium(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, egui::Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 40.0, 30.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let t = motion::hover(ui, resp.id, resp.hovered());
        let p = ui.painter();
        p.rect(
            rect,
            egui::CornerRadius::same(15),
            motion::mix(c_field(), c_field_hover(), t),
            egui::Stroke::new(1.0, motion::mix(c_border(), c_control(), t)),
            egui::StrokeKind::Inside,
        );
        let fg = motion::mix(c_muted(), c_text(), t);
        p.galley(egui::pos2(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0), galley, fg);
        // a small arrow up and to the right, a little further out under the pointer
        let c = egui::pos2(rect.right() - 16.0 + t * 1.5, rect.center().y - t * 1.5);
        let ink = if is_dark() { c_accent() } else { c_accent_hover() };
        let stroke = egui::Stroke::new(1.5, ink);
        p.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], stroke);
        p.add(egui::Shape::line(vec![c + vec2(-2.0, -3.5), c + vec2(3.5, -3.5), c + vec2(3.5, 2.0)], stroke));
    }
    focus_ring(ui, &resp, rect, 15);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
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
