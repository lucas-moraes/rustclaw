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

    // Soft content frame: left accent rail + subtle top chrome.
    let frame_block = Block::default()
        .borders(Borders::LEFT | Borders::TOP)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            content_title(app),
            Style::default().fg(theme.text_dim),
        ))
        .style(Style::default().bg(theme.bg));
    let inner = frame_block.inner(area);
    frame.render_widget(frame_block, area);

    // Keep a 1-col gutter on the left and room for the scrollbar on the right.
    let content = Rect {
        x: inner.x.saturating_add(1),
        y: inner.y,
        width: inner.width.saturating_sub(2).max(1),
        height: inner.height,
    };
    let width = content.width as usize;

    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut row_map: Vec<usize> = Vec::new();
    for (li, line) in app.lines.iter().enumerate() {
        let base = rows.len();
        rows.extend(render_line(line, &theme, width, tick, false));
        // Breathing room between messages.
        rows.push(Line::from(""));
        for _ in base..rows.len() {
            row_map.push(li);
        }
    }
    if let Some(s) = &app.streaming {
        let stream_line = TranscriptLine {
            kind: LineKind::Assistant,
            text: s.clone(),
        };
        let base = rows.len();
        rows.extend(render_line(&stream_line, &theme, width, tick, true));
        for _ in base..rows.len() {
            row_map.push(app.lines.len());
        }
    }
    if let Some(b) = &app.tool_status {
        if b.pending > 0 {
            let live = TranscriptLine {
                kind: LineKind::ToolStart,
                text: b.live_label(),
            };
            let base = rows.len();
            rows.extend(render_line(&live, &theme, width, tick, false));
            for _ in base..rows.len() {
                row_map.push(app.lines.len());
            }
        }
    }
    app.transcript_row_map = row_map;

    let total = rows.len();
    let view_h = content.height as usize;
    app.clamp_scroll(total, view_h);
    app.transcript_scroll = app.scroll;
    app.transcript_area = content;

    let start = app.scroll.min(total);
    let end = (start + view_h).min(total);
    let visible: Vec<Line> = if start < end {
        rows[start..end].to_vec()
    } else {
        Vec::new()
    };

    let para = Paragraph::new(visible)
        .style(Style::default().bg(theme.bg))
        .wrap(Wrap { trim: false });
    frame.render_widget(para, content);

    if total > view_h && area.width > 0 {
        draw_scrollbar(frame, area, start, view_h, total, &theme);
    }
}

fn content_title(app: &App) -> String {
    let agent = app.session.agent.as_str();
    let model = app.runtime.config.model.clone();
    let short = if model.chars().count() > 28 {
        let t: String = model.chars().take(26).collect();
        format!("{t}…")
    } else {
        model
    };
    format!(" conversation · {agent} · {short} ")
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
                if streaming {
                    "claw · streaming"
                } else {
                    "claw"
                },
                "✦",
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
            let mut out = vec![Line::from(vec![
                Span::styled("  ╭ ", Style::default().fg(t.border)),
                Span::styled(
                    "💭 reasoning",
                    Style::default().fg(t.text_dim).add_modifier(Modifier::DIM),
                ),
            ])];
            let body_w = width.saturating_sub(6).max(8);
            for w in markdown::wrap_plain(&line.text, body_w) {
                out.push(Line::from(vec![
                    Span::styled("  │ ".to_string(), Style::default().fg(t.border)),
                    Span::styled(w, Style::default().fg(t.text_dim)),
                ]));
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
                Span::styled("  ╭ ".to_string(), Style::default().fg(t.accent3)),
                Span::styled(
                    "diff",
                    Style::default().fg(t.accent3).add_modifier(Modifier::BOLD),
                ),
            ])];
            for dl in markdown::render_diff(&line.text, t).into_iter().take(40) {
                let mut spans = vec![Span::styled(
                    "  │ ".to_string(),
                    Style::default().fg(t.border),
                )];
                spans.extend(dl.spans);
                out.push(Line::from(spans));
            }
            out.push(Line::from(Span::styled(
                "  ╰────",
                Style::default().fg(t.accent3),
            )));
            out
        }
    }
}

fn bubble(
    title: &str,
    glyph: &str,
    text: &str,
    title_fg: ratatui::style::Color,
    body_fg: ratatui::style::Color,
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
    // "  │ " prefix = 4 cols; keep body readable.
    let pad = 4usize;
    let inner_w = width.saturating_sub(pad).max(8);
    let body = markdown::render_text(text, t, Style::default().fg(body_fg));

    let mut body_lines: Vec<Line<'static>> = Vec::new();
    if body.len() == 1 && text.lines().count() <= 1 {
        for w in markdown::wrap_plain(text, inner_w) {
            body_lines.push(Line::from(
                markdown::render_text(&w, t, Style::default().fg(body_fg))
                    .into_iter()
                    .next()
                    .map(|l| l.spans)
                    .unwrap_or_default(),
            ));
        }
    } else {
        for bl in body {
            let plain: String = bl.spans.iter().map(|s| s.content.as_ref()).collect();
            if plain.chars().count() > inner_w {
                for w in markdown::wrap_plain(&plain, inner_w) {
                    body_lines.extend(markdown::render_text(&w, t, Style::default().fg(body_fg)));
                }
            } else {
                body_lines.push(bl);
            }
        }
    }

    // Rounded header: ╭─ ◆ you ────────────
    let label = format!(" {glyph} {title} ");
    let label_w = label.chars().count();
    let rule_w = width.saturating_sub(2 + label_w).max(1);
    let top = Line::from(vec![
        Span::styled("╭─".to_string(), Style::default().fg(border)),
        Span::styled(
            label,
            Style::default().fg(title_fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled("─".repeat(rule_w), Style::default().fg(border)),
    ]);

    let mut out = vec![top];
    if body_lines.is_empty() {
        out.push(Line::from(vec![
            Span::styled("│ ".to_string(), Style::default().fg(border)),
            Span::styled(" ".to_string(), Style::default().fg(body_fg)),
        ]));
    } else {
        for bl in body_lines {
            let mut spans = vec![Span::styled("│ ".to_string(), Style::default().fg(border))];
            spans.extend(bl.spans);
            out.push(Line::from(spans));
        }
    }
    // Soft footer rule (not a full box — keeps density low).
    out.push(Line::from(Span::styled(
        format!("╰{}", "─".repeat(width.saturating_sub(1).max(1))),
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
