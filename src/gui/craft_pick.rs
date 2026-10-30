//! The left column of the Crafting page: choose the item and the amount, and say what you already have.

use std::sync::Arc;

use egui::{Align, CornerRadius, FontId, Layout, RichText, Sense, Stroke, Ui, vec2};

use super::craft::{Book, CraftState, Dialog, Entry};
use super::craft_fmt::*;
use super::theme::*;
use super::widgets::*;
use crate::craft::{Category, ItemId};

/// The height of a row of the item list, and the room between two rows.
const ROW_H: f32 = 46.0;
const ROW_GAP: f32 = 2.0;
/// The amounts offered as chips beside "How many?".
pub(super) const QUICK_AMOUNTS: [u64; 4] = [1, 4, 16, 64];

/// The entries matching `query` (and the category), best first; at most `limit`. An empty query matches all.
pub(super) fn search_entries(book: &Book, query: &str, cat: Option<Category>, limit: usize) -> Vec<u32> {
    // an id copied from the game or a wiki ("minecraft:oak_planks") finds its item too
    let q = query.trim().to_lowercase();
    let q = q.strip_prefix("minecraft:").unwrap_or(&q).to_string();
    let q_id = q.replace(' ', "_");
    let mut hits: Vec<(u8, u32)> = Vec::new();
    for (i, e) in book.entries.iter().enumerate() {
        if cat.is_some_and(|c| c != e.category) {
            continue;
        }
        let rank = if q.is_empty() || e.lname.starts_with(&q) {
            0
        } else if e.lname.split(' ').any(|w| w.starts_with(&q)) {
            1
        } else if e.lname.contains(&q) {
            2
        } else if e.lid.contains(&q_id) {
            3
        } else {
            continue;
        };
        hits.push((rank, i as u32));
        // an empty query is already in order; with a query, every hit is needed to rank them
        if q.is_empty() && hits.len() >= limit {
            break;
        }
    }
    hits.sort_by_key(|&(r, i)| (r, i));
    hits.into_iter().take(limit).map(|(_, i)| i).collect()
}

impl CraftState {
    /// The card that picks the item: the search, the category and the list of items, which takes the height it is
    /// given (`list_h`).
    pub(super) fn left_pick(&mut self, ui: &mut Ui, book: &Arc<Book>, list_h: f32) {
        panel(ui, |ui| {
            let field = search_line(ui, &mut self.search);
            if self.target.is_none() {
                // nothing is picked yet: typing can start at once
                focus_when_shown(ui, &field);
            }
            let fresh = self
                .filtered_for
                .as_ref()
                .is_some_and(|(s, c, g)| *s == self.search && *c == self.category && *g == self.rev);
            if !fresh {
                self.filtered = search_entries(book, &self.search, self.category, usize::MAX);
                self.filtered_for = Some((self.search.clone(), self.category, self.rev));
            }
            let n = self.filtered.len();
            // how many there are, and the category, on one quiet line under the search
            ui.horizontal(|ui| {
                ui.set_min_height(28.0);
                small_muted(
                    ui,
                    if self.search.trim().is_empty() && self.category.is_none() {
                        format!("{} items", thousands(n as u64))
                    } else if n == 1 {
                        "1 match".to_string()
                    } else {
                        format!("{} matches", thousands(n as u64))
                    },
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let shown = self.category.map_or("All categories", |c| c.label());
                    egui::ComboBox::from_id_salt("craft-category").selected_text(shown).width(132.0).show_ui(
                        ui,
                        |ui| {
                            if ui.selectable_label(self.category.is_none(), "All categories").clicked() {
                                self.category = None;
                            }
                            for c in Category::ALL {
                                if ui.selectable_label(self.category == Some(c), c.label()).clicked() {
                                    self.category = Some(c);
                                }
                            }
                        },
                    );
                });
            });
            let mut pick: Option<ItemId> = None;
            if n == 0 {
                ui.add_space(16.0);
                // a category can be what hides the item: say so, and offer to look in all of them
                let in_all = self.category.is_some() && !search_entries(book, &self.search, None, 1).is_empty();
                ui.vertical_centered(|ui| {
                    if in_all {
                        muted(ui, "Nothing in this category matches that.");
                        if ghost(ui, "Search all categories").clicked() {
                            self.category = None;
                        }
                    } else {
                        muted(ui, "Nothing matches that. Try fewer letters.");
                    }
                });
                ui.add_space(16.0);
            } else {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut list = egui::ScrollArea::vertical().id_salt("craft-items");
                // a plan opened from elsewhere: its item is brought into view, a third of the way down
                if std::mem::take(&mut self.reveal) {
                    let at = self
                        .target
                        .as_deref()
                        .and_then(|t| self.filtered.iter().position(|&k| book.entries[k as usize].id == t));
                    if let Some(k) = at {
                        list = list.vertical_scroll_offset((k as f32 * (ROW_H + ROW_GAP) - list_h / 3.0).max(0.0));
                    }
                }
                list.max_height(list_h).auto_shrink([false, false]).show_rows(ui, ROW_H + ROW_GAP, n, |ui, range| {
                    for k in range {
                        let e = &book.entries[self.filtered[k] as usize];
                        let selected = self.target.as_deref() == Some(e.id.as_str());
                        if self.item_row(ui, e, selected) {
                            pick = Some(e.id.clone());
                        }
                        ui.add_space(ROW_GAP);
                    }
                });
            }
            if let Some(id) = pick {
                self.set_target(&id);
            }
        });
    }

    /// One row of the item list: picture, name, and how it stacks under it. True when clicked.
    fn item_row(&mut self, ui: &mut Ui, e: &Entry, selected: bool) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
        if !ui.is_rect_visible(rect) {
            return false;
        }
        // the hover fill fades in; the chosen row is tinted with the brand colour and outlined in it
        let t = super::motion::hover(ui, resp.id, resp.hovered() && !selected);
        let s = super::motion::toward(ui.ctx(), resp.id.with("on"), selected, super::motion::FAST);
        let shape = CornerRadius::same(R_CONTROL);
        if t > 0.0 {
            ui.painter().rect_filled(rect, shape, c_hover().gamma_multiply(t));
        }
        if s > 0.0 {
            ui.painter().rect(
                rect,
                shape,
                c_accent_soft().gamma_multiply(s),
                Stroke::new(1.0, c_accent().gamma_multiply(0.55 * s)),
                egui::StrokeKind::Inside,
            );
        }
        let icon = egui::Rect::from_min_size(rect.left_top() + vec2(6.0, (ROW_H - 34.0) / 2.0), vec2(34.0, 34.0));
        self.paint_slot(ui, icon, &e.id, &e.name);
        let sub = if e.stack > 1 { format!("Stacks of {}", e.stack) } else { "Does not stack".to_string() };
        let clip = rect.shrink2(vec2(4.0, 0.0));
        let p = ui.painter().with_clip_rect(clip);
        let font = if selected { super::fonts::semibold(FS_BODY) } else { super::fonts::medium(FS_BODY) };
        p.text(rect.left_top() + vec2(50.0, 6.0), egui::Align2::LEFT_TOP, &e.name, font, c_text());
        // faint text never sits on a tinted fill
        let sub_color = if selected { c_muted() } else { c_faint() };
        p.text(
            rect.left_top() + vec2(50.0, 25.0),
            egui::Align2::LEFT_TOP,
            sub,
            FontId::proportional(FS_META),
            sub_color,
        );
        // the id is there for whoever looks for it, not in every row
        let short = e.id.rsplit(':').next().unwrap_or(&e.id).to_string();
        focus_ring(ui, &resp, rect, R_CONTROL);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &e.name));
        resp.on_hover_text(short).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    }

    /// What was picked and how many of it, in three short lines so the band at the head of the plan stays low: the
    /// item's slot, name and how it stacks; the amount with the quick amounts and, for what stacks, a menu of whole
    /// stacks and chests; and what the player's own items change (or what is wrong with the amount). `change` offers
    /// to pick another item (the one-column page folds the list away once something is picked).
    pub(super) fn quantity_block(&mut self, ui: &mut Ui, book: &Arc<Book>, change: bool) {
        let Some(target) = self.target.clone() else { return };
        let item = book.ds.item(&target);
        let name = item.map_or_else(|| book.ds.name(&target), |i| i.name.clone());
        let stack = item.map_or(64, |i| i.stack);
        let parsed = parse_qty(&self.qty_text);
        let current = parsed.as_ref().ok().copied();
        ui.horizontal(|ui| {
            ui.set_height(28.0);
            ui.spacing_mut().item_spacing.x = 10.0;
            self.slot_icon(ui, &target, &name, 28.0);
            if change {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ink = if is_dark() { c_accent() } else { c_accent_hover() };
                    if link_text(ui, "Change item", ink).on_hover_text("Pick another item").clicked() {
                        self.picking = true;
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        self.item_title(ui, &target, &name, stack)
                    });
                });
            } else {
                self.item_title(ui, &target, &name, stack);
            }
        });
        ui.horizontal(|ui| {
            ui.set_height(32.0);
            ui.spacing_mut().item_spacing.x = 4.0;
            let mut edit = egui::TextEdit::singleline(&mut self.qty_text)
                .id(egui::Id::unique("craft-qty"))
                .font(super::fonts::mono(FS_BODY))
                .margin(egui::Margin::symmetric(10, 7))
                .background_color(c_field());
            if parsed.is_err() {
                edit = edit.text_color(c_danger());
            }
            let note = current.map(|q| stack_note(q, stack)).filter(|n| !n.is_empty());
            let tip = match &note {
                Some(n) => format!("How many to make: {n}"),
                None => "How many to make: any whole number, such as 250 or 3,000".to_string(),
            };
            // the hint names the field for a screen reader too, and shows when it is empty
            let r = ui.add_sized([80.0, 32.0], edit.hint_text("How many?")).on_hover_text(tip);
            if r.changed() {
                self.touch_plan();
            }
            ui.add_space(2.0);
            for v in QUICK_AMOUNTS {
                if amount_chip(ui, &v.to_string(), current == Some(v)).clicked() {
                    self.qty_text = thousands(v);
                    self.touch_plan();
                }
            }
            if stack > 1 {
                // whole stacks and chests, for building in bulk
                let st = u64::from(stack);
                let bulk = [
                    ("1 stack", st, "one stack"),
                    ("27 stacks", 27 * st, "fills a chest"),
                    ("54 stacks", 54 * st, "fills a double chest"),
                ];
                let on = bulk.iter().any(|(_, v, _)| current == Some(*v));
                let r = amount_chip(ui, "Stacks \u{25BE}", on).on_hover_text("Whole stacks and chests");
                egui::Popup::menu(&r).show(|ui| {
                    ui.set_min_width(220.0);
                    for (label, v, tip) in bulk {
                        let text = format!("{label} \u{00B7} {tip} \u{00B7} {}", thousands(v));
                        if ui.selectable_label(current == Some(v), text).clicked() {
                            self.qty_text = thousands(v);
                            self.touch_plan();
                            ui.close();
                        }
                    }
                });
            }
        });
        ui.horizontal(|ui| {
            ui.set_height(18.0);
            match &parsed {
                Err(e) => {
                    ui.add(egui::Label::new(RichText::new(e).color(c_danger()).size(FS_META)).truncate());
                }
                Ok(_) => self.stock_line(ui),
            }
        });
    }

    /// The item's name and how it stacks, on one line; the game's id is there for whoever looks for it, under the
    /// pointer.
    fn item_title(&mut self, ui: &mut Ui, target: &str, name: &str, stack: u32) {
        ui.spacing_mut().item_spacing.x = 8.0;
        let stacks = if stack > 1 { format!("Stacks of {stack}") } else { "Does not stack".to_string() };
        let w = ui.painter().layout_no_wrap(stacks.clone(), FontId::proportional(FS_META), c_muted()).size().x;
        ui.scope(|ui| {
            ui.set_max_width((ui.available_width() - w - 8.0).max(60.0));
            ui.add(egui::Label::new(RichText::new(name).size(FS_SECTION).semibold()).truncate())
                .on_hover_text(RichText::new(target).font(super::fonts::mono(FS_META)));
        });
        small_muted(ui, stacks);
    }

    /// What the player's own items do to the plan, and the way to them: "Your items · 52 applied · Edit". With
    /// nothing in them, only the way to add some.
    fn stock_line(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.x = 6.0;
        let kinds = self.stock().map_or(0, |s| s.len());
        let applied = self.applied_from_stock();
        let (text, tip) = if kinds == 0 {
            (None, "Say what is already in your chests, and the plan takes it off what to gather and make".to_string())
        } else if !self.use_stock {
            (
                Some("not used in the plan".to_string()),
                "Your items are kept, but the plan is worked out without them".to_string(),
            )
        } else if applied == 0 {
            (Some("none needed here".to_string()), "None of your items are needed for this plan".to_string())
        } else {
            (
                Some(format!("{} applied", thousands(applied))),
                format!(
                    "{} of your items are taken off the plan: the lists show what is still needed",
                    thousands(applied)
                ),
            )
        };
        let ink = if is_dark() { c_accent() } else { c_accent_hover() };
        match text {
            None => {
                if link_text(ui, "+ Items you already have", ink).on_hover_text(tip).clicked() {
                    self.dialog = Some(Dialog::Stock);
                }
            }
            Some(t) => {
                let on = self.use_stock && applied > 0;
                ui.label(RichText::new("Your items").size(FS_META).color(if on { c_text() } else { c_muted() }));
                ui.label(RichText::new("\u{00B7}").size(FS_META).color(c_faint()));
                ui.label(RichText::new(t).font(super::fonts::medium(FS_META)).color(if on { ink } else { c_muted() }))
                    .on_hover_text(tip);
                ui.label(RichText::new("\u{00B7}").size(FS_META).color(c_faint()));
                if link_text(ui, "Edit", ink).on_hover_text("Edit the items you already have").clicked() {
                    self.dialog = Some(Dialog::Stock);
                }
            }
        }
    }

    /// How many items the plan takes from the player's own, over all of them.
    pub(super) fn applied_from_stock(&self) -> u64 {
        match &self.plan {
            Some(Ok(v)) if self.use_stock => v.plan.ledger.iter().map(|r| r.from_stock).sum(),
            _ => 0,
        }
    }

    /// What the player already has, to edit: whether the plan uses it, a line per item and a form to add one. It is
    /// the body of the "Your items" window.
    pub(super) fn stock_editor(&mut self, ui: &mut Ui, book: &Arc<Book>) {
        let version = self.version.clone().unwrap_or_default();
        small_muted(ui, format!("Kept for Minecraft {version} and taken off what the plan asks for."));
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.set_min_height(32.0);
            ui.label("Use my items in the plan");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut on = self.use_stock;
                if toggle_named(ui, &mut on, "Use my items in the plan").changed() {
                    self.use_stock = on;
                    self.touch_plan();
                }
            });
        });
        ui.add_space(2.0);
        // the rows
        let ids: Vec<ItemId> = self.stock().map(|s| s.keys().cloned().collect()).unwrap_or_default();
        let mut remove: Option<ItemId> = None;
        for id in &ids {
            let name = book.ds.name(id);
            let value = self.stock().and_then(|s| s.get(id)).copied().unwrap_or(0);
            ui.horizontal(|ui| {
                self.slot_icon(ui, id, &name, 32.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if x_button(ui, "Remove") {
                        remove = Some(id.clone());
                    }
                    let buf = self.stock_bufs.entry(id.clone()).or_insert_with(|| value.to_string());
                    let bad = parse_qty(buf).is_err();
                    let mut edit = egui::TextEdit::singleline(buf)
                        .margin(egui::Margin::symmetric(8, 5))
                        .background_color(c_field());
                    if bad {
                        edit = edit.text_color(c_danger());
                    }
                    let r = ui.add_sized([88.0, 30.0], edit);
                    let typed = parse_qty(buf);
                    if r.changed() {
                        if let Ok(q) = typed {
                            self.set_stock(id, q);
                            self.touch_store();
                            if self.use_stock {
                                self.touch_plan();
                            }
                        }
                    } else if !r.has_focus() {
                        *self.stock_bufs.entry(id.clone()).or_default() = value.to_string();
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(name.as_str()).truncate());
                    });
                });
            });
        }
        if let Some(id) = remove {
            self.set_stock(&id, 0);
            self.stock_bufs.remove(&id);
            self.touch_store();
            if self.use_stock {
                self.touch_plan();
            }
        }
        if ids.is_empty() {
            small_muted(ui, "Nothing yet. Add what is already in your chests.");
        }
        ui.add_space(4.0);
        self.stock_adder(ui, book);
    }

    /// The small form that adds an item to the stock.
    fn stock_adder(&mut self, ui: &mut Ui, book: &Arc<Book>) {
        ui.add_sized(
            [ui.available_width(), 32.0],
            egui::TextEdit::singleline(&mut self.stock_search)
                .hint_text(RichText::new("Add an item you have").color(c_faint()))
                .margin(egui::Margin::symmetric(10, 7))
                .background_color(c_field()),
        );
        if self.stock_search != self.stock_hits_for {
            self.stock_hits = if self.stock_search.trim().is_empty() {
                Vec::new()
            } else {
                search_entries(book, &self.stock_search, None, 6)
            };
            self.stock_hits_for = self.stock_search.clone();
        }
        let mut chosen: Option<ItemId> = None;
        if !self.stock_search.trim().is_empty() {
            if self.stock_hits.is_empty() {
                small_muted(ui, "Nothing matches that.");
            }
            for k in 0..self.stock_hits.len() {
                let e = &book.entries[self.stock_hits[k] as usize];
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                let t = super::motion::hover(ui, resp.id, resp.hovered());
                if t > 0.0 {
                    ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), c_hover().gamma_multiply(t));
                }
                let icon = egui::Rect::from_min_size(rect.left_top() + vec2(4.0, 5.0), vec2(24.0, 24.0));
                self.paint_icon(ui, icon, &e.id, &e.name);
                ui.painter().with_clip_rect(rect).text(
                    rect.left_center() + vec2(36.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &e.name,
                    FontId::proportional(FS_BODY),
                    c_text(),
                );
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    chosen = Some(e.id.clone());
                }
            }
        }
        if let Some(id) = chosen {
            self.stock_pick = Some(id);
            self.stock_search.clear();
            self.stock_hits.clear();
            self.stock_hits_for.clear();
        }
        if let Some(id) = self.stock_pick.clone() {
            let name = book.ds.name(&id);
            let parsed = parse_qty(&self.stock_qty);
            let mut add = false;
            let mut cancel = false;
            ui.horizontal(|ui| {
                self.slot_icon(ui, &id, &name, 32.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if x_button(ui, "Cancel") {
                        cancel = true;
                    }
                    if primary_enabled(ui, "Add", parsed.is_ok()).clicked() {
                        add = true;
                    }
                    let mut edit = egui::TextEdit::singleline(&mut self.stock_qty)
                        .margin(egui::Margin::symmetric(8, 5))
                        .background_color(c_field());
                    if parsed.is_err() {
                        edit = edit.text_color(c_danger());
                    }
                    ui.add_sized([76.0, 30.0], edit);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(name.as_str()).truncate());
                    });
                });
            });
            if add {
                if let Ok(q) = parsed {
                    let have = self.stock().and_then(|s| s.get(&id)).copied().unwrap_or(0);
                    self.set_stock(&id, have.saturating_add(q));
                    self.stock_bufs.remove(&id);
                    self.stock_pick = None;
                    self.touch_store();
                    if self.use_stock {
                        self.touch_plan();
                    }
                }
            }
            if cancel {
                self.stock_pick = None;
            }
        }
    }

    /// The items the player will bring, as chips to take away again.
    pub(super) fn left_supplied(&mut self, ui: &mut Ui, book: &Arc<Book>) {
        if self.supplied.is_empty() {
            return;
        }
        panel(ui, |ui| {
            card_title(ui, "Items I will bring");
            small_muted(ui, "The plan stops at these and lists them to gather.");
            ui.add_space(2.0);
            let items: Vec<ItemId> = self.supplied.iter().cloned().collect();
            let mut remove: Option<ItemId> = None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for id in &items {
                    let name = book.ds.name(id);
                    egui::Frame::new()
                        .fill(c_field())
                        .stroke(Stroke::new(1.0, c_border()))
                        .corner_radius(CornerRadius::same(16))
                        .inner_margin(egui::Margin { left: 5, right: 3, top: 3, bottom: 3 })
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.horizontal(|ui| {
                                self.icon(ui, id, &name, 22.0);
                                ui.label(RichText::new(&name).size(FS_BODY));
                                if x_button(ui, "Take away") {
                                    remove = Some(id.clone());
                                }
                            });
                        });
                }
            });
            if let Some(id) = remove {
                self.supplied.remove(&id);
                self.touch_plan();
            }
        });
    }
}

/// The search over the item list: a magnifier at its left, and the "/" that reaches it named in its hint.
fn search_line(ui: &mut Ui, text: &mut String) -> egui::Response {
    let r = ui.add_sized(
        [ui.available_width(), 36.0],
        egui::TextEdit::singleline(text)
            .id(egui::Id::unique("craft-search"))
            .hint_text(RichText::new("Search items   /").color(c_faint()))
            .margin(egui::Margin { left: 34, right: 10, top: 8, bottom: 8 })
            .background_color(c_field()),
    );
    let fg = if r.has_focus() { c_text() } else { c_muted() };
    super::icons::paint(
        ui.painter(),
        super::icons::Icon::Search,
        egui::pos2(r.rect.left() + 18.0, r.rect.center().y),
        15.0,
        fg,
    );
    r
}

/// A quick amount beside "How many?": a small square-cornered chip with the number in the monospace face, tinted
/// with the brand colour when it is the amount asked for.
fn amount_chip(ui: &mut Ui, text: &str, selected: bool) -> egui::Response {
    let font = super::fonts::mono(FS_BODY - 1.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, egui::Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2((galley.size().x + 18.0).max(34.0), 32.0), Sense::click());
    let t = super::motion::hover(ui, resp.id, resp.hovered());
    let s = super::motion::toward(ui.ctx(), resp.id.with("on"), selected, super::motion::FAST);
    let bg = super::motion::mix(super::motion::mix(c_field(), c_field_hover(), t), c_accent_soft(), s);
    let line = super::motion::mix(c_border(), c_accent().gamma_multiply(0.7), s);
    let ink = if is_dark() { c_accent() } else { c_accent_hover() };
    let fg = super::motion::mix(c_text(), ink, s);
    ui.painter().rect(rect, CornerRadius::same(R_CONTROL), bg, Stroke::new(1.0, line), egui::StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small text that acts like a link: underlined under the pointer, focusable, with the pointing hand.
fn link_text(ui: &mut Ui, text: &str, color: egui::Color32) -> egui::Response {
    let font = super::fonts::medium(FS_META);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(0.0, 2.0), Sense::click());
    let at = egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
    if resp.hovered() || resp.has_focus() {
        let y = at.y + galley.size().y;
        ui.painter().hline(rect.left()..=rect.right(), y, Stroke::new(1.0, color));
    }
    ui.painter().galley(at, galley, color);
    focus_ring(ui, &resp, rect.expand(2.0), R_TAG);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
