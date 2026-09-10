//! Ctrl+F transcript search modal (query input + match list).

use crate::harness::ui::tui::app::{App, SearchState};
use crate::harness::ui::tui::draw::centered_rect_fixed;
use crate::harness::ui::tui::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let Some(st) = &app.search else {
        return;
    };
    let fixed = centered_rect_fixed(64, 14, area);
    frame.render_widget(Clear, fixed);
    draw_search(frame, st, &app.theme, app.tick, fixed, app);
}

fn draw_search(frame: &mut Frame, st: &SearchState, t: &Theme, tick: u64, area: Rect, app: &App) {
    let _ = tick;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent3))
        .title(Span::styled(
            " ❯ search transcript ",
            Style::default().fg(t.accent3).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface).fg(t.text));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cursor_glyph = if (app.tick / 5).is_multiple_of(2) {
        "▌"
    } else {
        " "
    };
    let mut lines = vec![Line::from(vec![
        Span::styled("  find: ", Style::default().fg(t.text_dim)),
        Span::styled(st.input.clone(), Style::default().fg(t.text_bright)),
        Span::styled(cursor_glyph.to_string(), Style::default().fg(t.accent)),
    ])];

    if st.input.trim().is_empty() {
        lines.push(Line::from(Span::styled(
            "  type to search…",
            Style::default().fg(t.text_dim),
        )));
    } else if st.matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no matches",
            Style::default().fg(t.error),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!(
                "  {} match{}  (↑/↓ select, Enter jump)",
                st.matches.len(),
                if st.matches.len() == 1 { "" } else { "es" }
            ),
            Style::default().fg(t.text_dim),
        )));
        lines.push(Line::from(""));
        // Show a window of matches around the selection.
        let visible = inner.height.saturating_sub(5) as usize;
        let start = st.selected.saturating_sub(visible / 2).min(
            st.matches
                .len()
                .saturating_sub(visible.min(st.matches.len())),
        );
        for (i, idx) in st.matches.iter().enumerate().skip(start).take(visible) {
            let sel = i == st.selected;
            let style = if sel {
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.text)
            };
            let text = app
                .lines
                .get(*idx)
                .map(|l| l.text.clone())
                .unwrap_or_default();
            lines.push(Line::from(Span::styled(
                format!("  {:>4}  {}", idx + 1, truncate(&text, 52)),
                style,
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Enter", Style::default().fg(t.success)),
        Span::styled(" jump   ", Style::default().fg(t.text_dim)),
        Span::styled("Esc", Style::default().fg(t.accent2)),
        Span::styled(" close", Style::default().fg(t.text_dim)),
    ]));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut o: String = s.chars().take(max.saturating_sub(1)).collect();
        o.push('…');
        o
    }
}
