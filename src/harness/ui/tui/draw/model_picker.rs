//! `/models` picker overlay (provider → model) and `/auth` token prompt.

use crate::harness::ui::tui::app::{App, AuthPromptState, ModelPickerState};
use crate::harness::ui::tui::draw::centered_rect_fixed;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw_picker(frame: &mut Frame, app: &App, picker: &ModelPickerState, area: Rect) {
    let t = &app.theme;
    let items = picker.items();
    let n = items.len() as u16;
    let stage = if picker.stage_models {
        format!(" models · {}", picker.provider)
    } else {
        " select provider ".to_string()
    };
    let h = (6 + n).min(area.height.saturating_sub(2)).min(40);
    let parea = centered_rect_fixed(64, h.max(10), area);
    frame.render_widget(Clear, parea);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent2))
        .title(Span::styled(
            format!(" models · {} ", stage.trim()),
            Style::default().fg(t.accent2).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(parea);
    frame.render_widget(block, parea);

    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  Enter select · Esc back/cancel · ↑↓ navigate",
        Style::default().fg(t.text_dim),
    ))];

    if let Some(form) = &picker.add_provider {
        // Multi-field form for adding a user-defined provider.
        let labels = ["name", "base_url", "default_model"];
        lines.push(Line::from(""));
        for (i, label) in labels.iter().enumerate() {
            let active = i == form.field;
            let value = &form.fields[i];
            let display = if i == 0 && value.is_empty() {
                "my-provider".to_string()
            } else if i == 1 && value.is_empty() {
                "https://api.example.com/v1".to_string()
            } else {
                value.clone()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {}: ", label),
                    Style::default()
                        .fg(if active { t.accent } else { t.text_dim })
                        .add_modifier(if active {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    display,
                    Style::default().fg(t.text_bright).add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
                ),
                if active {
                    Span::styled("▏", Style::default().fg(t.accent))
                } else {
                    Span::styled("", Style::default())
                },
            ]));
        }
        lines.push(Line::from(Span::styled(
            "  Tab/↑↓ switch field · Enter save · Esc cancel",
            Style::default().fg(t.text_dim),
        )));
    } else if let Some(inp) = &picker.custom_input {
        // Free-text custom model input (masked display not needed for models).
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" model: ", Style::default().fg(t.text_dim)),
            Span::styled(
                inp.clone(),
                Style::default()
                    .fg(t.text_bright)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("▏", Style::default().fg(t.accent)),
        ]));
        lines.push(Line::from(Span::styled(
            "  Enter confirm · Esc back to list",
            Style::default().fg(t.text_dim),
        )));
    } else {
        let visible = inner.height.saturating_sub(3) as usize;
        let current = if picker.stage_models {
            app.runtime.config.model.clone()
        } else {
            app.runtime.config.provider.clone()
        };
        for (i, item) in items.iter().enumerate().take(visible) {
            let sel = i == picker.selected;
            let bg = if sel { t.bg } else { t.surface };
            let arrow = if sel { "▸" } else { " " };
            // Mark the currently active selection.
            let is_current = *item == current;
            let mark = if is_current { "●" } else { " " };
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", arrow), Style::default().fg(t.accent).bg(bg)),
                Span::styled(format!("{} ", mark), Style::default().fg(t.success).bg(bg)),
                Span::styled(
                    item.clone(),
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
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  selection is saved to rustclaw.json (this project)",
        Style::default().fg(t.text_dim),
    )));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub fn draw_auth(frame: &mut Frame, app: &App, prompt: &AuthPromptState, area: Rect) {
    let t = &app.theme;
    let parea = centered_rect_fixed(60, 7, area);
    frame.render_widget(Clear, parea);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent2))
        .title(Span::styled(
            format!(" auth · {} ", prompt.provider),
            Style::default().fg(t.accent2).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(parea);
    frame.render_widget(block, parea);

    let masked: String = "*".repeat(prompt.input.chars().count());
    let lines = vec![
        Line::from(Span::styled(
            "  Paste the API key for this provider.",
            Style::default().fg(t.text_dim),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(" token: ", Style::default().fg(t.text_dim)),
            Span::styled(
                masked,
                Style::default()
                    .fg(t.text_bright)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("▏", Style::default().fg(t.accent)),
        ]),
        Line::from(Span::styled(
            "  Enter save (auth.json, chmod 600) · Esc cancel",
            Style::default().fg(t.text_dim),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
