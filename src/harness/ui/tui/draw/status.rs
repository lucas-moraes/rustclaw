//! Status bar with spinner, chips, and token usage.

use crate::harness::provider::catalog::format_cost;
use crate::harness::provider::format_tokens;
use crate::harness::ui::tui::anim;
use crate::harness::ui::tui::app::{App, DoomLevel};
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

/// One status-bar chip (priority order: state → ctx → model → Σ → cost → iters → doom).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusChip {
    pub id: &'static str,
    pub text: String,
}

fn truncate_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let take = n.saturating_sub(1);
    format!("{}…", s.chars().take(take).collect::<String>())
}

/// Builds chips in drop-from-end priority order (state and ctx stay longest).
pub fn status_chips(
    model: &str,
    provider: &str,
    session_total: u64,
    cost: Option<f64>,
    current_iter: usize,
    max_iter: usize,
    doom: DoomLevel,
) -> Vec<StatusChip> {
    let mut chips = Vec::new();
    let mut model_txt = truncate_chars(model, 18);
    if !provider.is_empty() && provider.chars().count() <= 10 {
        model_txt = format!("{model_txt} ({provider})");
    }
    chips.push(StatusChip {
        id: "model",
        text: model_txt,
    });
    chips.push(StatusChip {
        id: "sigma",
        text: format!("Σ {}", format_tokens(session_total)),
    });
    if let Some(usd) = cost {
        if session_total > 0 {
            chips.push(StatusChip {
                id: "cost",
                text: format_cost(usd),
            });
        }
    }
    if max_iter > 0 {
        let cur = current_iter.max(1);
        chips.push(StatusChip {
            id: "iters",
            text: format!("iters {cur}/{max_iter}"),
        });
    }
    match doom {
        DoomLevel::Ok => {}
        DoomLevel::Warn => chips.push(StatusChip {
            id: "doom",
            text: "doom warn".into(),
        }),
        DoomLevel::Stop => chips.push(StatusChip {
            id: "doom",
            text: "doom stop".into(),
        }),
    }
    chips
}

/// Drops lowest-priority chips (end of vec) until they fit `width`.
pub fn fit_status_chips(mut chips: Vec<StatusChip>, width: usize) -> Vec<StatusChip> {
    loop {
        let used: usize = chips.iter().map(|c| c.text.chars().count() + 3).sum();
        if used <= width || chips.len() <= 1 {
            break;
        }
        chips.pop();
    }
    chips
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
    let ctx_pct = (ctx * 100).checked_div(max).unwrap_or(0);
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
    ];

    let extra = status_chips(
        &app.runtime.config.model,
        &app.runtime.config.provider,
        app.session_usage.total(),
        if app.session_usage.total() > 0 {
            Some(app.session_cost())
        } else {
            None
        },
        app.current_iteration,
        app.runtime.config.max_iterations,
        app.doom_level,
    );
    let extra_w = area.width.saturating_sub(40) as usize;
    for chip in fit_status_chips(extra, extra_w.max(8)) {
        spans.push(Span::styled("  ·  ", Style::default().fg(t.border)));
        let fg = match chip.id {
            "doom" if app.doom_level == DoomLevel::Stop => t.error,
            "doom" => t.warn,
            "cost" => t.success,
            "model" => t.accent3,
            _ => t.text_bright,
        };
        spans.push(Span::styled(chip.text, Style::default().fg(fg)));
    }

    if app.session_usage.cache_read_tokens > 0 {
        spans.push(Span::styled(" ↻", Style::default().fg(t.text_dim)));
        spans.push(Span::styled(
            format_tokens(app.session_usage.cache_read_tokens),
            Style::default().fg(t.text_dim),
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

    let mut line_spans = vec![Span::styled("─ ", Style::default().fg(t.border))];
    line_spans.extend(spans);
    frame.render_widget(
        Paragraph::new(Line::from(line_spans)).style(Style::default().bg(t.status_bg).fg(t.text)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::ui::tui::app::DoomLevel;
    use std::time::Duration;

    #[test]
    fn test_format_elapsed_compact_units() {
        assert_eq!(format_elapsed(Duration::from_secs(42)), "42s");
        assert_eq!(format_elapsed(Duration::from_secs(192)), "3m12s");
        assert_eq!(format_elapsed(Duration::from_secs(3840)), "1h04m");
        assert_eq!(format_elapsed(Duration::from_secs(0)), "0s");
    }

    #[test]
    fn test_status_includes_model() {
        let chips = status_chips(
            "deepseek-v4-flash",
            "opencode-go",
            0,
            None,
            0,
            20,
            DoomLevel::Ok,
        );
        assert!(chips
            .iter()
            .any(|c| c.id == "model" && c.text.contains("deepseek")));
    }

    #[test]
    fn test_status_includes_cost_when_usage_nonzero() {
        let chips = status_chips("m", "p", 100, Some(1.25), 1, 20, DoomLevel::Ok);
        assert!(chips.iter().any(|c| c.id == "cost" && c.text.contains('$')));
        let none = status_chips("m", "p", 0, Some(1.25), 1, 20, DoomLevel::Ok);
        assert!(!none.iter().any(|c| c.id == "cost"));
    }

    #[test]
    fn test_status_iteration_current_over_max() {
        let chips = status_chips("m", "p", 0, None, 3, 20, DoomLevel::Ok);
        let iters = chips.iter().find(|c| c.id == "iters").unwrap();
        assert_eq!(iters.text, "iters 3/20");
    }

    #[test]
    fn test_status_doom_warn_and_stop_chips() {
        let w = status_chips("m", "p", 0, None, 1, 20, DoomLevel::Warn);
        assert!(w.iter().any(|c| c.id == "doom" && c.text.contains("warn")));
        let s = status_chips("m", "p", 0, None, 1, 20, DoomLevel::Stop);
        assert!(s.iter().any(|c| c.id == "doom" && c.text.contains("stop")));
        let ok = status_chips("m", "p", 0, None, 1, 20, DoomLevel::Ok);
        assert!(!ok.iter().any(|c| c.id == "doom"));
    }

    #[test]
    fn test_status_drops_low_priority_chips_when_narrow() {
        let chips = status_chips(
            "verylongmodelidhere",
            "prov",
            9999,
            Some(12.3),
            4,
            20,
            DoomLevel::Stop,
        );
        let fitted = fit_status_chips(chips, 20);
        assert!(fitted.iter().any(|c| c.id == "model"));
        assert!(!fitted.iter().any(|c| c.id == "doom"));
    }
}
