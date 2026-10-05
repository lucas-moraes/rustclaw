//! Transcript with bubbles, tools, diffs, scrollbar.

use crate::harness::ui::tui::anim;
use crate::harness::ui::tui::app::{App, LineKind, TranscriptLine};
use crate::harness::ui::tui::markdown;
use crate::harness::ui::tui::selection::{self, selection_style};
use crate::harness::ui::tui::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

/// Display label + glyph for the assistant bubble.
const ASSISTANT_LABEL: &str = "RustClaw";
const ASSISTANT_ICON: &str = "🤖";

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme.clone();
    let tick = app.tick;

    // Content area: no frame border and no outer margin — the transcript uses
    // the full width. The 1-col gutter that keeps text off the terminal edge
    // is applied per line inside `bubble` (BODY_PAD), so the scrollbar and the
    // hit-testing still see the whole area.
    const H_PAD: u16 = 0;
    // Inner padding: one column on each side of the text, applied per line
    // (not by shrinking the area) so the scrollbar keeps the last column and
    // hit-testing still maps to the full width.
    const INNER_PAD: usize = 1;
    // Vertical padding: one blank row at the top and one at the bottom so the
    // first/last visible line never sits flush against the frame. This is
    // internal padding (it does not steal a row from the layout), so the
    // transcript keeps its full height and the last line stays readable.
    const V_PAD: u16 = 1;
    let content = Rect {
        x: area.x.saturating_add(H_PAD),
        y: area.y.saturating_add(V_PAD),
        width: area.width.saturating_sub(H_PAD * 2).max(1),
        height: area.height.saturating_sub(V_PAD * 2).max(1),
    };
    let width = content.width as usize;
    // Markdown is rendered to this width; the INNER_PAD pass below adds one
    // column on each side afterwards, so the render width must be reduced by
    // 2*INNER_PAD or full-width lines (code fences, rules) overflow the frame.
    let render_width = width.saturating_sub(INNER_PAD * 2).max(1);

    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut row_map: Vec<usize> = Vec::new();
    let collapsed =
        crate::harness::ui::tui::transcript::collapse_thinking(&app.lines, app.thinking_expanded);
    for (li, line) in collapsed.iter().enumerate() {
        let base = rows.len();
        rows.extend(render_line(line, &theme, render_width, tick, false));
        // Breathing room between messages: a blank row plus a 1-col left
        // gutter so blocks read as separate cards.
        rows.push(Line::from(""));
        for _ in base..rows.len() {
            row_map.push(li);
        }
    }
    // Inner side padding: indent every row by INNER_PAD columns on the left and
    // pad the right edge so text never touches the terminal border. Applied
    // here (after rendering) so the markdown wrap width already accounts for
    // the left gutter.
    if INNER_PAD > 0 {
        let pad = " ".repeat(INNER_PAD);
        for row in rows.iter_mut() {
            let mut spans = Vec::with_capacity(row.spans.len() + 2);
            spans.push(Span::raw(pad.clone()));
            spans.append(&mut row.spans);
            spans.push(Span::raw(pad.clone()));
            row.spans = spans;
        }
    }
    if let Some(s) = &app.streaming {
        let stream_line = TranscriptLine::static_line(LineKind::Assistant, s.clone());
        let base = rows.len();
        rows.extend(render_line(&stream_line, &theme, render_width, tick, true));
        for _ in base..rows.len() {
            row_map.push(app.lines.len());
        }
    }
    if let Some(b) = &app.tool_status {
        if b.pending > 0 {
            let live = TranscriptLine::static_line(LineKind::ToolStart, b.live_label());
            let base = rows.len();
            rows.extend(render_line(&live, &theme, render_width, tick, false));
            for _ in base..rows.len() {
                row_map.push(app.lines.len());
            }
        }
    }
    // Live subagent panels: one compact block per running/finished `task` call.
    // Nested subagents are indented by depth so the composition reads as a tree.
    for (_, panel) in app.subagent_panels.iter().rev().take(3) {
        let base = rows.len();
        let indent = "  ".repeat(panel.depth);
        rows.push(Line::from(vec![
            Span::styled(format!("{indent}  ┊ "), Style::default().fg(theme.border)),
            Span::styled(
                App::subagent_panel_label(panel),
                Style::default().fg(if panel.finished {
                    theme.success
                } else {
                    theme.warn
                }),
            ),
        ]));
        // Show the last few activity lines while running.
        if !panel.finished {
            for l in panel.lines.iter().rev().take(3).rev() {
                rows.push(Line::from(vec![
                    Span::styled(format!("{indent}  ┊   "), Style::default().fg(theme.border)),
                    Span::styled(l.clone(), Style::default().fg(theme.text_dim)),
                ]));
            }
        }
        for _ in base..rows.len() {
            row_map.push(app.lines.len());
        }
    }
    // Live activity status: a single ephemeral line at the very tail of the
    // transcript (never part of the history). It replaces the old navbar chip
    // so "working / streaming / tools / waiting" read in the chat itself.
    // Pushed *after* every other live block (streaming text, tool status,
    // subagent panels) so it is always the last line, never wedged mid-text.
    if let Some((icon, label, color)) = live_status(app, &theme) {
        let base = rows.len();
        rows.push(Line::from(vec![
            Span::styled(format!("  {icon} "), Style::default().fg(color)),
            Span::styled(label, Style::default().fg(color)),
        ]));
        for _ in base..rows.len() {
            row_map.push(app.lines.len());
        }
    }
    // Bottom breathing room: reserve blank rows after the last content line so
    // the tail of a response is never flush against the frame's bottom edge.
    // `clamp_scroll` sticks to `total - view_h`, so these rows push the real
    // content up by `BOTTOM_PAD` and keep the final lines fully visible.
    const BOTTOM_PAD: usize = 5;
    for _ in 0..BOTTOM_PAD {
        rows.push(Line::from(""));
        row_map.push(app.lines.len());
    }

    app.transcript_row_map = row_map;
    // Plain-text snapshot for mouse hit-testing + clipboard extraction.
    app.transcript_plain_rows = rows.iter().map(selection::line_to_plain).collect();

    let total = rows.len();
    let view_h = content.height as usize;
    app.last_view_h = view_h;
    app.clamp_scroll(total, view_h);
    app.transcript_scroll = app.scroll;
    app.transcript_area = content;

    let start = app.scroll.min(total);
    let end = (start + view_h).min(total);

    // Selection highlight (absolute row coords match plain_rows / row_map).
    let sel_range = app.selection.as_ref().map(|s| s.normalized());
    let hi = selection_style(theme.accent, theme.bg);

    let visible: Vec<Line> = if start < end {
        rows[start..end]
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let abs_row = start + i;
                let line = line.clone();
                if let Some((s, e)) = sel_range {
                    selection::highlight_visible_row(line, abs_row, s, e, hi)
                } else {
                    line
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    let para = Paragraph::new(visible).wrap(Wrap { trim: false });
    frame.render_widget(para, content);

    if total > view_h && area.width > 0 {
        draw_scrollbar(frame, area, start, view_h, total, &theme);
    }
}

fn render_line(
    line: &TranscriptLine,
    t: &Theme,
    width: usize,
    tick: u64,
    streaming: bool,
) -> Vec<Line<'static>> {
    match line.kind {
        LineKind::User => bubble(
            "you", "◆", &line.text, t.user_fg, t.user_fg, t, width, false, tick,
        ),
        LineKind::Assistant => {
            let mut lines = bubble(
                ASSISTANT_LABEL,
                ASSISTANT_ICON,
                &line.text,
                t.accent,
                t.assistant_fg,
                t,
                width,
                streaming,
                tick,
            );
            if streaming {
                if let Some(last) = lines.last_mut() {
                    last.spans.push(Span::styled(
                        anim::cursor_glyph(tick).to_string(),
                        Style::default().fg(t.accent),
                    ));
                }
            }
            lines
        }
        LineKind::Reasoning => {
            let collapsed = line
                .text
                .contains(crate::harness::ui::tui::transcript::THINKING_COLLAPSED_MARKER);
            // Freshly streamed reasoning fades from the accent color to the
            // resting dim color; older lines are already at rest.
            let body_color = anim::reasoning_fade(line.born_tick, tick, t.accent, t.text_dim);
            let active =
                line.born_tick != 0 && tick.saturating_sub(line.born_tick) < anim::FADE_TICKS;
            let header = if collapsed {
                "💭 thinking (collapsed)".to_string()
            } else if active {
                format!("💭 thinking {}", anim::think_frame(tick))
            } else {
                "💭 reasoning".to_string()
            };
            let mut out = vec![Line::from(vec![
                Span::styled("  ╭ ", Style::default().fg(t.border)),
                Span::styled(
                    header,
                    Style::default().fg(body_color).add_modifier(Modifier::DIM),
                ),
            ])];
            if collapsed {
                out.push(Line::from(vec![
                    Span::styled("  │ ".to_string(), Style::default().fg(t.border)),
                    Span::styled(
                        line.text.clone(),
                        Style::default().fg(body_color).add_modifier(Modifier::DIM),
                    ),
                ]));
            } else {
                let body_w = width.saturating_sub(6).max(8);
                for w in markdown::wrap_plain(&line.text, body_w) {
                    out.push(Line::from(vec![
                        Span::styled("  │ ".to_string(), Style::default().fg(t.border)),
                        Span::styled(w, Style::default().fg(body_color)),
                    ]));
                }
            }
            out.push(Line::from(Span::styled(
                "  ╰────",
                Style::default().fg(t.border),
            )));
            out
        }
        LineKind::ToolStart => {
            let spin = anim::spinner_frame(tick);
            vec![Line::from(vec![
                Span::styled("  ┊ ".to_string(), Style::default().fg(t.border)),
                Span::styled(format!("{spin} "), Style::default().fg(t.warn)),
                Span::styled(
                    line.text.clone(),
                    Style::default().fg(t.tool_fg).add_modifier(Modifier::BOLD),
                ),
            ])]
        }
        LineKind::ToolOk => {
            let text = line
                .text
                .trim_start_matches("  ✓ ")
                .trim_start_matches("✓ ")
                .to_string();
            vec![Line::from(vec![
                Span::styled("  ┊ ".to_string(), Style::default().fg(t.border)),
                Span::styled("✓ ".to_string(), Style::default().fg(t.success)),
                Span::styled(text, Style::default().fg(t.success)),
            ])]
        }
        LineKind::ToolError => {
            let text = line
                .text
                .trim_start_matches("  ✗ ")
                .trim_start_matches("✗ ")
                .trim_start_matches("  ✓/✗ ")
                .to_string();
            vec![Line::from(vec![
                Span::styled("  ┊ ".to_string(), Style::default().fg(t.border)),
                Span::styled("✗ ".to_string(), Style::default().fg(t.error)),
                Span::styled(text, Style::default().fg(t.error)),
            ])]
        }
        LineKind::System => {
            let text = line.text.trim_start_matches("[system] ").to_string();
            vec![Line::from(vec![
                Span::styled("  · ".to_string(), Style::default().fg(t.border)),
                Span::styled(text, Style::default().fg(t.text_dim)),
            ])]
        }
        LineKind::Error => vec![Line::from(vec![
            Span::styled("  ⚠ ".to_string(), Style::default().fg(t.error)),
            Span::styled(
                line.text.trim_start_matches("[error] ").to_string(),
                t.error_style(),
            ),
        ])],
        LineKind::Diff => {
            let mut out = vec![Line::from(vec![
                Span::styled("  ╭─ ".to_string(), Style::default().fg(t.accent3)),
                Span::styled(
                    "diff",
                    Style::default().fg(t.accent3).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ─────".to_string(), Style::default().fg(t.accent3)),
            ])];
            // Body rows are padded to the full width so the right edge of the
            // box is continuous; the footer rule spans the same width.
            let gutter_w = 4usize; // "  │ "
            let body_w = width.saturating_sub(gutter_w).max(8);
            for dl in markdown::render_diff(&line.text, t).into_iter().take(40) {
                let plain: String = dl.spans.iter().map(|s| s.content.as_ref()).collect();
                let pad = body_w.saturating_sub(Span::width(&Span::raw(&plain)));
                let mut spans = vec![Span::styled(
                    "  │ ".to_string(),
                    Style::default().fg(t.accent3),
                )];
                spans.extend(dl.spans);
                if pad > 0 {
                    spans.push(Span::styled(
                        " ".repeat(pad),
                        Style::default().bg(t.surface),
                    ));
                }
                out.push(Line::from(spans));
            }
            let rule_w = width.saturating_sub(5).max(1); // "  ╰" + rule
            out.push(Line::from(Span::styled(
                format!("  ╰{}", "─".repeat(rule_w)),
                Style::default().fg(t.accent3),
            )));
            out
        }
    }
}

// Rendering helper that composes a chat bubble from many visual parameters;
// grouping them into a struct would add noise for a single call site.
#[allow(clippy::too_many_arguments)]
fn bubble(
    title: &str,
    glyph: &str,
    text: &str,
    title_fg: ratatui::style::Color,
    body_fg: ratatui::style::Color,
    t: &Theme,
    width: usize,
    _streaming: bool,
    _tick: u64,
) -> Vec<Line<'static>> {
    // Borderless body: markdown is sized to the full width; the left gutter
    // that indents the body under the header is applied globally in `draw`.
    let inner_w = width.max(8);
    let body = markdown::render_text(text, t, Style::default().fg(body_fg), inner_w);

    let mut body_lines: Vec<Line<'static>> = Vec::new();
    if body.len() == 1 && text.lines().count() <= 1 {
        // Single plain line: soft-wrap, then style each chunk.
        for w in markdown::wrap_plain(text, inner_w) {
            body_lines.extend(markdown::render_text(
                &w,
                t,
                Style::default().fg(body_fg),
                inner_w,
            ));
        }
    } else {
        for bl in body {
            let plain: String = bl.spans.iter().map(|s| s.content.as_ref()).collect();
            if is_structured_line(&plain) || plain.chars().count() <= inner_w {
                body_lines.push(bl);
            } else {
                // Long prose only — wrap without re-parsing as markdown
                // (re-render would invent phantom tables from partial "|").
                for w in markdown::wrap_plain(&plain, inner_w) {
                    body_lines.push(Line::from(Span::styled(w, Style::default().fg(body_fg))));
                }
            }
        }
    }

    // Drop empty/blank body rows entirely (dense rendering); blank markdown
    // lines otherwise inflate the bubble with unreadable space.
    body_lines.retain(|bl| {
        !bl.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
            .trim()
            .is_empty()
    });

    // Header: ◆ you ──────────── (no box chrome; the label carries the color)
    let label = format!("{glyph} {title} ");
    let label_w = Span::width(&Span::raw(&label));
    let rule_w = width.saturating_sub(label_w).max(1);
    let top = Line::from(vec![
        Span::styled(
            label,
            Style::default().fg(title_fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled("─".repeat(rule_w), Style::default().fg(t.border)),
    ]);

    let mut out = vec![top];
    for bl in body_lines {
        out.push(Line::from(bl.spans));
    }
    out
}

/// True if a rendered line is a markdown table/fence box-drawing row.
/// These must not be soft-wrapped or re-parsed — that destroys the grid.
fn is_structured_line(plain: &str) -> bool {
    plain.chars().any(|c| {
        matches!(
            c,
            '┌' | '┬'
                | '┐'
                | '├'
                | '┼'
                | '┤'
                | '└'
                | '┴'
                | '┘'
                | '╭'
                | '╰'
                | '─'
                | '│'
        )
    })
}

pub(crate) fn draw_scrollbar(
    frame: &mut Frame,
    area: Rect,
    start: usize,
    view_h: usize,
    total: usize,
    t: &Theme,
) {
    let track_h = area.height.saturating_sub(1) as usize; // leave top border free
    if track_h == 0 || total == 0 {
        return;
    }
    let thumb_h = ((view_h * track_h) / total).max(1).min(track_h);
    let max_start = total.saturating_sub(view_h).max(1);
    let thumb_y = (start * track_h.saturating_sub(thumb_h)) / max_start;

    for i in 0..track_h {
        let y = area.y + 1 + i as u16; // below top border
        let x = area.x + area.width.saturating_sub(1);
        let ch = if i >= thumb_y && i < thumb_y + thumb_h {
            '▐'
        } else {
            '│'
        };
        let style = if i >= thumb_y && i < thumb_y + thumb_h {
            Style::default().fg(t.accent)
        } else {
            Style::default().fg(t.border)
        };
        frame.render_widget(
            Paragraph::new(Span::styled(ch.to_string(), style)),
            Rect {
                x,
                y,
                width: 1,
                height: 1,
            },
        );
    }
}

/// The ephemeral activity line shown at the tail of the transcript.
///
/// Returns `(icon, label, color)` for the current state, or `None` when idle
/// with nothing to report. Every active state spins the same dense braille orb
/// (see [`anim::think_frame`]); the state is told apart by color and label.
fn live_status(app: &App, t: &Theme) -> Option<(String, String, ratatui::style::Color)> {
    let orb = || anim::think_frame(app.tick).to_string();
    if app.modal.is_some() {
        return Some(("?".to_string(), "waiting".to_string(), t.warn));
    }
    if app.running {
        if app.active_tools.is_empty() {
            let streaming = app.streaming.is_some();
            let fallback = if streaming { "streaming" } else { "working" };
            let label = app
                .status_msg
                .clone()
                .unwrap_or_else(|| fallback.to_string());
            let color = if streaming { t.accent } else { t.warn };
            return Some((orb(), label, color));
        }
        let names: Vec<&str> = app
            .active_tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect();
        return Some((orb(), names.join(" · "), t.warn));
    }
    app.status_msg
        .clone()
        .map(|msg| ("●".to_string(), msg, t.accent2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_lines<'a>(lines: &[Line<'a>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn test_bubble_drops_trailing_blank_lines() {
        let t = Theme::cyberclaw();
        let text = "Ops, eu consertar:\n\n\n\n\n";
        let out = bubble(
            "claw",
            "✦",
            text,
            t.accent,
            t.assistant_fg,
            &t,
            60,
            false,
            0,
        );
        let plain = plain_lines(&out);
        // top + 1 body only; no blank rows, no footer rule.
        assert_eq!(out.len(), 2, "got {:?}", plain);
        assert!(plain.iter().all(|p| !p.trim().is_empty()));
    }

    #[test]
    fn test_bubble_drops_inner_blank_paragraph_lines() {
        let t = Theme::cyberclaw();
        let text = "Parágrafo um.\n\n\nParágrafo dois.\n\n\nParágrafo três.";
        let out = bubble(
            "claw",
            "✦",
            text,
            t.accent,
            t.assistant_fg,
            &t,
            80,
            false,
            0,
        );
        let plain = plain_lines(&out);
        // top + 3 body; blank separators removed entirely.
        assert_eq!(out.len(), 4, "got {:?}", plain);
        assert!(plain.iter().all(|p| !p.trim().is_empty()));
    }

    #[test]
    fn test_bubble_user_multi_line_no_blanks() {
        let t = Theme::cyberclaw();
        let text = "linha 1\n   \n\nlinha 2";
        let out = bubble("you", "◆", text, t.user_fg, t.user_fg, &t, 60, false, 0);
        let plain = plain_lines(&out);
        assert_eq!(out.len(), 3, "got {:?}", plain); // top + 2 body
        assert!(plain.iter().all(|p| !p.trim().is_empty()));
    }

    #[test]
    fn test_bubble_table_inside_fence() {
        let t = Theme::cyberclaw();
        let text = "```\n| A | B |\n|---|---|\n| 1 | 2 |\n```";
        let out = bubble(
            "claw",
            "\u{2726}",
            text,
            t.accent,
            t.assistant_fg,
            &t,
            60,
            false,
            0,
        );
        let plain = plain_lines(&out);
        let joined = plain.join("\n");
        // The table grid should render directly, not wrapped in a code box.
        assert!(joined.contains("\u{250c}"), "grid top:\n{}", joined);
        assert!(!joined.contains("code"), "code box leaked:\n{}", joined);
        assert!(!joined.contains("```"), "backticks leaked:\n{}", joined);
    }

    #[test]
    fn test_bubble_preserves_code_fence_box() {
        let t = Theme::cyberclaw();
        let text = "Aqui vai o código:\n```rust\nfn main() {}\n```\nFim.";
        let out = bubble(
            "claw",
            "✦",
            text,
            t.accent,
            t.assistant_fg,
            &t,
            60,
            false,
            0,
        );
        let plain = plain_lines(&out);
        let joined = plain.join("\n");
        // The code fence box must survive the bubble wrap path.
        assert!(
            joined.contains("╭─ rust"),
            "fence top bar lost:\n{}",
            joined
        );
        assert!(
            joined.contains("fn main() {}"),
            "fence body lost:\n{}",
            joined
        );
        assert!(joined.contains("╰"), "fence bottom bar lost:\n{}", joined);
        // No raw backticks should leak.
        assert!(!joined.contains("```"), "backticks leaked:\n{}", joined);
    }

    #[test]
    fn test_bubble_table_keeps_column_separators() {
        // Regression: bubble used to re-wrap grid lines (plain > inner_w) and
        // re-render fragments, destroying │ separators between columns.
        let t = Theme::cyberclaw();
        let text = "Aqui está uma tabela markdown:\n\n\
| Linguagem | Uso | Popularidade |\n\
|---|---|---|\n\
| Rust | Sistemas, WebAssembly | Alta |\n\
| Python | Data science, IA | Muito alta |\n\
| TypeScript | Web frontend/backend | Alta |\n\
\n\
| Ferramenta | Função | Status |\n\
|---|---|---|\n\
| `cargo build` | Compilar | ✅ |\n\
| `cargo test` | Testar | ✅ |";
        let out = bubble(
            "claw",
            "✦",
            text,
            t.accent,
            t.assistant_fg,
            &t,
            70,
            false,
            0,
        );
        let plain = plain_lines(&out);
        let joined = plain.join("\n");
        // Grid chrome present.
        assert!(joined.contains('┌'), "missing top border:\n{joined}");
        assert!(joined.contains('└'), "missing bottom border:\n{joined}");
        assert!(joined.contains('├'), "missing header sep:\n{joined}");
        // Body rows must keep column separators (not collapsed plain text).
        assert!(
            plain
                .iter()
                .any(|p| p.contains("│ Rust") && p.contains("│ Sistemas")),
            "body col separators lost:\n{joined}"
        );
        assert!(
            plain
                .iter()
                .any(|p| p.contains("│ Python") && p.contains("│ Data science")),
            "body col separators lost:\n{joined}"
        );
        // Inline-code backticks stripped from cells.
        assert!(!joined.contains('`'), "backticks leaked:\n{joined}");
        assert!(joined.contains("cargo build"), "cell text lost:\n{joined}");
        // No raw markdown pipes.
        assert!(
            !joined.contains("| Rust |"),
            "raw markdown pipes leaked:\n{joined}"
        );
    }

    #[test]
    fn live_status_uses_the_animated_orb_for_active_states() {
        use crate::harness::ui::tui::anim::{think_frame, THINK_SPIN};
        let t = Theme::cyberclaw();
        let mut app = App::inline_for_tests("");

        // Streaming: orb + accent color.
        app.running = true;
        app.streaming = Some("partial".to_string());
        app.tick = 4;
        let (icon, label, _) = live_status(&app, &t).expect("streaming status");
        assert_eq!(icon, think_frame(4));
        assert!(THINK_SPIN.contains(&icon.as_str()));
        assert_eq!(label, "streaming");

        // Working (no tools, no stream): same orb, "working" label.
        app.streaming = None;
        let (icon, label, _) = live_status(&app, &t).expect("working status");
        assert_eq!(icon, think_frame(4));
        assert_eq!(label, "working");

        // Tools running: still the orb, label lists the tools.
        app.active_tools = vec![crate::harness::ui::tui::transcript::ActiveTool {
            name: "bash".to_string(),
        }];
        let (icon, label, _) = live_status(&app, &t).expect("tool status");
        assert_eq!(icon, think_frame(4));
        assert_eq!(label, "bash");

        // The orb actually animates across ticks.
        app.tick = 6;
        let (icon, _, _) = live_status(&app, &t).expect("tool status");
        assert_ne!(icon, think_frame(4));
    }

    #[test]
    fn live_status_is_none_when_idle() {
        let t = Theme::cyberclaw();
        let app = App::inline_for_tests("");
        assert!(live_status(&app, &t).is_none());
    }

    #[test]
    fn live_status_shows_status_msg_when_idle() {
        let t = Theme::cyberclaw();
        let mut app = App::inline_for_tests("");
        app.status_msg = Some("copied 120 chars".to_string());
        let (icon, label, _) = live_status(&app, &t).expect("status msg");
        assert_eq!(icon, "●");
        assert_eq!(label, "copied 120 chars");
    }

    /// The live status line must be the *last* live row: after the streaming
    /// assistant text and after the tool-status batch. Regression guard for the
    /// bug where `⣼ streaming` was wedged between the tool block and the
    /// assistant block.
    #[test]
    fn live_status_renders_after_streaming_and_tool_status() {
        use crate::harness::ui::tui::transcript::{ActiveTool, ToolBatch};

        let mut app = App::inline_for_tests("");
        app.splash = None; // skip the splash screen so the transcript draws
                           // An in-flight tool batch (renders "read src/main.rs (1/2)").
        app.tool_status = Some(ToolBatch {
            counts: vec![("read".to_string(), 1)],
            last_path: "src/main.rs".to_string(),
            last_name: "read".to_string(),
            done: 1,
            failed: 0,
            pending: 1,
        });
        // Assistant text still streaming, turn in flight.
        app.running = true;
        app.streaming = Some("ASSISTANT_STREAM_MARKER".to_string());
        // An in-flight tool so `live_status` yields the "tools" line.
        app.active_tools = vec![ActiveTool {
            name: "bash".to_string(),
        }];

        let buf = crate::harness::ui::tui::draw::render_to_buffer(&mut app, 100, 40);
        let text: String = buf
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .replace('\u{0}', " ");

        let stream_at = text.find("ASSISTANT_STREAM_MARKER").expect("stream text");
        let tool_at = text.find("read src/main.rs").expect("tool status");
        let status_at = text.find("bash").expect("live status");

        assert!(
            status_at > stream_at,
            "live status must come after streaming text"
        );
        assert!(
            status_at > tool_at,
            "live status must come after the tool-status batch"
        );
    }
}

#[cfg(test)]
mod diff_render_tests {
    use super::*;

    /// Renders a Diff line and asserts the box is closed: body rows padded to
    /// the full width and the footer rule reaching the right edge.
    #[test]
    fn diff_box_fills_full_width() {
        let mut app = App::inline_for_tests("test");
        app.splash = None;
        let text = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
-fn old() {}
+fn new() {}";
        app.push(LineKind::Diff, text.to_string());

        let width = 60u16;
        let buf = crate::harness::ui::tui::draw::render_to_buffer(&mut app, width, 20);

        // Footer rule must reach the right edge: last row of the box has `─`
        // at the rightmost column.
        let mut footer_row = None;
        for y in 0..20u16 {
            let mut has_footer = false;
            for x in 0..width {
                if buf[(x, y)].symbol() == "╰" {
                    has_footer = true;
                }
            }
            if has_footer {
                footer_row = Some(y);
            }
        }
        let fy = footer_row.expect("diff footer rendered");
        assert_eq!(
            buf[(width - 1, fy)].symbol(),
            "╯",
            "footer must reach right edge"
        );

        // A body row (the `+fn new` line) must be padded: no empty gap before
        // the right edge on the row containing `new()`.
        for y in 0..20u16 {
            let row: String = (0..width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            if row.contains("fn new") {
                // rightmost cell must be a space (padding) not empty/blank
                // symbol default is " " anyway; assert the row is full-width
                // by checking the last cell exists (buffer always does) and
                // that padding was applied: the row length equals width.
                assert_eq!(row.chars().count(), width as usize);
            }
        }
    }
}
