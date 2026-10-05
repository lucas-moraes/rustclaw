//! Top navbar: row 1 is app/system status (brand, ready state, git branch,
//! status chip); row 2 is session info (messages, skills, model) and the mode
//! selector; row 3 is the Cursor CLI feedback rail, shown only when a Cursor
//! mode is on. Each row is left/right aligned; on narrow terminals the right
//! side sheds optional pieces instead of squeezing. Width is measured with
//! `Span::width`, not `chars().count()`.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;

use crate::harness::ui::tui::app::{App, MODES};
use crate::harness::ui::tui::draw::modal::cyber_badge;
use crate::harness::ui::tui::theme::Theme;

/// Two text rows, plus a third when the Cursor feedback rail is visible.
pub const HEIGHT: u16 = 2;
/// Hide the bar when the terminal is too short to keep a transcript under it.
pub const MIN_TERMINAL_HEIGHT: u16 = 16;

/// Rows the navbar actually occupies: `HEIGHT`, plus three for the Cursor
/// feedback rail (rounded border top, content, border bottom) when a Cursor
/// mode is on.
pub fn height(app: &App) -> u16 {
    let cfg = &app.runtime.config;
    if cfg.cursor_agent || cfg.cursor_plan {
        HEIGHT + 3
    } else {
        HEIGHT
    }
}

/// Cursor CLI feedback rail content: ` cursor ● build ● plan`. `None` when no
/// Cursor mode is on.
fn feedback_spans(app: &App) -> Option<Vec<Span<'static>>> {
    let t = &app.theme;
    let cfg = &app.runtime.config;
    if !cfg.cursor_agent && !cfg.cursor_plan {
        return None;
    }
    let accent = t.accent2;
    let mut spans = vec![Span::styled(
        " cursor ",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )];
    let mut push = |label: &str, on: bool, model: &str| {
        let (mark, fg) = if on {
            ("●", t.success)
        } else {
            ("○", t.text_dim)
        };
        spans.push(Span::styled(
            format!(" {mark} {label}"),
            Style::default().fg(fg),
        ));
        if on && !model.is_empty() {
            spans.push(Span::styled(
                format!(" ({model})"),
                Style::default().fg(t.text_dim),
            ));
        }
    };
    push("build", cfg.cursor_agent, &cfg.cursor_model);
    push("plan", cfg.cursor_plan, &cfg.cursor_plan_model);
    Some(spans)
}

/// Draws the Cursor rail as a rounded box, right-aligned under the mode
/// selector. Skipped when the terminal is too narrow to fit the box.
fn draw_feedback(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let Some(spans) = feedback_spans(app) else {
        return;
    };
    let content_w = spans_width(&spans) as u16;
    let box_w = content_w + 2;
    if box_w > area.width || area.height < 3 {
        return;
    }
    let x = area.x + area.width - box_w;
    let rail = Rect {
        x,
        y: area.y + 2,
        width: box_w,
        height: 3,
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.accent2));
    frame.render_widget(Paragraph::new(Line::from(spans)).block(block), rail);
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let lines = vec![system_line(app, t, width), session_line(app, t, width)];
    frame.render_widget(Paragraph::new(lines), area);
    draw_feedback(frame, app, area);
    cyber_badge(frame, area, t.accent, &badge_label());
}

/// Brand badge text: `RUSTCLAW v<version>` (version from Cargo.toml).
fn badge_label() -> String {
    format!("RUSTCLAW v{}", env!("CARGO_PKG_VERSION"))
}

/// Width of the brand badge chip, including the diagonal `╱` cut.
/// cyber_badge renders the label padded with one space on each side
/// (`" {title} "`), plus the 1-column diagonal cut — so the text after the
/// badge must start past all of that.
fn badge_width() -> usize {
    Span::width(&Span::raw(format!(" {} ", badge_label()))) + 1
}

// ---------------------------------------------------------------- row 1

struct SystemOpts {
    branch: bool,
}

fn system_line(app: &App, t: &Theme, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }
    let mut opts = SystemOpts { branch: true };
    loop {
        let left = system_left(app, t);
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
        if opts.branch {
            opts.branch = false;
        } else {
            let fallback = clip_spans(left, width);
            return Line::from(fallback);
        }
    }
}

fn system_left(app: &App, t: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    // The badge is painted over the first columns of this row, so pad past it
    // before drawing the branch.
    spans.push(Span::raw(" ".repeat(badge_width())));
    if !app.project_name.is_empty() {
        spans.push(Span::styled(
            format!(" {}", app.project_name),
            Style::default().fg(t.text),
        ));
    }
    if !app.git_branch.is_empty() {
        spans.push(Span::styled(
            format!(" /{} ", app.git_branch),
            Style::default().fg(t.text_dim),
        ));
    }
    spans
}

// ---------------------------------------------------------------- row 2

struct SessionOpts {
    title: bool,
    skills: bool,
    provider: bool,
    model_max: usize,
}

/// Per-mode accent so the selector reads at a glance: the active mode is a
/// filled chip in its own color, the others stay dim. Colors come from
/// `Theme::mode_accent` (the same source the theme tint uses), with a
/// `text_dim` fallback for agents outside the cycle.
fn mode_color(mode: &str, t: &Theme) -> Color {
    let accent = if t.is_light() {
        Theme::mode_accent_light(mode)
    } else {
        Theme::mode_accent(mode)
    };
    accent.unwrap_or(t.text_dim)
}

/// The mode selector: every known mode, the active one highlighted.
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
        if on {
            let fg = mode_color(name, t);
            spans.push(Span::styled(
                format!(" {name} "),
                Style::default().fg(fg).add_modifier(Modifier::BOLD),
            ));
            if cursor {
                spans.push(Span::styled(
                    " →".to_string(),
                    Style::default().fg(t.accent2),
                ));
            }
        } else {
            spans.push(Span::styled(
                format!(" {name} "),
                Style::default().fg(t.text_dim),
            ));
        }
    };
    if !MODES.contains(&active) {
        push(active, true);
    }
    for mode in MODES {
        push(mode, *mode == active);
    }
    spans
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
        let left_w = spans_width(&left);
        if left_w <= width {
            return Line::from(left);
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
            return Line::from(clip_spans(left, width));
        }
    }
}

fn session_left(app: &App, t: &Theme, opts: &SessionOpts) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    if opts.skills {
        if let Some(skill) = skills_span(app, t) {
            spans.push(skill);
        }
    }
    spans
}

// ---------------------------------------------------------------- helpers

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
