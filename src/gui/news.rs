//! The launcher news on the home page, and the window that shows one article in full.

use egui::{Context, CornerRadius, RichText, ScrollArea, Sense, Stroke, Ui};

use super::services::{SvcMsg, modal_scroll_height, modal_width};
use super::theme::*;
use super::widgets::*;
use super::{App, Msg};
use crate::news::{NewsItem, Span};

/// How many articles the home page lists (the Java edition shows six).
const SHOWN: usize = 6;

#[derive(Default)]
pub enum Status {
    #[default]
    Idle,
    Loading,
    Loaded,
    Failed(String),
}

/// One article as the page shows it, its text worked out once when the news arrives (not on every frame).
struct Shown {
    title: String,
    date: String,
    preview: String,
    spans: Vec<Span>,
}

impl Shown {
    fn of(item: &NewsItem) -> Shown {
        Shown { title: item.title.clone(), date: item.date(), preview: preview(item), spans: item.spans() }
    }
}

#[derive(Default)]
pub struct NewsState {
    items: Vec<Shown>,
    status: Status,
    /// The article open in its own window.
    open: Option<usize>,
}

impl NewsState {
    pub fn arrived(&mut self, result: Result<Vec<NewsItem>, String>) {
        match result {
            Ok(items) => {
                self.items = items.iter().take(SHOWN).map(Shown::of).collect();
                // the list changed under an open article: close it rather than show another one
                self.open = None;
                self.status = Status::Loaded;
            }
            Err(e) => self.status = Status::Failed(e),
        }
    }
}

/// Draw spans as wrapped lines: bold, italic and links keep their style, and `\n` starts a new line.
pub fn render_spans(ui: &mut Ui, spans: &[Span]) {
    let mut lines: Vec<Vec<(&str, &Span)>> = vec![Vec::new()];
    for s in spans {
        for (n, part) in s.text.split('\n').enumerate() {
            if n > 0 {
                lines.push(Vec::new());
            }
            if let (false, Some(line)) = (part.is_empty(), lines.last_mut()) {
                line.push((part, s));
            }
        }
    }
    for line in lines {
        if line.is_empty() {
            ui.add_space(8.0);
            continue;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (text, span) in line {
                match &span.link {
                    Some(url) => {
                        ui.hyperlink_to(text, url);
                    }
                    None => {
                        let mut t = RichText::new(text).color(c_text());
                        if span.bold {
                            t = t.semibold();
                        }
                        if span.italic {
                            t = t.italics();
                        }
                        ui.label(t);
                    }
                }
            }
        });
    }
}

/// The article's text on one line, cut at a word (never inside one) with an ellipsis when it is long.
fn preview(item: &NewsItem) -> String {
    let flat = item.plain_text().split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 220 {
        return flat;
    }
    let cut: String = flat.chars().take(217).collect();
    // back up to the end of the last whole word, unless that would throw most of it away
    let whole = match cut.rfind(' ') {
        Some(i) if i > 120 => &cut[..i],
        _ => cut.as_str(),
    };
    format!("{}\u{2026}", whole.trim_end_matches([' ', ',', ';', ':', '.', '-']))
}

/// `text` as at most two lines of wrapped muted text; what does not fit ends in an ellipsis at the edge.
fn two_lines(ui: &Ui, text: &str, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat::simple(egui::FontId::proportional(13.0), c_muted()),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: width,
        max_rows: 2,
        break_anywhere: false,
        overflow_character: Some('\u{2026}'),
    };
    ui.painter().layout_job(job)
}

impl App {
    /// The news list for the bottom of the home page.
    pub(super) fn news_section(&mut self, ui: &mut Ui) {
        if matches!(self.svc.news.status, Status::Idle) {
            self.svc.news.status = Status::Loading;
            let (dirs, n) = (self.dirs.clone(), self.notifier());
            self.rt.spawn(async move {
                n.send(Msg::Svc(SvcMsg::News(crate::news::load(&dirs).await.map_err(|e| format!("{e:#}")))));
            });
        }
        ui.add_space(12.0);
        section(ui, "News");
        match &self.svc.news.status {
            Status::Idle | Status::Loading => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    muted(ui, "Loading the news");
                });
            }
            Status::Failed(e) => {
                small_muted(ui, format!("Couldn't load the news: {e}"));
                if secondary(ui, "Try again").clicked() {
                    self.svc.news.status = Status::Idle;
                }
            }
            Status::Loaded if self.svc.news.items.is_empty() => muted(ui, "No news right now."),
            Status::Loaded => {
                let mut open = None;
                for (i, item) in self.svc.news.items.iter().enumerate() {
                    // the fill is put in behind the contents once it is known whether the row is pointed at
                    let fill = ui.painter().add(egui::Shape::Noop);
                    let resp = egui::Frame::new()
                        .stroke(Stroke::new(1.0, c_border()))
                        .corner_radius(CornerRadius::same(R_CARD))
                        .inner_margin(egui::Margin::same(16))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(&item.title).semibold().size(15.0));
                                if !item.date.is_empty() {
                                    small_muted(ui, &item.date);
                                }
                            });
                            let galley = two_lines(ui, &item.preview, ui.available_width());
                            let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
                            ui.painter().galley(rect.min, galley, c_muted());
                        })
                        .response
                        .interact(Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    ui.painter().set(
                        fill,
                        egui::Shape::rect_filled(
                            resp.rect,
                            CornerRadius::same(R_CARD),
                            if resp.hovered() { c_card_hover() } else { c_card() },
                        ),
                    );
                    if resp.clicked() {
                        open = Some(i);
                    }
                    ui.add_space(8.0);
                }
                if open.is_some() {
                    self.svc.news.open = open;
                }
            }
        }
    }

    /// One article in a window of its own.
    pub(super) fn news_overlay(&mut self, ctx: &Context) {
        let Some(i) = self.svc.news.open else { return };
        let Some(item) = self.svc.news.items.get(i) else {
            self.svc.news.open = None;
            return;
        };
        let mut close = false;
        egui::Modal::new(egui::Id::unique("news-article")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(modal_width(ctx, 620.0));
            ui.add(egui::Label::new(RichText::new(&item.title).size(18.0).semibold()).wrap());
            if !item.date.is_empty() {
                small_muted(ui, &item.date);
            }
            ui.add_space(8.0);
            ScrollArea::vertical()
                .max_height(modal_scroll_height(ctx, 420.0, 140.0))
                .auto_shrink([false, true])
                .show(ui, |ui| render_spans(ui, &item.spans));
            ui.add_space(10.0);
            if secondary(ui, "Close").clicked() {
                close = true;
            }
        });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.svc.news.open = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draw the real news text in a window-less egui, to be sure nothing panics and something is painted.
    #[test]
    fn an_article_draws_without_a_window() {
        let item = NewsItem {
            title: "T".into(),
            content: "Hello <b>bold</b> <i>it</i> and <a href=\"https://example.com\">a link</a>.<br/><br/><ul><li>one</li><li>two</li></ul>".into(),
            created_at: "2024-11-15T12:55:33.000Z".into(),
        };
        let ctx = egui::Context::default();
        let mut shapes = 0;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            render_spans(ui, &item.spans());
        });
        shapes += out.shapes.len();
        out.textures_delta.clear();
        assert!(shapes > 0, "something was painted");
        assert_eq!(
            preview(&item),
            "Hello bold it and a link.\n\n\u{2022} one\n\u{2022} two".split_whitespace().collect::<Vec<_>>().join(" ")
        );
    }
}
