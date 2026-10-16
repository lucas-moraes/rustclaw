//! Session-memory skill picker overlay.

use crate::harness::ui::tui::app::App;
use crate::harness::ui::tui::draw::centered_rect_fixed;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let t = &app.theme;
    let Some(picker) = &mut app.skill_picker else {
        return;
    };

    let n = picker.ids.len() as u16;
    // Wide enough for a full skill id plus a readable description; clamped to
    // the terminal so narrow layouts still fit.
    let w = 96u16.min(area.width.saturating_sub(4)).max(40);
    let h = (10 + n).min(area.height.saturating_sub(2)).min(40);
    let parea = centered_rect_fixed(w, h, area);
    frame.render_widget(Clear, parea);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent2))
        .title(Span::styled(
            " session memory · choose skills ",
            Style::default().fg(t.accent2).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(parea);
    frame.render_widget(block, parea);

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            " Skills are this session's memory: checked skills are injected into the prompt.",
            Style::default().fg(t.text_dim),
        )),
        Line::from(Span::styled(
            " Pick none, or change them later with /skills.",
            Style::default().fg(t.text_dim),
        )),
        Line::from(""),
    ];

    // Row layout: " arrow box id  desc ". Everything is pre-truncated so no
    // line ever wraps (Wrap is disabled below).
    let id_col = 22usize;
    let desc_col = (inner.width as usize).saturating_sub(id_col + 7).max(10);

    let catalog = &app.runtime.skills;
    // 3 header lines + 1 blank + 1 footer.
    let visible = inner.height.saturating_sub(5) as usize;
    picker.ensure_selected_visible(visible);
    let start = picker.scroll_offset.min(picker.ids.len());
    for i in start..picker.ids.len().min(start + visible) {
        let id = &picker.ids[i];
        let checked = picker.checked[i];
        let sel = i == picker.selected;
        let bg = if sel { t.bg } else { t.surface };
        let spec = catalog.get(id);
        let desc = spec.map(|s| s.description.as_str()).unwrap_or("");
        let box_marker = if checked { "▣" } else { "□" };
        let box_fg = if checked { t.success } else { t.text_dim };
        let arrow = if sel { "▸" } else { " " };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", arrow), Style::default().fg(t.accent).bg(bg)),
            Span::styled(box_marker.to_string(), Style::default().fg(box_fg).bg(bg)),
            Span::styled(
                format!(" {:<width$}", truncate(id, id_col), width = id_col),
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
            Span::styled(
                truncate(desc, desc_col),
                Style::default().fg(t.text_dim).bg(bg),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ↑↓ ", Style::default().fg(t.accent)),
        Span::styled("nav  ", Style::default().fg(t.text_dim)),
        Span::styled("Space ", Style::default().fg(t.accent)),
        Span::styled("toggle  ", Style::default().fg(t.text_dim)),
        Span::styled("a ", Style::default().fg(t.accent2)),
        Span::styled("all/none  ", Style::default().fg(t.text_dim)),
        Span::styled("Enter ", Style::default().fg(t.success)),
        Span::styled("confirm  ", Style::default().fg(t.text_dim)),
        Span::styled("Esc ", Style::default().fg(t.error)),
        Span::styled("no skills", Style::default().fg(t.text_dim)),
    ]));

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Draws the skill chips row above the prompt input.
pub fn draw_chips(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let Some(toggles) = &app.prompt_toggles else {
        return;
    };
    if toggles.is_empty() {
        return;
    }

    let focused = app.skills_focused;
    let mut spans = vec![Span::styled(" ", Style::default().fg(t.text_dim))];
    if focused {
        spans.push(Span::styled(
            "⇥ ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ));
    }
    for (i, tg) in toggles.iter().enumerate() {
        let is_cur = focused && i == app.skills_idx;
        let fg = if tg.include { t.success } else { t.text_dim };
        let bg = if is_cur { t.bg } else { t.surface };
        let mark = if tg.include { "✓" } else { "·" };
        spans.push(Span::styled(
            format!("[{}] {} ", mark, truncate(&tg.skill_id, 14)),
            Style::default().fg(fg).bg(bg).add_modifier(if is_cur {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        ));
    }
    if !focused {
        spans.push(Span::styled("Ctrl+S", Style::default().fg(t.accent2)));
        spans.push(Span::styled(" edit", Style::default().fg(t.text_dim)));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(t.bg)),
        area,
    );
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
