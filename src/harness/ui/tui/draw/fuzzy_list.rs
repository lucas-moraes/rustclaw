//! Shared fuzzy-list widget for picker overlays.

use crate::harness::ui::tui::fuzzy::{FuzzyItem, FuzzyList};
use crate::harness::ui::tui::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

/// Draws a titled, scrollable, filterable list. `render_row` builds one line
/// per visible item (`selected` is true for the highlighted filtered row).
#[allow(dead_code)]
pub fn draw_fuzzy_list<T, F>(
    frame: &mut Frame,
    area: Rect,
    list: &mut FuzzyList<T>,
    theme: &Theme,
    title: &str,
    mut render_row: F,
) where
    T: FuzzyItem,
    F: FnMut(&T, bool) -> Line<'static>,
{
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent2))
        .title(ratatui::text::Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme.accent2)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(theme.surface));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = inner.height as usize;
    list.ensure_visible(visible.max(1));
    let items = list.visible_items();
    let start = list.scroll_offset.min(items.len());
    let mut lines: Vec<Line> = Vec::new();
    for (row, (_orig, item)) in items.iter().enumerate().skip(start).take(visible) {
        let sel = row == list.selected;
        lines.push(render_row(item, sel));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
