// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Text escaping helpers.

/// Chunker's `ControlCodeEscaper`: control characters (`<= 0x1F` and `0x7F`) as `\uXXXX` with four upper-case hex
/// digits.
pub fn escape_control_codes(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c <= '\u{1f}' || c == '\u{7f}' {
            let _ = write!(out, "\\u{:04X}", c as u32);
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_controls_only() {
        assert_eq!(escape_control_codes("a\nb\u{7f}\u{0}c\u{e9}"), "a\\u000Ab\\u007F\\u0000c\u{e9}");
        assert_eq!(escape_control_codes("plain \"quoted\""), "plain \"quoted\"");
    }
}
