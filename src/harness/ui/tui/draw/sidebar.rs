//! Left-side info panel: rustclaw header, LLM data, mode selector, session.

use crate::harness::provider::format_tokens;
use crate::harness::ui::tui::app::{App, MODES};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let active = app.session.agent.as_str();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border))
        .title(Span::styled(
            " rustclaw ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let inner_w = inner.width.saturating_sub(2) as usize;

    let mut lines: Vec<Line> = Vec::new();

    // LLM data.
    lines.push(section("LLM", t.accent));
    lines.push(kv("model", truncate(&app.runtime.config.model, inner_w), t));
    lines.push(kv(
        "provider",
        truncate(&app.runtime.config.provider, inner_w),
        t,
    ));
    lines.push(kv(
        "context",
        format_tokens(app.max_context_tokens() as u64),
        t,
    ));
    lines.push(kv(
        "temperature",
        format!("{:.1}", app.runtime.turn_temperature(&app.session.agent)),
        t,
    ));
    lines.push(Line::from(""));

    // Modes (build / plan / explore), active highlighted.
    lines.push(section("Mode", t.accent));
    for mode in MODES {
        let is_active = *mode == active;
        let label = format!("  {} {}", if is_active { "▶" } else { " " }, mode);
        lines.push(Line::from(vec![Span::styled(
            label,
            if is_active {
                Style::default()
                    .fg(t.bg)
                    .bg(t.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.text_dim)
            },
        )]));
    }
    lines.push(Line::from(""));

    // Session data.
    lines.push(section("Session", t.accent));
    lines.push(kv("id", truncate(&app.session.id, inner_w), t));
    lines.push(kv("agent", app.session.agent.clone(), t));
    lines.push(kv(
        "cwd",
        truncate(&app.cwd.display().to_string(), inner_w),
        t,
    ));
    lines.push(kv("messages", app.session.messages.len().to_string(), t));
    let ctx = app.context_tokens() as u64;
    let max = app.max_context_tokens() as u64;
    let pct = if max == 0 { 0 } else { (ctx * 100) / max };
    lines.push(kv(
        "context",
        format!("{}% ({} / {})", pct, format_tokens(ctx), format_tokens(max)),
        t,
    ));
    lines.push(kv(
        "session tok",
        format_tokens(app.session_usage.total()),
        t,
    ));
    lines.push(kv("last turn", format_tokens(app.last_usage.total()), t));
    lines.push(kv("iterations", app.last_iterations.to_string(), t));
    lines.push(kv(
        "created",
        app.session.created_at.format("%H:%M").to_string(),
        t,
    ));

    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(t.surface))
            .wrap(Wrap { trim: false }),
        inner,
    );
}

fn section(label: &str, fg: ratatui::style::Color) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {} ", label.to_uppercase()),
        Style::default().fg(fg).add_modifier(Modifier::BOLD),
    ))
}

fn kv(key: &str, value: String, t: &crate::harness::ui::tui::theme::Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {:<12}", key), Style::default().fg(t.text_dim)),
        Span::styled(value, Style::default().fg(t.text_bright)),
    ])
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let take = max - 1;
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}
