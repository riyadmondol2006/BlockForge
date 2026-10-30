//! Accounts: Microsoft sign-in and offline players.

use egui::{Align, Color32, CornerRadius, Layout, RichText, ScrollArea, Stroke, Ui};

use super::App;
use super::icons::Icon;
use super::motion;
use super::theme::*;
use super::widgets::*;
use crate::msauth::DeviceCode;

/// The widest the list of accounts gets.
const COLUMN: f32 = 720.0;
/// The column of ways to add an account, beside the list on a wide page.
const ADD_W: f32 = 380.0;
/// The room between the list and that column (the list's scroll bar sits in it).
const ADD_GAP: f32 = 28.0;
/// The narrowest the list gets with the column beside it; on a narrower page the column goes under the list.
const LIST_MIN_BESIDE: f32 = 520.0;

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
        self.account_corner(
            ui,
            super::Page::Accounts,
            "Accounts",
            "Choose who plays. Microsoft accounts can join online servers; offline players can't.",
            COLUMN,
            |app, ui| app.accounts_body(ui),
        );
    }

    /// The accounts and the ways to add one, beside the list of the account corner.
    fn accounts_body(&mut self, ui: &mut Ui) {
        let mut acts = Acts::default();
        let current = self.current_account().map(|a| a.uuid.clone());
        let area = ui.available_rect_before_wrap();
        // a wide page has the accounts and the ways to add one side by side; a narrower one has them one above
        // the other
        if area.width() >= LIST_MIN_BESIDE + ADD_GAP + ADD_W {
            let list_w = (area.width() - ADD_W - ADD_GAP).min(COLUMN);
            let list_rect = egui::Rect::from_min_size(area.min, egui::vec2(list_w + 14.0, area.height()));
            let add_rect = egui::Rect::from_min_size(
                egui::pos2(area.left() + list_w + ADD_GAP, area.top()),
                egui::vec2(ADD_W + 14.0, area.height()),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
                ScrollArea::vertical().id_salt("accounts").auto_shrink(false).show(ui, |ui| {
                    ui.set_max_width(list_w);
                    self.account_list(ui, current.as_deref(), &mut acts);
                    ui.add_space(24.0);
                });
            });
            ui.scope_builder(egui::UiBuilder::new().max_rect(add_rect), |ui| {
                ScrollArea::vertical().id_salt("accounts-add").auto_shrink(false).show(ui, |ui| {
                    ui.set_max_width(ADD_W);
                    column_head(ui, Icon::Plus, "Add an account", "Microsoft, or a name for offline games");
                    self.add_account_cards(ui);
                    ui.add_space(24.0);
                });
            });
            ui.advance_cursor_after_rect(area);
        } else {
            ScrollArea::vertical().id_salt("accounts").auto_shrink(false).show(ui, |ui| {
                // at most 720 wide, but never wider than the window
                ui.set_max_width((ui.available_width() - 14.0).min(COLUMN));
                self.account_list(ui, current.as_deref(), &mut acts);
                ui.add_space(26.0);
                column_head(
                    ui,
                    Icon::Plus,
                    "Add an account",
                    "Sign in with Microsoft, or add a player for offline games",
                );
                self.add_account_cards(ui);
                ui.add_space(24.0);
            });
        }
        let Acts { select, remove, change_skin, copied, sign_in } = acts;
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

    /// The accounts there are, one card each, under a head that counts them.
    fn account_list(&mut self, ui: &mut Ui, current: Option<&str>, acts: &mut Acts) {
        let current = current.map(str::to_string);
        let n = self.accounts.len();
        let hint = match n {
            0 => "None yet".to_string(),
            1 => "1 account. The one outlined plays".to_string(),
            n => format!("{n} accounts. The one outlined plays; Use switches to another"),
        };
        column_head(ui, Icon::User, "Your accounts", &hint);
        let accounts = self.accounts.clone();
        if accounts.is_empty() {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    super::settings::icon_tile(ui, Icon::User, 40.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new("No accounts yet").semibold());
                        small_muted(ui, "Sign in with Microsoft, or add a name for offline games.");
                    });
                });
            });
        } else {
            ui.spacing_mut().item_spacing.y = 10.0;
            for acc in &accounts {
                let playing = current.as_ref() == Some(&acc.uuid);
                account_card(ui, egui::Id::unique(("account-card", &acc.uuid)), playing, |ui| {
                    // the buttons keep their room on the right (as wide as on the last frame)
                    let buttons_id = ui.scope_id().with(("account-buttons", &acc.uuid));
                    let buttons_w = remembered_width(ui, buttons_id, 260.0);
                    let mut used = buttons_w;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;
                        let face = self.account_avatar(ui, &acc.uuid, acc.name(), acc.offline, 44.0);
                        // the one playing has a ring in the brand colour around the face
                        let on = motion::toward(
                            ui.ctx(),
                            egui::Id::unique(("account-ring", &acc.uuid)),
                            playing,
                            motion::MEDIUM,
                        );
                        if on > 0.0 && face.is_positive() {
                            ui.painter().rect_stroke(
                                face.expand(3.0),
                                CornerRadius::same(((face.width() * 0.24).round() as u8).saturating_add(3)),
                                Stroke::new(2.0, c_accent().gamma_multiply(on)),
                                egui::StrokeKind::Outside,
                            );
                        }
                        ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - buttons_w - 12.0).max(120.0));
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(acc.name()).size(FS_SECTION - 1.0).semibold());
                                if playing {
                                    brand_tag(ui, "Playing as");
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
                                    acts.remove = Some(acc.uuid.clone());
                                }
                                if secondary(ui, "Cancel").clicked() {
                                    self.acct.confirm_remove = None;
                                }
                                used = ui.min_rect().width();
                                return;
                            }
                            kebab_menu(ui, |ui| {
                                if acc.must_login && !acc.offline && ui.button("Sign in again").clicked() {
                                    acts.sign_in = true;
                                    ui.close();
                                }
                                if ui.button("Copy UUID").clicked() {
                                    ui.ctx().copy_text(acc.uuid.clone());
                                    acts.copied = true;
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("Remove").clicked() {
                                    self.acct.confirm_remove = Some(acc.uuid.clone());
                                    ui.close();
                                }
                            });
                            if secondary(ui, "Change skin").clicked() {
                                acts.change_skin = Some(acc.uuid.clone());
                            }
                            if acc.must_login && !acc.offline {
                                if primary(ui, "Sign in again").clicked() {
                                    acts.sign_in = true;
                                }
                            } else if current.as_ref() != Some(&acc.uuid) && secondary(ui, "Use").clicked() {
                                acts.select = Some(acc.uuid.clone());
                            }
                            used = ui.min_rect().width();
                        });
                    });
                    remember_width(ui, buttons_id, used);
                });
            }
        }
    }

    /// The ways to add an account, a card each: signing in with Microsoft (with the code to type once it is asked
    /// for), and an offline player's name.
    fn add_account_cards(&mut self, ui: &mut Ui) {
        card(ui, |ui| {
            method_row(
                ui,
                Icon::User,
                "Microsoft",
                "Your browser opens a Microsoft page; enter the code shown here to sign in.",
                220.0,
                |ui| {
                    if self.acct.device.is_none()
                        && !self.acct.signing_in
                        && primary(ui, "Sign in with Microsoft").clicked()
                    {
                        self.start_sign_in();
                    }
                },
            );
            // what follows the sign-in lines up with the text of its row, not with the icon (and leaves no room
            // when there is nothing to show)
            let more = self.acct.last_error.is_some() || self.acct.device.is_some() || self.acct.signing_in;
            if more {
                under_head(ui, |ui| {
                    if let Some(e) = self.acct.last_error.clone() {
                        ui.add_space(8.0);
                        if notice(ui, NoticeKind::Error, &e, Some("Try again")).is_some_and(|r| r.clicked()) {
                            self.start_sign_in();
                        }
                    }
                    match self.acct.device.clone() {
                        Some(code) => {
                            ui.add_space(10.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 12.0;
                                // the code, large and in figures of equal width, so it can be read out and typed
                                // on the brand tint with a brand outline: the one thing on the page to look at now
                                egui::Frame::new()
                                    .fill(c_accent_soft())
                                    .stroke(Stroke::new(1.0, c_accent().gamma_multiply(0.6)))
                                    .corner_radius(CornerRadius::same(R_CONTROL))
                                    .inner_margin(egui::Margin::symmetric(18, 8))
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(&code.user_code).size(FS_TITLE).semibold().monospace());
                                    });
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 6.0;
                                    ui.add_space(6.0);
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
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                spinner_sized(ui, 18.0);
                                let text = format!(
                                    "The code is on your clipboard. Paste it at {} and finish there.",
                                    code.verification_uri
                                );
                                ui.add(egui::Label::new(RichText::new(text).color(c_muted())).wrap());
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
                });
            }
        });
        ui.add_space(10.0);
        card(ui, |ui| {
            method_row(
                ui,
                Icon::Play,
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
                            // the field gives up some width in a narrow column, so Add stays beside it
                            let width = (ui.available_width() - 72.0).clamp(150.0, 240.0);
                            r = Some(search_field(ui, &mut name, "Player name", width));
                        } else {
                            add_clicked = secondary_enabled(ui, "Add", valid).clicked();
                        }
                    }
                    // the same rule as everywhere else a name is typed (Minecraft's own: 3 to 16 characters)
                    let valid = super::whoplays::valid_player_name(name.trim());
                    if let Some(problem) =
                        super::whoplays::name_problem(name.trim()).filter(|_| !name.trim().is_empty())
                    {
                        ui.label(RichText::new(problem).size(FS_META).color(c_danger()));
                    }
                    let add = add_clicked
                        || (valid
                            && r.is_some_and(|r| r.lost_focus())
                            && ui.input(|i| i.key_pressed(egui::Key::Enter)));
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
    }
}

/// What was pressed on the accounts page, done once the page is drawn.
#[derive(Default)]
struct Acts {
    select: Option<String>,
    remove: Option<String>,
    change_skin: Option<String>,
    copied: bool,
    sign_in: bool,
}

/// The head of a part of the page: its icon in a tile, the title, and one muted line about it.
fn column_head(ui: &mut Ui, icon: Icon, title: &str, hint: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        super::settings::icon_tile(ui, icon, 34.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(title).size(FS_SECTION).semibold());
            ui.add(egui::Label::new(RichText::new(hint).size(FS_META).color(c_muted())).truncate());
        });
    });
    ui.add_space(10.0);
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

/// A small label in the brand gradient: what is true of this one row (the account that is playing).
fn brand_tag(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::semibold(FS_META), c_on_accent());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(galley.size().x + 16.0, 22.0), egui::Sense::hover());
    motion::brand_rect(ui.painter(), rect, 11);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, c_on_accent());
}

/// The card of one account. It rises a little under the pointer, and the account that is playing is outlined in the
/// brand colour with a faint glow of it, so which one is in use is seen at a glance.
fn account_card(ui: &mut Ui, id: egui::Id, playing: bool, add: impl FnOnce(&mut Ui)) {
    let was_hot = ui.data(|d| d.get_temp::<bool>(id.with("hot"))).unwrap_or(false);
    let t = motion::hover(ui, id, was_hot);
    let on = motion::toward(ui.ctx(), id.with("playing"), playing, motion::MEDIUM);
    let (a, b) = (card_shadow(), card_shadow_raised());
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    let mut shadow = egui::Shadow {
        offset: [0, lerp(a.offset[1] as f32, b.offset[1] as f32).round() as i8],
        blur: lerp(a.blur as f32, b.blur as f32).round() as u8,
        spread: 0,
        color: motion::mix(a.color, b.color, t),
    };
    if on > 0.0 {
        // the glow of the one that plays: the shadow takes on the brand colour
        let g = c_accent();
        let alpha = (if is_dark() { 46.0 } else { 34.0 }) * on;
        shadow.color = motion::mix(shadow.color, Color32::from_rgba_unmultiplied(g.r(), g.g(), g.b(), alpha as u8), on);
    }
    let line =
        motion::mix(motion::mix(c_border(), c_control().gamma_multiply(0.7), t), c_accent().gamma_multiply(0.7), on);
    let out = egui::Frame::new()
        .fill(motion::mix(c_card(), c_raised(), t))
        .stroke(Stroke::new(1.0, line))
        .corner_radius(CornerRadius::same(R_CARD))
        .inner_margin(egui::Margin::symmetric(18, 14))
        .shadow(shadow)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    let rect = out.response.rect;
    glass_edge(ui.painter(), rect, R_CARD);
    let hot = ui.rect_contains_pointer(rect);
    ui.data_mut(|d| d.insert_temp(id.with("hot"), hot));
}

/// The part of a card under a [`method_row`], lined up with the row's text rather than its icon.
fn under_head<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        ui.visuals_mut().indent_has_left_vline = false;
        ui.spacing_mut().indent = 54.0;
        ui.indent("under-head", add).inner
    })
    .inner
}

/// The head of one way of adding an account: its icon in a tile beside a setting row.
fn method_row(ui: &mut Ui, icon: Icon, title: &str, hint: &str, control_width: f32, control: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        super::settings::icon_tile(ui, icon, 40.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            super::settings::wide_row(ui, title, hint, control_width, control);
        });
    });
}
