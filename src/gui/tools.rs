//! The Tools page: clean up, diagnose problems and keep things tidy.

use egui::{Align, Color32, CornerRadius, Layout, RichText, ScrollArea, Stroke, Ui, vec2};

use super::icons::{self, Icon};
use super::motion;
use super::services::{NetReport, SvcMsg, confirm_modal, modal_scroll_height, modal_width};
use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::tools::{self, NetworkCheckOptions, TOOLS};

/// The widest the grid of tools gets.
const GRID_WIDTH: f32 = 1040.0;

/// The tools that look at things and the ones that tidy up, in the order each group shows them. Every tool of
/// [`TOOLS`] is in one of the two.
const DIAGNOSE: [&str; 3] = ["network_checker", "support_bundle", "debug_mode"];
const CLEAN_UP: [&str; 5] =
    ["log_clearer", "download_clearer", "class_cache_clearer", "libraries_deleter", "skin_updater"];

/// The tools that delete things the player may want to think about: their button is red and asks first.
fn is_destructive(id: &str) -> bool {
    matches!(id, "download_clearer" | "libraries_deleter")
}

/// Where the player can fix what stops a tool from running.
#[derive(Clone, Copy)]
enum Fix {
    Settings,
    Accounts,
}

impl Fix {
    fn label(self) -> &'static str {
        match self {
            Fix::Settings => "Open Settings",
            Fix::Accounts => "Open Accounts",
        }
    }

    fn page(self) -> Page {
        match self {
            Fix::Settings => Page::Settings,
            Fix::Accounts => Page::Accounts,
        }
    }
}

/// Whether this copy runs in a Flatpak; it can't change while the launcher runs, so it is looked up once.
static FLATPAK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Why a tool that deletes shared files can't run now: a game or another job (an install, a download) may be using
/// them. `None` when nothing is running.
fn busy_reason(games_running: bool, jobs_running: bool) -> Option<&'static str> {
    match (games_running, jobs_running) {
        (true, _) => Some("Close Minecraft first, it is using these files"),
        (false, true) => Some("Wait for the jobs in progress to finish first, they may be using these files"),
        _ => None,
    }
}

/// The sidebar label of the job a tool starts, so its button can wait while that job runs.
fn task_label(id: &str) -> Option<&'static str> {
    Some(match id {
        "network_checker" => "Network checker",
        "support_bundle" => "Creating a support bundle",
        "log_clearer" => "Clearing old logs",
        "download_clearer" => "Clearing downloads",
        "class_cache_clearer" => "Clearing the game start cache",
        "libraries_deleter" => "Deleting libraries",
        "skin_updater" => "Updating skins",
        _ => return None,
    })
}

/// The space a clean-up gave back, as the message after it says it.
fn freed(before: u64, after: u64) -> String {
    format!("{} freed", human_size(before.saturating_sub(after)))
}

impl App {
    pub(super) fn tools_ui(&mut self, ui: &mut Ui) {
        heading(ui, "Tools");
        muted(ui, "Clean up, diagnose problems and keep things tidy");
        // the network checker needs `enableLogs`, which is a plain field of the settings (no need to read them all)
        let logs_on = self.settings.enable_logs;
        let flatpak = *FLATPAK.get_or_init(tools::is_flatpak);
        let no_accounts = self.accounts.is_empty();
        // what is running decides which tools can start: they are greyed out with the reason, not refused after the
        // player has confirmed
        let games_running = self.games.values().any(super::Game::active);
        let busy = busy_reason(games_running, !self.tasks.is_empty());
        let running: Vec<String> = self.tasks.iter().map(|t| t.label.clone()).collect();
        let mut clicked: Option<&'static str> = None;
        let mut go: Option<Fix> = None;
        #[cfg(feature = "converter")]
        let mut open_convert = false;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // as wide as the grid gets, but never wider than the window
            ui.set_max_width((ui.available_width() - 14.0).min(GRID_WIDTH));
            for (title, ids) in [("Diagnose", &DIAGNOSE[..]), ("Clean up", &CLEAN_UP[..])] {
                let mut tiles = Vec::new();
                for tool in ids.iter().filter_map(|id| TOOLS.iter().find(|t| t.id == *id)) {
                    // what stops a tool from running now, and where that is fixed
                    let (enabled, hint): (bool, Option<(&str, Option<Fix>)>) = match tool.id {
                        "network_checker" if !logs_on => (
                            false,
                            Some((
                                "Turn on \"Report pack installs and play time\" in Settings first.",
                                Some(Fix::Settings),
                            )),
                        ),
                        "debug_mode" if flatpak => (false, Some(("Not available in a Flatpak.", None))),
                        "debug_mode" if tools::is_debug_enabled() => (false, Some(("Debug mode is already on.", None))),
                        "skin_updater" if no_accounts => (false, Some(("Add an account first.", Some(Fix::Accounts)))),
                        id if task_label(id).is_some_and(|l| running.iter().any(|r| r == l)) => {
                            (false, Some(("Running now. Its progress is in the sidebar.", None)))
                        }
                        "debug_mode" if games_running => (
                            false,
                            Some((
                                "Close Minecraft first. The restarted launcher can't follow a game that runs.",
                                None,
                            )),
                        ),
                        "debug_mode" if !running.is_empty() => (
                            false,
                            Some(("Wait for the jobs in progress to finish first. A restart stops them.", None)),
                        ),
                        "download_clearer" | "libraries_deleter" if busy.is_some() => {
                            (false, busy.map(|why| (why, None)))
                        }
                        "class_cache_clearer" if games_running => {
                            (false, Some(("Close Minecraft first, it is using the cache.", None)))
                        }
                        _ => (true, None),
                    };
                    let action = if is_destructive(tool.id) {
                        format!("{}\u{2026}", tool.action)
                    } else {
                        tool.action.to_string()
                    };
                    tiles.push(Tile {
                        glyph: glyph_of(tool.id),
                        name: tool.name,
                        description: tool.description,
                        action,
                        button: if is_destructive(tool.id) { Button::Danger } else { Button::Plain },
                        enabled,
                        hint,
                    });
                }
                section_head(ui, title);
                for (out, tool) in tile_grid(ui, title, &tiles).into_iter().zip(ids) {
                    if out.pressed {
                        clicked = Some(*tool);
                    }
                    if out.fix.is_some() {
                        go = out.fix;
                    }
                }
            }
            // the world converter has a tile of its own (it opens a page, not a job)
            #[cfg(feature = "converter")]
            {
                section_head(ui, "Worlds");
                if self.convert_tool_card(ui) {
                    open_convert = true;
                }
            }
            ui.add_space(16.0);
        });
        if let Some(fix) = go {
            self.page = fix.page();
        }
        if let Some(id) = clicked {
            self.tool_clicked(id);
        }
        #[cfg(feature = "converter")]
        if open_convert {
            self.open_convert(None);
        }
    }

    /// A tool's button was pressed.
    fn tool_clicked(&mut self, id: &'static str) {
        match id {
            // these three ask first, as the Java edition's tool panels do before their Launch button
            "network_checker" | "download_clearer" | "libraries_deleter" => self.svc.tool_confirm = Some(id),
            _ => self.run_tool(id),
        }
    }

    fn run_tool(&mut self, id: &'static str) {
        let dirs = self.dirs.clone();
        match id {
            "network_checker" => {
                let opts = NetworkCheckOptions { speed_test: !self.svc.skip_speed_test, ..Default::default() };
                self.start_task("Network checker", move |progress, n| async move {
                    let text = tools::run_network_check(&dirs, &opts, &progress).await?;
                    progress.stage("Sharing the result");
                    let (link, error) = match tools::upload_report("BlockForge Network Test Log", &text).await {
                        Ok(url) => (Some(url), None),
                        Err(e) => {
                            crate::logs::error(format!("Network Test failed to submit: {e:#}"));
                            (None, Some(format!("{e:#}")))
                        }
                    };
                    n.send(Msg::Svc(SvcMsg::NetworkReport(NetReport { text, link, error })));
                    Ok(String::new())
                });
            }
            "support_bundle" => {
                let instance = tools::last_played_instance(&self.instances).cloned();
                let (settings, accounts) = (self.settings.clone(), self.accounts.clone());
                self.start_task("Creating a support bundle", move |_p, _n| async move {
                    let (zip, found) = tokio::task::spawn_blocking(move || {
                        tools::create_support_bundle(&dirs, &settings, &accounts, instance.as_ref())
                    })
                    .await??;
                    if let Some(folder) = zip.parent() {
                        let _ = open::that_detached(folder);
                    }
                    Ok(match found.first() {
                        Some(d) => format!("Support bundle saved to your downloads ({})", d.title),
                        None => "Support bundle saved to your downloads".to_string(),
                    })
                });
            }
            // the clean-up tools can take a while on a big folder, so they run off the window's thread
            "log_clearer" => {
                self.start_task("Clearing old logs", move |_p, _n| async move {
                    let n = tokio::task::spawn_blocking(move || tools::clear_logs(&dirs)).await?;
                    Ok(format!("Old logs cleared ({n} removed)"))
                });
            }
            "debug_mode" => match tools::relaunch_in_debug() {
                Ok(()) => {
                    self.quit_anyway = true;
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(e) => self.toast_err(format!("{e:#}")),
            },
            "download_clearer" => {
                if let Some(why) = busy_reason(self.games.values().any(super::Game::active), !self.tasks.is_empty()) {
                    self.toast_err(why);
                    return;
                }
                self.start_task("Clearing downloads", move |_p, _n| async move {
                    let freed = tokio::task::spawn_blocking(move || {
                        let size = |d: &crate::paths::Dirs| {
                            crate::packutil::dir_size(&tools::downloads_dir(d))
                                + crate::packutil::dir_size(&tools::failed_downloads_dir(d))
                        };
                        let before = size(&dirs);
                        tools::clear_downloads(&dirs);
                        freed(before, size(&dirs))
                    })
                    .await?;
                    Ok(format!("Downloads cleared ({freed})"))
                });
            }
            "skin_updater" => {
                let accounts = self.accounts.clone();
                self.start_task("Updating skins", move |progress, n| async move {
                    let results = crate::skins::update_all(&dirs, &accounts, &progress).await;
                    let failed = results.iter().filter(|(_, r)| r.is_err()).count();
                    for a in &accounts {
                        n.send(Msg::Svc(SvcMsg::SkinFetched(a.uuid.clone())));
                    }
                    if failed == 0 {
                        Ok("Skins updated".to_string())
                    } else {
                        Err(anyhow::anyhow!(
                            "{failed} of {} skins couldn't be updated. Try again later.",
                            results.len()
                        ))
                    }
                });
            }
            "class_cache_clearer" => {
                if self.games.values().any(super::Game::active) {
                    self.toast_err("Close Minecraft first, it is using the cache");
                } else {
                    self.start_task("Clearing the game start cache", move |_p, _n| async move {
                        let bytes = tokio::task::spawn_blocking(move || crate::classcache::clear(&dirs)).await?;
                        Ok(format!("Game start cache cleared ({})", freed(bytes, 0)))
                    });
                }
            }
            "libraries_deleter" => {
                if let Some(why) = busy_reason(self.games.values().any(super::Game::active), !self.tasks.is_empty()) {
                    self.toast_err(why);
                    return;
                }
                self.start_task("Deleting libraries", move |_p, _n| async move {
                    let freed = tokio::task::spawn_blocking(move || {
                        let before = crate::packutil::dir_size(&dirs.libraries);
                        tools::delete_libraries(&dirs);
                        freed(before, crate::packutil::dir_size(&dirs.libraries))
                    })
                    .await?;
                    Ok(format!("Libraries deleted ({freed}). They download again when a game starts"))
                });
            }
            _ => {}
        }
    }

    /// The windows over the Tools page: confirmations and the network report.
    pub(super) fn tools_overlays(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.svc.tool_confirm {
            let answer = match id {
                "network_checker" => {
                    let mut answer = None;
                    egui::Modal::new(egui::Id::unique("tool-confirm")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                        ui.set_width(modal_width(ctx, 460.0));
                        ui.label(RichText::new("Network checker").size(FS_SECTION).semibold());
                        ui.add_space(4.0);
                        muted(ui, tools::NETWORK_CHECKER_WARNING);
                        ui.add_space(6.0);
                        let mut include = !self.svc.skip_speed_test;
                        ui.checkbox(&mut include, "Include the 100 MB download speed test");
                        self.svc.skip_speed_test = !include;
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if primary(ui, "Run the test").clicked() {
                                answer = Some(true);
                            }
                            if secondary(ui, "Cancel").clicked() {
                                answer = Some(false);
                            }
                        });
                    });
                    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                        answer = Some(false);
                    }
                    answer
                }
                "download_clearer" => confirm_modal(
                    ctx,
                    "tool-confirm",
                    "Clear downloads?",
                    "This clears out all the downloads done by the launcher. This will not affect any instances, but means new pack installs may take longer as it needs to redownload mods.",
                    "Clear downloads",
                    true,
                ),
                "libraries_deleter" => confirm_modal(
                    ctx,
                    "tool-confirm",
                    "Delete libraries?",
                    "This clears out all the libraries used by Minecraft. Instances may need to be reinstalled if launching them once doesn't download all libraries.",
                    "Delete libraries",
                    true,
                ),
                _ => Some(false),
            };
            match answer {
                Some(true) => {
                    self.svc.tool_confirm = None;
                    self.run_tool(id);
                }
                Some(false) => self.svc.tool_confirm = None,
                None => {}
            }
        }

        if self.svc.net_report.is_some() {
            let mut close = false;
            let mut copy: Option<String> = None;
            if let Some(report) = &self.svc.net_report {
                egui::Modal::new(egui::Id::unique("net-report")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                    ui.set_width(modal_width(ctx, 680.0));
                    ui.label(RichText::new("Network checker").size(FS_SECTION).semibold());
                    match (&report.link, &report.error) {
                        (Some(link), _) => {
                            small_muted(ui, "The result was shared so problems can be diagnosed. Link:");
                            ui.horizontal(|ui| {
                                ui.add(egui::Hyperlink::from_label_and_url(RichText::new(link).underline(), link));
                                if secondary(ui, "Copy link").clicked() {
                                    copy = Some(link.clone());
                                }
                            });
                        }
                        (None, Some(e)) => {
                            ui.label(RichText::new(format!("The result could not be shared: {e}")).color(c_danger()));
                        }
                        _ => {}
                    }
                    ui.add_space(6.0);
                    ScrollArea::vertical()
                        .max_height(modal_scroll_height(ctx, 360.0, 160.0))
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut report.text.as_str())
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if secondary(ui, "Copy report").clicked() {
                            copy = Some(report.text.clone());
                        }
                        if primary(ui, "Close").clicked() {
                            close = true;
                        }
                    });
                });
            }
            if let Some(text) = copy {
                ctx.copy_text(text);
                self.toast("Copied");
            }
            if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.svc.net_report = None;
            }
        }
    }
}

/// The title of a group of tiles, with the room above and under it.
fn section_head(ui: &mut Ui, title: &str) {
    super::settings::section_head(ui, title, "");
}

/// How a tile's button looks.
#[derive(Clone, Copy, PartialEq)]
enum Button {
    Plain,
    Danger,
}

/// The picture in a tile's badge: one of the shared icons, or one drawn here for a tool the set has none for.
#[derive(Clone, Copy)]
enum Glyph {
    Icon(Icon),
    Network,
    Terminal,
    Logs,
    Download,
    Bolt,
}

/// The picture of each tool.
fn glyph_of(id: &str) -> Glyph {
    match id {
        "network_checker" => Glyph::Network,
        "support_bundle" => Glyph::Icon(Icon::Package),
        "debug_mode" => Glyph::Terminal,
        "log_clearer" => Glyph::Logs,
        "download_clearer" => Glyph::Download,
        "class_cache_clearer" => Glyph::Bolt,
        "libraries_deleter" => Glyph::Icon(Icon::Library),
        "skin_updater" => Glyph::Icon(Icon::User),
        _ => Glyph::Icon(Icon::Wrench),
    }
}

/// Draw `glyph` centred on `c` in a square of side `size`, in the line style of the shared icons.
fn paint_glyph(p: &egui::Painter, glyph: Glyph, c: egui::Pos2, size: f32, color: Color32) {
    let s = size / 2.0;
    let stroke = Stroke::new((size / 11.0).max(1.4), color);
    let at = |x: f32, y: f32| egui::pos2(c.x + x * s, c.y + y * s);
    let line = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), stroke));
    };
    match glyph {
        Glyph::Icon(icon) => icons::paint(p, icon, c, size, color),
        Glyph::Network => {
            // three arcs over a dot, the usual picture of a connection
            for r in [0.45, 0.9, 1.35] {
                let arc: Vec<(f32, f32)> = (0..=10)
                    .map(|i| {
                        let a = -std::f32::consts::FRAC_PI_4 * 3.0 + std::f32::consts::FRAC_PI_2 * i as f32 / 10.0;
                        (r * a.cos(), 0.8 + r * a.sin())
                    })
                    .collect();
                line(&arc);
            }
            p.circle_filled(at(0.0, 0.8), 0.12 * s + 0.6, color);
        }
        Glyph::Terminal => {
            p.rect_stroke(
                egui::Rect::from_min_max(at(-0.95, -0.75), at(0.95, 0.75)),
                2.5,
                stroke,
                egui::StrokeKind::Inside,
            );
            line(&[(-0.5, -0.28), (-0.15, 0.02), (-0.5, 0.32)]);
            line(&[(0.05, 0.34), (0.5, 0.34)]);
        }
        Glyph::Logs => {
            p.rect_stroke(
                egui::Rect::from_min_max(at(-0.72, -0.92), at(0.72, 0.92)),
                2.5,
                stroke,
                egui::StrokeKind::Inside,
            );
            for y in [-0.4, 0.0, 0.4] {
                line(&[(-0.36, y), (0.36, y)]);
            }
        }
        Glyph::Download => {
            line(&[(0.0, -0.92), (0.0, 0.3)]);
            line(&[(-0.45, -0.12), (0.0, 0.32), (0.45, -0.12)]);
            line(&[(-0.9, 0.3), (-0.9, 0.85), (0.9, 0.85), (0.9, 0.3)]);
        }
        Glyph::Bolt => {
            line(&[
                (0.18, -0.95),
                (-0.6, 0.15),
                (-0.02, 0.15),
                (-0.18, 0.95),
                (0.6, -0.15),
                (0.02, -0.15),
                (0.18, -0.95),
            ]);
        }
    }
}

/// One tile of the grid: what it shows and whether it can be used now.
struct Tile<'a> {
    glyph: Glyph,
    name: &'a str,
    description: &'a str,
    action: String,
    button: Button,
    enabled: bool,
    /// Why the tool can't run now, with where that is fixed.
    hint: Option<(&'a str, Option<Fix>)>,
}

/// What a tile did this frame.
struct RowOut {
    pressed: bool,
    /// The link next to the reason a tool can't run was pressed.
    fix: Option<Fix>,
}

/// The narrowest a tile gets before the grid takes a column away, and the room between tiles.
const TILE_MIN: f32 = 250.0;
const TILE_GAP: f32 = 14.0;
/// The side of the badge with a tool's picture.
const BADGE: f32 = 40.0;

/// The tiles in a grid that fills the width. The tiles of one row are as tall as each other, with their buttons on
/// one line along the bottom: each row remembers the tallest text of its tiles. Returns what each tile did.
fn tile_grid(ui: &mut Ui, salt: &str, tiles: &[Tile]) -> Vec<RowOut> {
    let (cols, w) = grid_columns(ui.available_width(), TILE_MIN, TILE_GAP);
    let mut outs = Vec::with_capacity(tiles.len());
    for (r, row) in tiles.chunks(cols).enumerate() {
        let row_id = ui.scope_id().with(("tool-tiles", salt, r, cols));
        let row_h = ui.data(|d| d.get_temp::<f32>(row_id)).unwrap_or(0.0);
        let mut tallest = 0.0f32;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = TILE_GAP;
            for t in row {
                let out = ui
                    .allocate_ui_with_layout(vec2(w, 600.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(w);
                        let (out, text_h) = tile(ui, row_id.with(t.name), t, row_h);
                        tallest = tallest.max(text_h);
                        out
                    })
                    .inner;
                outs.push(out);
            }
        });
        if (tallest - row_h).abs() > 0.5 {
            // the row learned how tall its text is: draw it again at once with the buttons lined up
            ui.data_mut(|d| d.insert_temp(row_id, tallest));
            ui.ctx().request_discard("a row of tool tiles changed its height");
        }
        ui.add_space(TILE_GAP - ui.spacing().item_spacing.y);
    }
    outs
}

/// A tile: the badge, the name, what the tool does and, when it can't be used now, why (with a link to where that is
/// fixed); then its button along the bottom, `text_h` under the top of the text. Returns what it did and how tall its
/// own text was.
fn tile(ui: &mut Ui, id: egui::Id, t: &Tile, text_h: f32) -> (RowOut, f32) {
    let mut out = RowOut { pressed: false, fix: None };
    let was_hot = ui.data(|d| d.get_temp::<bool>(id.with("hot"))).unwrap_or(false);
    let lit = motion::hover(ui, id.with("badge"), was_hot && t.enabled);
    let (natural, _) = hover_card(ui, id, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 4.0;
        let top = ui.cursor().top();
        let (badge, _) = ui.allocate_exact_size(vec2(BADGE, BADGE), egui::Sense::hover());
        let destructive = t.button == Button::Danger;
        let (fill, fg) = match (t.enabled, destructive) {
            (false, _) => (c_hover(), c_faint()),
            (true, true) => (c_danger_bg(), c_danger()),
            (true, false) => (c_accent_soft(), c_accent()),
        };
        let p = ui.painter();
        p.rect_filled(badge, CornerRadius::same(R_CONTROL), fill);
        // the badge of a tool that is safe to run takes the brand gradient while its tile is under the pointer
        let fg = if !destructive && lit > 0.0 {
            motion::gradient_rect(p, badge, R_CONTROL, c_accent().gamma_multiply(lit), c_accent2().gamma_multiply(lit));
            motion::mix(fg, c_on_accent(), lit)
        } else {
            fg
        };
        paint_glyph(p, t.glyph, badge.center(), 18.0, fg);
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(t.name).size(FS_SECTION - 1.0).semibold()).wrap());
        ui.add(egui::Label::new(RichText::new(t.description).color(c_muted()).size(FS_META)).wrap());
        if let Some((reason, fix)) = t.hint {
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(reason).color(c_faint()).size(FS_META)).wrap());
            if let Some(fix) = fix {
                if ui.link(RichText::new(fix.label()).size(FS_META).underline()).clicked() {
                    out.fix = Some(fix);
                }
            }
        }
        let natural = ui.cursor().top() - top;
        ui.add_space((text_h - natural).max(0.0) + 8.0);
        // a tool that can't run now shows its button greyed out, and the reason above it
        out.pressed = match (t.button, t.enabled) {
            (Button::Plain, _) => secondary_enabled(ui, &t.action, t.enabled).clicked(),
            (Button::Danger, true) => danger(ui, &t.action).clicked(),
            (Button::Danger, false) => danger_enabled(ui, &t.action, false).clicked(),
        };
        natural
    });
    (out, natural)
}

#[cfg_attr(not(feature = "converter"), allow(dead_code))]
/// A tile on its own in the grid, as the world converter's entry uses it. Returns whether its button was pressed.
pub(super) fn card_frame(
    ui: &mut Ui,
    name: &str,
    description: &str,
    action: &str,
    enabled: bool,
    hint: Option<&str>,
) -> bool {
    let tile = Tile {
        glyph: Glyph::Icon(Icon::Convert),
        name,
        description,
        action: action.to_string(),
        button: Button::Plain,
        enabled,
        hint: hint.map(|h| (h, None)),
    };
    tile_grid(ui, name, &[tile]).into_iter().any(|o| o.pressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_card_draws_without_a_window() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for tool in TOOLS.iter() {
                assert!(!card_frame(ui, tool.name, tool.description, tool.action, true, None), "nothing was pressed");
            }
            card_frame(ui, TOOLS[0].name, TOOLS[0].description, TOOLS[0].action, false, Some("Turn on logs first."));
        });
        assert!(out.shapes.len() >= TOOLS.len());
        out.textures_delta.clear();
    }

    #[test]
    fn every_tool_is_in_a_group() {
        for tool in TOOLS.iter() {
            let groups = [&DIAGNOSE[..], &CLEAN_UP[..]].iter().filter(|g| g.contains(&tool.id)).count();
            assert_eq!(groups, 1, "{} is in one group", tool.id);
        }
    }

    #[test]
    fn shared_files_are_only_deleted_when_nothing_uses_them() {
        assert_eq!(busy_reason(false, false), None);
        assert!(busy_reason(true, false).is_some_and(|r| r.contains("Minecraft")));
        assert!(busy_reason(true, true).is_some_and(|r| r.contains("Minecraft")), "the game is named first");
        assert!(busy_reason(false, true).is_some_and(|r| r.contains("jobs")));
    }
}
