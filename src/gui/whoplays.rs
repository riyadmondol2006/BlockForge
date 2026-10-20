//! "Who's playing?", shown when the launcher opens (if the setting is on): same steps and wording as the Java edition.

use egui::{RichText, Sense, Ui};

use super::icons::{self, Icon};
use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::accounts::Account;

#[derive(Default)]
pub struct WhoState {
    name_step: bool,
    name: String,
    error: bool,
}

impl WhoState {
    /// Straight at the "Play offline" step (for the screenshot tour).
    pub fn offline_step() -> WhoState {
        WhoState { name_step: true, ..Default::default() }
    }
}

/// 3-16 letters, numbers or underscores.
pub fn valid_player_name(n: &str) -> bool {
    (3..=16).contains(&n.len()) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// One of the two ways in. The Microsoft one is the filled, main choice; the offline one is plain.
fn choice(ui: &mut Ui, width: f32, icon: Icon, title: &str, sub: &str, main: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 96.0), Sense::click());
    let hovered = resp.hovered();
    let (bg, line, fg, sub_fg) = if main {
        (if hovered { c_accent_hover() } else { c_accent() }, egui::Color32::TRANSPARENT, c_on_accent(), c_on_accent())
    } else {
        (if hovered { c_field_hover() } else { c_field() }, c_control(), c_text(), c_muted())
    };
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(R_CARD),
        bg,
        egui::Stroke::new(1.0, line),
        egui::StrokeKind::Inside,
    );
    icons::paint(ui.painter(), icon, rect.left_top() + egui::vec2(34.0, 30.0), 20.0, fg);
    ui.painter().text(
        rect.left_top() + egui::vec2(20.0, 58.0),
        egui::Align2::LEFT_CENTER,
        title,
        super::fonts::semibold(15.0),
        fg,
    );
    ui.painter().text(
        rect.left_top() + egui::vec2(20.0, 77.0),
        egui::Align2::LEFT_CENTER,
        sub,
        egui::FontId::proportional(13.0),
        sub_fg,
    );
    focus_ring(ui, &resp, rect, R_CARD);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

impl App {
    pub(super) fn who_ui(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.who.take() else { return };
        let mut close = false;
        let mut sign_in = false;
        let mut pick: Option<String> = None;
        let mut offline: Option<String> = None;
        let accounts: Vec<(String, String, &'static str, bool)> = self
            .accounts
            .iter()
            .rev()
            .take(3)
            .map(|a| (a.uuid.clone(), a.name().to_string(), a.kind(), a.offline))
            .collect();
        let modal = egui::Modal::new(egui::Id::unique("who")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(500.0);
            if !st.name_step {
                ui.label(RichText::new("Who's playing?").size(18.0).semibold());
                muted(ui, "Sign in with your Microsoft account, or play offline with any name.");
                ui.add_space(12.0);
                let w = ((ui.available_width() - 8.0) / 2.0).floor();
                ui.horizontal(|ui| {
                    if choice(ui, w, Icon::User, "Sign in", "Use your Microsoft account", true) {
                        sign_in = true;
                    }
                    if choice(ui, w, Icon::Play, "Play offline", "Pick any player name", false) {
                        st.name_step = true;
                    }
                });
                if !accounts.is_empty() {
                    ui.add_space(16.0);
                    ui.label(RichText::new("Continue as").size(13.0).color(c_muted()));
                    for (uuid, name, kind, offline) in &accounts {
                        // the row's fill is put in behind its contents once it is known whether it is pointed at
                        let fill = ui.painter().add(egui::Shape::Noop);
                        let row = egui::Frame::new()
                            .stroke(egui::Stroke::new(1.0, c_border()))
                            .corner_radius(egui::CornerRadius::same(R_CONTROL))
                            .inner_margin(egui::Margin::same(10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    // the player's face, like on the sidebar and the Accounts page
                                    self.account_avatar(ui, uuid, name, *offline, 32.0);
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 0.0;
                                        ui.label(RichText::new(name).semibold());
                                        small_muted(ui, *kind);
                                    });
                                });
                            });
                        let resp =
                            row.response.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                        ui.painter().set(
                            fill,
                            egui::Shape::rect_filled(
                                resp.rect,
                                egui::CornerRadius::same(R_CONTROL),
                                if resp.hovered() { c_field_hover() } else { c_field() },
                            ),
                        );
                        if resp.clicked() {
                            pick = Some(uuid.clone());
                        }
                    }
                }
            } else {
                ui.label(RichText::new("Play offline").size(18.0).semibold());
                muted(ui, "3-16 letters, numbers or underscores. No account needed.");
                ui.add_space(12.0);
                let r = ui.add_sized(
                    [ui.available_width(), 36.0],
                    egui::TextEdit::singleline(&mut st.name)
                        .hint_text(RichText::new("Player name").color(c_faint()))
                        .margin(egui::Margin::symmetric(12, 9))
                        .background_color(c_field()),
                );
                // typing can start at once
                focus_when_shown(ui, &r);
                if st.error {
                    ui.label(RichText::new("Use 3-16 letters, numbers or underscores.").color(c_danger()));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let go = primary(ui, "Continue").clicked()
                        || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if go {
                        if valid_player_name(st.name.trim()) {
                            offline = Some(st.name.trim().to_string());
                        } else {
                            st.error = true;
                        }
                    }
                    if secondary(ui, "Back").clicked() {
                        st.name_step = false;
                        st.error = false;
                    }
                });
            }
        });
        if modal.should_close() {
            close = true;
        }
        let offline_or_pick = offline.is_some() || pick.is_some();
        if let Some(name) = offline {
            let acc = Account::offline(&name);
            self.accounts.retain(|a| !(a.offline && a.name() == acc.name()));
            self.settings.last_account = Some(acc.uuid.clone());
            self.accounts.push(acc);
            self.save_accounts();
            self.save_settings();
            close = true;
        }
        if let Some(id) = pick {
            self.settings.last_account = Some(id);
            self.save_settings();
            close = true;
        }
        if sign_in {
            self.start_sign_in();
            close = true;
        }
        if !close {
            self.who = Some(st);
        } else if self.accounts.is_empty() || sign_in {
            // nobody was chosen (or the sign-in is still going on): the game waits for another press of Play
            self.who_then_play = None;
        } else if let Some(folder) = self.who_then_play.take() {
            // the question came up because Play was pressed without an account: carry on with the game
            if offline_or_pick {
                self.play(&folder, crate::launch::LaunchOptions::default());
            }
        }
    }

    /// Open the Microsoft sign-in: the Accounts page shows the code to enter.
    pub(super) fn start_sign_in(&mut self) {
        self.page = Page::Accounts;
        if self.acct.signing_in {
            return; // one is going on already: its code is on the Accounts page
        }
        self.acct.signing_in = true;
        self.acct.device = None;
        self.acct.attempt += 1;
        let (attempt, n) = (self.acct.attempt, self.notifier());
        self.rt.spawn(async move {
            n.send(Msg::Device(attempt, crate::msauth::start_device_login().await.map_err(|e| format!("{e:#}"))));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::valid_player_name;

    #[test]
    fn player_names_follow_minecraft_rules() {
        assert!(valid_player_name("Steve"));
        assert!(valid_player_name("abc"));
        assert!(valid_player_name("Bob_the_Builder1"));
        assert!(!valid_player_name("ab"));
        assert!(!valid_player_name("seventeen_letters"));
        assert!(!valid_player_name("with space"));
        assert!(!valid_player_name("caf\u{e9}"));
        assert!(!valid_player_name(""));
    }
}
