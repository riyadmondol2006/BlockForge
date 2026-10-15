//! The launcher news shown on the home page: the Java edition's `NewsManager`, `News` and `AbstractNews`.
//!
//! News comes from a GraphQL query (`generalNews(first: 10)`) on the same server the Java edition asks. The answer
//! is kept for 30 minutes, like the Java edition's HTTP cache, and an older copy is used when the network is down.
//! The text of an item is a little HTML (`<br/>`, `<b>`, `<a href>`); [`NewsItem::spans`] turns it into plain
//! pieces a window can draw.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use serde::{Deserialize, Serialize};

use crate::paths::Dirs;

pub const GRAPHQL_ENDPOINT: &str = "https://api.atlauncher.com/v2/graphql";
/// How many items the home page asks for.
pub const NEWS_COUNT: u32 = 10;
/// How long an answer is reused before asking again.
pub const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const QUERY: &str = "query GetNews($first: Int!) { generalNews(first: $first) { title content createdAt } }";

/// One news article.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct NewsItem {
    #[serde(default)]
    pub title: String,
    /// HTML.
    #[serde(default)]
    pub content: String,
    /// When it was published, as the server wrote it (`2024-11-15T12:55:33.000Z`).
    #[serde(default)]
    pub created_at: String,
}

impl NewsItem {
    pub fn created(&self) -> Option<DateTime<Local>> {
        parse_date(&self.created_at)
    }

    /// `15 Nov 2024`, the way the Java edition shows it; empty when the date can't be read.
    pub fn date(&self) -> String {
        self.created().map(|d| d.format("%-d %b %Y").to_string()).unwrap_or_default()
    }

    /// The HTML the Java edition builds for an item: `<h2>title (date)</h2><p>content</p>`.
    pub fn html_entry(&self) -> String {
        let date = self.date();
        let heading = if date.is_empty() { self.title.clone() } else { format!("{} ({date})", self.title) };
        format!("<h2>{heading}</h2><p>{}</p>", self.content)
    }

    /// The text without any markup, for the short preview on a card.
    pub fn plain_text(&self) -> String {
        spans_to_plain(&self.spans())
    }

    /// The content as pieces of text with their style.
    pub fn spans(&self) -> Vec<Span> {
        parse_html(&self.content)
    }
}

/// A piece of text from the news, with the style it should be drawn in.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    /// Where the text links to, if it is a link.
    pub link: Option<String>,
}

fn parse_date(s: &str) -> Option<DateTime<Local>> {
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&Local));
    }
    // the older news file's format: 2019-05-04T10:20:30.123456Z
    let trimmed = s.trim_end_matches('Z');
    let naive = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
    Local.from_utc_datetime(&naive).into()
}

/// Read news items from the Java edition's older file format (`[{"title":..,"content":..,"created_at":..}]`).
pub fn parse_file_news(json: &[u8]) -> Result<Vec<NewsItem>> {
    #[derive(Deserialize)]
    struct FileNews {
        #[serde(default)]
        title: String,
        #[serde(default)]
        content: String,
        #[serde(default)]
        created_at: String,
    }
    let items: Vec<FileNews> = serde_json::from_slice(json).context("the news file isn't what was expected")?;
    Ok(items.into_iter().map(|n| NewsItem { title: n.title, content: n.content, created_at: n.created_at }).collect())
}

/// Read the GraphQL answer.
pub fn parse_graphql(json: &[u8]) -> Result<Vec<NewsItem>> {
    #[derive(Deserialize)]
    struct Answer {
        data: Option<Data>,
        #[serde(default)]
        errors: Vec<serde_json::Value>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Data {
        #[serde(default)]
        general_news: Vec<NewsItem>,
    }
    let answer: Answer = serde_json::from_slice(json).context("the news server's answer wasn't what was expected")?;
    match answer.data {
        Some(d) => Ok(d.general_news),
        None => bail!(
            "the news server returned no data{}",
            answer.errors.first().map(|e| format!(": {e}")).unwrap_or_default()
        ),
    }
}

// ---------------------------------------------------------------------------------------------------------
// Fetching

fn cache_file(dirs: &Dirs) -> PathBuf {
    dirs.cache.join("news.json")
}

fn read_cache(path: &Path) -> Option<(Vec<NewsItem>, Duration)> {
    let age = std::fs::metadata(path).and_then(|m| m.modified()).ok()?.elapsed().ok()?;
    let items: Vec<NewsItem> = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some((items, age))
}

/// Ask the server for the latest news.
pub async fn fetch(count: u32) -> Result<Vec<NewsItem>> {
    let body = serde_json::json!({ "operationName": "GetNews", "query": QUERY, "variables": { "first": count } });
    let r = crate::net::send_with_retry(|| crate::http::client().post(GRAPHQL_ENDPOINT).json(&body), GRAPHQL_ENDPOINT)
        .await
        .context("couldn't reach the news server")?;
    let status = r.status();
    let bytes = r.bytes().await?;
    if !status.is_success() {
        bail!("the news server answered HTTP {status}");
    }
    parse_graphql(&bytes)
}

/// The news for the home page: the saved copy while it is younger than 30 minutes, otherwise a fresh one (which is
/// saved), and when the network fails whatever copy there is. An empty list with no copy at all is an error.
pub async fn load(dirs: &Dirs) -> Result<Vec<NewsItem>> {
    let file = cache_file(dirs);
    let cached = read_cache(&file);
    if let Some((items, age)) = &cached {
        if *age < CACHE_TTL {
            return Ok(items.clone());
        }
    }
    match fetch(NEWS_COUNT).await {
        Ok(items) => {
            let _ = std::fs::create_dir_all(&dirs.cache);
            let _ = std::fs::write(&file, serde_json::to_vec(&items).unwrap_or_default());
            Ok(items)
        }
        Err(e) => {
            crate::logs::error(format!("Error fetching news: {e:#}"));
            match cached {
                Some((items, _)) => Ok(items),
                None => Err(e),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// The HTML of an item

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let ch = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            e if e.starts_with("#x") || e.starts_with("#X") => {
                u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32)
            }
            e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let at = lower.find(&format!("{name}="))? + name.len() + 1;
    let rest = &tag[at..];
    let mut chars = rest.chars();
    match chars.next()? {
        q @ ('"' | '\'') => {
            let end = rest[1..].find(q)?;
            Some(decode_entities(&rest[1..1 + end]))
        }
        _ => Some(decode_entities(rest.split(|c: char| c.is_whitespace() || c == '>').next()?)),
    }
}

/// Turn the little HTML the news uses into styled pieces of text. Line breaks and paragraphs become `\n`;
/// anything unknown is dropped but its text is kept.
pub fn parse_html(html: &str) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let (mut bold, mut italic) = (0u32, 0u32);
    let mut link: Option<String> = None;
    let mut text = String::new();
    let flush = |text: &mut String, spans: &mut Vec<Span>, bold: u32, italic: u32, link: &Option<String>| {
        if !text.is_empty() {
            spans.push(Span {
                text: decode_entities(&std::mem::take(text)),
                bold: bold > 0,
                italic: italic > 0,
                link: link.clone(),
            });
        }
    };
    let mut rest = html.replace("\r\n", "\n").replace('\r', "");
    // raw newlines in HTML are only spaces
    rest = rest.replace('\n', " ");
    let mut s = rest.as_str();
    while let Some(i) = s.find('<') {
        text.push_str(&s[..i]);
        let Some(end) = s[i..].find('>') else {
            text.push_str(&s[i..]);
            s = "";
            break;
        };
        let tag = &s[i + 1..i + end];
        s = &s[i + end + 1..];
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let closing = tag.starts_with('/');
        match name.as_str() {
            "br" => {
                flush(&mut text, &mut spans, bold, italic, &link);
                spans.push(Span { text: "\n".into(), ..Default::default() });
            }
            "p" | "div" | "h1" | "h2" | "h3" | "h4" | "ul" | "ol" => {
                flush(&mut text, &mut spans, bold, italic, &link);
                if closing || !spans.is_empty() {
                    spans.push(Span { text: "\n".into(), ..Default::default() });
                }
            }
            "li" => {
                flush(&mut text, &mut spans, bold, italic, &link);
                if closing {
                    spans.push(Span { text: "\n".into(), ..Default::default() });
                } else {
                    spans.push(Span { text: "\u{2022} ".into(), ..Default::default() });
                }
            }
            "b" | "strong" => {
                flush(&mut text, &mut spans, bold, italic, &link);
                bold = if closing { bold.saturating_sub(1) } else { bold + 1 };
            }
            "i" | "em" => {
                flush(&mut text, &mut spans, bold, italic, &link);
                italic = if closing { italic.saturating_sub(1) } else { italic + 1 };
            }
            "a" => {
                flush(&mut text, &mut spans, bold, italic, &link);
                link = if closing { None } else { attribute(tag, "href") };
            }
            _ => {}
        }
    }
    text.push_str(s);
    flush(&mut text, &mut spans, bold, italic, &link);
    tidy(spans)
}

/// Merge neighbours with the same style, and remove blank lines at the ends and runs of more than two.
fn tidy(spans: Vec<Span>) -> Vec<Span> {
    let mut merged: Vec<Span> = Vec::new();
    for s in spans {
        match merged.last_mut() {
            Some(l) if l.bold == s.bold && l.italic == s.italic && l.link == s.link => l.text.push_str(&s.text),
            _ => merged.push(s),
        }
    }
    // squeeze spaces that came from the HTML's own line breaks
    for s in &mut merged {
        while s.text.contains("  ") {
            s.text = s.text.replace("  ", " ");
        }
    }
    let mut out: Vec<Span> = Vec::new();
    for s in merged {
        if s.text.is_empty() {
            continue;
        }
        out.push(s);
    }
    // trim the start and end
    if let Some(first) = out.first_mut() {
        first.text = first.text.trim_start().to_string();
    }
    if let Some(last) = out.last_mut() {
        last.text = last.text.trim_end().to_string();
    }
    // never more than one blank line in a row
    let mut result: Vec<Span> = Vec::new();
    for mut s in out {
        if s.link.is_none() && !s.bold && !s.italic {
            s.text = collapse_newlines(&s.text);
        }
        // a newline followed by a space looks ragged
        s.text = s.text.replace("\n ", "\n");
        match result.last_mut() {
            Some(l) if l.bold == s.bold && l.italic == s.italic && l.link == s.link => l.text.push_str(&s.text),
            _ => result.push(s),
        }
    }
    result.retain(|s| !s.text.is_empty());
    result
}

fn collapse_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = 0;
    for c in s.chars() {
        if c == '\n' {
            run += 1;
            if run <= 2 {
                out.push(c);
            }
        } else {
            run = 0;
            out.push(c);
        }
    }
    out
}

pub fn spans_to_plain(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect::<String>().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = r#"{"data":{"generalNews":[
        {"title":"FTB Packs Available Again","content":"Previously we had to remove FTB packs.\r<br/>\r<br/>If you face any issues, please visit <a href=\"https://atl.pw/discord\">our Discord</a>.","createdAt":"2024-11-15T12:55:33.000Z"},
        {"title":"Second","content":"Plain &amp; simple <b>bold</b>","createdAt":"2024-06-10T07:40:57.000Z"}]}}"#;

    #[test]
    fn reads_the_servers_answer() {
        let items = parse_graphql(ANSWER.as_bytes()).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "FTB Packs Available Again");
        assert!(items[0].created().is_some());
        // 12:55 UTC lands on the 15th or 16th depending on the time zone
        assert!(items[0].date().ends_with(" Nov 2024"), "{}", items[0].date());
        assert!(items[0].html_entry().starts_with("<h2>FTB Packs Available Again ("));
        assert!(items[0].html_entry().ends_with("</p>"));
        assert_eq!(parse_graphql(br#"{"data":{"generalNews":[]}}"#).unwrap().len(), 0);
        assert!(parse_graphql(br#"{"errors":[{"message":"nope"}]}"#).is_err());
        assert!(parse_graphql(b"<html>").is_err());
    }

    #[test]
    fn dates_in_both_formats() {
        let a = parse_date("2019-05-04T10:20:30.123456Z").unwrap();
        let b = parse_date("2019-05-04T10:20:30.000Z").unwrap();
        assert_eq!(a.timestamp(), b.timestamp());
        assert!(parse_date("yesterday").is_none());
        let i = NewsItem { title: "T".into(), content: "C".into(), created_at: "nonsense".into() };
        assert_eq!(i.html_entry(), "<h2>T</h2><p>C</p>", "an unreadable date is left out");
    }

    #[test]
    fn reads_the_older_news_file() {
        let items =
            parse_file_news(br#"[{"title":"Old","content":"x","created_at":"2019-05-04T10:20:30.123456Z"}]"#).unwrap();
        assert_eq!(items[0].title, "Old");
        assert!(items[0].date().contains("2019"));
    }

    #[test]
    fn html_becomes_styled_text() {
        let spans = parse_html(
            "Hello <b>big</b> <i>wide</i> world.\r<br/>\r<br/>Visit <a href=\"https://atl.pw/discord\">our Discord</a>.",
        );
        let plain = spans_to_plain(&spans);
        assert_eq!(plain, "Hello big wide world.\n\nVisit our Discord.");
        assert!(spans.iter().any(|s| s.bold && s.text == "big"));
        assert!(spans.iter().any(|s| s.italic && s.text == "wide"));
        let link = spans.iter().find(|s| s.link.is_some()).unwrap();
        assert_eq!((link.text.as_str(), link.link.as_deref()), ("our Discord", Some("https://atl.pw/discord")));
    }

    #[test]
    fn entities_lists_and_odd_markup() {
        assert_eq!(
            spans_to_plain(&parse_html("Tom &amp; Jerry &lt;3 &#169; &#x41; &unknown; &")),
            "Tom & Jerry <3 \u{a9} A &unknown; &"
        );
        assert_eq!(spans_to_plain(&parse_html("<ul><li>one</li><li>two</li></ul>")), "\u{2022} one\n\u{2022} two");
        assert_eq!(spans_to_plain(&parse_html("<p>a</p><p>b</p>")), "a\n\nb");
        assert_eq!(spans_to_plain(&parse_html("text <unknown>kept</unknown> <b>open")), "text kept open");
        assert_eq!(spans_to_plain(&parse_html("a<br/><br/><br/><br/>b")), "a\n\nb", "no more than one blank line");
        assert_eq!(spans_to_plain(&parse_html("")), "");
        assert_eq!(spans_to_plain(&parse_html("1 < 2")), "1 < 2");
    }

    #[test]
    fn the_cache_is_used_while_fresh() {
        let base = std::env::temp_dir().join(format!("blockforge-news-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dirs = Dirs::at(&base);
        std::fs::create_dir_all(&dirs.cache).unwrap();
        let items = vec![NewsItem {
            title: "Cached".into(),
            content: "c".into(),
            created_at: "2024-01-01T00:00:00.000Z".into(),
        }];
        std::fs::write(cache_file(&dirs), serde_json::to_vec(&items).unwrap()).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let got = rt.block_on(load(&dirs)).unwrap();
        assert_eq!(got, items, "a copy younger than 30 minutes is used without asking the server");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    #[ignore = "uses the network"]
    async fn live_news_fetch() {
        let items = fetch(3).await.unwrap();
        println!("{} items", items.len());
        for i in &items {
            println!("- {} ({}): {}", i.title, i.date(), i.plain_text().chars().take(80).collect::<String>());
        }
        assert!(!items.is_empty());
        assert!(items.iter().all(|i| !i.title.is_empty() && i.created().is_some()));
    }
}
