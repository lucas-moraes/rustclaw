//! Lightweight markdown-ish styling for assistant bubbles.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::theme::Theme;

/// Render a plain text block into styled lines (headers, code fences, inline code, lists).
pub fn render_text(text: &str, theme: &Theme, base: Style, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut fence_buf: Vec<String> = Vec::new();
    let mut fence_lang = String::new();
    let mut table_buf: Vec<String> = Vec::new();

    // Flush a buffered markdown table (if any) into styled lines.
    let flush_table = |out: &mut Vec<Line<'static>>, buf: &mut Vec<String>| {
        if !buf.is_empty() {
            out.extend(table_lines(buf, theme, width));
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
                // Closing fence — walk the buffered lines, emitting valid
                // markdown-table groups as grids and everything else as
                // plain prose (models often mix prose + tables in fences).
                if fence_contains_table(&fence_buf) {
                    let mut table_open = false;
                    for l in &fence_buf {
                        if is_table_line(l) {
                            table_buf.push(l.clone());
                            table_open = true;
                        } else if table_open {
                            flush_table(&mut out, &mut table_buf);
                            table_open = false;
                            out.extend(render_text(l, theme, Style::default(), width));
                        } else if !l.trim().is_empty() {
                            out.extend(render_text(l, theme, Style::default(), width));
                        }
                    }
                    flush_table(&mut out, &mut table_buf);
                } else {
                    out.extend(fence_lines(&fence_buf, &fence_lang, theme, false, width));
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
        out.extend(fence_lines(&fence_buf, &fence_lang, theme, true, width));
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

/// True if a fence's buffered content contains a viable markdown table:
/// runs of `|`-rows that include a header AND a dash separator. Used to pick
/// table extraction over the plain code box when models mix prose + tables.
fn fence_contains_table(buf: &[String]) -> bool {
    let mut rows = 0usize;
    let mut separator = false;
    for raw in buf {
        if !is_table_line(raw) {
            continue;
        }
        rows += 1;
        let inner = raw.trim().trim_matches('|').to_string();
        let stripped: String = inner.chars().filter(|c| *c != '|').collect();
        if !stripped.is_empty() && stripped.chars().all(|c| matches!(c, '-' | ':' | ' ')) {
            separator = true;
        }
    }
    rows >= 2 && separator
}

/// Render a buffered markdown table as a styled grid with a header row.
/// `width` is the available display width: columns are capped and cell
/// content wraps into extra physical lines so the grid never overflows.
fn table_lines(buf: &[String], theme: &Theme, width: usize) -> Vec<Line<'static>> {
    // Parse rows into cells; cells with embedded newlines keep their inner
    // lines (they become extra physical lines inside the cell).
    let mut rows: Vec<Vec<Vec<String>>> = Vec::new();
    let mut header: Option<Vec<Vec<String>>> = None;
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
        let cells: Vec<Vec<String>> = inner
            .split('|')
            .map(|c| {
                c.trim()
                    .replace('`', "")
                    .split('\n')
                    .map(str::to_string)
                    .collect()
            })
            .collect();
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

    // Column widths = max wrapped-cell line length, then shrink to fit the
    // known display width.
    let ncols = header.len();
    let mut widths: Vec<usize> = header
        .iter()
        .map(|c| c.iter().map(|l| l.chars().count()).max().unwrap_or(0))
        .collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if i < ncols {
                let w = cell.iter().map(|l| l.chars().count()).max().unwrap_or(0);
                widths[i] = widths[i].max(w);
            }
        }
    }
    if ncols == 0 || widths.iter().sum::<usize>() == 0 {
        return Vec::new();
    }
    // Borders: "│ " per col + trailing "│" + one pad space per cell ≈ 3*ncols + 1.
    let avail = width.saturating_sub(3 * ncols + 1).max(ncols);
    let total: usize = widths.iter().sum();
    if total > avail {
        // Shrink widest columns toward the mean; last resort, halve all.
        let mut excess = total.saturating_sub(avail);
        let mean = (avail / ncols).max(3);
        for w in widths.iter_mut() {
            if excess == 0 {
                break;
            }
            if *w > mean {
                let cut = (*w - mean).min(excess);
                *w -= cut;
                excess -= cut;
            }
        }
        if excess > 0 {
            let cap = (avail / ncols).max(1);
            for w in widths.iter_mut() {
                *w = (*w).min(cap).max(1);
            }
        }
    }

    // Wrap each cell to its column width; keep a line even when empty so the
    // rows stay aligned across the grid.
    let wrap_cell = |cell: &Vec<String>, w: usize| -> Vec<String> {
        let mut acc: Vec<String> = cell
            .join("\n")
            .split('\n')
            .flat_map(|l| wrap_plain(l, w.max(4)))
            .collect();
        if acc.is_empty() {
            acc.push(String::new());
        }
        acc
    };

    let rail = Style::default().fg(theme.accent3);
    let header_fg = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let body_fg = Style::default().fg(theme.text);

    let mut out = Vec::new();

    // Top border.
    out.push(hrule("┌─", "─┬─", "─┐", &widths, rail));

    // Header block.
    {
        let wrapped: Vec<Vec<String>> = (0..ncols)
            .map(|i| wrap_cell(&header[i], widths[i]))
            .collect();
        let h = wrapped.iter().map(|c| c.len()).max().unwrap_or(1);
        for py in 0..h {
            let mut spans = Vec::new();
            for (i, cell) in wrapped.iter().enumerate() {
                spans.push(Span::styled("│ ".to_string(), rail));
                let line = cell.get(py).cloned().unwrap_or_default();
                spans.push(Span::styled(
                    format!("{:<width$} ", line, width = widths[i]),
                    header_fg,
                ));
            }
            spans.push(Span::styled("│".to_string(), rail));
            out.push(Line::from(spans));
        }
    }

    // Separator under header.
    out.push(hrule("├─", "─┼─", "─┤", &widths, rail));

    // Body rows.
    for row in &rows {
        let wrapped: Vec<Vec<String>> = (0..ncols)
            .map(|i| {
                row.get(i)
                    .map(|c| wrap_cell(c, widths[i]))
                    .unwrap_or_else(|| vec![String::new()])
            })
            .collect();
        let h = wrapped.iter().map(|c| c.len()).max().unwrap_or(1);
        for py in 0..h {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (i, cell) in wrapped.iter().enumerate() {
                spans.push(Span::styled("│ ".to_string(), rail));
                let line = cell.get(py).cloned().unwrap_or_default();
                spans.push(Span::styled(
                    format!("{:<width$} ", line, width = widths[i]),
                    body_fg,
                ));
            }
            spans.push(Span::styled("│".to_string(), rail));
            out.push(Line::from(spans));
        }
    }

    // Bottom border.
    out.push(hrule("└─", "─┴─", "─┘", &widths, rail));

    out
}

/// Border rule line shaped to the column widths.
fn hrule(
    left: &str,
    mid: &str,
    right: &str,
    widths: &[usize],
    rail: ratatui::style::Style,
) -> Line<'static> {
    Line::from(Span::styled(
        format!(
            "{}{}{}",
            left,
            widths
                .iter()
                .map(|w| "─".repeat(w + 2))
                .collect::<Vec<_>>()
                .join(mid),
            right
        ),
        rail,
    ))
}

/// Build the visual lines for a code fence block: a top bar with the language,
/// body lines with a left accent rail, and a bottom rule. Top/bottom rules
/// always span the full available `width` so the box reaches the right edge.
fn fence_lines(
    buf: &[String],
    lang: &str,
    theme: &Theme,
    streaming: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let rail = Style::default().fg(theme.accent3);
    let code_fg = Style::default().fg(theme.text_bright);
    let w = width.max(8);

    // Top edge: ╭─ rust ──────────────── (fills full width)
    let label = if lang.is_empty() {
        " code ".to_string()
    } else {
        format!(" {} ", lang)
    };
    // "╭─" (2) + label + trailing ─s = w
    let rule_len = w.saturating_sub(2 + label.chars().count()).max(1);
    out.push(Line::from(vec![
        Span::styled("╭─".to_string(), rail),
        Span::styled(
            label,
            Style::default()
                .fg(theme.accent3)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("─".repeat(rule_len), rail),
    ]));

    // Body lines with left rail; soft-wrap long lines to stay inside the box.
    let body_w = w.saturating_sub(2).max(4); // after "│ "
    for l in buf {
        for chunk in wrap_plain(l, body_w) {
            out.push(Line::from(vec![
                Span::styled("│ ".to_string(), rail),
                Span::styled(chunk, code_fg),
            ]));
        }
    }

    // Bottom edge (or streaming indicator) — same full width.
    if streaming {
        out.push(Line::from(vec![
            Span::styled("│ ".to_string(), rail),
            Span::styled("▌".to_string(), Style::default().fg(theme.accent3)),
        ]));
    } else {
        out.push(Line::from(Span::styled(
            format!("╰{}", "─".repeat(w.saturating_sub(1).max(1))),
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
        let out = render_text(text, &t, Style::default(), 80);
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
    fn test_code_fence_borders_fill_width() {
        // Regression: top/bottom rules used a hardcoded ~40-col width and
        // stopped short of the right edge while body lines ran longer.
        let t = Theme::cyberclaw();
        let width = 60;
        let text = "```rust\nmessages: std::sync::Arc::new(session.messages.clone()),\n```";
        let out = render_text(text, &t, Style::default(), width);
        let p = plain(&out);
        assert!(p.len() >= 3, "got {:?}", p);
        let top = &p[0];
        let bottom = p.iter().find(|l| l.starts_with('╰')).expect("bottom");
        assert_eq!(
            top.chars().count(),
            width,
            "top border short: {:?} (len {})",
            top,
            top.chars().count()
        );
        assert_eq!(
            bottom.chars().count(),
            width,
            "bottom border short: {:?} (len {})",
            bottom,
            bottom.chars().count()
        );
        assert!(top.starts_with("╭─"), "top shape: {:?}", top);
        assert!(top.contains("rust"), "lang missing: {:?}", top);
        assert!(bottom.starts_with('╰'), "bottom shape: {:?}", bottom);
    }

    #[test]
    fn test_code_fence_no_lang() {
        let t = Theme::cyberclaw();
        let text = "```\nhello\n```";
        let out = render_text(text, &t, Style::default(), 80);
        let p = plain(&out);
        assert_eq!(p.len(), 3, "got {:?}", p);
        assert!(p[0].contains("code"), "default label: {:?}", p[0]);
    }

    #[test]
    fn test_unclosed_fence_streaming() {
        let t = Theme::cyberclaw();
        let text = "```python\nprint('hi')";
        let out = render_text(text, &t, Style::default(), 80);
        let p = plain(&out);
        // top bar + code line + streaming indicator
        assert_eq!(p.len(), 3, "got {:?}", p);
        assert!(p[2].contains("▌"), "streaming cursor: {:?}", p[2]);
    }

    #[test]
    fn test_no_backticks_in_output() {
        let t = Theme::cyberclaw();
        let text = "```js\nconsole.log(1)\n```";
        let out = render_text(text, &t, Style::default(), 80);
        let p = plain(&out);
        for line in &p {
            assert!(!line.contains("```"), "backticks leaked: {:?}", line);
        }
    }

    #[test]
    fn test_header_strips_hash() {
        let t = Theme::cyberclaw();
        let out = render_text("# Title", &t, Style::default(), 80);
        let p = plain(&out);
        assert_eq!(p[0], "Title");
    }

    #[test]
    fn test_bold_strips_asterisks() {
        let t = Theme::cyberclaw();
        let out = render_text("hello **world** end", &t, Style::default(), 80);
        let p = plain(&out);
        assert_eq!(p[0], "hello world end");
    }

    #[test]
    fn test_inline_code_strips_backticks() {
        let t = Theme::cyberclaw();
        let out = render_text("use `std::fs` here", &t, Style::default(), 80);
        let p = plain(&out);
        assert_eq!(p[0], "use std::fs here", "got {:?}", p);
        assert!(!p[0].contains('`'), "backticks leaked: {:?}", p);
    }

    #[test]
    fn test_list_bullet() {
        let t = Theme::cyberclaw();
        let out = render_text("- item one\n- item two", &t, Style::default(), 80);
        let p = plain(&out);
        assert!(p[0].contains("✦"), "bullet: {:?}", p[0]);
        assert!(p[0].contains("item one"));
    }

    #[test]
    fn test_table_renders_grid() {
        let t = Theme::cyberclaw();
        let text = "| Comando | Desc |\n|---|---|\n| build | compila |\n| test | roda |";
        let out = render_text(text, &t, Style::default(), 80);
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
        let out = render_text(text, &t, Style::default(), 80);
        let p = plain(&out);
        let joined = p.join("\n");
        assert!(p[0].contains("Tabela:"), "intro text:\n{}", joined);
        assert!(joined.contains("┌"), "table rendered:\n{}", joined);
    }

    #[test]
    fn test_table_inside_code_fence() {
        let t = Theme::cyberclaw();
        let text = "```\n| A | B |\n|---|---|\n| 1 | 2 |\n```";
        let out = render_text(text, &t, Style::default(), 80);
        let p = plain(&out);
        let joined = p.join("\n");
        assert!(joined.contains("┌"), "grid top:\n{}", joined);
        assert!(joined.contains("A"), "header:\n{}", joined);
        assert!(joined.contains("└"), "grid bottom:\n{}", joined);
        assert!(!joined.contains("╭─"), "code box leaked:\n{}", joined);
        assert!(!joined.contains("```"), "backticks leaked:\n{}", joined);
    }

    #[test]
    fn test_mixed_fence_table_and_prose() {
        // Fence with prose + table: prose renders normally, table renders as
        // a grid, no code box and no raw pipes.
        let t = Theme::cyberclaw();
        let text = "```\nTable info:\n| A | B |\n|---|---|\n| 1 | 2 |\nDone\n```";
        let out = render_text(text, &t, Style::default(), 80);
        let p = plain(&out);
        let joined = p.join("\n");
        assert!(joined.contains("┌"), "grid top:\n{}", joined);
        assert!(joined.contains("│ A │"), "grid rows:\n{}", joined);
        assert!(joined.contains("Table info:"), "prose kept:\n{}", joined);
        assert!(!joined.contains("╭─"), "code box leaked:\n{}", joined);
        assert!(
            !joined.contains("| A |"),
            "raw pipe rows leaked:\n{}",
            joined
        );
        assert!(
            !joined.contains("|---|"),
            "raw separator leaked:\n{}",
            joined
        );
    }

    #[test]
    fn test_rust_code_fence_still_code_box() {
        // Code containing '|' at start of lines must stay a code box (not a
        // phantom table): no separator row => no table extraction.
        let t = Theme::cyberclaw();
        let text = "```rust\n| band = 1;\nvec.push(x);\n```";
        let out = render_text(text, &t, Style::default(), 80);
        let p = plain(&out);
        let joined = p.join("\n");
        assert!(joined.contains("╭─"), "expected code box:\n{}", joined);
        assert!(joined.contains("| band = 1;"), "code kept:\n{}", joined);
    }
}
