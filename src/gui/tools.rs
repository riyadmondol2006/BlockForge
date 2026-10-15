//! The Tools page: clean up, diagnose problems and keep things tidy.

use egui::{CornerRadius, RichText, ScrollArea, Stroke, Ui};

use super::services::{NetReport, SvcMsg, confirm_modal};
use super::theme::*;
use super::widgets::*;
use super::{App, Msg};
use crate::svcsettings::ServiceSettings;
use crate::tools::{self, NetworkCheckOptions, TOOLS, ToolInfo};

const CARD_WIDTH: f32 = 300.0;

impl App {
    pub(super) fn tools_ui(&mut self, ui: &mut Ui) {
        heading(ui, "Tools");
        muted(ui, "Clean up, diagnose problems and keep things tidy");
        ui.add_space(10.0);
        let svc = ServiceSettings::from_settings(&self.settings);
        let per_row = ((ui.available_width() + 12.0) / (CARD_WIDTH + 12.0)).floor().max(1.0) as usize;
        let mut clicked: Option<&'static str> = None;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for row in TOOLS.chunks(per_row) {
                ui.horizontal_top(|ui| {
                    for tool in row {
                        let (enabled, hint) = match tool.id {
                            "network_checker" if !tools::can_run_network_checker(&svc) => {
                                (false, Some("Turn on \"Report pack installs and play time\" in Settings first."))
                            }
                            "debug_mode" if tools::is_flatpak() => (false, Some("Not available in a Flatpak.")),
                            "debug_mode" if tools::is_debug_enabled() => (false, Some("Debug mode is already on.")),
                            "skin_updater" if self.accounts.is_empty() => (false, Some("Add an account first.")),
                            _ => (true, None),
                        };
                        if tool_card(ui, tool, enabled, hint) {
                            clicked = Some(tool.id);
                        }
                    }
                });
                ui.add_space(12.0);
            }
        });
        if let Some(id) = clicked {
            self.tool_clicked(id);
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
                        let _ = open::that(folder);
                    }
                    Ok(match found.first() {
                        Some(d) => format!("Support bundle saved to your downloads ({})", d.title),
                        None => "Support bundle saved to your downloads".to_string(),
                    })
                });
            }
            "log_clearer" => {
                tools::clear_logs(&dirs);
                self.toast("Successfully cleared the logs.");
            }
            "debug_mode" => match tools::relaunch_in_debug() {
                Ok(()) => self.ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(e) => self.toast_err(format!("{e:#}")),
            },
            "download_clearer" => {
                tools::clear_downloads(&dirs);
                self.toast("Successfully cleared the downloads.");
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
                        Ok("Successfully updated skins.".to_string())
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
                    let freed = crate::classcache::clear(&dirs);
                    self.toast(format!("Game start cache cleared ({} MB freed)", freed / (1024 * 1024)));
                }
            }
            "libraries_deleter" => {
                tools::delete_libraries(&dirs);
                self.toast("Successfully deleted libraries.");
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
                    egui::Modal::new(egui::Id::unique("tool-confirm")).show(ctx, |ui| {
                        ui.set_width(460.0);
                        ui.label(RichText::new("Network checker").size(19.0).strong());
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
                egui::Modal::new(egui::Id::unique("net-report")).show(ctx, |ui| {
                    ui.set_width(680.0);
                    ui.label(RichText::new("Network checker").size(21.0).strong());
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
                    ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
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
    let mut clicked = false;
    egui::Frame::new()
        .fill(c_card())
        .stroke(Stroke::new(1.0, c_border()))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(CARD_WIDTH - 34.0);
            ui.set_min_height(130.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(tool.name).strong().size(15.5));
                ui.add(egui::Label::new(RichText::new(tool.description).color(c_muted()).size(12.5)).wrap());
                if let Some(h) = hint {
                    ui.add_space(2.0);
                    ui.add(egui::Label::new(RichText::new(h).color(c_warn()).size(12.0)).wrap());
                }
                ui.add_space(8.0);
                clicked = primary_enabled(ui, tool.action, enabled).clicked();
            });
        });
    ui.add_space(4.0);
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
}
