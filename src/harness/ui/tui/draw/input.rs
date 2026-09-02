//! Prompt input box with placeholder and cursor.

use crate::harness::ui::tui::anim;
use crate::harness::ui::tui::app::App;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let focused = !app.running && app.modal.is_none() && app.palette.is_none();
    let border = if focused { t.border_focus } else { t.border };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
        .title(Span::styled(
            " prompt ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let prefix = "✦ › ";
    let (display, cursor_col) = if app.input.is_empty() && focused {
        let ph = anim::placeholder(app.tick);
        (
            Line::from(vec![
                Span::styled(prefix, Style::default().fg(t.accent2)),
                Span::styled(ph, Style::default().fg(t.text_dim)),
            ]),
            prefix.chars().count() as u16,
        )
    } else {
        let before: String = app.input.chars().take(app.input_cursor).collect();
        let after: String = app.input.chars().skip(app.input_cursor).collect();
        let cur = if focused {
            anim::cursor_glyph(app.tick)
        } else {
            " "
        };
        (
            Line::from(vec![
                Span::styled(prefix, Style::default().fg(t.accent2)),
                Span::styled(before, Style::default().fg(t.text_bright)),
                Span::styled(cur.to_string(), Style::default().fg(t.accent)),
                Span::styled(after, Style::default().fg(t.text)),
            ]),
            (prefix.chars().count() + app.input_cursor) as u16,
        )
    };

    frame.render_widget(
        Paragraph::new(display).style(Style::default().bg(t.surface)),
        inner,
    );

    // Set terminal cursor near the glyph for accessibility.
    if focused && inner.width > 0 {
        let x = (inner.x + cursor_col.min(inner.width.saturating_sub(1)))
            .min(inner.right().saturating_sub(1));
        frame.set_cursor_position((x, inner.y));
    }
}
