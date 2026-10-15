//! Collecting the texts to translate: the Java edition's `generatePots` task, for Rust. It reads the source files, finds
//! every `tr("...")`, `tr!("...", args)`, `tr_args`, `trc`, `tr_n` and `trnc` call with a text written out in the code,
//! and writes a `.pot` template. Translators copy it to `<language>-<COUNTRY>.po` (`de-DE.po`) in `assets/lang`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One text that can be translated.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Message {
    pub context: String,
    pub id: String,
    pub plural: Option<String>,
}

/// The calls that take texts, and which of their first arguments are texts: (call, [is it context? ...]).
const CALLS: &[(&str, &[Role])] = &[
    ("tr!(", &[Role::Id]),
    ("tr(", &[Role::Id]),
    ("tr_args(", &[Role::Id]),
    ("trc(", &[Role::Context, Role::Id]),
    ("tr_n(", &[Role::Id, Role::Plural]),
    ("trnc(", &[Role::Context, Role::Id, Role::Plural]),
];

#[derive(Clone, Copy)]
enum Role {
    Context,
    Id,
    Plural,
}

/// Reads a Rust string literal at the start of `s` (after spaces): returns the text and the rest.
fn read_literal(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    if let Some(rest) = s.strip_prefix('r') {
        // raw string: r"..." or r#"..."#
        let hashes = rest.chars().take_while(|c| *c == '#').count();
        let after = rest[hashes..].strip_prefix('"')?;
        let end_mark = format!("\"{}", "#".repeat(hashes));
        let end = after.find(&end_mark)?;
        return Some((after[..end].to_string(), &after[end + end_mark.len()..]));
    }
    let body = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = body.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &body[i + 1..])),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                '"' => out.push('"'),
                '\'' => out.push('\''),
                '\\' => out.push('\\'),
                'u' => {
                    let (_, open) = chars.next()?;
                    if open != '{' {
                        return None;
                    }
                    let mut hex = String::new();
                    for (_, h) in chars.by_ref() {
                        if h == '}' {
                            break;
                        }
                        hex.push(h);
                    }
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                // a backslash at the end of a line skips the line break and the next line's indentation
                '\n' => {
                    while let Some((_, w)) = chars.clone().next() {
                        if w.is_whitespace() {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                other => {
                    out.push('\\');
                    out.push(other);
                }
            },
            c => out.push(c),
        }
    }
    None
}

/// Every text in one file's source, with the line it is on.
pub fn extract_from_source(source: &str) -> Vec<(Message, usize)> {
    let mut found = Vec::new();
    for (call, roles) in CALLS {
        let mut from = 0;
        while let Some(at) = source[from..].find(call) {
            let start = from + at;
            from = start + call.len();
            // `str(`, `fn tr(` and `obj.tr(` are not calls to the translation functions
            let before = source[..start].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                continue;
            }
            if source[..start].trim_end().ends_with("fn") {
                continue;
            }
            let mut rest = &source[from..];
            let mut ctx = String::new();
            let mut id = None;
            let mut plural = None;
            let mut ok = true;
            for (n, role) in roles.iter().enumerate() {
                if n > 0 {
                    // the next argument follows a comma
                    match rest.trim_start().strip_prefix(',') {
                        Some(r) => rest = r,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                match read_literal(rest) {
                    Some((text, r)) => {
                        rest = r;
                        match role {
                            Role::Context => ctx = text,
                            Role::Id => id = Some(text),
                            Role::Plural => plural = Some(text),
                        }
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if let (true, Some(id)) = (ok, id) {
                let line = source[..start].matches('\n').count() + 1;
                found.push((Message { context: ctx, id, plural }, line));
            }
        }
    }
    found
}

fn visit(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            visit(&p, files);
        } else if p.extension().is_some_and(|e| e == "rs") {
            files.push(p);
        }
    }
}

fn po_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The `.pot` text for all the sources under `root` (usually the `src` folder).
pub fn extract_template(root: &Path) -> String {
    let mut files = Vec::new();
    visit(root, &mut files);
    let mut messages: BTreeMap<Message, Vec<String>> = BTreeMap::new();
    for file in files {
        let Ok(source) = std::fs::read_to_string(&file) else { continue };
        let shown =
            file.strip_prefix(root.parent().unwrap_or(root)).unwrap_or(&file).to_string_lossy().replace('\\', "/");
        for (m, line) in extract_from_source(&source) {
            messages.entry(m).or_default().push(format!("{shown}:{line}"));
        }
    }
    let mut out = String::new();
    out.push_str("msgid \"\"\nmsgstr \"\"\n\"Project-Id-Version: BlockForge\\n\"\n\"MIME-Version: 1.0\\n\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\"Content-Transfer-Encoding: 8bit\\n\"\n\n");
    for (m, refs) in &messages {
        for r in refs {
            out.push_str(&format!("#: {r}\n"));
        }
        if !m.context.is_empty() {
            out.push_str(&format!("msgctxt {}\n", po_quote(&m.context)));
        }
        out.push_str(&format!("msgid {}\n", po_quote(&m.id)));
        match &m.plural {
            Some(p) => {
                out.push_str(&format!("msgid_plural {}\nmsgstr[0] \"\"\nmsgstr[1] \"\"\n\n", po_quote(p)));
            }
            None => out.push_str("msgstr \"\"\n\n"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_calls_with_written_out_texts() {
        let src = r##"
            let a = tr("Play");
            let b = i18n::tr_args("Downloading {0}", &[x.into()]);
            let c = tr!("Open \"{0}\"", name);
            let d = trc("menu", "Open");
            let e = tr_n("{0} mod", "{0} mods", n, &[n.into()]);
            let f = tr(variable);
            let g = something.tr("not ours");
            let h = str("also not ours");
            fn tr(text: &str) -> String { text.into() }
            let i = tr("a long text that \
                        goes on");
            let j = tr(r#"raw "quoted""#);
        "##;
        let found: Vec<(String, String, Option<String>)> =
            extract_from_source(src).into_iter().map(|(m, _)| (m.context, m.id, m.plural)).collect();
        let has = |ctx: &str, id: &str, pl: Option<&str>| {
            found.contains(&(ctx.to_string(), id.to_string(), pl.map(str::to_string)))
        };
        assert!(has("", "Play", None));
        assert!(has("", "Downloading {0}", None));
        assert!(has("", "Open \"{0}\"", None));
        assert!(has("menu", "Open", None));
        assert!(has("", "{0} mod", Some("{0} mods")));
        assert!(has("", "a long text that goes on", None));
        assert!(has("", "raw \"quoted\"", None));
        assert_eq!(found.len(), 7, "{found:?}");
    }

    #[test]
    fn the_template_lists_each_text_once_with_its_places() {
        let dir = std::env::temp_dir().join(format!("bf-pot-{}", std::process::id())).join("src");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "fn x() { tr(\"Play\"); tr!(\"Stop {0}\", 1); }\n").unwrap();
        std::fs::write(dir.join("b.rs"), "\n\nfn y() { tr(\"Play\"); tr_n(\"{0} mod\", \"{0} mods\", 2, &[]); }\n")
            .unwrap();
        let pot = extract_template(&dir);
        assert_eq!(pot.matches("msgid \"Play\"").count(), 1);
        assert!(pot.contains("#: src/a.rs:1\n#: src/b.rs:3\nmsgid \"Play\""));
        assert!(pot.contains("msgid_plural \"{0} mods\"\nmsgstr[0] \"\"\nmsgstr[1] \"\""));
        // a template reads back as a catalog with nothing translated
        assert!(crate::i18n::Catalog::parse(&pot).is_empty());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
