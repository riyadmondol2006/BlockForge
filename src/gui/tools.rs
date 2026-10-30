//! The Tools page: clean up, diagnose problems and keep things tidy.

use egui::{Align, Color32, CornerRadius, Layout, RichText, ScrollArea, Stroke, Ui, vec2};

use super::icons::{self, Icon};
use super::services::{
    NetReport, Share, SvcMsg, SvcState, ToolResult, confirm_buttons, confirm_modal, modal_scroll_height, modal_width,
};
use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::tools::{self, NetworkCheckOptions, TOOLS};

/// The widest the list of tools gets.
const GRID_WIDTH: f32 = 860.0;

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
    Accounts,
}

impl Fix {
    fn label(self) -> &'static str {
        match self {
            Fix::Accounts => "Open Accounts",
        }
    }

    fn page(self) -> Page {
        match self {
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

/// Note how a job ended when it is one a tool started, so the tool's row can say so.
pub(super) fn note_task_end(svc: &mut SvcState, label: &str, result: &Result<String, String>) {
    if TOOLS.iter().any(|t| task_label(t.id) == Some(label)) {
        let (ok, text) = match result {
            Ok(t) => (true, t.clone()),
            Err(e) => (false, e.clone()),
        };
        svc.tool_results.insert(label.to_string(), ToolResult { ok, text, at: std::time::Instant::now() });
    }
}

/// What a tool's row says under its description: running, what stops it (and so what to do), or how it went last
/// time. Nothing when it is simply ready.
fn row_status(running: bool, blocked: Option<&str>, last: Option<&ToolResult>, ago: &str) -> Option<(Status, String)> {
    if running {
        return Some((Status::Busy, "Running now. Its progress is in the sidebar.".to_string()));
    }
    if let Some(why) = blocked {
        return Some((Status::Warn, why.to_string()));
    }
    let last = last?;
    let text = last.text.trim().trim_end_matches('.');
    Some(if last.ok {
        let text = if text.is_empty() { "Done" } else { text };
        (Status::Good, format!("{text} \u{00B7} {ago}"))
    } else {
        (Status::Error, format!("Didn't work: {text}. Try again, or make a support bundle to ask for help"))
    })
}

impl App {
    pub(super) fn tools_ui(&mut self, ui: &mut Ui) {
        let width = (ui.available_width() - 14.0).min(GRID_WIDTH);
        ui.scope(|ui| {
            ui.set_max_width(width);
            PageHeader::new("Tools & diagnostics")
                .subtitle("Find out what is wrong, gather what to send for help, and free up space")
                .show(ui, |_| {});
        });
        let flatpak = *FLATPAK.get_or_init(tools::is_flatpak);
        let no_accounts = self.accounts.is_empty();
        // what is running decides which tools can start: they are greyed out with the reason, not refused after the
        // player has confirmed
        let games_running = self.games.values().any(super::Game::active);
        let busy = busy_reason(games_running, !self.tasks.is_empty());
        let running: Vec<String> = self.tasks.iter().map(|t| t.label.clone()).collect();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut clicked: Option<&'static str> = None;
        let mut go: Option<Fix> = None;
        #[cfg(feature = "converter")]
        let mut open_convert = false;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // as wide as the list gets, but never wider than the window
            ui.set_max_width(width);
            for (title, hint, ids) in [
                ("Diagnose", "When a game or a download doesn't work", &DIAGNOSE[..]),
                ("Clean up", "Free up space and refresh what the launcher keeps", &CLEAN_UP[..]),
            ] {
                let mut rows = Vec::new();
                for tool in ids.iter().filter_map(|id| TOOLS.iter().find(|t| t.id == *id)) {
                    let is_running = task_label(tool.id).is_some_and(|l| running.iter().any(|r| r == l));
                    // what stops a tool from running now, and where that is fixed
                    let (enabled, hint): (bool, Option<(&str, Option<Fix>)>) = match tool.id {
                        _ if is_running => (false, None),
                        "debug_mode" if flatpak => (false, Some(("Not available in a Flatpak.", None))),
                        "debug_mode" if tools::is_debug_enabled() => (false, Some(("Debug mode is already on.", None))),
                        "skin_updater" if no_accounts => (false, Some(("Add an account first.", Some(Fix::Accounts)))),
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
                    let last = task_label(tool.id).and_then(|l| self.svc.tool_results.get(l));
                    let ago = last
                        .map_or(String::new(), |r| time_ago(now_ms - r.at.elapsed().as_millis() as i64).to_lowercase());
                    let action = if is_destructive(tool.id) {
                        format!("{}\u{2026}", tool.action)
                    } else {
                        tool.action.to_string()
                    };
                    rows.push(Row {
                        glyph: glyph_of(tool.id),
                        name: tool.name,
                        description: tool.description,
                        action,
                        button: if is_destructive(tool.id) { Button::Danger } else { Button::Plain },
                        enabled,
                        status: row_status(is_running, hint.map(|h| h.0), last, &ago),
                        fix: hint.and_then(|h| h.1),
                    });
                }
                group_head(ui, title, hint);
                for (out, tool) in row_list(ui, title, &rows).into_iter().zip(ids) {
                    if out.pressed {
                        clicked = Some(*tool);
                    }
                    if out.fix.is_some() {
                        go = out.fix;
                    }
                }
            }
            // the world converter has a row of its own (it opens a page, not a job)
            #[cfg(feature = "converter")]
            {
                group_head(ui, "Worlds", "Bring worlds over from other editions");
                if self.convert_tool_card(ui) {
                    open_convert = true;
                }
            }
            ui.add_space(16.0);
        });
        if let Some(fix) = go {
            self.go(fix.page());
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
                // the check only looks: its report is shown here, and sharing it is a step of its own (share_net_report)
                self.start_task("Network checker", move |progress, n| async move {
                    let text = tools::run_network_check(&dirs, &opts, &progress).await?;
                    n.send(Msg::Svc(SvcMsg::NetworkReport(text)));
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

    /// Upload the network report on display, as the player asked and agreed to: the copy with anything personal
    /// taken out. The answer comes back as [`SvcMsg::NetworkShared`].
    fn share_net_report(&mut self) {
        let Some(report) = self.svc.net_report.as_mut() else { return };
        report.share = Share::Sending;
        let text = tools::shareable_report(&report.text);
        self.start_task("Sharing the network report", move |_p, n| async move {
            let result = tools::upload_report("BlockForge Network Test Log", &text).await.map_err(|e| {
                crate::logs::error(format!("Network Test failed to submit: {e:#}"));
                format!("{e:#}")
            });
            let ok = result.is_ok();
            n.send(Msg::Svc(SvcMsg::NetworkShared(result)));
            if ok {
                Ok("Network report shared".to_string())
            } else {
                anyhow::bail!("The network report could not be shared")
            }
        });
    }

    /// For the screenshot tour: the report of a network check on display (`share`: with the question whether to
    /// share it).
    pub(super) fn shot_net_report(&mut self, share: bool) {
        let text = "Ping results to 1.1.1.1 was\nPING 1.1.1.1 (1.1.1.1) 56(84) bytes of data.\n64 bytes from 1.1.1.1: icmp_seq=1 ttl=57 time=12.4 ms\n64 bytes from 1.1.1.1: icmp_seq=2 ttl=57 time=11.9 ms\n\n--- 1.1.1.1 ping statistics ---\n10 packets transmitted, 10 received, 0% packet loss\n\n----------------\n\nResolution of authserver.mojang.com was 52.216.10.4\n\nResolution of libraries.minecraft.net was 13.107.246.40\n\n----------------\n\nResponse code to https://download.nodecdn.net/containers/atl was 200\n";
        self.page = Page::Tools;
        self.svc.net_report =
            Some(NetReport { text: text.to_string(), share: if share { Share::Asking } else { Share::Local } });
    }

    /// The windows over the Tools page: confirmations and the network report.
    pub(super) fn tools_overlays(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.svc.tool_confirm {
            let answer = match id {
                "network_checker" => {
                    let mut answer = None;
                    let modal = egui::Modal::new(egui::Id::unique("tool-confirm")).frame(super::modal_frame(ctx)).show(
                        ctx,
                        |ui| {
                            ui.set_width(modal_width(ctx, 460.0));
                            ui.label(RichText::new("Run the network check?").size(FS_SECTION).semibold());
                            ui.add_space(6.0);
                            ui.add(egui::Label::new(RichText::new(tools::NETWORK_CHECK_NOTE).color(c_muted())).wrap());
                            ui.add_space(8.0);
                            // the big download is a choice of its own: it uses about 100 MB of the player's data
                            let mut include = !self.svc.skip_speed_test;
                            ui.checkbox(&mut include, "Also measure the download speed (downloads 100 MB)");
                            self.svc.skip_speed_test = !include;
                            ui.add_space(16.0);
                            answer = confirm_buttons(ui, "network-check", "Cancel", "Run the check", false, true);
                        },
                    );
                    if modal.should_close() && answer.is_none() {
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
            let mut share = false;
            if let Some(report) = self.svc.net_report.as_mut() {
                let modal = egui::Modal::new(egui::Id::unique("net-report")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                    ui.set_width(modal_width(ctx, 680.0));
                    if super::discover::dialog_title(ui, "Network check") {
                        close = true;
                    }
                    // the consent step needs room under the report: the report scrolls in less then
                    let asking = report.share == Share::Asking;
                    match &report.share {
                        Share::Local | Share::Asking => {
                            small_muted(ui, "This report is only on this computer. Copy it, or share it when someone helping you asks for it.");
                        }
                        Share::Sending => {
                            ui.horizontal(|ui| {
                                spinner_sized(ui, 14.0);
                                small_muted(ui, "Sharing the report\u{2026}");
                            });
                        }
                        Share::Shared(link) => {
                            small_muted(ui, "The report was shared. Send this link to whoever is helping you:");
                            ui.horizontal_wrapped(|ui| {
                                ui.add(egui::Hyperlink::from_label_and_url(RichText::new(link).underline(), link));
                                if secondary(ui, "Copy link").clicked() {
                                    copy = Some(link.clone());
                                }
                            });
                        }
                        Share::Failed(e) => {
                            ui.add(egui::Label::new(RichText::new(format!("The report could not be shared: {e}")).color(c_danger())).wrap());
                        }
                    }
                    ui.add_space(8.0);
                    let rest = if asking { 330.0 } else { 200.0 };
                    ScrollArea::vertical()
                        .max_height(modal_scroll_height(ctx, 360.0, rest))
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut report.text.as_str())
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                    ui.add_space(12.0);
                    if asking {
                        // the consent, in the window: what is uploaded, where, and what is taken out first
                        card(ui, |ui| {
                            ui.label(RichText::new("Share this report?").semibold());
                            ui.add_space(2.0);
                            ui.add(egui::Label::new(RichText::new(tools::NETWORK_SHARE_NOTE).size(FS_META).color(c_muted())).wrap());
                            ui.add_space(12.0);
                            match confirm_buttons(ui, "net-share", "Don't share", "Share report", false, true) {
                                Some(true) => share = true,
                                Some(false) => report.share = Share::Local,
                                None => {}
                            }
                        });
                        return;
                    }
                    super::discover::dialog_footer(
                        ui,
                        "net-report",
                        |ui| {
                            if ghost(ui, "Copy report").clicked() {
                                copy = Some(report.text.clone());
                            }
                        },
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            if matches!(report.share, Share::Local | Share::Failed(_)) {
                                let label = if matches!(report.share, Share::Failed(_)) { "Try again" } else { "Share report\u{2026}" };
                                if secondary(ui, label).clicked() {
                                    report.share = Share::Asking;
                                }
                            }
                            if primary(ui, "Close").clicked() {
                                close = true;
                            }
                        },
                    );
                });
                if modal.should_close() {
                    // Escape steps back out of the question to share before it closes the window
                    if report.share == Share::Asking {
                        report.share = Share::Local;
                    } else {
                        close = true;
                    }
                }
            }
            if let Some(text) = copy {
                ctx.copy_text(text);
                self.toast("Copied");
            }
            if share {
                self.share_net_report();
            }
            if close {
                self.svc.net_report = None;
            }
        }
    }
}

/// The title of a group of tools and a quiet line about it, above its card.
fn group_head(ui: &mut Ui, title: &str, hint: &str) {
    super::settings::section_head(ui, title, hint);
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

/// One row of a group: what it shows and whether it can be used now.
struct Row<'a> {
    glyph: Glyph,
    name: &'a str,
    description: &'a str,
    action: String,
    button: Button,
    enabled: bool,
    /// How it stands: running, what stops it, or how it went last time.
    status: Option<(Status, String)>,
    /// Where what stops it is fixed.
    fix: Option<Fix>,
}

/// What a row did this frame.
struct RowOut {
    pressed: bool,
    /// The link after the reason a tool can't run was pressed.
    fix: Option<Fix>,
}

/// The side of the badge with a tool's picture.
const BADGE: f32 = 38.0;

/// The rows of a group in one card, a hairline between two. Returns what each row did.
fn row_list(ui: &mut Ui, salt: &str, rows: &[Row]) -> Vec<RowOut> {
    let mut outs = Vec::with_capacity(rows.len());
    card(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (k, r) in rows.iter().enumerate() {
            if k > 0 {
                ui.add_space(12.0);
                rule(ui);
                ui.add_space(12.0);
            }
            outs.push(tool_row(ui, ui.scope_id().with(("tool-row", salt, r.name)), r));
        }
    });
    outs
}

/// A row: the badge, the name, what the tool does and how it stands, and its button at the right (under the text
/// when the card is narrow).
fn tool_row(ui: &mut Ui, id: egui::Id, r: &Row) -> RowOut {
    let mut out = RowOut { pressed: false, fix: None };
    let button_w = remembered_width(ui, id.with("button"), 120.0);
    let narrow = ui.available_width() < 460.0;
    let destructive = r.button == Button::Danger;
    let text = |ui: &mut Ui, out: &mut RowOut| {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.add(egui::Label::new(RichText::new(r.name).font(super::fonts::semibold(FS_BODY))).wrap());
        ui.add(egui::Label::new(RichText::new(r.description).color(c_muted()).size(FS_META)).wrap());
        if let Some((status, line)) = &r.status {
            ui.add_space(2.0);
            // the mark, the words, and where to fix it right after them
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                // rows as tall as the text, so the words sit level with the mark
                ui.spacing_mut().interact_size.y = 16.0;
                let (mark, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
                paint_status_mark(ui, mark, *status);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new(line).color(status.color()).size(FS_META));
                    if let Some(fix) = r.fix {
                        if ui.link(RichText::new(fix.label()).size(FS_META).underline()).clicked() {
                            out.fix = Some(fix);
                        }
                    }
                });
            });
        }
    };
    let button = |ui: &mut Ui| match (r.button, r.enabled) {
        (Button::Plain, _) => secondary_enabled(ui, &r.action, r.enabled).clicked(),
        (Button::Danger, true) => danger(ui, &r.action).clicked(),
        (Button::Danger, false) => danger_enabled(ui, &r.action, false).clicked(),
    };
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (badge, _) = ui.allocate_exact_size(vec2(BADGE, BADGE), egui::Sense::hover());
        let (fill, fg) = match (r.enabled, destructive) {
            (false, _) => (c_hover(), c_faint()),
            (true, true) => (c_danger_bg(), c_danger()),
            (true, false) => (c_accent_soft(), if is_dark() { c_accent() } else { c_accent_hover() }),
        };
        ui.painter().rect_filled(badge, CornerRadius::same(R_CONTROL), fill);
        paint_glyph(ui.painter(), r.glyph, badge.center(), 17.0, fg);
        if narrow {
            ui.vertical(|ui| {
                text(ui, &mut out);
                ui.add_space(8.0);
                out.pressed = button(ui);
            });
            return;
        }
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - button_w - 20.0).max(160.0));
            text(ui, &mut out);
        });
        let used = ui
            .with_layout(Layout::right_to_left(Align::Min), |ui| {
                out.pressed = button(ui);
                ui.min_rect().width()
            })
            .inner;
        remember_width(ui, id.with("button"), used);
    });
    out
}

#[cfg_attr(not(feature = "converter"), allow(dead_code))]
/// A group of one row, as the world converter's entry uses it. Returns whether its button was pressed.
pub(super) fn card_frame(
    ui: &mut Ui,
    name: &str,
    description: &str,
    action: &str,
    enabled: bool,
    hint: Option<&str>,
) -> bool {
    let row = Row {
        glyph: Glyph::Icon(Icon::Convert),
        name,
        description,
        action: action.to_string(),
        button: Button::Plain,
        enabled,
        status: hint.map(|h| (Status::Warn, h.to_string())),
        fix: None,
    };
    row_list(ui, name, &[row]).into_iter().any(|o| o.pressed)
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
    fn a_row_says_first_that_it_runs_then_what_stops_it_then_how_it_went() {
        let ok = ToolResult { ok: true, text: "Old logs cleared (3 removed)".into(), at: std::time::Instant::now() };
        let bad = ToolResult { ok: false, text: "The folder is in use".into(), at: std::time::Instant::now() };
        assert_eq!(row_status(false, None, None, ""), None, "a tool that is simply ready says nothing");
        assert_eq!(row_status(true, Some("Close Minecraft first"), Some(&bad), "").unwrap().0, Status::Busy);
        let (s, text) = row_status(false, Some("Close Minecraft first"), Some(&ok), "").unwrap();
        assert_eq!((s, text.as_str()), (Status::Warn, "Close Minecraft first"));
        let (s, text) = row_status(false, None, Some(&ok), "just now").unwrap();
        assert_eq!((s, text.as_str()), (Status::Good, "Old logs cleared (3 removed) \u{00B7} just now"));
        let (s, text) = row_status(false, None, Some(&bad), "").unwrap();
        assert_eq!(s, Status::Error);
        assert!(text.starts_with("Didn't work: The folder is in use."), "{text}");
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
