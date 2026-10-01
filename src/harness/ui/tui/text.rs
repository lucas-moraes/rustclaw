//! Width-aware text helpers for the TUI.

use ratatui::text::Span;

/// Truncates `s` so its display width is at most `max`, appending "…" when cut.
pub fn truncate_to_width(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = Span::raw(ch.to_string()).width();
        if w + cw > max.saturating_sub(1) {
            out.push('…');
            return out;
        }
        out.push(ch);
        w += cw;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_width() {
        assert_eq!(truncate_to_width("hello", 10), "hello");
        assert_eq!(truncate_to_width("hello world", 6), "hello…");
        assert_eq!(truncate_to_width("hello", 0), "");
    }
}
