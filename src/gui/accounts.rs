//! Accounts: Microsoft sign-in and offline players.

use egui::{Align, Layout, RichText, ScrollArea, Ui};

use super::App;
use super::theme::*;
use super::widgets::*;
use crate::msauth::DeviceCode;

/// The widest the page's content gets.
const COLUMN: f32 = 720.0;

#[derive(Default)]
pub struct AcctState {
    pub offline_name: String,
    pub device: Option<DeviceCode>,
    pub signing_in: bool,
    /// Counts the sign-ins started, so the answer of one that was cancelled is told apart from the current one.
    pub attempt: u64,
    /// The account whose Remove was pressed, waiting for "Remove" to be pressed again.
    pub confirm_remove: Option<String>,
    /// Why the last sign-in failed. It stays under the Microsoft row until the next try: the player is usually still
    /// in the browser when it fails, and a toast would be gone by the time they come back.
    pub last_error: Option<String>,
}

impl App {
    pub(super) fn accounts_ui(&mut self, ui: &mut Ui) {
        heading(ui, "Accounts");
        muted(ui, "Choose who plays. Microsoft accounts can join online servers; offline players can't.");
        ui.add_space(8.0);
        let mut select: Option<String> = None;
        let mut remove: Option<String> = None;
        let mut change_skin: Option<String> = None;
        let mut copied = false;
        let mut sign_in = false;
        let current = self.current_account().map(|a| a.uuid.clone());
        ScrollArea::vertical().id_salt("accounts").auto_shrink(false).show(ui, |ui| {
            // at most 720 wide, but never wider than the window
            ui.set_max_width((ui.available_width() - 14.0).min(COLUMN));
            let accounts = self.accounts.clone();
            if accounts.is_empty() {
                card(ui, |ui| {
                    ui.label(RichText::new("No accounts yet").semibold());
                    small_muted(ui, "Add one below.");
                });
            } else {
                card(ui, |ui| {
                    let mut rows = super::settings::Rows::new();
                    for acc in &accounts {
                        rows.gap(ui);
                        // the buttons keep their room on the right (as wide as on the last frame)
                        let buttons_id = ui.scope_id().with(("account-buttons", &acc.uuid));
                        let buttons_w = remembered_width(ui, buttons_id, 260.0);
                        let mut used = buttons_w;
                        ui.horizontal(|ui| {
                            self.account_avatar(ui, &acc.uuid, acc.name(), acc.offline, 40.0);
                            ui.vertical(|ui| {
                                ui.set_max_width((ui.available_width() - buttons_w - 12.0).max(120.0));
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(acc.name()).semibold());
                                    if current.as_ref() == Some(&acc.uuid) {
                                        ink_tag(ui, "Playing as");
                                    }
                                    if acc.must_login {
                                        tag(ui, "Sign in again", c_danger());
                                    }
                                });
                                account_meta(ui, acc.offline, &acc.uuid);
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                // removing asks once more: a Microsoft account has to be signed in again afterwards
                                if self.acct.confirm_remove.as_deref() == Some(acc.uuid.as_str()) {
                                    let after = if acc.offline {
                                        "The name can be added again at any time"
                                    } else {
                                        "To use this account again, sign in with Microsoft again"
                                    };
                                    if danger(ui, "Yes, remove").on_hover_text(after).clicked() {
                                        remove = Some(acc.uuid.clone());
                                    }
                                    if secondary(ui, "Cancel").clicked() {
                                        self.acct.confirm_remove = None;
                                    }
                                    used = ui.min_rect().width();
                                    return;
                                }
                                kebab_menu(ui, |ui| {
                                    if acc.must_login && !acc.offline && ui.button("Sign in again").clicked() {
                                        sign_in = true;
                                        ui.close();
                                    }
                                    if ui.button("Copy UUID").clicked() {
                                        ui.ctx().copy_text(acc.uuid.clone());
                                        copied = true;
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.button("Remove").clicked() {
                                        self.acct.confirm_remove = Some(acc.uuid.clone());
                                        ui.close();
                                    }
                                });
                                if secondary(ui, "Change skin").clicked() {
                                    change_skin = Some(acc.uuid.clone());
                                }
                                if acc.must_login && !acc.offline {
                                    if primary(ui, "Sign in again").clicked() {
                                        sign_in = true;
                                    }
                                } else if current.as_ref() != Some(&acc.uuid) && secondary(ui, "Use").clicked() {
                                    select = Some(acc.uuid.clone());
                                }
                                used = ui.min_rect().width();
                            });
                        });
                        remember_width(ui, buttons_id, used);
                    }
                });
            }

            super::settings::section_head(ui, "Add an account", "");
            card(ui, |ui| {
                super::settings::wide_row(
                    ui,
                    "Microsoft",
                    "Your browser opens a Microsoft page; enter the code shown here to sign in.",
                    220.0,
                    |ui| {
                        if self.acct.device.is_none() && !self.acct.signing_in && primary(ui, "Sign in with Microsoft").clicked()
                        {
                            self.start_sign_in();
                        }
                    },
                );
                if let Some(e) = self.acct.last_error.clone() {
                    ui.add_space(8.0);
                    if notice(ui, NoticeKind::Error, &e, Some("Try again")).is_some_and(|r| r.clicked()) {
                        self.start_sign_in();
                    }
                }
                match self.acct.device.clone() {
                    Some(code) => {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            // the code, large and in figures of equal width, so it can be read out and typed
                            egui::Frame::new()
                                .fill(c_field())
                                .stroke(egui::Stroke::new(1.0, c_control()))
                                .corner_radius(egui::CornerRadius::same(R_CONTROL))
                                .inner_margin(egui::Margin::symmetric(16, 8))
                                .show(ui, |ui| {
                                    ui.label(RichText::new(&code.user_code).size(FS_TITLE).semibold().monospace());
                                });
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 6.0;
                                ui.horizontal(|ui| {
                                    if secondary(ui, "Copy code").clicked() {
                                        ui.ctx().copy_text(code.user_code.clone());
                                        self.toast("Copied the code");
                                    }
                                    if secondary(ui, "Open the page again").clicked() {
                                        self.open_link(&code.verification_uri);
                                    }
                                    if secondary(ui, "Cancel").clicked() {
                                        self.acct.device = None;
                                        self.acct.signing_in = false;
                                    }
                                });
                            });
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            spinner(ui);
                            muted(
                                ui,
                                format!(
                                    "The code is on your clipboard. Paste it at {} and finish there.",
                                    code.verification_uri
                                ),
                            );
                        });
                    }
                    None if self.acct.signing_in => {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            spinner(ui);
                            muted(ui, "Asking Microsoft for a code");
                            if secondary(ui, "Cancel").clicked() {
                                self.acct.signing_in = false;
                            }
                        });
                    }
                    None => {}
                }
                ui.add_space(8.0);
                rule(ui);
                ui.add_space(8.0);
                super::settings::wide_row(
                    ui,
                    "Offline player",
                    "Just a name. Works for single player and offline-mode servers. 3\u{2013}16 letters, numbers or underscores.",
                    330.0,
                    |ui| {
                        let mut name = std::mem::take(&mut self.acct.offline_name);
                        // the row lays out from the right, so the field and button go in the order that reads right
                        let reversed = ui.layout().prefer_right_to_left();
                        let mut r = None;
                        let mut add_clicked = false;
                        let valid = super::whoplays::valid_player_name(name.trim());
                        for step in if reversed { [1, 0] } else { [0, 1] } {
                            if step == 0 {
                                r = Some(search_field(ui, &mut name, "Player name", 240.0));
                            } else {
                                add_clicked = secondary_enabled(ui, "Add", valid).clicked();
                            }
                        }
                        // the same rule as everywhere else a name is typed (Minecraft's own: 3 to 16 characters)
                        let valid = super::whoplays::valid_player_name(name.trim());
                        if let Some(problem) = super::whoplays::name_problem(name.trim()).filter(|_| !name.trim().is_empty()) {
                            ui.label(RichText::new(problem).size(FS_META).color(c_danger()));
                        }
                        let add = add_clicked
                            || (valid && r.is_some_and(|r| r.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                        if add && valid {
                            let acc = crate::accounts::Account::offline(name.trim());
                            self.accounts.retain(|a| !(a.offline && a.name() == acc.name()));
                            self.settings.last_account = Some(acc.uuid.clone());
                            self.toast(format!("Added {}", acc.name()));
                            self.accounts.push(acc);
                            self.save_accounts();
                            self.save_settings();
                            name.clear();
                        }
                        self.acct.offline_name = name;
                    },
                );
            });
            ui.add_space(16.0);
        });
        if copied {
            self.toast("Copied the UUID");
        }
        if let Some(id) = change_skin {
            self.open_skin_dialog(&id);
        }
        if let Some(id) = select {
            self.settings.last_account = Some(id);
            self.save_settings();
        }
        if sign_in {
            self.start_sign_in();
        }
        if let Some(id) = remove {
            self.acct.confirm_remove = None;
            let name = self.accounts.iter().find(|a| a.uuid == id).map(|a| a.name().to_string()).unwrap_or_default();
            self.accounts.retain(|a| a.uuid != id);
            let was_playing = current.as_deref() == Some(id.as_str());
            if self.settings.last_account.as_deref() == Some(&id) {
                self.settings.last_account = None;
            }
            self.save_accounts();
            self.save_settings();
            match self.current_account().map(|a| a.name().to_string()) {
                Some(next) if was_playing => self.toast(format!("Removed {name}. Now playing as {next}")),
                _ => self.toast(format!("Removed {name}")),
            }
        }
    }
}

/// The second line of an account row: what kind of account it is and its UUID, as one muted line. The UUID is set in
/// the monospace style (figures of equal width) and cut with an ellipsis when the row is narrow; the tooltip has it all.
fn account_meta(ui: &mut Ui, offline: bool, uuid: &str) {
    let format = |font_id: egui::FontId| egui::TextFormat {
        font_id,
        color: c_muted(),
        valign: Align::Center,
        ..Default::default()
    };
    let kind = if offline { "Offline player" } else { "Microsoft account" };
    let mut job = egui::text::LayoutJob::default();
    job.append(&format!("{kind} \u{00B7} "), 0.0, format(egui::FontId::proportional(FS_META)));
    job.append(uuid, 0.0, format(egui::TextStyle::Monospace.resolve(ui.style())));
    ui.add(egui::Label::new(job).truncate()).on_hover_text(uuid);
}

/// A small filled label in the ink colour: what is true of this one row (the account that is playing).
fn ink_tag(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(FS_META), c_on_ink());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(galley.size().x + 12.0, 20.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, egui::CornerRadius::same(R_TAG), c_ink());
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, c_on_ink());
}
