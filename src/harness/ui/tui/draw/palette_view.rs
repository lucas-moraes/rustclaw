//! Command palette and autocomplete overlays.

use crate::harness::ui::tui::draw::{centered_rect, centered_rect_fixed};
use crate::harness::ui::tui::palette::{kind_label, AutoComplete, PaletteKind, PaletteState};
use crate::harness::ui::tui::theme::Theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn draw_palette(frame: &mut Frame, pal: &PaletteState, t: &Theme, area: Rect) {
    let parea = centered_rect(64, 55, area);
    frame.render_widget(Clear, parea);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent))
        .title(Span::styled(
            " ⌘ command palette ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));

    let inner = block.inner(parea);
    frame.render_widget(block, parea);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(inner);

    // Query
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  🔍 ", Style::default().fg(t.accent2)),
            Span::styled(pal.query.clone(), Style::default().fg(t.text_bright)),
            Span::styled("▌", Style::default().fg(t.accent)),
        ])),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(rows[1].width as usize),
            Style::default().fg(t.border),
        )),
        rows[1],
    );

    let mut list_lines = Vec::new();
    let max_show = rows[2].height as usize;
    let start = pal.selected.saturating_sub(max_show.saturating_sub(1));
    for (vis_i, &idx) in pal.filtered.iter().enumerate().skip(start).take(max_show) {
        let item = &pal.items[idx];
        let selected = vis_i == pal.selected;
        let kind_color = match item.kind {
            PaletteKind::Command => t.accent,
            PaletteKind::Agent => t.accent3,
            PaletteKind::Theme => t.accent2,
            PaletteKind::Action => t.warn,
        };
        let bg = if selected { t.bg } else { t.surface };
        let marker = if selected { "▸ " } else { "  " };
        list_lines.push(Line::from(vec![
            Span::styled(marker.to_string(), Style::default().fg(t.accent).bg(bg)),
            Span::styled(
                format!("{:<6}", kind_label(item.kind)),
                Style::default().fg(kind_color).bg(bg),
            ),
            Span::styled(
                format!(" {:<22}", truncate(&item.label, 22)),
                Style::default()
                    .fg(if selected { t.text_bright } else { t.text })
                    .bg(bg)
                    .add_modifier(if selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(
                truncate(&item.description, 36),
                Style::default().fg(t.text_dim).bg(bg),
            ),
        ]));
    }
    if list_lines.is_empty() {
        list_lines.push(Line::from(Span::styled(
            "  no matches",
            Style::default().fg(t.text_dim),
        )));
    }
    frame.render_widget(Paragraph::new(list_lines), rows[2]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ↑↓ ", Style::default().fg(t.accent)),
            Span::styled("navigate  ", Style::default().fg(t.text_dim)),
            Span::styled("Enter ", Style::default().fg(t.accent)),
            Span::styled("run  ", Style::default().fg(t.text_dim)),
            Span::styled("Esc ", Style::default().fg(t.accent2)),
            Span::styled("close", Style::default().fg(t.text_dim)),
        ])),
        rows[3],
    );
}

pub fn draw_autocomplete(
    frame: &mut Frame,
    ac: &AutoComplete,
    t: &Theme,
    input_area: Rect,
    full: Rect,
) {
    let n = ac.matches.len().min(8) as u16;
    if n == 0 {
        return;
    }
    let height = n + 2;
    let width = 56.min(full.width.saturating_sub(4));
    let y = input_area.y.saturating_sub(height);
    let area = Rect {
        x: input_area.x + 1,
        y,
        width,
        height,
    };
    let area = if area.y < full.y {
        centered_rect_fixed(width, height, full)
    } else {
        area
    };
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent2))
        .title(Span::styled(" / ", Style::default().fg(t.accent2)))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = Vec::new();
    for (i, item) in ac.matches.iter().take(8).enumerate() {
        let sel = i == ac.selected;
        let style = if sel {
            Style::default()
                .fg(t.text_bright)
                .bg(t.bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text)
        };
        lines.push(Line::from(vec![
            Span::styled(
                if sel { " ▸ " } else { "   " }.to_string(),
                Style::default()
                    .fg(t.accent)
                    .bg(if sel { t.bg } else { t.surface }),
            ),
            Span::styled(format!("{:<14}", item.label), style),
            Span::styled(
                item.description.clone(),
                Style::default()
                    .fg(t.text_dim)
                    .bg(if sel { t.bg } else { t.surface }),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn truncate(s: &str, max: usize) -> String {
    let c = s.chars().count();
    if c <= max {
        s.to_string()
    } else {
        let mut o: String = s.chars().take(max.saturating_sub(1)).collect();
        o.push('…');
        o
    }
}
