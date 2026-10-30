//! The console window: everything the launcher and the games print, with the Java edition's buttons
//! (Clear, Copy Log, Upload Log, Kill Minecraft).

use egui::{RichText, ScrollArea};

use super::theme::*;
use super::widgets::*;
use super::{App, Msg};
use crate::config::{Pos, Size};
use crate::launch::{Level, LogLine};

const PASTE_URL: &str = "https://paste.atlauncher.com/api/create-v2";
const MAX_CONSOLE_LINES: usize = 8000;

impl App {
    /// Add launcher output to the console (and keep it from growing without end).
    pub fn console_push(&mut self, level: Level, text: &str) {
        for part in text.split('\n') {
            self.console_lines.push(LogLine { level, text: part.trim_end_matches('\r').to_string() });
        }
        if self.console_lines.len() > MAX_CONSOLE_LINES {
            self.console_lines.drain(..MAX_CONSOLE_LINES / 10);
        }
    }

    pub(super) fn console_ui(&mut self, ctx: &egui::Context) {
        if !self.settings.enable_console {
            return;
        }
        let size = self
            .settings
            .console_size
            .filter(|_| self.settings.remember_window_size_position)
            .unwrap_or(Size { width: 650, height: 400 });
        let mut builder = egui::ViewportBuilder::default()
            .with_title(format!("{} Console", crate::NAME))
            .with_inner_size([size.width as f32, size.height as f32])
            .with_min_inner_size([400.0, 200.0]);
        if let Some(p) = self.settings.console_position.filter(|_| self.settings.remember_window_size_position) {
            builder = builder.with_position([p.x as f32, p.y as f32]);
        }
        let mut action = ConsoleAction::None;
        let playing = self.games.values().any(|g| g.active());
        if !playing {
            // the question about stopping the game is gone with the game
            self.console_confirm_kill = false;
        }
        let uploading = self.console_uploading;
        let kill_confirm = self.console_confirm_kill;
        let lines = &self.console_lines;
        let mut closed = false;
        let mut geometry: Option<(Size, Pos)> = None;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("blockforge-console"), builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            if ctx.input(|i| i.viewport().close_requested()) {
                closed = true;
            }
            ctx.input(|i| {
                let v = i.viewport();
                if let (Some(inner), Some(outer)) = (v.inner_rect, v.outer_rect) {
                    geometry = Some((
                        Size { width: inner.width() as i32, height: inner.height() as i32 },
                        Pos { x: outer.min.x as i32, y: outer.min.y as i32 },
                    ));
                }
            });
            egui::Panel::bottom("console-bar")
                .frame(
                    egui::Frame::new()
                        .fill(c_surface())
                        .stroke(egui::Stroke::new(1.0, c_border()))
                        .inner_margin(egui::Margin::symmetric(16, 12)),
                )
                .show(ui, |ui| {
                    // the faint light along the top of the bar, as on the other panes of glass
                    let top = ui.max_rect().expand2(egui::vec2(16.0, 12.0));
                    ui.painter().hline(top.x_range(), top.top() + 1.5, egui::Stroke::new(1.0, c_glass_edge()));
                    ui.horizontal(|ui| {
                        if secondary(ui, "Copy log").clicked() {
                            action = ConsoleAction::Copy;
                        }
                        if uploading {
                            spinner_sized(ui, 14.0);
                            muted(ui, "Uploading the log");
                        } else if secondary(ui, "Upload log")
                            .on_hover_text(
                                "Puts the log on a public page and copies its link, to share when asking for help",
                            )
                            .clicked()
                        {
                            action = ConsoleAction::Upload;
                        }
                        if ghost(ui, "Clear").clicked() {
                            action = ConsoleAction::Clear;
                        }
                        if playing && danger(ui, "Stop Minecraft").clicked() {
                            action = ConsoleAction::AskKill;
                        }
                    });
                });
            // the question is a dialog over the log, not a toolbar that grows
            if playing && kill_confirm {
                let modal = egui::Modal::new(egui::Id::unique("console-kill")).frame(super::modal_frame(&ctx)).show(
                    &ctx,
                    |ui| {
                        ui.set_width(380.0);
                        ui.label(RichText::new("Stop Minecraft?").size(FS_SECTION).semibold());
                        ui.add_space(4.0);
                        muted(ui, super::STOP_WARNING);
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            if danger(ui, "Stop Minecraft").clicked() {
                                action = ConsoleAction::Kill;
                            }
                            if secondary(ui, "Cancel").clicked() {
                                action = ConsoleAction::CancelKill;
                            }
                        });
                    },
                );
                if modal.should_close() {
                    action = ConsoleAction::CancelKill;
                }
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(c_field()).inner_margin(egui::Margin::symmetric(16, 12)))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 1.0;
                    ScrollArea::both().auto_shrink(false).stick_to_bottom(true).show_rows(
                        ui,
                        row_h,
                        lines.len(),
                        |ui, range| {
                            for line in &lines[range] {
                                super::library::log_row(ui, line, false);
                            }
                        },
                    );
                });
        });
        if closed {
            self.settings.enable_console = false;
            self.save_settings();
        }
        if let (Some((size, pos)), true) = (geometry, self.settings.remember_window_size_position) {
            if self.settings.console_size != Some(size) || self.settings.console_position != Some(pos) {
                self.settings.console_size = Some(size);
                self.settings.console_position = Some(pos);
                // changes on every frame while the window is dragged: written once it stops
                self.save_settings_soon();
            }
        }
        match action {
            ConsoleAction::None => {}
            ConsoleAction::Clear => self.console_lines.clear(),
            ConsoleAction::Copy => {
                let text: Vec<&str> = self.console_lines.iter().map(|l| l.text.as_str()).collect();
                ctx.copy_text(text.join("\n"));
                self.toast("Log copied");
            }
            ConsoleAction::Upload => {
                self.console_uploading = true;
                let text: Vec<String> = self.console_lines.iter().map(|l| l.text.clone()).collect();
                // the paste is public: sign-in tokens, e-mail addresses, the home folder and addresses are taken out
                // first, whatever printed them (a mod, an old version's "Session ID")
                let secrets: Vec<String> =
                    self.accounts.iter().filter_map(|a| a.access_token.clone()).filter(|t| t.len() > 3).collect();
                let n = self.notifier();
                self.rt.spawn(async move {
                    let text = crate::diagnostics::scrub(&text.join("\n"), &secrets);
                    let body = serde_json::json!({"title": format!("{} - Log", crate::NAME), "text": text});
                    let r = async {
                        let r = crate::http::client().post(PASTE_URL).json(&body).send().await?;
                        anyhow::ensure!(r.status().is_success(), "the paste service answered {}", r.status());
                        let t = r.text().await?;
                        Ok::<String, anyhow::Error>(t.trim().trim_matches('"').to_string())
                    }
                    .await
                    .map_err(|e| format!("{e:#}"));
                    n.send(Msg::LogUploaded(r));
                });
            }
            ConsoleAction::AskKill => self.console_confirm_kill = true,
            ConsoleAction::CancelKill => self.console_confirm_kill = false,
            ConsoleAction::Kill => {
                self.console_confirm_kill = false;
                let folders: Vec<String> =
                    self.games.iter().filter(|(_, g)| g.active()).map(|(f, _)| f.clone()).collect();
                for f in folders {
                    self.stop_session(&f);
                }
            }
        }
    }
}

enum ConsoleAction {
    None,
    Clear,
    Copy,
    Upload,
    AskKill,
    CancelKill,
    Kill,
}
