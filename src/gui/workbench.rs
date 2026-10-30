//! The Workbench: the five places of the window (Home, Library, Explore, Workshop, Utilities) and what every page
//! shares in them: the tabs between the pages of one place, and the instance covers.
//!
//! Account, Settings and About sit with the account at the bottom of the sidebar, outside the five.

// the pages take up the covers as they are rebuilt for 5.0
#![allow(dead_code)]

use egui::{Rect, TextureHandle, Ui, vec2};

use super::covers::{CoverSpec, CoverTheme};
use super::icons::Icon;
use super::widgets::*;
use super::{App, Page};
use crate::instances::Instance;

/// One of the five places of the sidebar, or the account corner below them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Section {
    Home,
    Library,
    Explore,
    Workshop,
    Utilities,
    /// Accounts, Settings and About: reached from the bottom of the sidebar.
    Account,
}

impl Section {
    /// The places in the sidebar, in order.
    pub const MAIN: [Section; 5] =
        [Section::Home, Section::Library, Section::Explore, Section::Workshop, Section::Utilities];

    /// The place a page belongs to.
    pub fn of(page: Page) -> Section {
        match page {
            Page::Home => Section::Home,
            Page::Library | Page::Instance | Page::Create => Section::Library,
            Page::Discover | Page::Packs | Page::Ftb => Section::Explore,
            Page::Craft => Section::Workshop,
            #[cfg(feature = "converter")]
            Page::Convert => Section::Workshop,
            Page::Servers | Page::Tools => Section::Utilities,
            Page::Accounts | Page::Settings | Page::About => Section::Account,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Section::Home => "Home",
            Section::Library => "Library",
            Section::Explore => "Explore",
            Section::Workshop => "Workshop",
            Section::Utilities => "Utilities",
            Section::Account => "Account",
        }
    }

    /// What the place holds, in a line.
    pub fn hint(self) -> &'static str {
        match self {
            Section::Home => "Play and recent activity",
            Section::Library => "Instances and backups",
            Section::Explore => "Mods and modpacks",
            Section::Workshop => "Crafting and world conversion",
            Section::Utilities => "Servers and diagnostics",
            Section::Account => "Accounts, settings and about",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Section::Home => Icon::Home,
            Section::Library => Icon::Library,
            Section::Explore => Icon::Cube,
            Section::Workshop => Icon::Craft,
            Section::Utilities => Icon::Layers,
            Section::Account => Icon::User,
        }
    }

    /// The page a place opens on the first time.
    pub fn first(self) -> Page {
        match self {
            Section::Home => Page::Home,
            Section::Library => Page::Library,
            Section::Explore => Page::Discover,
            Section::Workshop => Page::Craft,
            Section::Utilities => Page::Tools,
            Section::Account => Page::Settings,
        }
    }

    /// The pages of a place that get a tab of their own, with the tab's name. Places with one page (or whose other
    /// pages are opened from inside, like an instance from the Library) have none.
    pub fn tabs(self) -> Vec<(Page, &'static str)> {
        match self {
            Section::Explore => vec![
                (Page::Discover, "Modrinth & CurseForge"),
                (Page::Packs, "ATLauncher & Technic"),
                (Page::Ftb, "FTB"),
            ],
            Section::Workshop => {
                #[allow(unused_mut)]
                let mut tabs = vec![(Page::Craft, "Crafting calculator")];
                #[cfg(feature = "converter")]
                tabs.push((Page::Convert, "World converter"));
                tabs
            }
            Section::Utilities => vec![(Page::Tools, "Tools & diagnostics"), (Page::Servers, "Servers")],
            _ => Vec::new(),
        }
    }

    /// Whether the pages of the place carry the switch between them in their own heads instead of the tabs above
    /// them: Explore's pages are one marketplace, and its sources sit beside the title with the kinds under it.
    pub fn tabs_in_page(self) -> bool {
        self == Section::Explore
    }
}

/// The theme an instance's cover is drawn in: the one picked in its settings (`coverTheme` in its file), else Auto.
pub fn cover_theme(inst: &Instance) -> CoverTheme {
    inst.file
        .launcher
        .extras
        .get("coverTheme")
        .and_then(|v| v.as_str())
        .and_then(CoverTheme::from_id)
        .unwrap_or_default()
}

/// Store the theme of an instance's cover in its file's fields (Auto removes the field). The caller saves the file.
pub fn set_cover_theme(inst: &mut Instance, theme: CoverTheme) {
    if theme == CoverTheme::Auto {
        inst.file.launcher.extras.remove("coverTheme");
    } else {
        inst.file.launcher.extras.insert("coverTheme".into(), serde_json::Value::String(theme.id().into()));
    }
}

/// What an instance's cover is drawn from: its id (so a rename keeps the picture), version, loader and theme.
pub fn cover_spec(inst: &Instance) -> CoverSpec {
    let key = if inst.file.uuid.is_empty() { inst.folder.as_str() } else { inst.file.uuid.as_str() };
    CoverSpec::new(key, inst.mc_version(), inst.loader().map(|(kind, _)| kind), cover_theme(inst))
}

/// The few sizes covers are drawn at, so a window being resized does not draw a new one every frame: the one whose
/// shape is nearest the place is used and cropped to fill it.
const COVER_SIZES: [(usize, usize); 4] = [(480, 270), (720, 240), (320, 240), (128, 128)];

/// The drawn size for a place of `size` points: nearest in shape, and big enough not to blur on a large banner.
pub fn cover_size(size: egui::Vec2) -> (usize, usize) {
    let aspect = (size.x / size.y.max(1.0)).max(0.1);
    let (w, h) = COVER_SIZES
        .iter()
        .copied()
        .min_by(|a, b| {
            let da = ((a.0 as f32 / a.1 as f32) / aspect).ln().abs();
            let db = ((b.0 as f32 / b.1 as f32) / aspect).ln().abs();
            da.total_cmp(&db)
        })
        .unwrap_or(COVER_SIZES[0]);
    // a banner wider than the picture gets it at twice the size (whole blocks stay whole)
    if size.x > w as f32 * 1.5 { (w * 2, h * 2) } else { (w, h) }
}

/// The part of a `tex_size` picture that fills a place of `size` without stretching (the middle, like CSS
/// `object-fit: cover`), as texture coordinates.
pub fn cover_uv(tex_size: (usize, usize), size: egui::Vec2) -> Rect {
    let tex_aspect = tex_size.0 as f32 / tex_size.1 as f32;
    let aspect = size.x / size.y.max(1.0);
    if aspect > tex_aspect {
        // the place is wider: keep the full width, crop top and bottom (more off the top: the ground matters)
        let h = tex_aspect / aspect;
        let top = (1.0 - h) * 0.35;
        Rect::from_min_max(egui::pos2(0.0, top), egui::pos2(1.0, top + h))
    } else {
        let w = aspect / tex_aspect;
        let left = (1.0 - w) / 2.0;
        Rect::from_min_max(egui::pos2(left, 0.0), egui::pos2(left + w, 1.0))
    }
}

impl App {
    /// The cover of `inst` for a place of `size` points (see [`cover_size`]); `fade_left` darkens its left side for
    /// text written over it. Drawn once, then kept.
    pub(super) fn cover_texture(&mut self, inst: &Instance, size: egui::Vec2, fade_left: bool) -> TextureHandle {
        let (w, h) = cover_size(size);
        let spec = cover_spec(inst).with_fade_left(fade_left);
        self.covers.texture(&self.ctx, &spec, w, h)
    }

    /// Paint the cover of `inst` filling `rect`, with rounded corners of `radius`.
    pub(super) fn paint_cover(&mut self, ui: &Ui, inst: &Instance, rect: Rect, radius: u8, fade_left: bool) {
        let tex = self.cover_texture(inst, rect.size(), fade_left);
        let size = tex.size();
        egui::Image::from_texture((tex.id(), vec2(size[0] as f32, size[1] as f32)))
            .uv(cover_uv((size[0], size[1]), rect.size()))
            .corner_radius(radius)
            .paint_at(ui, rect);
    }

    /// The tabs between the pages of the open place (Workshop: the calculator and the converter; Utilities: tools
    /// and servers), as a row of pills above the page. Nothing for a place with one page, or one whose pages carry
    /// the switch in their own heads (Explore, see [`Section::tabs_in_page`]).
    pub(super) fn section_tabs(&mut self, ui: &mut Ui) {
        let section = Section::of(self.page);
        let tabs = section.tabs();
        if tabs.len() < 2 || section.tabs_in_page() {
            return;
        }
        let items: Vec<(&str, Option<egui::Color32>)> = tabs.iter().map(|t| (t.1, None)).collect();
        let current = tabs.iter().position(|t| t.0 == self.page).unwrap_or(0);
        let mut go = None;
        ui.horizontal_wrapped(|ui| {
            if let Some(i) = pill_tabs(ui, &items, current) {
                go = Some(tabs[i].0);
            }
        });
        ui.add_space(16.0);
        if let Some(page) = go {
            self.go(page);
        }
    }

    /// Open `page`, and remember it as the last page of its place, so the sidebar goes back to it.
    pub(super) fn go(&mut self, page: Page) {
        #[cfg(feature = "converter")]
        if page == Page::Convert {
            self.open_convert(None);
            self.section_last.insert(Section::Workshop, Page::Convert);
            return;
        }
        self.page = page;
        self.section_last.insert(Section::of(page), page);
    }

    /// Open a place of the sidebar on the page last seen in it.
    pub(super) fn go_section(&mut self, section: Section) {
        let page = self.section_last.get(&section).copied().unwrap_or_else(|| section.first());
        // inside the Library an instance or the new-instance page is not where the sidebar should go back to
        let page = if section == Section::Library { Page::Library } else { page };
        self.go(page);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_a_place_and_the_tabs_lead_back_to_it() {
        for s in Section::MAIN {
            assert_eq!(Section::of(s.first()), s, "{s:?} opens on one of its own pages");
            for (page, _) in s.tabs() {
                assert_eq!(Section::of(page), s);
            }
        }
        assert_eq!(Section::of(Page::Instance), Section::Library);
        assert_eq!(Section::of(Page::Ftb), Section::Explore);
        assert_eq!(Section::of(Page::Settings), Section::Account);
        // Explore's pages draw their sources themselves; the other places with pages get the tabs above them
        assert!(Section::Explore.tabs_in_page());
        assert!(!Section::Workshop.tabs_in_page() && !Section::Utilities.tabs_in_page());
    }

    #[test]
    fn a_cover_fills_its_place_without_stretching() {
        // a wide banner takes the wide picture at double size; a square tile the square one
        assert_eq!(cover_size(vec2(1100.0, 300.0)), (1440, 480));
        assert_eq!(cover_size(vec2(96.0, 96.0)), (128, 128));
        assert_eq!(cover_size(vec2(200.0, 120.0)), (480, 270));
        // the crop keeps the shape of the place
        let uv = cover_uv((480, 270), vec2(400.0, 100.0));
        let shown = (480.0 * uv.width()) / (270.0 * uv.height());
        assert!((shown - 4.0).abs() < 0.01, "{shown}");
        let uv = cover_uv((480, 270), vec2(100.0, 100.0));
        assert!((uv.center().x - 0.5).abs() < 0.001 && uv.height() == 1.0);
    }
}
