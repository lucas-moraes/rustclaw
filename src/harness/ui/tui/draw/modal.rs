//! Permission and question modals.

use crate::harness::ui::tui::app::Modal;
use crate::harness::ui::tui::draw::{centered_rect, centered_rect_fixed};
use crate::harness::ui::tui::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, modal: &Modal, theme: &Theme, tick: u64, area: Rect) {
    let modal_area = centered_rect(70, 45, area);
    frame.render_widget(Clear, modal_area);

    match modal {
        Modal::Permission(req) => draw_permission(frame, req, theme, tick, modal_area),
        Modal::Question { req, draft, cursor } => {
            draw_question(frame, req, draft, *cursor, theme, tick, modal_area)
        }
        Modal::UserPrompt { .. } => {
            let fixed = centered_rect_fixed(48, 11, area);
            frame.render_widget(Clear, fixed);
            draw_user_prompt(frame, theme, fixed)
        }
    }
}

fn draw_user_prompt(frame: &mut Frame, t: &Theme, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent3))
        .title(Span::styled(
            " ❯ prompt ",
            Style::default().fg(t.accent3).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface).fg(t.text));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(Span::styled(
            "  action for the clicked prompt:",
            Style::default().fg(t.text_dim),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            key_btn("R", "revert", t.error, t),
            Span::styled(
                "  undo this prompt + replies",
                Style::default().fg(t.text_dim),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            key_btn("C", "copy", t.success, t),
            Span::styled(
                "  copy prompt to clipboard",
                Style::default().fg(t.text_dim),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("Esc", Style::default().fg(t.accent2)),
            Span::styled(" dismiss", Style::default().fg(t.text_dim)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_permission(
    frame: &mut Frame,
    req: &crate::harness::ui::tui::askers::PermissionRequest,
    t: &Theme,
    tick: u64,
    area: Rect,
) {
    let warn_icon = if (tick / 6) % 2 == 0 { "⚠" } else { "!" };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.warn))
        .title(Span::styled(
            format!(" {} permission required ", warn_icon),
            Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface).fg(t.text));

    let path = req.input.path.as_deref().unwrap_or("—");
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  tool  ", Style::default().fg(t.text_dim)),
            Span::styled(
                req.input.tool.clone(),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  path  ", Style::default().fg(t.text_dim)),
            Span::styled(path.to_string(), Style::default().fg(t.info)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", truncate(&req.input.args_summary, 200)),
            Style::default().fg(t.text),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            key_btn("Y", "allow", t.success, t),
            Span::raw("  "),
            key_btn("N", "deny", t.error, t),
            Span::raw("  "),
            key_btn("A", "always", t.accent2, t),
        ]),
    ];

    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_question(
    frame: &mut Frame,
    req: &crate::harness::ui::tui::askers::QuestionRequest,
    draft: &str,
    cursor: usize,
    t: &Theme,
    tick: u64,
    area: Rect,
) {
    // Question + options + answer box + hints.
    let opt_rows = req.options.len() as u16;
    let h = (13 + opt_rows).min(area.height).max(11);
    let w = area.width.min(76).max(44);
    let area = centered_rect_fixed(w, h, area);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent3))
        .title(Span::styled(
            " ❯ question — type your answer ",
            Style::default().fg(t.accent3).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface).fg(t.text));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut y = inner.y;
    let mut push_line = |frame: &mut Frame, line: Line<'static>, row: &mut u16| {
        if *row < inner.y + inner.height {
            frame.render_widget(
                Paragraph::new(line),
                Rect {
                    x: inner.x,
                    y: *row,
                    width: inner.width,
                    height: 1,
                },
            );
            *row = row.saturating_add(1);
        }
    };

    push_line(frame, Line::from(""), &mut y);
    // Question text (single visual row; long text is truncated to keep the input visible).
    let q = truncate(&req.question, inner.width.saturating_sub(4) as usize);
    push_line(
        frame,
        Line::from(Span::styled(
            format!("  {}", q),
            Style::default()
                .fg(t.text_bright)
                .add_modifier(Modifier::BOLD),
        )),
        &mut y,
    );
    push_line(frame, Line::from(""), &mut y);

    for (i, o) in req.options.iter().enumerate() {
        let label = truncate(o, inner.width.saturating_sub(8) as usize);
        push_line(
            frame,
            Line::from(vec![
                Span::styled(format!("  [{}] ", i + 1), Style::default().fg(t.accent)),
                Span::styled(label, Style::default().fg(t.text)),
            ]),
            &mut y,
        );
    }
    if !req.options.is_empty() {
        push_line(frame, Line::from(""), &mut y);
    }

    // Answer box (2 rows: border-like label + input).
    push_line(
        frame,
        Line::from(Span::styled(
            "  answer",
            Style::default().fg(t.text_dim).add_modifier(Modifier::BOLD),
        )),
        &mut y,
    );

    let cursor_glyph = if (tick / 5) % 2 == 0 { "▌" } else { " " };
    let chars: Vec<char> = draft.chars().collect();
    let at = cursor.min(chars.len());
    let before: String = chars[..at].iter().collect();
    let after: String = chars[at..].iter().collect();
    let input_width = inner.width.saturating_sub(4) as usize;

    let answer_line = if draft.is_empty() {
        Line::from(vec![
            Span::styled("  ┌ ", Style::default().fg(t.border_focus)),
            Span::styled(cursor_glyph.to_string(), Style::default().fg(t.accent)),
            Span::styled(
                truncate("type here, then press Enter", input_width.saturating_sub(1)),
                Style::default().fg(t.text_dim),
            ),
        ])
    } else {
        // Keep the cursor visible by windowing long drafts.
        let total = chars.len();
        let max_shown = input_width.saturating_sub(1).max(8);
        let mut start = 0usize;
        if at >= max_shown {
            start = at + 1 - max_shown;
        }
        let end = (start + max_shown).min(total);
        let vis_before: String = chars[start..at.min(end)].iter().collect();
        let vis_after: String = chars[at.min(end)..end].iter().collect();
        let _ = (before, after); // kept for clarity of cursor split
        Line::from(vec![
            Span::styled("  ┌ ", Style::default().fg(t.border_focus)),
            Span::styled(vis_before, Style::default().fg(t.text_bright)),
            Span::styled(cursor_glyph.to_string(), Style::default().fg(t.accent)),
            Span::styled(vis_after, Style::default().fg(t.text)),
        ])
    };
    let answer_row = y;
    push_line(frame, answer_line, &mut y);
    push_line(
        frame,
        Line::from(Span::styled(
            format!(
                "  └{}",
                "─".repeat(inner.width.saturating_sub(4).max(1) as usize)
            ),
            Style::default().fg(t.border_focus),
        )),
        &mut y,
    );

    push_line(frame, Line::from(""), &mut y);
    let mut hint = vec![
        Span::styled("  ", Style::default()),
        Span::styled("Enter", Style::default().fg(t.success)),
        Span::styled(" send   ", Style::default().fg(t.text_dim)),
        Span::styled("Esc", Style::default().fg(t.accent2)),
        Span::styled(" cancel", Style::default().fg(t.text_dim)),
    ];
    if !req.options.is_empty() {
        hint.extend([
            Span::styled("   ", Style::default()),
            Span::styled("1..n + Enter", Style::default().fg(t.accent)),
            Span::styled(" pick option", Style::default().fg(t.text_dim)),
        ]);
    }
    push_line(frame, Line::from(hint), &mut y);

    // Real terminal caret on the answer field.
    let col = 4u16 + {
        let max_shown = input_width.saturating_sub(1).max(8);
        let start = if at >= max_shown {
            at + 1 - max_shown
        } else {
            0
        };
        (at - start) as u16
    };
    if answer_row < inner.y + inner.height && col < inner.width {
        frame.set_cursor_position((
            (inner.x + col).min(inner.x + inner.width.saturating_sub(1)),
            answer_row,
        ));
    }
}

fn key_btn(key: &str, label: &str, color: ratatui::style::Color, t: &Theme) -> Span<'static> {
    Span::styled(
        format!("[{}] {} ", key, label),
        Style::default()
            .fg(color)
            .bg(t.bg)
            .add_modifier(Modifier::BOLD),
    )
}

fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        let mut o: String = s.chars().take(max.saturating_sub(1)).collect();
        o.push('…');
        o
    }
}
