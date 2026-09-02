//! Lightweight markdown-ish styling for assistant bubbles.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::theme::Theme;

/// Render a plain text block into styled lines (headers, code fences, inline code, lists).
pub fn render_text(text: &str, theme: &Theme, base: Style) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for raw in text.lines() {
        let line = raw.to_string();
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            out.push(Line::from(Span::styled(
                line,
                Style::default().fg(theme.text_dim),
            )));
            continue;
        }
        if in_fence {
            out.push(Line::from(Span::styled(
                format!("  {}", line),
                Style::default().fg(theme.accent3),
            )));
            continue;
        }
        if let Some(rest) = line.strip_prefix("### ") {
            out.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            out.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(theme.accent2)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            out.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            continue;
        }
        if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            let mut spans = vec![Span::styled(
                "  ✦ ".to_string(),
                Style::default().fg(theme.accent),
            )];
            spans.extend(inline_spans(rest, theme, base));
            out.push(Line::from(spans));
            continue;
        }
        // Diff-aware lines if they sneak into assistant text.
        if line.starts_with('+') && !line.starts_with("+++") {
            out.push(Line::from(Span::styled(
                line,
                Style::default().fg(theme.diff_add),
            )));
            continue;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            out.push(Line::from(Span::styled(
                line,
                Style::default().fg(theme.diff_del),
            )));
            continue;
        }
        out.push(Line::from(inline_spans(&line, theme, base)));
    }
    if out.is_empty() {
        out.push(Line::from(Span::styled(text.to_string(), base)));
    }
    out
}

/// Colorize a unified diff string.
pub fn render_diff(diff: &str, theme: &Theme) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for raw in diff.lines() {
        let line = raw.to_string();
        let style = if line.starts_with("+++") || line.starts_with("---") {
            Style::default().fg(theme.text_dim)
        } else if line.starts_with("@@") {
            Style::default().fg(theme.diff_hunk)
        } else if line.starts_with('+') {
            Style::default().fg(theme.diff_add)
        } else if line.starts_with('-') {
            Style::default().fg(theme.diff_del)
        } else {
            Style::default().fg(theme.text_dim)
        };
        out.push(Line::from(Span::styled(line, style)));
    }
    out
}

fn inline_spans(s: &str, theme: &Theme, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut buf = String::new();

    let flush = |buf: &mut String, spans: &mut Vec<Span<'static>>, style: Style| {
        if !buf.is_empty() {
            spans.push(Span::styled(std::mem::take(buf), style));
        }
    };

    while i < chars.len() {
        // inline code `...`
        if chars[i] == '`' {
            flush(&mut buf, &mut spans, base);
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '`' {
                i += 1;
            }
            let code: String = chars[start..i].iter().collect();
            spans.push(Span::styled(
                format!("`{}`", code),
                Style::default().fg(theme.accent2),
            ));
            if i < chars.len() {
                i += 1; // closing `
            }
            continue;
        }
        // **bold**
        if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            flush(&mut buf, &mut spans, base);
            i += 2;
            let start = i;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '*') {
                i += 1;
            }
            let bold: String = chars[start..i.min(chars.len())].iter().collect();
            spans.push(Span::styled(
                bold,
                base.add_modifier(Modifier::BOLD).fg(theme.text_bright),
            ));
            if i + 1 < chars.len() {
                i += 2;
            }
            continue;
        }
        buf.push(chars[i]);
        i += 1;
    }
    flush(&mut buf, &mut spans, base);
    if spans.is_empty() {
        spans.push(Span::styled(s.to_string(), base));
    }
    spans
}

/// Wrap a line of text to `width`, preserving style by re-rendering each wrapped chunk simply.
pub fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_inclusive(char::is_whitespace) {
            if current.chars().count() + word.chars().count() > width && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            // Hard-break very long tokens.
            if word.chars().count() > width {
                if !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                }
                let chars: Vec<char> = word.chars().collect();
                for chunk in chars.chunks(width) {
                    lines.push(chunk.iter().collect());
                }
            } else {
                current.push_str(word);
            }
        }
        if !current.is_empty() || paragraph.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}
