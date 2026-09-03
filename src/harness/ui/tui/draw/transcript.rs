//! Transcript with bubbles, tools, diffs, scrollbar.

use crate::harness::ui::tui::anim;
use crate::harness::ui::tui::app::{App, LineKind, TranscriptLine};
use crate::harness::ui::tui::markdown;
use crate::harness::ui::tui::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme.clone();
    let tick = app.tick;
    let width = area.width.saturating_sub(2) as usize;

    let inner = Block::default()
        .borders(Borders::NONE)
        .style(Style::default().bg(theme.bg));
    frame.render_widget(inner, area);

    let mut rows: Vec<Line<'static>> = Vec::new();
    for line in &app.lines {
        rows.extend(render_line(line, &theme, width, tick, false));
        rows.push(Line::from(""));
    }
    if let Some(s) = &app.streaming {
        let stream_line = TranscriptLine {
            kind: LineKind::Assistant,
            text: s.clone(),
        };
        rows.extend(render_line(&stream_line, &theme, width, tick, true));
    }
    if let Some(b) = &app.tool_status {
        if b.pending > 0 {
            let live = TranscriptLine {
                kind: LineKind::ToolStart,
                text: b.live_label(),
            };
            rows.extend(render_line(&live, &theme, width, tick, false));
        }
    }

    let total = rows.len();
    let view_h = area.height as usize;
    app.clamp_scroll(total, view_h);

    let start = app.scroll.min(total);
    let end = (start + view_h).min(total);
    let visible: Vec<Line> = rows[start..end].to_vec();

    let para = Paragraph::new(visible)
        .style(Style::default().bg(theme.bg))
        .wrap(Wrap { trim: false });
    frame.render_widget(para, area);

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
        LineKind::User => bubble("you", &line.text, t.user_fg, t, width, false, tick),
        LineKind::Assistant => {
            let mut lines = bubble(
                if streaming {
                    "claw · streaming"
                } else {
                    "claw"
                },
                &line.text,
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
            let mut out = vec![Line::from(Span::styled(
                "  💭 reasoning",
                Style::default().fg(t.text_dim).add_modifier(Modifier::DIM),
            ))];
            for w in markdown::wrap_plain(&line.text, width.saturating_sub(4)) {
                out.push(Line::from(Span::styled(
                    format!("    {}", w),
                    Style::default().fg(t.text_dim),
                )));
            }
            out
        }
        LineKind::ToolStart => {
            let spin = anim::spinner_frame(tick);
            vec![Line::from(vec![
                Span::styled(format!("  {} ", spin), Style::default().fg(t.warn)),
                Span::styled(line.text.clone(), Style::default().fg(t.tool_fg)),
            ])]
        }
        LineKind::ToolOk => vec![Line::from(vec![
            Span::styled("  ✓ ", Style::default().fg(t.success)),
            Span::styled(
                line.text.trim_start_matches("  ✓ ").to_string(),
                Style::default().fg(t.success),
            ),
        ])],
        LineKind::ToolError => vec![Line::from(vec![
            Span::styled("  ✗ ", Style::default().fg(t.error)),
            Span::styled(
                line.text.trim_start_matches("  ✗ ").to_string(),
                Style::default().fg(t.error),
            ),
        ])],
        LineKind::System => vec![Line::from(Span::styled(
            format!("  ▸ {}", line.text),
            Style::default().fg(t.text_dim),
        ))],
        LineKind::Error => vec![Line::from(Span::styled(
            format!("  ⚠ {}", line.text),
            t.error_style(),
        ))],
        LineKind::Diff => {
            let mut out = vec![Line::from(Span::styled(
                "  ┌ diff",
                Style::default().fg(t.accent3),
            ))];
            for dl in markdown::render_diff(&line.text, t).into_iter().take(40) {
                let mut spans = vec![Span::styled(
                    "  │ ".to_string(),
                    Style::default().fg(t.border),
                )];
                spans.extend(dl.spans);
                out.push(Line::from(spans));
            }
            out.push(Line::from(Span::styled(
                "  └─────",
                Style::default().fg(t.accent3),
            )));
            out
        }
    }
}

fn bubble(
    title: &str,
    text: &str,
    fg: ratatui::style::Color,
    t: &Theme,
    width: usize,
    streaming: bool,
    tick: u64,
) -> Vec<Line<'static>> {
    let border = if streaming {
        anim::pulse_border(tick, t)
    } else {
        t.border
    };
    let inner_w = width.saturating_sub(4).max(8);
    let body = markdown::render_text(text, t, Style::default().fg(fg));
    // Re-wrap overly long plain spans simply by using wrap on original if single line huge.
    let mut body_lines: Vec<Line<'static>> = Vec::new();
    if body.len() == 1 && text.lines().count() <= 1 {
        for w in markdown::wrap_plain(text, inner_w) {
            body_lines.push(Line::from(
                markdown::render_text(&w, t, Style::default().fg(fg))
                    .into_iter()
                    .next()
                    .map(|l| l.spans)
                    .unwrap_or_default(),
            ));
        }
    } else {
        for bl in body {
            // If a rendered line is very long, hard split the content.
            let plain: String = bl.spans.iter().map(|s| s.content.as_ref()).collect();
            if plain.chars().count() > inner_w {
                for w in markdown::wrap_plain(&plain, inner_w) {
                    body_lines.extend(markdown::render_text(&w, t, Style::default().fg(fg)));
                }
            } else {
                body_lines.push(bl);
            }
        }
    }

    let title_owned = title.to_string();
    let top = Line::from(vec![
        Span::styled("╭─ ".to_string(), Style::default().fg(border)),
        Span::styled(
            title_owned,
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " {}",
                "─".repeat(inner_w.saturating_sub(title.len() + 2).min(80))
            ),
            Style::default().fg(border),
        ),
        Span::styled("╮".to_string(), Style::default().fg(border)),
    ]);
    let mut out = vec![top];
    for bl in body_lines {
        let mut spans = vec![Span::styled("│ ".to_string(), Style::default().fg(border))];
        spans.extend(bl.spans);
        out.push(Line::from(spans));
    }
    out.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(inner_w.min(100) + 1)),
        Style::default().fg(border),
    )));
    out
}

fn draw_scrollbar(
    frame: &mut Frame,
    area: Rect,
    start: usize,
    view_h: usize,
    total: usize,
    t: &Theme,
) {
    let track_h = area.height as usize;
    if track_h == 0 || total == 0 {
        return;
    }
    let thumb_h = ((view_h * track_h) / total).max(1).min(track_h);
    let max_start = total.saturating_sub(view_h).max(1);
    let thumb_y = (start * track_h.saturating_sub(thumb_h)) / max_start;

    for i in 0..track_h {
        let y = area.y + i as u16;
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
