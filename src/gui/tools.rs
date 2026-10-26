//! The Tools page: clean up, diagnose problems and keep things tidy.

use egui::{Align, Layout, RichText, ScrollArea, Ui};

use super::services::{NetReport, SvcMsg, confirm_modal, modal_scroll_height, modal_width};
use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::tools::{self, NetworkCheckOptions, TOOLS};

/// The widest the list of tools gets.
const LIST_WIDTH: f32 = 720.0;

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
            // at most 720 wide, but never wider than the window
            ui.set_max_width((ui.available_width() - 14.0).min(LIST_WIDTH));
            for (title, ids) in [("Diagnose", &DIAGNOSE[..]), ("Clean up", &CLEAN_UP[..])] {
                group(ui, title, |ui, rows| {
                    for tool in ids.iter().filter_map(|id| TOOLS.iter().find(|t| t.id == *id)) {
                        rows.gap(ui);
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
                            "debug_mode" if tools::is_debug_enabled() => {
                                (false, Some(("Debug mode is already on.", None)))
                            }
                            "skin_updater" if no_accounts => {
                                (false, Some(("Add an account first.", Some(Fix::Accounts))))
                            }
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
                        let out = tool_row(
                            ui,
                            tool.name,
                            tool.description,
                            &action,
                            if is_destructive(tool.id) { Button::Danger } else { Button::Plain },
                            enabled,
                            hint,
                        );
                        if out.pressed {
                            clicked = Some(tool.id);
                        }
                        if out.fix.is_some() {
                            go = out.fix;
                        }
                    }
                });
            }
            // the world converter has a row of its own (it opens a page, not a job)
            #[cfg(feature = "converter")]
            group(ui, "Worlds", |ui, _rows| {
                if self.convert_tool_card(ui) {
                    open_convert = true;
                }
            });
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

/// A titled group of rows in one card, divided by rules: call `rows.gap(ui)` before each row.
fn group(ui: &mut Ui, title: &str, rows: impl FnOnce(&mut Ui, &mut super::settings::Rows)) {
    super::settings::section_head(ui, title, "");
    card(ui, |ui| {
        // the name of a row sits close to what it says
        ui.spacing_mut().item_spacing.y = 2.0;
        rows(ui, &mut super::settings::Rows::new());
    });
}

/// How a row's button looks.
#[derive(Clone, Copy, PartialEq)]
enum Button {
    Plain,
    Danger,
}

/// What a row did this frame.
struct RowOut {
    pressed: bool,
    /// The link next to the reason a tool can't run was pressed.
    fix: Option<Fix>,
}

/// A row of the list: the name, what the tool does and, when it can't be used now, why (with a link to where that is
/// fixed); then its button on the right.
fn tool_row(
    ui: &mut Ui,
    name: &str,
    description: &str,
    action: &str,
    button: Button,
    enabled: bool,
    hint: Option<(&str, Option<Fix>)>,
) -> RowOut {
    let mut out = RowOut { pressed: false, fix: None };
    // the text makes room for the button (as wide as it was on the last frame)
    let id = ui.scope_id().with(("tool-row", name));
    let button_w = remembered_width(ui, id, 80.0);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - button_w - 16.0).max(140.0));
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(name).semibold());
            ui.add(egui::Label::new(RichText::new(description).color(c_muted()).size(FS_META)).wrap());
            if let Some((reason, fix)) = hint {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 2.0);
                    ui.spacing_mut().interact_size.y = 16.0;
                    ui.label(RichText::new(reason).color(c_faint()).size(FS_META));
                    if let Some(fix) = fix {
                        if ui.link(RichText::new(fix.label()).size(FS_META).underline()).clicked() {
                            out.fix = Some(fix);
                        }
                    }
                });
            }
        });
        let used = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                // a tool that can't run now shows its button greyed out, and the reason under its name
                out.pressed = match (button, enabled) {
                    (Button::Plain, _) => secondary_enabled(ui, action, enabled).clicked(),
                    (Button::Danger, true) => danger(ui, action).clicked(),
                    (Button::Danger, false) => danger_enabled(ui, action, false).clicked(),
                };
                ui.min_rect().width()
            })
            .inner;
        remember_width(ui, id, used);
    });
    out
}

#[cfg_attr(not(feature = "converter"), allow(dead_code))]
/// A row of the list, as the world converter's entry uses it. Returns whether its button was pressed.
pub(super) fn card_frame(
    ui: &mut Ui,
    name: &str,
    description: &str,
    action: &str,
    enabled: bool,
    hint: Option<&str>,
) -> bool {
    tool_row(ui, name, description, action, Button::Plain, enabled, hint.map(|h| (h, None))).pressed
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
