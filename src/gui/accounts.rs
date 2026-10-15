//! Accounts: Microsoft sign-in and offline players.

use egui::{Align, Layout, RichText, ScrollArea, Ui};

use super::theme::*;
use super::widgets::*;
use super::{App, Msg};
use crate::msauth::{self, DeviceCode};

#[derive(Default)]
pub struct AcctState {
    pub offline_name: String,
    pub device: Option<DeviceCode>,
    pub signing_in: bool,
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
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let accounts = self.accounts.clone();
            for acc in &accounts {
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        self.account_avatar(ui, &acc.uuid, acc.name(), acc.offline, 44.0);
                        ui.vertical(|ui| {
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
                            if secondary(ui, "Remove").clicked() {
                                remove = Some(acc.uuid.clone());
                            }
                            if secondary(ui, "Change skin").clicked() {
                                change_skin = Some(acc.uuid.clone());
                            }
                            if current.as_ref() == Some(&acc.uuid) {
                                tag(ui, "Playing as", c_accent());
                            } else if primary(ui, "Use").clicked() {
                                select = Some(acc.uuid.clone());
                            }
                        });
                    });
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
                        });
                    }
                    None => {
                        if primary(ui, "Sign in with Microsoft").clicked() {
                            self.acct.signing_in = true;
                            let n = self.notifier();
                            self.rt.spawn(async move {
                                n.send(Msg::Device(msauth::start_device_login().await.map_err(|e| format!("{e:#}"))));
                            });
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
                    let valid = !name.trim().is_empty()
                        && name.trim().len() <= 16
                        && name.trim().chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
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
                small_muted(ui, "1\u{2013}16 letters, digits or underscores.");
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
            self.accounts.retain(|a| a.uuid != id);
            if self.settings.last_account.as_deref() == Some(&id) {
                self.settings.last_account = None;
            }
            self.save_accounts();
            self.save_settings();
        }
    }
}
