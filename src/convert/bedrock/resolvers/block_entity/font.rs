// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Text width of the default Java font and the line splitting used for sign text (`FontUtil`).

use super::font_data::WIDTH_RANGES;
use crate::convert::util::json_text::CHAT_COLOR;

/// The width of one code point (0 for unknown ones).
pub fn char_width(code_point: u32) -> i32 {
    match WIDTH_RANGES.binary_search_by(|&(first, last, _)| {
        if code_point < first {
            std::cmp::Ordering::Greater
        } else if code_point > last {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(i) => WIDTH_RANGES[i].2 as i32,
        Err(_) => 0,
    }
}

/// The width of a string.
pub fn text_width(input: &str) -> i32 {
    input.chars().map(|c| char_width(c as u32)).sum()
}

/// Java's `Character.isSpaceChar`: the Unicode separator categories Zs, Zl and Zp.
fn is_space_char(c: char) -> bool {
    matches!(
        c,
        ' ' | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

/// Split one line into lines of at most `max_width` at word boundaries (`splitLine`). Colour codes do not count.
pub fn split_line(max_width: i32, input: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line_width = 0;
    let mut word_width = 0;
    let mut skip_next = false;
    let mut current_line = String::new();
    let mut current_word = String::new();
    let space_width = text_width(" ");

    for c in input.chars() {
        let width = if skip_next || c == CHAT_COLOR { 0 } else { char_width(c as u32) };
        if skip_next {
            skip_next = false;
        } else if c == CHAT_COLOR {
            skip_next = true;
        }

        if is_space_char(c) {
            if line_width + word_width + space_width > max_width {
                lines.push(std::mem::take(&mut current_line));
                line_width = 0;
            } else if !current_line.is_empty() {
                current_line.push(' ');
                line_width += space_width;
            }
            current_line.push_str(&current_word);
            line_width += word_width;
            current_word.clear();
            word_width = 0;
        } else {
            current_word.push(c);
            word_width += width;
        }
    }

    if !current_word.is_empty() {
        if line_width + word_width + space_width > max_width {
            lines.push(std::mem::take(&mut current_line));
        } else if !current_line.is_empty() {
            current_line.push(' ');
        }
        current_line.push_str(&current_word);
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }
    lines
}

/// Split every line that is too wide, unless there already are `max_lines` lines; the result has at most
/// `max_lines` lines (`split`).
pub fn split(max_width: i32, max_lines: usize, original: Vec<String>) -> Vec<String> {
    if original.len() >= max_lines || original.is_empty() {
        return original;
    }
    let mut lines: Vec<String> = original.iter().flat_map(|l| split_line(max_width, l)).collect();
    lines.truncate(max_lines);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        assert_eq!(char_width('a' as u32), 5);
        assert_eq!(char_width('i' as u32), 1);
        assert_eq!(char_width(' ' as u32), 3);
        assert_eq!(text_width("abc"), 18);
    }

    #[test]
    fn splits_long_lines() {
        // Short lines stay as they are.
        assert_eq!(split(88, 4, vec!["Hello".into()]), vec!["Hello".to_string()]);
        // Four lines are never split.
        let four: Vec<String> = (0..4).map(|_| "a ".repeat(40)).collect();
        assert_eq!(split(88, 4, four.clone()), four);
        // A long line wraps at spaces and keeps all the words.
        let lines = split(88, 4, vec!["the quick brown fox jumps over the lazy dog again and again".into()]);
        assert!(lines.len() > 1 && lines.len() <= 4);
        assert!(lines.iter().all(|l| text_width(l) <= 88));
        assert_eq!(lines.join(" "), "the quick brown fox jumps over the lazy dog again and again");
    }
}
