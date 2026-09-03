//! `/resume` session picker overlay: pick a past session by title (no id).

use crate::harness::ui::tui::app::{App, ResumePickerState};
use crate::harness::ui::tui::draw::centered_rect_fixed;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, picker: &ResumePickerState, area: Rect) {
    let t = &app.theme;
    let n = picker.sessions.len() as u16;
    let h = (6 + n).min(area.height.saturating_sub(2)).min(40);
    let parea = centered_rect_fixed(70, h.max(10), area);
    frame.render_widget(Clear, parea);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent2))
        .title(Span::styled(
            " resume session ",
            Style::default().fg(t.accent2).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(parea);
    frame.render_widget(block, parea);

    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  Enter resume · Esc cancel · ↑↓ navigate",
        Style::default().fg(t.text_dim),
    ))];

    let visible = inner.height.saturating_sub(3) as usize;
    for (i, s) in picker.sessions.iter().enumerate().take(visible) {
        let sel = i == picker.selected;
        let bg = if sel { t.bg } else { t.surface };
        let arrow = if sel { "▸" } else { " " };
        let title = if s.preview.trim().is_empty() {
            "untitled session".to_string()
        } else {
            s.preview.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", arrow), Style::default().fg(t.accent).bg(bg)),
            Span::styled(
                format!("{} ", title),
                Style::default()
                    .fg(if sel { t.text_bright } else { t.text })
                    .bg(bg)
                    .add_modifier(if sel {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(
                format!("· {} · {} msgs", s.agent, s.message_count),
                Style::default().fg(t.text_dim).bg(bg),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  select a previous session to continue in that context",
        Style::default().fg(t.text_dim),
    )));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
