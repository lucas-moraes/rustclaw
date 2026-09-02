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
        Modal::Question(req) => draw_question(frame, req, theme, tick, modal_area),
    }
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
    t: &Theme,
    _tick: u64,
    area: Rect,
) {
    let h = (8 + req.options.len() as u16).min(area.height);
    let area = centered_rect_fixed(area.width, h, area);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent3))
        .title(Span::styled(
            " ❯ question ",
            Style::default().fg(t.accent3).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface).fg(t.text));

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", req.question),
            Style::default()
                .fg(t.text_bright)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    for (i, o) in req.options.iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(format!("  [{}] ", i + 1), Style::default().fg(t.accent)),
            Span::styled(o.clone(), Style::default().fg(t.text)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled("1..n", Style::default().fg(t.accent)),
        Span::styled(" choose   ", Style::default().fg(t.text_dim)),
        Span::styled("Esc", Style::default().fg(t.accent2)),
        Span::styled(" cancel", Style::default().fg(t.text_dim)),
    ]));

    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), inner);
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
