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

/// Right side of the rail: `NN% used/max  $session +$last`.
fn usage_spans(app: &App, t: &Theme) -> Vec<Span<'static>> {
    let ctx = app.context_tokens() as u64;
    let max = app.max_context_tokens() as u64;
    let pct = (ctx * 100).checked_div(max).unwrap_or(0).min(100) as u16;
    let color = context_color(pct, t);

    let mut spans = vec![
        Span::styled(
            format!("{pct:>3}%"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}/{}", format_tokens(ctx), format_tokens(max)),
            Style::default().fg(t.text_bright),
        ),
    ];

    let session = app.session_cost();
    let last = app.last_cost();
    let text = if last > 0.0 {
        format!("  {} +{}", format_cost(session), format_cost(last))
    } else {
        format!("  {}", format_cost(session))
    };
    spans.push(Span::styled(text, Style::default().fg(t.accent3)));
    spans
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
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
    // Usage meter on the right, dropped when the rail is too narrow for both.
    let usage = usage_spans(app, t);
    let used: usize = line_spans.iter().map(Span::width).sum::<usize>() + 3;
    let usage_w: usize = usage.iter().map(Span::width).sum();
    if used + usage_w <= area.width as usize {
        let gap = area.width as usize - used - usage_w;
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
