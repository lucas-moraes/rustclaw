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
    let mut table_buf: Vec<String> = Vec::new();

    // Flush a buffered markdown table (if any) into styled lines.
    let flush_table = |out: &mut Vec<Line<'static>>, buf: &mut Vec<String>| {
        if !buf.is_empty() {
            out.extend(table_lines(buf, theme));
            buf.clear();
        }
    };

    for raw in text.lines() {
        let line = raw.to_string();
        let trimmed = line.trim_start().to_string();

        // Code fence open/close.
        if trimmed.starts_with("```") {
            flush_table(&mut out, &mut table_buf);
            if in_fence {
                // Closing fence — flush the buffered code lines. If the
                // content is a markdown table, render it as a grid instead
                // of a code box (models often wrap tables in fences).
                if is_table_block(&fence_buf) {
                    out.extend(table_lines(&fence_buf, theme));
                } else {
                    out.extend(fence_lines(&fence_buf, &fence_lang, theme, false));
                }
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

        // Markdown table: a line starting with `|` (or a separator row of
        // dashes/pipes). Buffer consecutive table lines and render as a grid.
        if is_table_line(&line) {
            table_buf.push(line);
            continue;
        }
        flush_table(&mut out, &mut table_buf);

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

    // Flush any trailing table.
    flush_table(&mut out, &mut table_buf);

    // Unclosed fence — flush what we have with a streaming hint.
    if in_fence {
        out.extend(fence_lines(&fence_buf, &fence_lang, theme, true));
    }

    if out.is_empty() {
        out.push(Line::from(Span::styled(text.to_string(), base)));
    }
    out
}

/// True if a line looks like a markdown table row: starts with `|` or is a
/// separator row of dashes/pipes/colons.
fn is_table_line(line: &str) -> bool {
    let t = line.trim();
    if t.starts_with('|') {
        return true;
    }
    // Separator row like `|---|---|` or `| :--- | ---: |`.
    if t.contains('|') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) {
        return true;
    }
    false
}

/// True if a block of lines (e.g. inside a code fence) is a markdown table:
/// at least one header row and a separator row of dashes.
fn is_table_block(buf: &[String]) -> bool {
    if buf.is_empty() {
        return false;
    }
    let mut saw_header = false;
    let mut saw_separator = false;
    for raw in buf {
        let t = raw.trim();
        if !t.starts_with('|') {
            return false;
        }
        let inner = t.trim_matches('|');
        let stripped: String = inner.chars().filter(|c| *c != '|').collect();
        if !stripped.is_empty() && stripped.chars().all(|c| matches!(c, '-' | ':' | ' ')) {
            saw_separator = true;
        } else {
            saw_header = true;
        }
    }
    saw_header && saw_separator
}

/// Render a buffered markdown table as a styled grid with a header row.
fn table_lines(buf: &[String], theme: &Theme) -> Vec<Line<'static>> {
    // Parse rows into cells.
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut header: Option<Vec<String>> = None;
    for raw in buf {
        let t = raw.trim();
        if !t.starts_with('|') {
            continue; // not a table row
        }
        // Skip the separator row (e.g. `|---|---|` or `| :--- | ---: |`).
        let inner = t.trim_matches('|');
        let stripped: String = inner.chars().filter(|c| *c != '|').collect();
        if !stripped.is_empty() && stripped.chars().all(|c| matches!(c, '-' | ':' | ' ')) {
            continue;
        }
        let cells: Vec<String> = inner.split('|').map(|c| c.trim().to_string()).collect();
        if header.is_none() {
            header = Some(cells);
        } else {
            rows.push(cells);
        }
    }
    let header = match header {
        Some(h) => h,
        None => return Vec::new(),
    };

    // Column widths = max of header + body cells.
    let ncols = header.len();
    let mut widths: Vec<usize> = header.iter().map(|c| c.chars().count()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if i < ncols {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
    }

    let rail = Style::default().fg(theme.accent3);
    let header_fg = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let body_fg = Style::default().fg(theme.text);

    let mut out = Vec::new();

    // Top border.
    out.push(Line::from(Span::styled(
        format!(
            "  ┌─{}─┐",
            widths
                .iter()
                .map(|w| "─".repeat(w + 2))
                .collect::<Vec<_>>()
                .join("─┬─")
        ),
        rail,
    )));

    // Header row.
    let mut hspans = vec![Span::styled("  │ ".to_string(), rail)];
    for (i, cell) in header.iter().enumerate() {
        hspans.push(Span::styled(
            format!("{:<width$}", cell, width = widths[i]),
            header_fg,
        ));
        hspans.push(Span::styled(" │ ".to_string(), rail));
    }
    out.push(Line::from(hspans));

    // Separator under header.
    out.push(Line::from(Span::styled(
        format!(
            "  ├─{}─┤",
            widths
                .iter()
                .map(|w| "─".repeat(w + 2))
                .collect::<Vec<_>>()
                .join("─┼─")
        ),
        rail,
    )));

    // Body rows.
    for row in &rows {
        let mut spans = vec![Span::styled("  │ ".to_string(), rail)];
        for (i, cell) in row.iter().enumerate() {
            if i < ncols {
                spans.push(Span::styled(
                    format!("{:<width$}", cell, width = widths[i]),
                    body_fg,
                ));
            } else {
                spans.push(Span::styled(
                    format!("{:<width$}", "", width = widths[i]),
                    body_fg,
                ));
            }
            spans.push(Span::styled(" │ ".to_string(), rail));
        }
        out.push(Line::from(spans));
    }

    // Bottom border.
    out.push(Line::from(Span::styled(
        format!(
            "  └─{}─┘",
            widths
                .iter()
                .map(|w| "─".repeat(w + 2))
                .collect::<Vec<_>>()
                .join("─┴─")
        ),
        rail,
    )));

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

    #[test]
    fn test_table_renders_grid() {
        let t = Theme::cyberclaw();
        let text = "| Comando | Desc |\n|---|---|\n| build | compila |\n| test | roda |";
        let out = render_text(text, &t, Style::default());
        let p = plain(&out);
        let joined = p.join("\n");
        // Top border, header, separator, 2 body rows, bottom border.
        assert!(joined.contains("┌"), "top border:\n{}", joined);
        assert!(joined.contains("Comando"), "header:\n{}", joined);
        assert!(joined.contains("Desc"), "header:\n{}", joined);
        assert!(joined.contains("├"), "separator:\n{}", joined);
        assert!(joined.contains("build"), "body:\n{}", joined);
        assert!(joined.contains("compila"), "body:\n{}", joined);
        assert!(joined.contains("└"), "bottom border:\n{}", joined);
        // No raw pipes should leak.
        assert!(!joined.contains("|"), "pipes leaked:\n{}", joined);
    }

    #[test]
    fn test_table_after_text() {
        let t = Theme::cyberclaw();
        let text = "Tabela:\n| A | B |\n|---|---|\n| 1 | 2 |";
        let out = render_text(text, &t, Style::default());
        let p = plain(&out);
        let joined = p.join("\n");
        assert!(p[0].contains("Tabela:"), "intro text:\n{}", joined);
        assert!(joined.contains("┌"), "table rendered:\n{}", joined);
    }

    #[test]
    fn test_table_inside_code_fence() {
        let t = Theme::cyberclaw();
        // Models often wrap tables in code fences; we should render the grid
        // directly instead of a code box.
        let text = "```\n| A | B |\n|---|---|\n| 1 | 2 |\n```";
        let out = render_text(text, &t, Style::default());
        let p = plain(&out);
        let joined = p.join("\n");
        assert!(joined.contains("┌"), "grid top:\n{}", joined);
        assert!(joined.contains("A"), "header:\n{}", joined);
        assert!(joined.contains("└"), "grid bottom:\n{}", joined);
        // No code box should wrap it.
        assert!(!joined.contains("╭─"), "code box leaked:\n{}", joined);
        assert!(!joined.contains("```"), "backticks leaked:\n{}", joined);
    }
}
