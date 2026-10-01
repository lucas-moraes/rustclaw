//! Permission and question modals.

use crate::harness::ui::tui::app::Modal;
use crate::harness::ui::tui::draw::{centered_rect, centered_rect_fixed};
use crate::harness::ui::tui::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(
    frame: &mut Frame,
    modal: &mut Modal,
    theme: &Theme,
    tick: u64,
    area: Rect,
    config: &crate::config::RuntimeConfig,
) {
    match modal {
        Modal::Permission(req) => {
            let modal_area = centered_rect(70, 45, area);
            frame.render_widget(Clear, modal_area);
            draw_permission(frame, req, theme, tick, modal_area)
        }
        Modal::Question { req, draft, cursor } => {
            // The question modal sizes itself against the full screen so it can
            // grow to use the available space for long questions/options.
            draw_question(frame, req, draft, *cursor, theme, tick, area)
        }
        Modal::UserPrompt { .. } => {
            // Search has its own draw module (draw/search.rs).
            let fixed = centered_rect_fixed(48, 11, area);
            frame.render_widget(Clear, fixed);
            draw_user_prompt(frame, theme, fixed)
        }
        Modal::Settings { selected } => {
            let fixed = centered_rect_fixed(64, 16, area);
            frame.render_widget(Clear, fixed);
            draw_settings(frame, theme, *selected, fixed, config)
        }
        Modal::Cursor { selected } => {
            let fixed = centered_rect_fixed(64, 10, area);
            frame.render_widget(Clear, fixed);
            draw_cursor(frame, theme, *selected, fixed, config)
        }
        Modal::AudioSettings {
            selected,
            custom_input,
        } => {
            let fixed = centered_rect_fixed(64, 9, area);
            frame.render_widget(Clear, fixed);
            draw_audio_settings(
                frame,
                theme,
                *selected,
                custom_input.as_deref(),
                fixed,
                config,
            )
        }
        Modal::CursorModel {
            selected,
            scroll_offset,
            filter,
            models,
            target,
        } => {
            let h = crate::harness::ui::tui::scroll::cursor_model_modal_height(
                models.len(),
                area.height,
            );
            let fixed = centered_rect_fixed(56, h, area);
            frame.render_widget(Clear, fixed);
            draw_cursor_model(
                frame,
                theme,
                fixed,
                CursorModelView {
                    selected,
                    scroll_offset,
                    filter,
                    models,
                    target: *target,
                },
            );
        }
    }
}

/// Rows of the settings modal: (label, value, toggleable).
///
/// Shared with the key handler so navigation and rendering always agree on the
/// row order and count.
pub(crate) fn settings_rows(c: &crate::config::RuntimeConfig) -> Vec<(String, String, bool)> {
    vec![
        ("provider".into(), c.provider.clone(), false),
        ("model".into(), c.model.clone(), false),
        ("max_iterations".into(), c.max_iterations.to_string(), false),
        (
            "max_context_tokens".into(),
            c.max_context_tokens.to_string(),
            false,
        ),
        (
            "turn_timeout_secs".into(),
            c.turn_timeout_secs.to_string(),
            false,
        ),
        (
            "compact_trigger_ratio".into(),
            format!("{:.2}", c.compact_trigger_ratio),
            false,
        ),
        (
            "summary_model".into(),
            if c.summary_model.is_empty() {
                "(same as model)".into()
            } else {
                c.summary_model.clone()
            },
            false,
        ),
    ]
}

/// Rows of the `/cursor` modal: (label, value, toggleable).
///
/// Mirrors `settings_rows` for the Cursor-specific knobs: `cursor_agent` is a
/// boolean toggled with Space, `cursor_model` opens the model picker on Enter.
pub(crate) fn cursor_rows(c: &crate::config::RuntimeConfig) -> Vec<(String, String, bool)> {
    vec![
        ("cursor_agent".into(), on_off(c.cursor_agent), true),
        (
            "cursor_model".into(),
            if c.cursor_model.is_empty() {
                "auto".into()
            } else {
                c.cursor_model.clone()
            },
            false,
        ),
        ("cursor_plan".into(), on_off(c.cursor_plan), true),
        (
            "cursor_plan_model".into(),
            if c.cursor_plan_model.is_empty() {
                "auto".into()
            } else {
                c.cursor_plan_model.clone()
            },
            false,
        ),
    ]
}

fn on_off(v: bool) -> String {
    if v {
        "on".into()
    } else {
        "off".into()
    }
}

/// Rows of the `/audio-settings` modal: (label, value, toggleable).
///
/// `voice_enabled` is a boolean toggled with Space; `stt_model` is edited on
/// Enter (empty = default whisper model).
pub(crate) fn audio_rows(c: &crate::config::RuntimeConfig) -> Vec<(String, String, bool)> {
    vec![
        ("voice_enabled".into(), on_off(c.voice_enabled), true),
        (
            "stt_model".into(),
            if c.stt_model.is_empty() {
                crate::config::DEFAULT_STT_MODEL.into()
            } else {
                c.stt_model.clone()
            },
            false,
        ),
    ]
}

/// The `/audio-settings` modal: push-to-talk toggle + STT model.
fn draw_audio_settings(
    frame: &mut Frame,
    t: &Theme,
    selected: usize,
    custom_input: Option<&str>,
    area: Rect,
    config: &crate::config::RuntimeConfig,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent2))
        .title(Span::styled(
            " 🎙 audio ",
            Style::default().fg(t.accent2).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface).fg(t.text));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = audio_rows(config);
    let mut lines: Vec<Line> = Vec::new();
    for (i, (label, value, toggleable)) in rows.iter().enumerate() {
        let is_sel = i == selected;
        let marker = if is_sel { "❯ " } else { "  " };
        let label_style = if is_sel {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text)
        };
        let value_style = if *toggleable {
            Style::default().fg(if config.voice_enabled {
                t.success
            } else {
                t.text_dim
            })
        } else {
            Style::default().fg(t.text_dim)
        };
        // While typing a new model name, show the live input instead of the
        // stored value.
        let shown = if is_sel && label == "stt_model" && custom_input.is_some() {
            format!("{}▏", custom_input.unwrap_or(""))
        } else {
            value.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(t.accent)),
            Span::styled(format!("{:<22}", label), label_style),
            Span::styled(shown, value_style),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ↑/↓", Style::default().fg(t.accent2)),
        Span::styled(" move  ", Style::default().fg(t.text_dim)),
        Span::styled("Space", Style::default().fg(t.accent2)),
        Span::styled(" toggle  ", Style::default().fg(t.text_dim)),
        Span::styled("Enter", Style::default().fg(t.accent2)),
        Span::styled(" model  ", Style::default().fg(t.text_dim)),
        Span::styled("Esc", Style::default().fg(t.accent2)),
        Span::styled(" close", Style::default().fg(t.text_dim)),
    ]));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_settings(
    frame: &mut Frame,
    t: &Theme,
    selected: usize,
    area: Rect,
    config: &crate::config::RuntimeConfig,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent2))
        .title(Span::styled(
            " ⚙ settings ",
            Style::default().fg(t.accent2).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface).fg(t.text));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = settings_rows(config);
    let mut lines: Vec<Line> = Vec::new();
    for (i, (label, value, toggleable)) in rows.iter().enumerate() {
        let is_sel = i == selected;
        let marker = if is_sel { "❯ " } else { "  " };
        let label_style = if is_sel {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text)
        };
        let value_style = if *toggleable {
            Style::default().fg(t.success)
        } else {
            Style::default().fg(t.text_dim)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(t.accent)),
            Span::styled(format!("{:<22}", label), label_style),
            Span::styled(value.clone(), value_style),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ↑/↓", Style::default().fg(t.accent2)),
        Span::styled(" move  ", Style::default().fg(t.text_dim)),
        Span::styled("Space", Style::default().fg(t.accent2)),
        Span::styled(" toggle  ", Style::default().fg(t.text_dim)),
        Span::styled("Esc", Style::default().fg(t.accent2)),
        Span::styled(" close", Style::default().fg(t.text_dim)),
    ]));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// The `/cursor` modal: Cursor-specific settings (toggle + model).
fn draw_cursor(
    frame: &mut Frame,
    t: &Theme,
    selected: usize,
    area: Rect,
    config: &crate::config::RuntimeConfig,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent2))
        .title(Span::styled(
            " ⬡ cursor ",
            Style::default().fg(t.accent2).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface).fg(t.text));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = cursor_rows(config);
    let mut lines: Vec<Line> = Vec::new();
    for (i, (label, value, toggleable)) in rows.iter().enumerate() {
        let is_sel = i == selected;
        let marker = if is_sel { "❯ " } else { "  " };
        let label_style = if is_sel {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text)
        };
        let value_style = if *toggleable {
            Style::default().fg(if config.cursor_agent {
                t.success
            } else {
                t.text_dim
            })
        } else {
            Style::default().fg(t.text_dim)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(t.accent)),
            Span::styled(format!("{:<22}", label), label_style),
            Span::styled(value.clone(), value_style),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  ↑/↓", Style::default().fg(t.accent2)),
        Span::styled(" move  ", Style::default().fg(t.text_dim)),
        Span::styled("Space", Style::default().fg(t.accent2)),
        Span::styled(" toggle  ", Style::default().fg(t.text_dim)),
        Span::styled("Enter", Style::default().fg(t.accent2)),
        Span::styled(" model  ", Style::default().fg(t.text_dim)),
        Span::styled("Esc", Style::default().fg(t.accent2)),
        Span::styled(" close", Style::default().fg(t.text_dim)),
    ]));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Draw state for [`draw_cursor_model`].
struct CursorModelView<'a> {
    selected: &'a mut usize,
    scroll_offset: &'a mut usize,
    filter: &'a str,
    models: &'a [(String, String)],
    target: crate::harness::ui::tui::app::state::CursorModelTarget,
}

/// Picker for the Cursor CLI model (`cursor_model`). The list comes from
/// `agent --list-models`; an empty selection means "auto".
fn draw_cursor_model(frame: &mut Frame, t: &Theme, area: Rect, view: CursorModelView<'_>) {
    let selected = view.selected;
    let scroll_offset = view.scroll_offset;
    let filter = view.filter;
    let models = view.models;
    let target = view.target;
    use crate::harness::ui::tui::app::state::cursor_model_filtered_indices;
    use crate::harness::ui::tui::scroll::{format_position_indicator, list_viewport};
    use crate::harness::ui::tui::text::truncate_to_width;

    const FILTER_ROWS: usize = 1;
    const HINT_ROWS: usize = 1;
    const PAGE_HINT: &str =
        "  ↑/↓ move  Enter select  Esc cancel  type to filter  PgUp/PgDn  Home/End";
    let title = match target {
        crate::harness::ui::tui::app::state::CursorModelTarget::Build => " cursor model ",
        crate::harness::ui::tui::app::state::CursorModelTarget::Plan => " cursor plan model ",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent2))
        .title(Span::styled(
            title,
            Style::default().fg(t.accent2).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface).fg(t.text));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(" filter> ", Style::default().fg(t.text_dim)),
        Span::styled(
            filter,
            Style::default()
                .fg(t.text_bright)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("▏", Style::default().fg(t.accent)),
    ]));

    if models.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no models found — is the `agent` CLI installed?",
            Style::default().fg(t.error),
        )));
        lines.push(Line::from(Span::styled(
            PAGE_HINT,
            Style::default().fg(t.text_dim),
        )));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    let indices = cursor_model_filtered_indices(models, filter);
    let n = indices.len();
    if n == 0 {
        lines.push(Line::from(Span::styled(
            "  no matching models",
            Style::default().fg(t.text_dim),
        )));
        lines.push(Line::from(Span::styled(
            PAGE_HINT,
            Style::default().fg(t.text_dim),
        )));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        *selected = 0;
        *scroll_offset = 0;
        return;
    }

    *selected = (*selected).min(n.saturating_sub(1));
    let inner_h = inner.height as usize;
    let list_h = inner_h.saturating_sub(FILTER_ROWS);
    let vp = list_viewport(list_h, n, HINT_ROWS);
    let item_rows = vp.item_rows.max(1);
    *scroll_offset =
        crate::harness::ui::tui::scroll::ensure_visible(*selected, *scroll_offset, item_rows, n);
    let start = *scroll_offset;
    let inner_w = inner.width as usize;
    let marker_w = 2usize;
    let avail = inner_w.saturating_sub(marker_w);

    for (row, &orig_i) in indices.iter().enumerate().skip(start).take(item_rows) {
        let (id, desc) = &models[orig_i];
        let is_sel = row == *selected;
        let marker = if is_sel { "❯ " } else { "  " };
        let style = if is_sel {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text)
        };
        let id_max = (avail * 2 / 5).clamp(8, avail);
        let desc_max = avail.saturating_sub(id_max + 1);
        let id_show = truncate_to_width(id, id_max);
        let line = if desc.is_empty() {
            format!("{}{}", marker, truncate_to_width(id, avail))
        } else {
            format!(
                "{}{} {}",
                marker,
                id_show,
                truncate_to_width(desc, desc_max)
            )
        };
        lines.push(Line::from(Span::styled(line, style)));
    }
    if vp.show_position {
        lines.push(Line::from(Span::styled(
            format_position_indicator(start, item_rows, n),
            Style::default().fg(t.text_dim),
        )));
    }
    lines.push(Line::from(Span::styled(
        PAGE_HINT,
        Style::default().fg(t.text_dim),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_user_prompt(frame: &mut Frame, t: &Theme, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent3))
        .title(Span::styled(
            " ❯ prompt ",
            Style::default().fg(t.accent3).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface).fg(t.text));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(Span::styled(
            "  action for the clicked prompt:",
            Style::default().fg(t.text_dim),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            key_btn("R", "revert", t.error, t),
            Span::styled(
                "  undo this prompt + replies",
                Style::default().fg(t.text_dim),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            key_btn("C", "copy", t.success, t),
            Span::styled(
                "  copy prompt to clipboard",
                Style::default().fg(t.text_dim),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("Esc", Style::default().fg(t.accent2)),
            Span::styled(" dismiss", Style::default().fg(t.text_dim)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_permission(
    frame: &mut Frame,
    req: &crate::harness::ui::tui::askers::PermissionRequest,
    t: &Theme,
    tick: u64,
    area: Rect,
) {
    let warn_icon = if (tick / 6).is_multiple_of(2) {
        "⚠"
    } else {
        "!"
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.warn))
        .style(Style::default().bg(t.surface).fg(t.text));

    let path = req.input.path.as_deref().unwrap_or("—");
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  tool  ", Style::default().fg(t.text_dim)),
            Span::styled(
                req.input.tool.clone(),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  path  ", Style::default().fg(t.text_dim)),
            Span::styled(path.to_string(), Style::default().fg(t.info)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", truncate(&req.input.args_summary, 200)),
            Style::default().fg(t.text),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            key_btn("Y", "allow", t.success, t),
            Span::raw("  "),
            key_btn("N", "deny", t.error, t),
            Span::raw("  "),
            key_btn("A", "always", t.accent2, t),
        ]),
    ];

    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    cyber_badge(
        frame,
        area,
        t.warn,
        &format!(" {} permission required ", warn_icon),
    );
}

/// Cyberpunk title badge: bold accent text with a diagonal `╱` cut, drawn
/// over the top-left corner of a bordered block.
///
/// `area` is the block's outer rect; the badge overlays the top border row.
/// The block must be rendered BEFORE calling this (the badge paints over it).
pub fn cyber_badge(frame: &mut Frame, area: Rect, accent: Color, title: &str) {
    if area.width < 4 || area.height < 2 {
        return;
    }
    let label = format!(" {title} ");
    let label_w = Span::width(&Span::raw(&label)) as u16;
    // badge + diagonal cut must fit inside the top border, leaving room for
    // the right corner.
    let w = (label_w + 1).min(area.width.saturating_sub(2));
    if w == 0 {
        return;
    }
    let chip = Rect {
        x: area.x + 1,
        y: area.y,
        width: w,
        height: 1,
    };
    let buf = frame.buffer_mut();
    for (i, ch) in label.chars().enumerate() {
        if i >= label_w as usize {
            break;
        }
        let x = chip.x + i as u16;
        buf[(x, chip.y)]
            .set_symbol(&ch.to_string())
            .set_style(Style::default().fg(accent).add_modifier(Modifier::BOLD));
    }
    // diagonal cut right after the chip
    if chip.x + label_w < area.x + area.width - 1 {
        buf[(chip.x + label_w, chip.y)]
            .set_symbol("╱")
            .set_style(Style::default().fg(accent).bg(Color::Reset));
    }
}

fn draw_question(
    frame: &mut Frame,
    req: &crate::harness::ui::tui::askers::QuestionRequest,
    draft: &str,
    cursor: usize,
    t: &Theme,
    tick: u64,
    area: Rect,
) {
    // Size the modal to use the available screen space: width grows with the
    // longest content line (capped at ~90% of the screen), and height grows
    // with the wrapped content (capped at ~90% of the screen) so long
    // questions/options are not clipped.
    let max_w = ((area.width as f32) * 0.9) as u16;
    let longest = req
        .question
        .lines()
        .chain(req.options.iter().flat_map(|o| o.lines()))
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);
    let w = (longest as u16 + 8).clamp(44, max_w.max(44)).min(110);

    // Question + options + answer box + hints (height grows with wrapped rows).
    let q_width = (w.saturating_sub(6) as usize).max(10);
    let q_rows = crate::harness::ui::tui::markdown::wrap_plain(&req.question, q_width).len() as u16;
    let mut opt_extra = 0u16;
    for o in &req.options {
        let rows = crate::harness::ui::tui::markdown::wrap_plain(o, q_width).len();
        opt_extra = opt_extra.saturating_add(rows.saturating_sub(1) as u16);
    }
    let opt_rows = req.options.len() as u16;
    let max_h = ((area.height as f32) * 0.9) as u16;
    let h = (13 + opt_rows + q_rows.saturating_sub(1) + opt_extra)
        .min(max_h.max(11))
        .max(11);
    let area = centered_rect_fixed(w, h, area);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent3))
        .title(Span::styled(
            " ❯ question — type your answer ",
            Style::default().fg(t.accent3).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface).fg(t.text));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut y = inner.y;
    let push_line = |frame: &mut Frame, line: Line<'static>, row: &mut u16| {
        if *row < inner.y + inner.height {
            frame.render_widget(
                Paragraph::new(line),
                Rect {
                    x: inner.x,
                    y: *row,
                    width: inner.width,
                    height: 1,
                },
            );
            *row = row.saturating_add(1);
        }
    };

    push_line(frame, Line::from(""), &mut y);
    // Question text (soft-wrapped over multiple rows; grows the modal height).
    let q_width_full = inner.width.saturating_sub(4) as usize;
    let q_lines = crate::harness::ui::tui::markdown::wrap_plain(&req.question, q_width_full);
    if q_lines.is_empty() {
        push_line(frame, Line::from(""), &mut y);
    }
    for q in q_lines.iter() {
        let prefix = "  ";
        push_line(
            frame,
            Line::from(Span::styled(
                format!("{}{}", prefix, q),
                Style::default()
                    .fg(t.text_bright)
                    .add_modifier(Modifier::BOLD),
            )),
            &mut y,
        );
    }
    push_line(frame, Line::from(""), &mut y);

    for (i, o) in req.options.iter().enumerate() {
        let label_width = inner.width.saturating_sub(8) as usize;
        let label_lines = crate::harness::ui::tui::markdown::wrap_plain(o, label_width);
        if label_lines.is_empty() {
            push_line(
                frame,
                Line::from(vec![
                    Span::styled(format!("  [{}] ", i + 1), Style::default().fg(t.accent)),
                    Span::styled("", Style::default().fg(t.text)),
                ]),
                &mut y,
            );
            continue;
        }
        for (j, label) in label_lines.iter().enumerate() {
            let spans = if j == 0 {
                vec![
                    Span::styled(format!("  [{}] ", i + 1), Style::default().fg(t.accent)),
                    Span::styled(label.clone(), Style::default().fg(t.text)),
                ]
            } else {
                vec![
                    Span::styled("     ", Style::default()),
                    Span::styled(label.clone(), Style::default().fg(t.text)),
                ]
            };
            push_line(frame, Line::from(spans), &mut y);
        }
    }
    if !req.options.is_empty() {
        push_line(frame, Line::from(""), &mut y);
    }

    // Answer box (2 rows: border-like label + input).
    push_line(
        frame,
        Line::from(Span::styled(
            "  answer",
            Style::default().fg(t.text_dim).add_modifier(Modifier::BOLD),
        )),
        &mut y,
    );

    let cursor_glyph = if (tick / 5).is_multiple_of(2) {
        "▌"
    } else {
        " "
    };
    let chars: Vec<char> = draft.chars().collect();
    let at = cursor.min(chars.len());
    let before: String = chars[..at].iter().collect();
    let after: String = chars[at..].iter().collect();
    let input_width = inner.width.saturating_sub(4) as usize;

    let answer_line = if draft.is_empty() {
        Line::from(vec![
            Span::styled("  ┌ ", Style::default().fg(t.border_focus)),
            Span::styled(cursor_glyph.to_string(), Style::default().fg(t.accent)),
            Span::styled(
                truncate("type here, then press Enter", input_width.saturating_sub(1)),
                Style::default().fg(t.text_dim),
            ),
        ])
    } else {
        // Keep the cursor visible by windowing long drafts.
        let total = chars.len();
        let max_shown = input_width.saturating_sub(1).max(8);
        let mut start = 0usize;
        if at >= max_shown {
            start = at + 1 - max_shown;
        }
        let end = (start + max_shown).min(total);
        let vis_before: String = chars[start..at.min(end)].iter().collect();
        let vis_after: String = chars[at.min(end)..end].iter().collect();
        let _ = (before, after); // kept for clarity of cursor split
        Line::from(vec![
            Span::styled("  ┌ ", Style::default().fg(t.border_focus)),
            Span::styled(vis_before, Style::default().fg(t.text_bright)),
            Span::styled(cursor_glyph.to_string(), Style::default().fg(t.accent)),
            Span::styled(vis_after, Style::default().fg(t.text)),
        ])
    };
    let answer_row = y;
    push_line(frame, answer_line, &mut y);
    push_line(
        frame,
        Line::from(Span::styled(
            format!(
                "  └{}",
                "─".repeat(inner.width.saturating_sub(4).max(1) as usize)
            ),
            Style::default().fg(t.border_focus),
        )),
        &mut y,
    );

    push_line(frame, Line::from(""), &mut y);
    let mut hint = vec![
        Span::styled("  ", Style::default()),
        Span::styled("Enter", Style::default().fg(t.success)),
        Span::styled(" send   ", Style::default().fg(t.text_dim)),
        Span::styled("Esc", Style::default().fg(t.accent2)),
        Span::styled(" cancel", Style::default().fg(t.text_dim)),
    ];
    if !req.options.is_empty() {
        hint.extend([
            Span::styled("   ", Style::default()),
            Span::styled("1..n + Enter", Style::default().fg(t.accent)),
            Span::styled(" pick option", Style::default().fg(t.text_dim)),
        ]);
    }
    push_line(frame, Line::from(hint), &mut y);

    // Real terminal caret on the answer field.
    let col = 4u16 + {
        let max_shown = input_width.saturating_sub(1).max(8);
        let start = if at >= max_shown {
            at + 1 - max_shown
        } else {
            0
        };
        (at - start) as u16
    };
    if answer_row < inner.y + inner.height && col < inner.width {
        frame.set_cursor_position((
            (inner.x + col).min(inner.x + inner.width.saturating_sub(1)),
            answer_row,
        ));
    }
}

fn key_btn(key: &str, label: &str, color: ratatui::style::Color, t: &Theme) -> Span<'static> {
    Span::styled(
        format!("[{}] {} ", key, label),
        Style::default()
            .fg(color)
            .bg(t.bg)
            .add_modifier(Modifier::BOLD),
    )
}

fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        let mut o: String = s.chars().take(max.saturating_sub(1)).collect();
        o.push('…');
        o
    }
}

#[cfg(test)]
mod cursor_model_draw_tests {
    use crate::harness::ui::tui::app::state::{CursorModelTarget, Modal};
    use crate::harness::ui::tui::app::App;
    use crate::harness::ui::tui::draw::render_to_buffer;

    fn sample_models(n: usize) -> Vec<(String, String)> {
        (0..n)
            .map(|i| {
                (
                    format!("model-with-a-very-long-id-number-{i}"),
                    format!("description that should be truncated for narrow terminals {i}"),
                )
            })
            .collect()
    }

    fn buffer_text(app: &mut App, width: u16, height: u16) -> String {
        let buf = render_to_buffer(app, width, height);
        buf.content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .replace('\u{0}', " ")
    }

    #[test]
    fn cursor_model_position_indicator_and_truncation() {
        let mut app = App::inline_for_tests("");
        app.splash = None;
        app.modal = Some(Modal::CursorModel {
            selected: 25,
            scroll_offset: 0,
            filter: String::new(),
            models: sample_models(42),
            target: CursorModelTarget::Plan,
        });
        let text = buffer_text(&mut app, 56, 16);
        assert!(text.contains("of 42"), "indicator missing: {text}");
        assert!(text.contains('…'), "expected width-aware truncation");
        assert!(
            text.contains("Enter select"),
            "hint must stay visible: {text}"
        );
        assert!(
            text.contains("type to filter"),
            "hint must mention filter: {text}"
        );
        assert!(
            text.contains("filter>"),
            "filter input row must show: {text}"
        );
    }

    #[test]
    fn cursor_model_adapts_to_short_terminal() {
        let mut app = App::inline_for_tests("");
        app.splash = None;
        app.modal = Some(Modal::CursorModel {
            selected: 0,
            scroll_offset: 0,
            filter: String::new(),
            models: sample_models(30),
            target: CursorModelTarget::Build,
        });
        let text = buffer_text(&mut app, 50, 10);
        assert!(text.contains("of 30"));
        assert!(text.contains("Enter select"));
    }

    #[test]
    fn cursor_model_filter_shows_filtered_total_in_indicator() {
        let mut app = App::inline_for_tests("");
        app.splash = None;
        let models = sample_models(30);
        app.modal = Some(Modal::CursorModel {
            selected: 0,
            scroll_offset: 0,
            filter: "model-with".to_string(),
            models,
            target: CursorModelTarget::Build,
        });
        let text = buffer_text(&mut app, 56, 14);
        assert!(
            text.contains("of 30"),
            "filtered set is all models here: {text}"
        );
        app.modal.as_mut().map(|m| {
            if let Modal::CursorModel { filter, .. } = m {
                filter.clear();
                filter.push_str("number-29");
            }
        });
        let narrowed = buffer_text(&mut app, 56, 14);
        assert!(
            narrowed.contains("number-29"),
            "only the filtered model should render: {narrowed}"
        );
        assert!(
            !narrowed.contains("of 30"),
            "indicator must use filtered count, not full list: {narrowed}"
        );
    }
}
