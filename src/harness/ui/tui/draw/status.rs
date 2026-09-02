//! Status bar with spinner, chips, and token usage.

use crate::harness::provider::format_tokens;
use crate::harness::ui::tui::anim;
use crate::harness::ui::tui::app::App;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;

    let (icon, state, state_fg) = if app.modal.is_some() {
        ("?", "waiting input", t.warn)
    } else if app.running {
        (
            anim::spinner_frame(app.tick),
            if let Some(msg) = &app.status_msg {
                msg.as_str()
            } else {
                "streaming"
            },
            t.accent2,
        )
    } else {
        ("●", "idle", t.success)
    };

    let ctx = app.context_tokens() as u64;
    let max = app.max_context_tokens() as u64;
    let ctx_pct = if max == 0 { 0 } else { (ctx * 100) / max };
    let ctx_fg = if ctx_pct >= 90 {
        t.error
    } else if ctx_pct >= 70 {
        t.warn
    } else {
        t.info
    };

    let mut spans = vec![
        Span::styled(
            format!(" {} ", icon),
            Style::default().fg(state_fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(state.to_string(), Style::default().fg(state_fg)),
        Span::styled("  ·  ", Style::default().fg(t.border)),
        Span::styled("ctx ", Style::default().fg(t.text_dim)),
        Span::styled(
            format!("{}/{}", format_tokens(ctx), format_tokens(max)),
            Style::default().fg(ctx_fg),
        ),
        Span::styled(format!(" {}%", ctx_pct), Style::default().fg(ctx_fg)),
        Span::styled("  ·  ", Style::default().fg(t.border)),
        Span::styled("in ", Style::default().fg(t.text_dim)),
        Span::styled(
            format_tokens(app.session_usage.input_tokens),
            Style::default().fg(t.accent),
        ),
        Span::styled(" out ", Style::default().fg(t.text_dim)),
        Span::styled(
            format_tokens(app.session_usage.output_tokens),
            Style::default().fg(t.accent2),
        ),
        Span::styled(" Σ ", Style::default().fg(t.text_dim)),
        Span::styled(
            format_tokens(app.session_usage.total()),
            Style::default()
                .fg(t.text_bright)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if app.last_iterations > 0 {
        spans.push(Span::styled("  ·  ", Style::default().fg(t.border)));
        spans.push(Span::styled("iters ", Style::default().fg(t.text_dim)));
        spans.push(Span::styled(
            app.last_iterations.to_string(),
            Style::default().fg(t.text_bright),
        ));
    }

    if !app.session.skills.is_empty() {
        spans.push(Span::styled("  ·  ", Style::default().fg(t.border)));
        spans.push(Span::styled("skills ", Style::default().fg(t.text_dim)));
        spans.push(Span::styled(
            app.session.skills.len().to_string(),
            Style::default().fg(t.accent3),
        ));
    }

    if !app.active_tools.is_empty() {
        spans.push(Span::styled("  ·  ", Style::default().fg(t.border)));
        spans.push(Span::styled(
            format!(
                "{} tool{}",
                app.active_tools.len(),
                if app.active_tools.len() == 1 { "" } else { "s" }
            ),
            Style::default().fg(t.warn),
        ));
    }

    if app.running {
        spans.push(Span::styled("  ·  ", Style::default().fg(t.border)));
        spans.push(Span::styled(
            anim::thinking_dots(app.tick),
            Style::default().fg(t.text_dim),
        ));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(t.status_bg).fg(t.text)),
        area,
    );
}
