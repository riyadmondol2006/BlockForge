//! The Tools page: clean up, diagnose problems and keep things tidy.

use egui::{Align, Layout, RichText, ScrollArea, Ui};

use super::services::{NetReport, SvcMsg, confirm_modal, modal_scroll_height, modal_width};
use super::theme::*;
use super::widgets::*;
use super::{App, Msg};
use crate::tools::{self, NetworkCheckOptions, TOOLS, ToolInfo};

/// The widest the list of tools gets.
const LIST_WIDTH: f32 = 760.0;

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

impl App {
    pub(super) fn tools_ui(&mut self, ui: &mut Ui) {
        heading(ui, "Tools");
        muted(ui, "Clean up, diagnose problems and keep things tidy");
        ui.add_space(12.0);
        // the network checker needs `enableLogs`, which is a plain field of the settings (no need to read them all)
        let logs_on = self.settings.enable_logs;
        let flatpak = *FLATPAK.get_or_init(tools::is_flatpak);
        let mut clicked: Option<&'static str> = None;
        #[cfg(feature = "converter")]
        let mut open_convert = false;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // at most 760 wide, but never wider than the window
            ui.set_max_width((ui.available_width() - 14.0).min(LIST_WIDTH));
            // the world converter has a row of its own, first (it opens a page, not a job)
            #[allow(unused_mut)]
            let mut tools: Vec<Option<&ToolInfo>> = Vec::new();
            #[cfg(feature = "converter")]
            tools.push(None);
            tools.extend(TOOLS.iter().map(Some));
            card(ui, |ui| {
                let mut rows = super::settings::Rows::new();
                for tool in tools {
                    rows.gap(ui);
                    let Some(tool) = tool else {
                        #[cfg(feature = "converter")]
                        if self.convert_tool_card(ui) {
                            open_convert = true;
                        }
                        continue;
                    };
                    let (enabled, hint) = match tool.id {
                        "network_checker" if !logs_on => {
                            (false, Some("Turn on \"Report pack installs and play time\" in Settings first."))
                        }
                        "debug_mode" if flatpak => (false, Some("Not available in a Flatpak.")),
                        "debug_mode" if tools::is_debug_enabled() => (false, Some("Debug mode is already on.")),
                        "skin_updater" if self.accounts.is_empty() => (false, Some("Add an account first.")),
                        _ => (true, None),
                    };
                    if tool_card(ui, tool, enabled, hint) {
                        clicked = Some(tool.id);
                    }
                }
            });
            ui.add_space(16.0);
        });
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
                Ok(()) => self.ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(e) => self.toast_err(format!("{e:#}")),
            },
            "download_clearer" => {
                if let Some(why) = busy_reason(self.games.values().any(super::Game::active), !self.tasks.is_empty()) {
                    self.toast_err(why);
                    return;
                }
                self.start_task("Clearing downloads", move |_p, _n| async move {
                    let n = tokio::task::spawn_blocking(move || tools::clear_downloads(&dirs)).await?;
                    Ok(format!("Downloads cleared ({n} removed)"))
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
                        let freed = tokio::task::spawn_blocking(move || crate::classcache::clear(&dirs)).await?;
                        Ok(format!("Game start cache cleared ({} MB freed)", freed / (1024 * 1024)))
                    });
                }
            }
            "libraries_deleter" => {
                if let Some(why) = busy_reason(self.games.values().any(super::Game::active), !self.tasks.is_empty()) {
                    self.toast_err(why);
                    return;
                }
                self.start_task("Deleting libraries", move |_p, _n| async move {
                    let n = tokio::task::spawn_blocking(move || tools::delete_libraries(&dirs)).await?;
                    Ok(format!("Libraries deleted ({n} removed). They download again when a game starts"))
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
                        ui.label(RichText::new("Network checker").size(18.0).semibold());
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
                    ui.label(RichText::new("Network checker").size(18.0).semibold());
                    match (&report.link, &report.error) {
                        (Some(link), _) => {
                            small_muted(ui, "The result was shared so problems can be diagnosed. Link:");
                            ui.horizontal(|ui| {
                                ui.hyperlink(link);
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

/// One card of the grid. Returns whether its button was pressed.
fn tool_card(ui: &mut Ui, tool: &ToolInfo, enabled: bool, hint: Option<&str>) -> bool {
    card_frame(ui, tool.name, tool.description, tool.action, enabled, hint)
}

/// A row of the list: the name, what the tool does and, when it can't be used now, why; then its button on the right.
/// Returns whether the button was pressed.
pub(super) fn card_frame(
    ui: &mut Ui,
    name: &str,
    description: &str,
    action: &str,
    enabled: bool,
    hint: Option<&str>,
) -> bool {
    let mut clicked = false;
    // the text makes room for the button (as wide as it was on the last frame)
    let id = ui.scope_id().with(("tool-row", name));
    let button_w = remembered_width(ui, id, 80.0);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - button_w - 16.0).max(140.0));
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(name).semibold());
            ui.add(egui::Label::new(RichText::new(description).color(c_muted()).size(12.5)).wrap());
            if let Some(h) = hint {
                ui.add(egui::Label::new(RichText::new(h).color(c_muted()).size(12.5)).wrap());
            }
        });
        let used = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                // a tool that can't run now shows its button greyed out, and the reason under its name
                clicked =
                    if enabled { secondary(ui, action).clicked() } else { danger_enabled(ui, action, false).clicked() };
                ui.min_rect().width()
            })
            .inner;
        remember_width(ui, id, used);
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_card_draws_without_a_window() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for tool in TOOLS.iter() {
                assert!(!tool_card(ui, tool, true, None), "nothing was pressed");
            }
            tool_card(ui, &TOOLS[0], false, Some("Turn on logs first."));
        });
        assert!(out.shapes.len() >= TOOLS.len());
        out.textures_delta.clear();
    }

    #[test]
    fn shared_files_are_only_deleted_when_nothing_uses_them() {
        assert_eq!(busy_reason(false, false), None);
        assert!(busy_reason(true, false).is_some_and(|r| r.contains("Minecraft")));
        assert!(busy_reason(true, true).is_some_and(|r| r.contains("Minecraft")), "the game is named first");
        assert!(busy_reason(false, true).is_some_and(|r| r.contains("jobs")));
    }
}
