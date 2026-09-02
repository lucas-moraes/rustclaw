//! Shared output truncation for tools.

/// Truncates `text` to at most `max_bytes` bytes (char-boundary safe),
/// appending a notice line when truncated.
pub fn truncate_output(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut cut = text.len().min(max_bytes);
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let kept = &text[..cut];
    format!(
        "{}\n\n[output truncated: showing {} of {} bytes]",
        kept,
        cut,
        text.len()
    )
}

/// Truncates by line count, appending a notice when truncated.
pub fn truncate_lines(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        return text.to_string();
    }
    let kept: Vec<&str> = lines.iter().take(max_lines).copied().collect();
    format!(
        "{}\n\n[output truncated: showing {} of {} lines]",
        kept.join("\n"),
        max_lines,
        lines.len()
    )
}

/// Short preview helper (delegates to session::preview).
pub fn preview(s: &str, max: usize) -> String {
    crate::harness::session::preview(s, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_noop() {
        assert_eq!(truncate_output("hello", 100), "hello");
    }

    #[test]
    fn test_truncate_bytes_adds_notice() {
        let out = truncate_output(&"a".repeat(1000), 100);
        assert!(out.contains("[output truncated"));
        assert!(out.len() < 200);
    }

    #[test]
    fn test_truncate_char_boundary() {
        // 'é' is 2 bytes; cutting at 1 byte would split it.
        let out = truncate_output("ééé", 2);
        assert!(out.starts_with("é"));
    }

    #[test]
    fn test_truncate_lines() {
        let text = "1\n2\n3\n4\n5";
        assert_eq!(truncate_lines(text, 10), text);
        let out = truncate_lines(text, 3);
        assert!(out.starts_with("1\n2\n3"));
        assert!(out.contains("showing 3 of 5 lines"));
    }
}
