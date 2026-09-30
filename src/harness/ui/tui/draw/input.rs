//! Prompt input box with placeholder, cursor and opencode-style soft-wrap.

use crate::harness::ui::tui::anim;
use crate::harness::ui::tui::app::App;
use crate::harness::ui::tui::input::{compact_window, visual_row_col, wrap_visual};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let t = &app.theme;
    let focused = !app.running && app.modal.is_none() && app.palette.is_none();

    // Rounded frame around the prompt. The border costs 2 rows and 2 columns;
    // the layout in `draw::draw` reserves them (see `input_h`/`est_inner`).
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.border))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // One column of padding inside the border so the prompt lines up with the
    // transcript.
    const H_PAD: u16 = 1;
    app.input_inner_width = inner.width.saturating_sub(H_PAD);

    let prefix = " ";
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
    // `app.input_inner_width` already accounts for the row-0 prefix.
    let rows = wrap_visual(&app.input, app.input_inner_width as usize);
    let (crow, ccol) = visual_row_col(&rows, app.input_cursor);
    let input_chars: Vec<char> = app.input.chars().collect();
    let cur = if focused {
        anim::cursor_glyph(app.tick).to_string()
    } else {
        " ".to_string()
    };

    // Opencode-style compaction: keep the top rows, a dim "hidden" marker in
    // the middle and the cursor region at the bottom (cursor stays visible).
    // Only the visible window matters now — the hardware-cursor offset this
    // used to compute went away with `set_cursor_position`.
    let compaction = compact_window(rows.len(), crow, max_rows);
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
        } else {
            spans.push(Span::styled(" ", Style::default()));
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

    // No `set_cursor_position` here on purpose: the terminal cursor is hidden
    // for the whole TUI session (see `runner::run_tui`) and the blinking
    // `anim::CURSOR_ON` glyph rendered above is the only cursor. Positioning
    // the hardware cursor too used to overlay it on the glyph.
}

fn row_chars(chars: &[char], idxs: &[usize], from: usize, to: usize) -> String {
    let end = to.min(idxs.len());
    (from..end)
        .filter_map(|k| idxs.get(k).map(|i| chars[*i]))
        .collect()
}

/// Chip row listing images queued for the next prompt.
pub fn draw_pending_images(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let label = match app.pending_images_label() {
        Some(s) => s,
        None => return,
    };
    let line = Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(
            label,
            Style::default().fg(t.accent2).add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(t.surface)),
        area,
    );
}
