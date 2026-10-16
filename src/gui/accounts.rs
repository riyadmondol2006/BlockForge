//! Accounts: Microsoft sign-in and offline players.

use egui::{Align, Layout, RichText, ScrollArea, Ui};

use super::App;
use super::theme::*;
use super::widgets::*;
use crate::msauth::DeviceCode;

#[derive(Default)]
pub struct AcctState {
    pub offline_name: String,
    pub device: Option<DeviceCode>,
    pub signing_in: bool,
    /// Counts the sign-ins started, so the answer of one that was cancelled is told apart from the current one.
    pub attempt: u64,
    /// The account whose Remove was pressed, waiting for "Remove" to be pressed again.
    pub confirm_remove: Option<String>,
}

impl App {
    pub(super) fn accounts_ui(&mut self, ui: &mut Ui) {
        heading(ui, "Accounts");
        muted(ui, "Choose who plays. Microsoft accounts can join online servers; offline players can't.");
        ui.add_space(8.0);
        let mut select: Option<String> = None;
        let mut remove: Option<String> = None;
        let mut change_skin: Option<String> = None;
        let current = self.current_account().map(|a| a.uuid.clone());
        ScrollArea::vertical().id_salt("accounts").auto_shrink(false).show(ui, |ui| {
            let accounts = self.accounts.clone();
            for acc in &accounts {
                card(ui, |ui| {
                    // the buttons keep their room on the right (as wide as on the last frame)
                    let buttons_id = ui.scope_id().with(("account-buttons", &acc.uuid));
                    let buttons_w = remembered_width(ui, buttons_id, 300.0);
                    let mut used = buttons_w;
                    ui.horizontal(|ui| {
                        self.account_avatar(ui, &acc.uuid, acc.name(), acc.offline, 44.0);
                        ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - buttons_w - 12.0).max(120.0));
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(acc.name()).strong().size(16.0));
                                tag(ui, acc.kind(), if acc.offline { c_warn() } else { c_accent() });
                                if acc.must_login {
                                    tag(ui, "Sign in again", c_danger());
                                }
                            });
                            small_muted(ui, &acc.uuid);
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            // removing asks once more: a Microsoft account has to be signed in again afterwards
                            if self.acct.confirm_remove.as_deref() == Some(acc.uuid.as_str()) {
                                if secondary(ui, "Cancel").clicked() {
                                    self.acct.confirm_remove = None;
                                }
                                if danger(ui, "Yes, remove").clicked() {
                                    remove = Some(acc.uuid.clone());
                                }
                                used = ui.min_rect().width();
                                return;
                            }
                            if secondary(ui, "Remove").clicked() {
                                self.acct.confirm_remove = Some(acc.uuid.clone());
                            }
                            if secondary(ui, "Change skin").clicked() {
                                change_skin = Some(acc.uuid.clone());
                            }
                            if current.as_ref() == Some(&acc.uuid) {
                                tag(ui, "Playing as", c_accent());
                            } else if primary(ui, "Use").clicked() {
                                select = Some(acc.uuid.clone());
                            }
                            used = ui.min_rect().width();
                        });
                    });
                    remember_width(ui, buttons_id, used);
                });
                ui.add_space(2.0);
            }
            if self.accounts.is_empty() {
                card(ui, |ui| muted(ui, "No accounts yet. Add one below."));
            }

            ui.add_space(8.0);
            section(ui, "Add an account");
            card(ui, |ui| {
                ui.label(RichText::new("Microsoft").strong());
                small_muted(ui, "Your browser opens a Microsoft page; enter the code shown here to sign in.");
                ui.add_space(4.0);
                match self.acct.device.clone() {
                    Some(code) => {
                        ui.label(RichText::new(&code.user_code).size(34.0).strong().monospace().color(c_accent()));
                        ui.horizontal(|ui| {
                            if secondary(ui, "Copy code").clicked() {
                                ui.ctx().copy_text(code.user_code.clone());
                            }
                            if secondary(ui, "Open the page again").clicked() {
                                let _ = open::that(&code.verification_uri);
                            }
                            if secondary(ui, "Cancel").clicked() {
                                self.acct.device = None;
                                self.acct.signing_in = false;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.spinner();
                            muted(ui, format!("Waiting for you to finish at {}", code.verification_uri));
                        });
                    }
                    None if self.acct.signing_in => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            muted(ui, "Asking Microsoft for a code");
                            if secondary(ui, "Cancel").clicked() {
                                self.acct.signing_in = false;
                            }
                        });
                    }
                    None => {
                        if primary(ui, "Sign in with Microsoft").clicked() {
                            self.start_sign_in();
                        }
                    }
                }
            });
            ui.add_space(4.0);
            card(ui, |ui| {
                ui.label(RichText::new("Offline player").strong());
                small_muted(ui, "Just a name. Works for single player and offline-mode servers.");
                ui.horizontal(|ui| {
                    let mut name = std::mem::take(&mut self.acct.offline_name);
                    let r = search_field(ui, &mut name, "Player name", 240.0);
                    // the same rule as everywhere else a name is typed (Minecraft's own: 3 to 16 characters)
                    let valid = super::whoplays::valid_player_name(name.trim());
                    let add = primary_enabled(ui, "Add", valid).clicked()
                        || (valid && r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if add {
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
                });
                small_muted(ui, "3\u{2013}16 letters, numbers or underscores.");
            });
        });
        if let Some(id) = change_skin {
            self.open_skin_dialog(&id);
        }
        if let Some(id) = select {
            self.settings.last_account = Some(id);
            self.save_settings();
        }
        if let Some(id) = remove {
            self.acct.confirm_remove = None;
            self.accounts.retain(|a| a.uuid != id);
            if self.settings.last_account.as_deref() == Some(&id) {
                self.settings.last_account = None;
            }
            self.save_accounts();
            self.save_settings();
        }
    }
}
