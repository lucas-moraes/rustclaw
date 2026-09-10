//! Status bar with spinner, chips, and token usage.

use crate::harness::provider::format_tokens;
use crate::harness::ui::tui::anim;
use crate::harness::ui::tui::app::App;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Formats a running duration compactly: `42s`, `3m12s`, `1h04m`.
pub fn format_elapsed(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;

    let (icon, state, state_fg) = if app.modal.is_some() {
        ("?", "waiting input".to_string(), t.warn)
    } else if app.running {
        let elapsed = app.turn_started_at.map(|s| s.elapsed()).unwrap_or_default();
        let state = if let Some(msg) = &app.status_msg {
            format!("{} · {}", msg, format_elapsed(elapsed))
        } else {
            format!("streaming · {}", format_elapsed(elapsed))
        };
        (anim::spinner_frame(app.tick), state, t.accent2)
    } else {
        ("●", "idle".to_string(), t.success)
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
        Span::styled(state, Style::default().fg(state_fg)),
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

    // Compact prompt-cache indicator (only when the provider reports reads).
    if app.session_usage.cache_read_tokens > 0 {
        spans.push(Span::styled(" ↻", Style::default().fg(t.text_dim)));
        spans.push(Span::styled(
            format_tokens(app.session_usage.cache_read_tokens),
            Style::default().fg(t.text_dim),
        ));
    }

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

    // Thin top rule so the status bar reads as a separator under the transcript.
    let mut line_spans = vec![Span::styled("─ ", Style::default().fg(t.border))];
    line_spans.extend(spans);
    frame.render_widget(
        Paragraph::new(Line::from(line_spans)).style(Style::default().bg(t.status_bg).fg(t.text)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::format_elapsed;
    use std::time::Duration;

    #[test]
    fn test_format_elapsed_compact_units() {
        assert_eq!(format_elapsed(Duration::from_secs(42)), "42s");
        assert_eq!(format_elapsed(Duration::from_secs(192)), "3m12s");
        assert_eq!(format_elapsed(Duration::from_secs(3840)), "1h04m");
        assert_eq!(format_elapsed(Duration::from_secs(0)), "0s");
    }
}
