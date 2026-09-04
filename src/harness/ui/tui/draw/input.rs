//! Prompt input box with placeholder, cursor and opencode-style soft-wrap.

use crate::harness::ui::tui::anim;
use crate::harness::ui::tui::app::{visual_row_col, wrap_visual, App};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
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
    app.input_inner_width = inner.width;
    frame.render_widget(block, area);

    let prefix = "✦ › ";
    let prefix_len = prefix.chars().count() as u16;

    let max_rows = inner.height.max(1) as usize;
    if app.input.is_empty() && focused {
        let ph = anim::placeholder(app.tick);
        let lines = vec![Line::from(vec![
            Span::styled(prefix, Style::default().fg(t.accent2)),
            Span::styled(ph, Style::default().fg(t.text_dim)),
        ])];
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(t.surface)),
            inner,
        );
        if inner.width > 0 {
            frame.set_cursor_position((
                (inner.x + prefix_len).min(inner.right().saturating_sub(1)),
                inner.y,
            ));
        }
        return;
    }

    // Soft-wrap the input into visual rows and locate the cursor.
    let rows = wrap_visual(&app.input, inner.width as usize);
    let (crow, ccol) = visual_row_col(&rows, app.input_cursor);
    let input_chars: Vec<char> = app.input.chars().collect();
    let cur = if focused {
        anim::cursor_glyph(app.tick).to_string()
    } else {
        " ".to_string()
    };

    // Opencode-style compaction: keep the top rows, a dim "hidden" marker in
    // the middle and the cursor region at the bottom (cursor stays visible).
    let compaction = crate::harness::ui::tui::app::compact_window(rows.len(), crow, max_rows);
    let display_row = |r: usize| -> Option<u16> {
        match compaction {
            None => Some(r as u16),
            Some((head, from, _)) => {
                if r < head {
                    Some(r as u16)
                } else if r >= from {
                    // +1 for the marker line drawn between head and bottom.
                    Some((head + 1 + (r - from)) as u16)
                } else {
                    None // hidden behind the marker
                }
            }
        }
    };
    let (vis_from, vis_to) = match compaction {
        Some((_, from, to)) => (from, to),
        None => {
            let start = if rows.len() > max_rows && crow + 1 > max_rows {
                crow + 1 - max_rows
            } else {
                0
            };
            (start, (start + max_rows).min(rows.len()))
        }
    };

    let (head_rows, hidden_count) = match compaction {
        Some((head, from, _)) => (Some(head), from - head),
        None => (None, 0),
    };

    let mut lines: Vec<Line> = Vec::new();
    for (r, row) in rows.iter().enumerate().take(vis_to).skip(vis_from) {
        let idxs = &row.idxs;
        let mut spans: Vec<Span> = Vec::new();
        if r == 0 {
            spans.push(Span::styled(prefix, Style::default().fg(t.accent2)));
        }
        let row_text: String = idxs.iter().map(|i| input_chars[*i]).collect();
        if focused && r == crow {
            let b: String = row_chars(&input_chars, idxs, 0, ccol);
            let a: String = row_chars(&input_chars, idxs, ccol, idxs.len());
            spans.push(Span::styled(b, Style::default().fg(t.text_bright)));
            spans.push(Span::styled(cur.clone(), Style::default().fg(t.accent)));
            spans.push(Span::styled(a, Style::default().fg(t.text)));
        } else if r < crow {
            spans.push(Span::styled(row_text, Style::default().fg(t.text_bright)));
        } else {
            spans.push(Span::styled(row_text, Style::default().fg(t.text)));
        }
        lines.push(Line::from(spans));
    }
    // Dim marker for the collapsed middle (after the head rows).
    if let Some(head) = head_rows {
        if hidden_count > 0 {
            let marker = Line::from(Span::styled(
                format!("  ⋯ {} linha(s) oculta(s) ⋯", hidden_count),
                Style::default().fg(t.text_dim),
            ));
            lines.insert(head, marker);
        }
    }

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.surface)),
        inner,
    );

    // Terminal cursor at the logical position (visual row/col + prefix).
    if focused && inner.width > 0 {
        if let Some(drow) = display_row(crow) {
            let x = (inner.x + (prefix_len + ccol as u16).min(inner.width.saturating_sub(1)))
                .min(inner.right().saturating_sub(1));
            let y = inner.y + drow.min(inner.height.saturating_sub(1));
            frame.set_cursor_position((x, y));
        }
    }
}

fn row_chars(chars: &[char], idxs: &[usize], from: usize, to: usize) -> String {
    let end = to.min(idxs.len());
    (from..end)
        .filter_map(|k| idxs.get(k).map(|i| chars[*i]))
        .collect()
}
