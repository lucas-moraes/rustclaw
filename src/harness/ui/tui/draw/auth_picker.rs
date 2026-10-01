//! `/auth` picker overlay: choose which provider/service token to set.

use crate::harness::ui::tui::app::App;
use crate::harness::ui::tui::draw::centered_rect_fixed;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(picker) = app.auth_picker.as_mut() else {
        return;
    };
    let t = &app.theme;
    let n = picker.items.len() as u16;
    let h = (6 + n).min(area.height.saturating_sub(2)).min(40);
    let parea = centered_rect_fixed(72, h.max(10), area);
    frame.render_widget(Clear, parea);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent2))
        .title(Span::styled(
            " auth tokens ",
            Style::default().fg(t.accent2).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(parea);
    frame.render_widget(block, parea);

    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  Enter: set token · d: delete token · Esc: cancel · ↑↓ navigate",
        Style::default().fg(t.text_dim),
    ))];

    let mut visible = inner.height.saturating_sub(3) as usize;
    let has_above = picker.scroll_offset > 0;
    let has_below = picker.scroll_offset + visible < picker.items.len();
    // Reserve a line for the scroll indicator when scrolling is possible.
    if has_above || has_below {
        visible = visible.saturating_sub(1);
    }
    picker.ensure_selected_visible(visible);
    let start = picker.scroll_offset.min(picker.items.len());
    let end = (start + visible).min(picker.items.len());

    let mut last_section = "";
    for i in start..end {
        let item = &picker.items[i];
        if item.section != last_section {
            last_section = item.section;
            lines.push(Line::from(Span::styled(
                format!("  {}", item.section),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            )));
        }
        let sel = i == picker.selected;
        let bg = if sel { t.bg } else { t.surface };
        let arrow = if sel { "▸" } else { " " };
        let (mark, mark_style) = if item.has_token {
            ("✓", Style::default().fg(t.accent))
        } else {
            ("✗", Style::default().fg(t.text_dim))
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", arrow), Style::default().fg(t.accent).bg(bg)),
            Span::styled(format!("{} ", mark), mark_style.bg(bg)),
            Span::styled(
                item.name.clone(),
                Style::default()
                    .fg(if sel { t.text_bright } else { t.text })
                    .bg(bg)
                    .add_modifier(if sel {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]));
    }

    // Scroll indicator when there are more items above/below.
    if has_above || has_below {
        let mut spans = vec![Span::styled("  ", Style::default())];
        if has_above {
            spans.push(Span::styled("↑ ", Style::default().fg(t.accent)));
        }
        if has_below {
            spans.push(Span::styled("↓ ", Style::default().fg(t.accent)));
        }
        spans.push(Span::styled(
            "scroll · ↑↓/j k · PgUp/PgDn".to_string(),
            Style::default().fg(t.text_dim),
        ));
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ✓ token stored · ✗ no token yet — Enter sets/overwrites, d deletes",
        Style::default().fg(t.text_dim),
    )));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
