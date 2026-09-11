//! Lightweight markdown-ish styling for assistant bubbles.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::theme::Theme;

/// Render a plain text block into styled lines (headers, code fences, inline code, lists).
pub fn render_text(text: &str, theme: &Theme, base: Style) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut fence_buf: Vec<String> = Vec::new();
    let mut fence_lang = String::new();

    for raw in text.lines() {
        let line = raw.to_string();
        let trimmed = line.trim_start().to_string();

        // Code fence open/close.
        if trimmed.starts_with("```") {
            if in_fence {
                // Closing fence — flush the buffered code lines.
                out.extend(fence_lines(&fence_buf, &fence_lang, theme, false));
                fence_buf.clear();
                fence_lang.clear();
                in_fence = false;
            } else {
                // Opening fence — capture the language tag.
                fence_lang = trimmed.trim_start_matches('`').trim().to_string();
                in_fence = true;
                fence_buf.clear();
            }
            continue;
        }
        if in_fence {
            fence_buf.push(line);
            continue;
        }

        // Headers.
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

        // Unordered lists.
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

    // Unclosed fence — flush what we have with a streaming hint.
    if in_fence {
        out.extend(fence_lines(&fence_buf, &fence_lang, theme, true));
    }

    if out.is_empty() {
        out.push(Line::from(Span::styled(text.to_string(), base)));
    }
    out
}

/// Build the visual lines for a code fence block: a top bar with the language,
/// indented body lines with a left accent rail, and a bottom rule.
fn fence_lines(buf: &[String], lang: &str, theme: &Theme, streaming: bool) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let rail = Style::default().fg(theme.accent3);
    let code_fg = Style::default().fg(theme.text_bright);

    // Top edge: ╭─ rust ────────
    let label = if lang.is_empty() {
        " code ".to_string()
    } else {
        format!(" {} ", lang)
    };
    let rule_len = 40usize.saturating_sub(label.len() + 4).max(1);
    out.push(Line::from(vec![
        Span::styled("  ╭─".to_string(), rail),
        Span::styled(
            label,
            Style::default()
                .fg(theme.accent3)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("─".repeat(rule_len), rail),
    ]));

    // Body lines with left rail.
    for l in buf {
        out.push(Line::from(vec![
            Span::styled("  │ ".to_string(), rail),
            Span::styled(l.clone(), code_fg),
        ]));
    }

    // Bottom edge (or streaming indicator).
    if streaming {
        out.push(Line::from(vec![
            Span::styled("  │ ".to_string(), rail),
            Span::styled("▌".to_string(), Style::default().fg(theme.accent3)),
        ]));
    } else {
        out.push(Line::from(Span::styled(
            format!("  ╰{}", "─".repeat(39)),
            rail,
        )));
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
            // Render inline code without the backtick markers; use a subtle
            // surface background + accent2 foreground to read as code.
            spans.push(Span::styled(
                code,
                Style::default()
                    .fg(theme.accent2)
                    .bg(theme.surface)
                    .add_modifier(Modifier::DIM),
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

#[cfg(test)]
mod tests {

    use super::*;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn test_code_fence_renders_box() {
        let t = Theme::cyberclaw();
        let text = "Before\n```rust\nfn main() {}\n```\nAfter";
        let out = render_text(text, &t, Style::default());
        let p = plain(&out);
        // Should have: Before, top bar, code line, bottom bar, After
        assert_eq!(p.len(), 5, "got {:?}", p);
        assert!(p[0].contains("Before"));
        assert!(p[1].contains("╭─"), "top bar: {:?}", p[1]);
        assert!(p[1].contains("rust"), "lang label: {:?}", p[1]);
        assert!(p[2].contains("fn main() {}"), "code: {:?}", p[2]);
        assert!(p[3].contains("╰"), "bottom bar: {:?}", p[3]);
        assert!(p[4].contains("After"));
    }

    #[test]
    fn test_code_fence_no_lang() {
        let t = Theme::cyberclaw();
        let text = "```\nhello\n```";
        let out = render_text(text, &t, Style::default());
        let p = plain(&out);
        assert_eq!(p.len(), 3, "got {:?}", p);
        assert!(p[0].contains("code"), "default label: {:?}", p[0]);
    }

    #[test]
    fn test_unclosed_fence_streaming() {
        let t = Theme::cyberclaw();
        let text = "```python\nprint('hi')";
        let out = render_text(text, &t, Style::default());
        let p = plain(&out);
        // top bar + code line + streaming indicator
        assert_eq!(p.len(), 3, "got {:?}", p);
        assert!(p[2].contains("▌"), "streaming cursor: {:?}", p[2]);
    }

    #[test]
    fn test_no_backticks_in_output() {
        let t = Theme::cyberclaw();
        let text = "```js\nconsole.log(1)\n```";
        let out = render_text(text, &t, Style::default());
        let p = plain(&out);
        for line in &p {
            assert!(!line.contains("```"), "backticks leaked: {:?}", line);
        }
    }

    #[test]
    fn test_header_strips_hash() {
        let t = Theme::cyberclaw();
        let out = render_text("# Title", &t, Style::default());
        let p = plain(&out);
        assert_eq!(p[0], "Title");
    }

    #[test]
    fn test_bold_strips_asterisks() {
        let t = Theme::cyberclaw();
        let out = render_text("hello **world** end", &t, Style::default());
        let p = plain(&out);
        assert_eq!(p[0], "hello world end");
    }

    #[test]
    fn test_inline_code_strips_backticks() {
        let t = Theme::cyberclaw();
        let out = render_text("use `std::fs` here", &t, Style::default());
        let p = plain(&out);
        assert_eq!(p[0], "use std::fs here", "got {:?}", p);
        assert!(!p[0].contains('`'), "backticks leaked: {:?}", p);
    }

    #[test]
    fn test_list_bullet() {
        let t = Theme::cyberclaw();
        let out = render_text("- item one\n- item two", &t, Style::default());
        let p = plain(&out);
        assert!(p[0].contains("✦"), "bullet: {:?}", p[0]);
        assert!(p[0].contains("item one"));
    }
}
