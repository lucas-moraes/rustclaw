//! Top navbar, two rows: row 1 is app/system status (brand, ready state,
//! git branch, context gauge, cost); row 2 is session info (messages,
//! model) and the mode selector. Each row is left/right aligned; on narrow
//! terminals the right side sheds optional pieces instead of squeezing.
//! Width is measured with `Span::width`, not `chars().count()`.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::harness::provider::catalog::format_cost;
use crate::harness::provider::format_tokens;
use crate::harness::ui::tui::app::{App, MODES};
use crate::harness::ui::tui::draw::modal::cyber_badge;
use crate::harness::ui::tui::theme::Theme;

/// Two text rows.
pub const HEIGHT: u16 = 2;
/// Hide the bar when the terminal is too short to keep a transcript under it.
pub const MIN_TERMINAL_HEIGHT: u16 = 16;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let lines = vec![system_line(app, t, width), session_line(app, t, width)];
    frame.render_widget(Paragraph::new(lines), area);
    cyber_badge(frame, area, t.accent, "rustclaw");
}

// ---------------------------------------------------------------- row 1

struct SystemOpts {
    branch: bool,
    gauge: bool,
    used: bool,
    free: bool,
    cost: bool,
    status_max: usize,
}

fn system_line(app: &App, t: &Theme, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }
    let mut opts = SystemOpts {
        branch: true,
        gauge: true,
        used: true,
        free: false,
        cost: true,
        status_max: 24,
    };
    loop {
        let left = system_left(app, t, opts.status_max);
        let right = system_right(app, t, &opts);
        let left_w = spans_width(&left);
        let right_w = spans_width(&right);
        if left_w + right_w <= width {
            let gap = width - left_w - right_w;
            let mut spans = left;
            if gap > 0 {
                spans.push(Span::raw(" ".repeat(gap)));
            }
            spans.extend(right);
            return Line::from(spans);
        }
        if opts.status_max > 8 {
            opts.status_max = 8;
        } else if opts.free {
            opts.free = false;
        } else if opts.used {
            opts.used = false;
        } else if opts.branch {
            opts.branch = false;
        } else if opts.cost {
            opts.cost = false;
        } else if opts.gauge {
            opts.gauge = false;
        } else {
            let fallback = clip_spans(left, width);
            return Line::from(fallback);
        }
    }
}

fn system_left(app: &App, t: &Theme, status_max: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    spans.extend(status_spans(app, t, status_max));
    if !app.git_branch.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!(" {} {}", '\u{e0a0}', app.git_branch),
            Style::default().fg(t.text_dim),
        ));
    }
    spans
}

fn system_right(app: &App, t: &Theme, opts: &SystemOpts) -> Vec<Span<'static>> {
    let ctx = app.context_tokens() as u64;
    let max = app.max_context_tokens() as u64;
    let pct = (ctx * 100).checked_div(max).unwrap_or(0).min(100) as u16;
    let color = context_color(pct, t);
    let mut spans = Vec::new();
    if opts.gauge {
        spans.extend(gauge(pct, t));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(
        format!("{pct:>3}%"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ));
    if opts.used {
        spans.push(Span::styled(
            format!(" {}/{}", format_tokens(ctx), format_tokens(max)),
            Style::default().fg(t.text_bright),
        ));
    }
    if opts.free {
        spans.push(Span::styled(
            format!(" · {} free", format_tokens(max.saturating_sub(ctx))),
            Style::default().fg(t.text_dim),
        ));
    }
    if opts.cost {
        let session = app.session_cost();
        let last = app.last_cost();
        let text = if last > 0.0 {
            format!("  {} +{}", format_cost(session), format_cost(last))
        } else {
            format!("  {}", format_cost(session))
        };
        spans.push(Span::styled(text, Style::default().fg(t.accent3)));
    }
    spans
}

fn status_spans(app: &App, t: &Theme, max_label: usize) -> Vec<Span<'static>> {
    let (icon, label, color) = if app.modal.is_some() {
        ("?".to_string(), "waiting".to_string(), t.warn)
    } else if app.running {
        if app.active_tools.is_empty() {
            let streaming = app.streaming.is_some();
            let fallback = if streaming { "streaming" } else { "working" };
            let label = app
                .status_msg
                .clone()
                .unwrap_or_else(|| fallback.to_string());
            let color = if streaming { t.accent } else { t.warn };
            ("●".to_string(), label, color)
        } else {
            let names: Vec<&str> = app
                .active_tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect();
            ("●".to_string(), names.join(" · "), t.warn)
        }
    } else if let Some(msg) = &app.status_msg {
        ("●".to_string(), msg.clone(), t.accent2)
    } else if app.runtime.config.is_configured() {
        ("●".to_string(), "ready".to_string(), t.success)
    } else {
        ("●".to_string(), "no auth".to_string(), t.error)
    };
    vec![
        Span::styled(icon, Style::default().fg(color)),
        Span::raw(" "),
        Span::styled(fit_width(&label, max_label), Style::default().fg(color)),
    ]
}

// ---------------------------------------------------------------- row 2

struct SessionOpts {
    title: bool,
    skills: bool,
    provider: bool,
    model_max: usize,
}

fn session_line(app: &App, t: &Theme, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }
    let mut opts = SessionOpts {
        title: true,
        skills: true,
        provider: true,
        model_max: 32,
    };
    loop {
        let left = session_left(app, t, &opts);
        let right = mode_spans(app, t);
        let left_w = spans_width(&left);
        let right_w = spans_width(&right);
        if left_w + right_w <= width {
            let gap = width - left_w - right_w;
            let mut spans = left;
            if gap > 0 {
                spans.push(Span::raw(" ".repeat(gap)));
            }
            spans.extend(right);
            return Line::from(spans);
        }
        if opts.title {
            opts.title = false;
        } else if opts.skills {
            opts.skills = false;
        } else if opts.provider {
            opts.provider = false;
        } else if opts.model_max > 8 {
            opts.model_max = 8;
        } else {
            let fallback = clip_spans(left, width);
            return Line::from(fallback);
        }
    }
}

fn session_left(app: &App, t: &Theme, opts: &SessionOpts) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled(
        format!(" {} msgs", app.session.messages.len()),
        Style::default().fg(t.text_dim),
    )];
    if opts.skills {
        if let Some(skill) = skills_span(app, t) {
            spans.push(skill);
        }
    }
    spans
}

fn mode_spans(app: &App, t: &Theme) -> Vec<Span<'static>> {
    let active = app.session.agent.as_str();
    let cursor = app.runtime.config.cursor_agent;
    let mut spans = Vec::new();
    let mut first = true;
    let mut push = |name: &str, on: bool| {
        if !first {
            spans.push(Span::styled(
                " · ".to_string(),
                Style::default().fg(t.text_dim),
            ));
        }
        first = false;
        let (icon, fg) = if on {
            ("► ", t.success)
        } else {
            ("· ", t.text_dim)
        };
        let mods = if on {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        spans.push(Span::styled(icon.to_string(), Style::default().fg(fg)));
        spans.push(Span::styled(
            name.to_string(),
            Style::default().fg(fg).add_modifier(mods),
        ));
        if on && cursor {
            spans.push(Span::styled(
                " →".to_string(),
                Style::default().fg(t.accent2),
            ));
        }
    };
    if !MODES.contains(&active) {
        push(active, true);
    }
    for mode in MODES {
        if *mode == active {
            push(mode, true);
        } else {
            push(mode, false);
        }
    }
    spans
}

// ---------------------------------------------------------------- helpers

fn gauge(pct: u16, t: &Theme) -> Vec<Span<'static>> {
    const BAR: usize = 10;
    let filled = (pct as usize * BAR) / 100;
    let empty = BAR - filled;
    let color = context_color(pct, t);
    vec![
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(empty), Style::default().fg(t.border)),
    ]
}

fn context_color(pct: u16, t: &Theme) -> ratatui::style::Color {
    if pct >= 90 {
        t.error
    } else if pct >= 70 {
        t.warn
    } else {
        t.success
    }
}

fn has_skills(app: &App) -> bool {
    app.prompt_toggles
        .as_ref()
        .is_some_and(|toggles| !toggles.is_empty())
        || !app.session.skills.is_empty()
}

fn skills_span(app: &App, t: &Theme) -> Option<Span<'static>> {
    if !has_skills(app) {
        return None;
    }
    let (named, count, color) = if let Some(toggles) = &app.prompt_toggles {
        if toggles.is_empty() {
            return None;
        }
        let on: Vec<&str> = toggles
            .iter()
            .filter(|skill| skill.include)
            .map(|skill| skill.skill_id.as_str())
            .collect();
        let total = toggles.len();
        let color = if on.is_empty() { t.text_dim } else { t.success };
        (skill_named(&on, total), skill_count(&on, total), color)
    } else {
        let names: Vec<&str> = app
            .session
            .skills
            .iter()
            .map(|skill| skill.skill_id.as_str())
            .collect();
        let total = names.len();
        (
            skill_named(&names, total),
            skill_count(&names, total),
            t.accent3,
        )
    };
    let text = if display_width(&named) <= 40 {
        named
    } else {
        count
    };
    Some(Span::styled(text, Style::default().fg(color)))
}

fn skill_count(names: &[&str], total: usize) -> String {
    if names.is_empty() {
        format!(" · {total} skills")
    } else {
        format!(" · {}/{total}", names.len())
    }
}

fn skill_named(names: &[&str], total: usize) -> String {
    if names.is_empty() {
        skill_count(names, total)
    } else {
        format!("{} {}", skill_count(names, total), names.join(" "))
    }
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(Span::width).sum()
}

fn display_width(text: &str) -> usize {
    Span::width(&Span::raw(text))
}

/// Truncate to a display width, reserving one column for an ellipsis.
fn fit_width(text: &str, max: usize) -> String {
    if max == 0 || text.is_empty() {
        return String::new();
    }
    if display_width(text) <= max {
        return text.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let next = format!("{out}{ch}");
        if display_width(&next) + 1 > max {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    while display_width(&out) > max && !out.is_empty() {
        out.pop();
    }
    out
}

fn clip_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for span in spans {
        let w = Span::width(&span);
        if used + w <= width {
            used += w;
            out.push(span);
            continue;
        }
        let remain = width.saturating_sub(used);
        if remain > 0 {
            let clipped = fit_width(span.content.as_ref(), remain);
            if !clipped.is_empty() {
                out.push(Span::styled(clipped, span.style));
            }
        }
        break;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_width_respects_display_columns() {
        assert_eq!(fit_width("build", 14), "build");
        let fitted = fit_width("abcdefghijklmnopqrstuvwxyz", 6);
        assert!(display_width(&fitted) <= 6, "{fitted}");
        assert!(fitted.ends_with('…'));
    }
}
