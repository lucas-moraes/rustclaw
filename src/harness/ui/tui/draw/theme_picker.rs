//! Theme picker overlay (Ctrl+T / `/theme`): live preview of each palette.

use crate::harness::ui::tui::app::App;
use crate::harness::ui::tui::draw::centered_rect_fixed;
use crate::harness::ui::tui::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(picker) = app.theme_picker.as_mut() else {
        return;
    };
    let t = &app.theme;
    let items = picker.items();
    let n = items.len() as u16;
    let h = (6 + n).min(area.height.saturating_sub(2)).min(40);
    let parea = centered_rect_fixed(64, h.max(10), area);
    frame.render_widget(Clear, parea);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent2))
        .title(Span::styled(
            " theme ",
            Style::default().fg(t.accent2).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(parea);
    frame.render_widget(block, parea);

    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  Enter apply · Esc cancel · ↑↓ navigate",
        Style::default().fg(t.text_dim),
    ))];

    let visible = inner.height.saturating_sub(3) as usize;
    picker.ensure_selected_visible(visible);
    let start = picker.scroll_offset.min(items.len());
    for (i, name) in items.iter().enumerate().skip(start).take(visible) {
        let sel = i == picker.selected;
        let bg = if sel { t.bg } else { t.surface };
        let arrow = if sel { "▸" } else { " " };
        let is_current = *name == app.theme.name;
        let mark = if is_current { "●" } else { " " };
        // Preview swatch: the candidate theme's own accent colors.
        let preview = Theme::by_name(name).unwrap_or_else(Theme::cyberclaw);
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", arrow), Style::default().fg(t.accent).bg(bg)),
            Span::styled(format!("{} ", mark), Style::default().fg(t.success).bg(bg)),
            Span::styled(
                format!("{:<12}", name),
                Style::default()
                    .fg(if sel { t.text_bright } else { t.text })
                    .bg(bg)
                    .add_modifier(if sel {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled("  ", Style::default().bg(bg)),
            Span::styled("██", Style::default().fg(preview.accent).bg(bg)),
            Span::styled("██", Style::default().fg(preview.accent2).bg(bg)),
            Span::styled("██", Style::default().fg(preview.accent3).bg(bg)),
            Span::styled("██", Style::default().fg(preview.success).bg(bg)),
            Span::styled("██", Style::default().fg(preview.warn).bg(bg)),
            Span::styled("██", Style::default().fg(preview.error).bg(bg)),
            Span::styled(
                if preview.is_light() {
                    "  light"
                } else {
                    "  dark"
                },
                Style::default().fg(t.text_dim).bg(bg),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  selection is saved to config.json (global)",
        Style::default().fg(t.text_dim),
    )));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
