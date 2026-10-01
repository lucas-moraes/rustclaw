//! Status bar: model, provider and the token/cost consumption meter.

use crate::harness::provider::catalog::format_cost;
use crate::harness::provider::format_tokens;
use crate::harness::ui::tui::app::App;
use crate::harness::ui::tui::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Context gauge color: green under 70%, amber under 90%, red above.
fn context_color(pct: u16, t: &Theme) -> ratatui::style::Color {
    if pct >= 90 {
        t.error
    } else if pct >= 70 {
        t.warn
    } else {
        t.success
    }
}

/// Horizontal gauge: `████░░░░` colored by fill level. `cells` is clamped to
/// the space actually available, so the meter degrades instead of vanishing.
fn gauge_spans(pct: u16, t: &Theme, cells: usize) -> Vec<Span<'static>> {
    let cells = cells.clamp(1, 10);
    let filled = ((pct as usize * cells) + 50) / 100;
    let color = context_color(pct, t);
    vec![
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(cells - filled), Style::default().fg(t.text_dim)),
        Span::raw(" "),
    ]
}

/// Right side of the rail: `████░░░░ NN% used/max  $session +$last`.
/// `avail` is the column budget for the whole meter; the gauge shrinks first,
/// then the token counts are dropped, so the percentage always survives.
fn usage_spans(app: &App, t: &Theme, avail: usize) -> Vec<Span<'static>> {
    let ctx = app.context_tokens() as u64;
    let max = app.max_context_tokens() as u64;
    let pct = (ctx * 100).checked_div(max).unwrap_or(0).min(100) as u16;
    let color = context_color(pct, t);

    let pct_span = Span::styled(
        format!("{pct:>3}%"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    );
    let counts = Span::styled(
        format!(" {}/{}", format_tokens(ctx), format_tokens(max)),
        Style::default().fg(t.text_bright),
    );
    let session = app.session_cost();
    let last = app.last_cost();
    let cost_text = if last > 0.0 {
        format!("  {} +{}", format_cost(session), format_cost(last))
    } else {
        format!("  {}", format_cost(session))
    };
    let cost = Span::styled(cost_text, Style::default().fg(t.accent3));

    // Fixed parts: percentage + cost are mandatory; counts and gauge are not.
    let fixed = pct_span.width() + cost.width();
    let counts_w = counts.width();
    let mut spans = Vec::new();
    let mut budget = avail;

    // Gauge gets whatever is left after the mandatory parts, capped at 10.
    let gauge_budget = budget.saturating_sub(fixed + counts_w + 1);
    if gauge_budget >= 3 {
        let cells = gauge_budget.min(10);
        spans.extend(gauge_spans(pct, t, cells));
        budget = budget.saturating_sub(cells + 1);
    }
    spans.push(pct_span);
    if counts_w <= budget.saturating_sub(fixed) {
        spans.push(counts);
    }
    spans.push(cost);
    spans
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect, tick: u64) {
    let t = &app.theme;

    // Full model name, never elided: the rail is the one place that must show
    // exactly which model is in use.
    let mut model_txt = app.runtime.config.model.clone();
    let provider = &app.runtime.config.provider;
    if !provider.is_empty() && provider.chars().count() <= 10 {
        model_txt = format!("{model_txt} ({provider})");
    }

    let mut line_spans = vec![
        Span::styled(" ─ ", Style::default().fg(t.border)),
        Span::styled(model_txt, Style::default().fg(t.accent3)),
    ];
    // Push-to-talk indicator: 🎙 recording (with elapsed time) or transcribing.
    #[cfg(feature = "voice")]
    if app.recording.is_some() {
        let secs = app
            .recording
            .as_ref()
            .map(|r| r.started_at.elapsed().as_secs())
            .unwrap_or(0);
        line_spans.push(Span::styled(
            format!(
                "  🎙 {} 0:{secs:02}",
                crate::harness::ui::tui::anim::wave_frame(
                    tick,
                    app.recording.as_ref().map(|r| r.recorder.level())
                )
            ),
            Style::default()
                .fg(t.error)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    } else if app.transcribing {
        line_spans.push(Span::styled(
            format!(
                "  ✎ transcrevendo {}",
                crate::harness::ui::tui::anim::think_frame(tick)
            ),
            Style::default().fg(t.accent2),
        ));
    }
    // Usage meter on the right. It degrades (gauge shrinks, then counts drop)
    // rather than disappearing, so the percentage is always visible.
    let used: usize = line_spans.iter().map(Span::width).sum::<usize>() + 3;
    let avail = (area.width as usize).saturating_sub(used);
    if avail >= 6 {
        let usage = usage_spans(app, t, avail);
        let usage_w: usize = usage.iter().map(Span::width).sum();
        let gap = avail.saturating_sub(usage_w);
        line_spans.push(Span::raw(" ".repeat(gap)));
        line_spans.extend(usage);
    }

    frame.render_widget(
        Paragraph::new(Line::from(line_spans)).style(Style::default().bg(t.status_bg).fg(t.text)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_color_thresholds() {
        let t = Theme::cyberclaw();
        assert_eq!(context_color(0, &t), t.success);
        assert_eq!(context_color(69, &t), t.success);
        assert_eq!(context_color(70, &t), t.warn);
        assert_eq!(context_color(89, &t), t.warn);
        assert_eq!(context_color(90, &t), t.error);
        assert_eq!(context_color(100, &t), t.error);
    }
}
