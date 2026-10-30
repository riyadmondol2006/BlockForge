//! "Who's playing?", shown when the launcher opens (if the setting is on): same steps and wording as the Java edition.

use egui::{RichText, Sense, Ui};

use super::icons::{self, Icon};
use super::motion;
use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::accounts::Account;

#[derive(Default)]
pub struct WhoState {
    name_step: bool,
    name: String,
    error: bool,
    /// For the screenshot tour: the window as it is the first time, with no account saved yet.
    first_run: bool,
}

impl WhoState {
    /// Straight at the "Play offline" step (for the screenshot tour).
    pub fn offline_step() -> WhoState {
        WhoState { name_step: true, ..Default::default() }
    }

    /// The window as the first start shows it, whatever accounts there are (for the screenshot tour).
    pub fn first_run() -> WhoState {
        WhoState { first_run: true, ..Default::default() }
    }
}

/// The accounts the window offers, the one that played last (`current`) first and then the ones added last, three at
/// most: indexes into `uuids`, the saved accounts in the order they are saved in.
fn offered(uuids: &[&str], current: Option<&str>) -> Vec<usize> {
    let last = current.and_then(|c| uuids.iter().position(|u| *u == c));
    let rest = (0..uuids.len()).rev().filter(|i| Some(*i) != last);
    last.into_iter().chain(rest).take(3).collect()
}

/// A way to add another account under the saved ones: a quiet outlined tile, its icon beside its name.
fn small_choice(ui: &mut Ui, width: f32, icon: Icon, title: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 44.0), Sense::click());
    let t = motion::hover(ui, resp.id, resp.hovered());
    let p = ui.painter();
    let radius = egui::CornerRadius::same(R_CONTROL);
    p.rect(
        rect,
        radius,
        motion::mix(c_field(), c_field_hover(), t),
        egui::Stroke::new(1.0, motion::mix(c_control(), c_accent(), t * 0.6)),
        egui::StrokeKind::Inside,
    );
    let ink = if is_dark() { c_accent() } else { c_accent_hover() };
    icons::paint(p, icon, egui::pos2(rect.left() + 22.0, rect.center().y), 16.0, ink);
    p.text(
        egui::pos2(rect.left() + 40.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        super::fonts::medium(FS_BODY),
        c_text(),
    );
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// 3-16 letters, numbers or underscores.
pub fn valid_player_name(n: &str) -> bool {
    crate::accounts::valid_offline_name(n)
}

/// What is wrong with a typed player name, said under the field (the rule itself is in the hint above it). `None`
/// for a name that can be used.
pub fn name_problem(n: &str) -> Option<&'static str> {
    if valid_player_name(n) {
        None
    } else if n.is_empty() {
        Some("Type a name first.")
    } else if !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some("Only letters, numbers and underscores (_) can be used.")
    } else if n.len() < 3 {
        Some("Too short: a name has at least 3 characters.")
    } else {
        Some("Too long: a name has at most 16 characters.")
    }
}

/// One of the two ways in. The Microsoft one is the main choice, in the brand gradient with a soft glow under it like
/// every main action; the offline one is a plain panel of glass. Both lift a little under the pointer.
fn choice(ui: &mut Ui, width: f32, icon: Icon, title: &str, sub: &str, main: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 104.0), Sense::click());
    let t = motion::hover(ui, resp.id, resp.hovered());
    let press = motion::toward(ui.ctx(), resp.id.with("press"), resp.is_pointer_button_down_on(), 0.08);
    let card = rect.translate(egui::vec2(0.0, -2.0 * t)).shrink(press);
    let radius = egui::CornerRadius::same(R_CARD);
    let p = ui.painter();
    let (fg, sub_fg) = if main {
        let g = c_accent();
        let alpha = if is_dark() { 60.0 + 50.0 * t } else { 40.0 + 30.0 * t };
        let shadow = egui::Shadow {
            offset: [0, 8],
            blur: (18.0 + 10.0 * t) as u8,
            spread: 0,
            color: egui::Color32::from_rgba_unmultiplied(g.r(), g.g(), g.b(), alpha as u8),
        };
        p.add(shadow.as_shape(card, radius));
        let (a, b) =
            (motion::mix(c_accent(), c_accent_hover(), t * 0.5), motion::mix(c_accent2(), c_accent_hover(), t * 0.25));
        motion::gradient_rect(p, card, R_CARD, a, b);
        p.hline(
            card.left() + R_CARD as f32..=card.right() - R_CARD as f32,
            card.top() + 0.5,
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(60)),
        );
        (c_on_accent(), c_on_accent().gamma_multiply(0.8))
    } else {
        let s = card_shadow();
        let shadow = egui::Shadow { color: s.color.gamma_multiply(t), ..card_shadow_raised() };
        if t > 0.0 {
            p.add(shadow.as_shape(card, radius));
        }
        p.rect(
            card,
            radius,
            motion::mix(c_field(), c_field_hover(), t),
            egui::Stroke::new(1.0, motion::mix(c_border(), c_control(), t)),
            egui::StrokeKind::Inside,
        );
        glass_edge(p, card, R_CARD);
        (c_text(), c_muted())
    };
    // the icon in a round of its own, a shade off the card
    let disc = card.left_top() + egui::vec2(36.0, 32.0);
    let disc_fill = if main { egui::Color32::from_white_alpha(56) } else { c_accent_soft() };
    p.circle_filled(disc, 17.0, disc_fill);
    let icon_fg = if main {
        fg
    } else if is_dark() {
        c_accent()
    } else {
        c_accent_hover()
    };
    icons::paint(p, icon, disc, 17.0, icon_fg);
    p.text(
        card.left_top() + egui::vec2(20.0, 68.0),
        egui::Align2::LEFT_CENTER,
        title,
        super::fonts::semibold(FS_BODY + 1.0),
        fg,
    );
    p.text(
        card.left_top() + egui::vec2(20.0, 87.0),
        egui::Align2::LEFT_CENTER,
        sub,
        egui::FontId::proportional(FS_META),
        sub_fg,
    );
    focus_ring(ui, &resp, rect, R_CARD);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// What is behind one of the accounts under "Continue as": a row of glass whose fill fades in under the pointer `t`,
/// with an arrow at its end that moves out a little. The account that played last (`main`) is the clear next step:
/// on the brand tint with a brand outline.
fn account_row_fill(rect: egui::Rect, t: f32, main: bool) -> Vec<egui::Shape> {
    let radius = egui::CornerRadius::same(R_CONTROL);
    let (fill, line) = if main {
        (motion::mix(c_accent_soft(), c_accent_soft().gamma_multiply(1.4), t), c_accent().gamma_multiply(0.7 + 0.3 * t))
    } else {
        (motion::mix(c_field(), c_field_hover(), t), motion::mix(c_border(), c_control(), t))
    };
    let mut shapes = vec![egui::Shape::rect_filled(rect, radius, fill)];
    shapes.push(egui::Shape::rect_stroke(
        rect,
        radius,
        egui::Stroke::new(if main { 1.5 } else { 1.0 }, line),
        egui::StrokeKind::Inside,
    ));
    let c = egui::pos2(rect.right() - 20.0 + 3.0 * t, rect.center().y);
    let stroke = egui::Stroke::new(1.6, motion::mix(c_faint(), c_text(), t));
    shapes.push(egui::Shape::line(
        vec![c + egui::vec2(-3.0, -5.5), c + egui::vec2(2.5, 0.0), c + egui::vec2(-3.0, 5.5)],
        stroke,
    ));
    shapes
}

impl App {
    pub(super) fn who_ui(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.who.take() else { return };
        let mut close = false;
        let mut sign_in = false;
        let mut pick: Option<String> = None;
        let mut offline: Option<String> = None;
        let mut manage = false;
        // the window lists the account that played last and the ones added last, three at most; the others are on
        // the Accounts page. The first start (or the tour's picture of it) has none.
        let saved = if st.first_run { &[][..] } else { &self.accounts[..] };
        let hidden = saved.len().saturating_sub(3);
        let current = self.current_account().map(|a| a.uuid.clone());
        let uuids: Vec<&str> = saved.iter().map(|a| a.uuid.as_str()).collect();
        let accounts: Vec<(String, String, &'static str, bool)> = offered(&uuids, current.as_deref())
            .into_iter()
            .map(|i| &saved[i])
            .map(|a| (a.uuid.clone(), a.name().to_string(), a.kind(), a.offline))
            .collect();
        let modal = egui::Modal::new(egui::Id::unique("who")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(super::services::modal_width(ctx, 480.0));
            if !st.name_step {
                ui.label(RichText::new("Who's playing?").size(FS_FIGURE).semibold());
                ui.add_space(14.0);
                // the two ways side by side, or one above the other when the window is too narrow for both
                let side_by_side = ui.available_width() >= 2.0 * 170.0 + 12.0;
                let w = if side_by_side { ((ui.available_width() - 12.0) / 2.0).floor() } else { ui.available_width() };
                let layout = if side_by_side {
                    egui::Layout::left_to_right(egui::Align::Min)
                } else {
                    egui::Layout::top_down(egui::Align::Min)
                };
                if accounts.is_empty() {
                    // the first start: the two ways in are the choice to make, large
                    ui.with_layout(layout, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
                        if choice(ui, w, Icon::User, "Sign in with Microsoft", "Use your Microsoft account", true) {
                            sign_in = true;
                        }
                        if choice(ui, w, Icon::Play, "Play offline", "Pick any player name", false) {
                            st.name_step = true;
                        }
                    });
                } else {
                    // a return: carrying on as someone saved is the next step, the one that played last first and
                    // focused (Enter continues); adding someone is the smaller choice under it
                    ui.label(RichText::new("Continue as").size(FS_META).color(c_faint()));
                    ui.add_space(4.0);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    for (n, (uuid, name, kind, offline)) in accounts.iter().enumerate() {
                        let main = n == 0;
                        // the row's fill is put in behind its contents once it is known whether it is pointed at
                        let fill = ui.painter().add(egui::Shape::Noop);
                        let row = egui::Frame::new()
                            .corner_radius(egui::CornerRadius::same(R_CONTROL))
                            .inner_margin(if main {
                                egui::Margin::symmetric(14, 12)
                            } else {
                                egui::Margin::symmetric(12, 8)
                            })
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 12.0;
                                    // the player's face, like on the sidebar and the Accounts page
                                    self.account_avatar(ui, uuid, name, *offline, if main { 40.0 } else { 30.0 });
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 0.0;
                                        let size = if main { FS_BODY + 2.0 } else { FS_BODY };
                                        ui.label(RichText::new(name).size(size).semibold());
                                        let sub = if main {
                                            format!("{kind} \u{00B7} played last")
                                        } else {
                                            kind.to_string()
                                        };
                                        small_muted(ui, sub);
                                    });
                                });
                            });
                        let resp =
                            row.response.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                        resp.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Continue as {name}, {kind}"))
                        });
                        if main {
                            focus_when_shown(ui, &resp);
                        }
                        let t = motion::hover(ui, resp.id, resp.hovered());
                        ui.painter().set(fill, egui::Shape::Vec(account_row_fill(resp.rect, t, main)));
                        focus_ring(ui, &resp, resp.rect, R_CONTROL);
                        if resp.clicked() {
                            pick = Some(uuid.clone());
                        }
                    }
                    ui.add_space(14.0);
                    ui.label(RichText::new("Add another").size(FS_META).color(c_faint()));
                    ui.add_space(4.0);
                    ui.with_layout(layout, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(12.0, 8.0);
                        if small_choice(ui, w, Icon::User, "Sign in with Microsoft") {
                            sign_in = true;
                        }
                        if small_choice(ui, w, Icon::Play, "Play offline") {
                            st.name_step = true;
                        }
                    });
                }
                // the Accounts page has every account, the skins and the other ways to add one
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ghost(ui, "Manage accounts").clicked() {
                        manage = true;
                    }
                    if hidden > 0 {
                        let more = if hidden == 1 {
                            "1 more account there".to_string()
                        } else {
                            format!("{hidden} more accounts there")
                        };
                        small_muted(ui, more);
                    }
                });
            } else {
                ui.label(RichText::new("Play offline").size(FS_FIGURE).semibold());
                ui.add_space(2.0);
                muted(ui, "3\u{2013}16 letters, numbers or underscores. No account needed.");
                ui.add_space(14.0);
                let r = ui.add_sized(
                    [ui.available_width(), 32.0],
                    egui::TextEdit::singleline(&mut st.name)
                        .hint_text(RichText::new("Player name").color(c_faint()))
                        .margin(egui::Margin::symmetric(10, 7))
                        .background_color(c_field()),
                );
                // typing can start at once
                focus_when_shown(ui, &r);
                // the rule is said as soon as what is typed breaks it, and when Continue is pressed on nothing
                let typed = st.name.trim();
                if let Some(problem) = name_problem(typed).filter(|_| !typed.is_empty() || st.error) {
                    ui.label(RichText::new(problem).size(FS_META).color(c_danger()));
                }
                ui.add_space(14.0);
                // the buttons at the right, the main one last, like every other window
                super::discover::dialog_footer(
                    ui,
                    "who-offline",
                    |_| {},
                    |ui| {
                        if secondary(ui, "Back").clicked() {
                            st.name_step = false;
                            st.error = false;
                        }
                        let go = primary(ui, "Continue").clicked()
                            || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                        if go {
                            if valid_player_name(st.name.trim()) {
                                offline = Some(st.name.trim().to_string());
                            } else {
                                st.error = true;
                            }
                        }
                    },
                );
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
        if manage {
            self.page = Page::Accounts;
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
        self.acct.last_error = None;
        self.acct.attempt += 1;
        let (attempt, n) = (self.acct.attempt, self.notifier());
        self.rt.spawn(async move {
            n.send(Msg::Device(attempt, crate::msauth::start_device_login().await.map_err(|e| sign_in_error(&e))));
        });
    }
}

/// A failed sign-in as the player reads it: a connection problem in plain words, anything else as Microsoft said it.
pub(super) fn sign_in_error(e: &anyhow::Error) -> String {
    if crate::msauth::is_offline_error(e) {
        "Couldn't reach Microsoft. Check your internet connection, then try again.".to_string()
    } else {
        format!("{e:#}")
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn the_account_that_played_last_is_offered_first_then_the_newest() {
        let uuids = ["a", "b", "c", "d", "e"];
        assert_eq!(offered(&uuids, Some("b")), [1, 4, 3], "b played last, then the newest");
        assert_eq!(offered(&uuids, Some("e")), [4, 3, 2]);
        assert_eq!(offered(&uuids, None), [4, 3, 2], "nobody chosen: the newest");
        assert_eq!(offered(&uuids, Some("gone")), [4, 3, 2]);
        assert_eq!(offered(&["a"], Some("a")), [0]);
        assert!(offered(&[], None).is_empty());
    }

    #[test]
    fn a_bad_name_is_told_what_is_wrong_with_it() {
        assert_eq!(name_problem("Steve"), None);
        assert_eq!(name_problem(""), Some("Type a name first."));
        assert!(name_problem("ab").unwrap().starts_with("Too short"));
        assert!(name_problem("a_name_far_too_long_x").unwrap().starts_with("Too long"));
        assert!(name_problem("Stéve").unwrap().starts_with("Only letters"));
        assert!(name_problem("no spaces").unwrap().starts_with("Only letters"));
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
