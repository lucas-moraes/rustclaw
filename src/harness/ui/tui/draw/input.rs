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
    let (lines, cursor_row, cursor_col): (Vec<Line<'static>>, u16, u16) = if app.input.is_empty()
        && focused
    {
        let ph = anim::placeholder(app.tick);
        (
            vec![Line::from(vec![
                Span::styled(prefix, Style::default().fg(t.accent2)),
                Span::styled(ph, Style::default().fg(t.text_dim)),
            ])],
            0u16,
            prefix.chars().count() as u16,
        )
    } else {
        let cur = if focused {
            anim::cursor_glyph(app.tick)
        } else {
            " "
        }
        .to_string();

        // Split the input into display rows; the prefix only on the first row.
        let per_row: Vec<String> = app.input.split('\n').map(str::to_string).collect();
        // Logical cursor row/col (newlines before the cursor index).
        let mut cursor_row = 0usize;
        let mut chars_before_row = 0usize;
        for (i, c) in app.input.chars().enumerate() {
            if i == app.input_cursor {
                break;
            }
            if c == '\n' {
                cursor_row += 1;
                chars_before_row = i + 1;
            }
        }
        let cursor_in_row = app.input_cursor.saturating_sub(chars_before_row);

        let mut lines: Vec<Line> = Vec::new();
        let styled_row = |s: String, row: usize| -> Line {
            // Insert the cursor glyph at the logical column where applicable.
            if focused && row == cursor_row && !(cursor_in_row == 0 && !cur.is_empty() && row > 0) {
                let chars: Vec<char> = s.chars().collect();
                let mut spans = Vec::new();
                if row == 0 {
                    spans.push(Span::styled(prefix, Style::default().fg(t.accent2)));
                }
                if cursor_in_row > chars.len() {
                    spans.push(Span::styled(s.clone(), Style::default().fg(t.text_bright)));
                    spans.push(Span::styled(cur.clone(), Style::default().fg(t.accent)));
                } else {
                    let b: String = chars[..cursor_in_row].iter().collect();
                    let a: String = chars[cursor_in_row..].iter().collect();
                    spans.push(Span::styled(b, Style::default().fg(t.text_bright)));
                    spans.push(Span::styled(cur.clone(), Style::default().fg(t.accent)));
                    spans.push(Span::styled(a, Style::default().fg(t.text)));
                }
                Line::from(spans)
            } else {
                let mut spans = Vec::new();
                if row == 0 {
                    spans.push(Span::styled(prefix, Style::default().fg(t.accent2)));
                }
                spans.push(Span::styled(s, Style::default().fg(t.text)));
                Line::from(spans)
            }
        };
        for (row, txt) in per_row.into_iter().enumerate() {
            lines.push(styled_row(txt, row));
        }

        let col = (prefix.chars().count() + cursor_in_row) as u16;
        (lines, cursor_row as u16, col)
    };

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.surface)),
        inner,
    );

    // Set terminal cursor at the logical position.
    if focused && inner.width > 0 {
        let x = (inner.x + cursor_col.min(inner.width.saturating_sub(1)))
            .min(inner.right().saturating_sub(1));
        let y = inner.y + cursor_row.min(inner.height.saturating_sub(1)) as u16;
        frame.set_cursor_position((x, y));
    }
}
