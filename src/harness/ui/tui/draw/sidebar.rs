//! Left-side info panel: status, model, mode, session, skills, todos.
//!
//! Designed for a fixed-ish narrow column (~28–36 cols). Values wrap under
//! labels when the panel is tight, and low-priority sections drop out when
//! the terminal is short.

use crate::harness::provider::format_tokens;
use crate::harness::session::TodoStatus;
use crate::harness::ui::tui::anim;
use crate::harness::ui::tui::app::{App, MODES};
use crate::harness::ui::tui::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

/// Preferred sidebar width in columns (borders included).
pub const PREFERRED_WIDTH: u16 = 32;
/// Hide the sidebar entirely below this terminal width.
pub const MIN_TERMINAL_WIDTH: u16 = 72;
/// Absolute minimum sidebar width when shown.
pub const MIN_WIDTH: u16 = 22;

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

    if inner.width < 8 || inner.height < 3 {
        return;
    }

    // Usable text width inside the block (leave a small left gutter).
    let w = inner.width.saturating_sub(1) as usize;
    let h = inner.height as usize;

    let mut lines: Vec<Line<'static>> = Vec::new();

    // ── Model (always first & prioritized so it's never clipped) ────────
    lines.push(section("Model", t.accent));
    // In very short panels, keep model/provider on a single line each so they
    // always fit; otherwise allow stacked (wrapped) values.
    if h < 12 {
        lines.push(kv_inline("model", app.runtime.config.model.clone(), t, w));
        lines.push(kv_inline(
            "provider",
            app.runtime.config.provider.clone(),
            t,
            w,
        ));
    } else {
        lines.extend(kv_block("model", &app.runtime.config.model, t, w));
        lines.extend(kv_block("provider", &app.runtime.config.provider, t, w));
    }
    if !app.runtime.config.is_configured() {
        lines.push(Line::from(Span::styled(
            "  ⚠ token missing".to_string(),
            Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(blank());

    // ── Status ──────────────────────────────────────────────────────────
    lines.extend(status_block(app, t, w));
    lines.push(blank());

    // ── Mode ────────────────────────────────────────────────────────────
    lines.push(section("Mode", t.accent));
    lines.extend(mode_rows(active, t, w));
    lines.push(blank());

    // ── Session ─────────────────────────────────────────────────────────
    lines.push(section("Session", t.accent));
    let title = session_title(app);
    lines.extend(kv_block("title", &title, t, w));
    let proj = project_name(app);
    lines.extend(kv_block("project", &proj, t, w));
    lines.push(kv_inline(
        "msgs",
        app.session.messages.len().to_string(),
        t,
        w,
    ));
    lines.push(blank());

    // ── Context bar ─────────────────────────────────────────────────────
    lines.push(section("Context", t.accent));
    lines.extend(context_block(app, t, w));
    lines.push(blank());

    // ── Skills (if any) ─────────────────────────────────────────────────
    if !app.session.skills.is_empty() {
        lines.push(section("Skills", t.accent));
        lines.extend(skills_block(app, t, w));
        lines.push(blank());
    }

    // ── Todos (if any) ──────────────────────────────────────────────────
    if !app.session.todos.is_empty() {
        lines.push(section("Todos", t.accent));
        lines.extend(todos_block(app, t, w));
        lines.push(blank());
    }

    // ── Active tools (while running) ────────────────────────────────────
    if !app.active_tools.is_empty() {
        lines.push(section("Tools", t.accent));
        for tool in app.active_tools.iter().take(4) {
            lines.push(Line::from(vec![
                Span::styled("  ▸ ".to_string(), Style::default().fg(t.warn)),
                Span::styled(
                    truncate(&tool.name, w.saturating_sub(4)),
                    Style::default().fg(t.text_bright),
                ),
            ]));
        }
        if app.active_tools.len() > 4 {
            lines.push(Line::from(Span::styled(
                format!("  +{} more", app.active_tools.len() - 4),
                Style::default().fg(t.text_dim),
            )));
        }
        lines.push(blank());
    }

    // ── Footer hints (only when there is spare vertical room) ───────────
    let spare = h.saturating_sub(lines.len());
    if spare >= 3 {
        while lines.len() + 3 < h {
            lines.push(blank());
        }
        lines.push(Line::from(Span::styled(
            "  Tab cycle mode".to_string(),
            Style::default().fg(t.text_dim),
        )));
        lines.push(Line::from(Span::styled(
            "  /models · /skills".to_string(),
            Style::default().fg(t.text_dim),
        )));
    }

    // Clip to available height (drop trailing blanks first, then tail).
    if lines.len() > h {
        while lines.len() > h {
            if lines
                .last()
                .map(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
                .unwrap_or(false)
            {
                lines.pop();
            } else {
                break;
            }
        }
        if lines.len() > h {
            lines.truncate(h);
        }
    }

    // Absolute guarantee: model + provider are always visible. If the panel
    // is too short for the full header, render a minimal model/provider-only
    // view so the active model is never hidden.
    if h < 5 {
        let mut minimal: Vec<Line<'static>> = Vec::new();
        minimal.push(section("Model", t.accent));
        minimal.push(kv_inline("model", app.runtime.config.model.clone(), t, w));
        if h >= 4 {
            minimal.push(kv_inline(
                "provider",
                app.runtime.config.provider.clone(),
                t,
                w,
            ));
        }
        lines = minimal;
    }

    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(t.surface))
            .wrap(Wrap { trim: false }),
        inner,
    );
}

// ─── blocks ────────────────────────────────────────────────────────────────

fn status_block(app: &App, t: &Theme, w: usize) -> Vec<Line<'static>> {
    let (icon, label, fg) = if app.modal.is_some() {
        ("?", "waiting", t.warn)
    } else if app.running {
        (
            anim::spinner_frame(app.tick),
            app.status_msg.as_deref().unwrap_or("streaming"),
            t.accent2,
        )
    } else if !app.runtime.config.is_configured() {
        ("○", "setup needed", t.warn)
    } else {
        ("●", "idle", t.success)
    };

    let label = truncate(label, w.saturating_sub(4));
    vec![Line::from(vec![
        Span::styled(
            format!(" {icon} "),
            Style::default().fg(fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(label, Style::default().fg(fg).add_modifier(Modifier::BOLD)),
    ])]
}

fn mode_rows(active: &str, t: &Theme, w: usize) -> Vec<Line<'static>> {
    let mut out = Vec::with_capacity(MODES.len());
    // If the active agent is outside the cycle (custom), show it first.
    let known = MODES.contains(&active);
    if !known && !active.is_empty() {
        out.push(mode_row(active, true, t, w));
    }
    for mode in MODES {
        out.push(mode_row(mode, *mode == active, t, w));
    }
    out
}

fn mode_row(mode: &str, is_active: bool, t: &Theme, w: usize) -> Line<'static> {
    let marker = if is_active { "▶" } else { " " };
    let label = truncate(&format!(" {marker} {mode}"), w.saturating_sub(1));
    if is_active {
        Line::from(Span::styled(
            label,
            Style::default()
                .fg(t.bg)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from(Span::styled(label, Style::default().fg(t.text_dim)))
    }
}

fn context_block(app: &App, t: &Theme, w: usize) -> Vec<Line<'static>> {
    let ctx = app.context_tokens() as u64;
    let max = app.max_context_tokens() as u64;
    let free = max.saturating_sub(ctx);
    let pct = if max == 0 {
        0u16
    } else {
        ((ctx * 100) / max).min(100) as u16
    };
    let bar_fg = context_color(pct, t);

    // Progress bar fills the usable width: "  ████░░░░  42%"
    let pct_label = format!("{pct:>3}%");
    let bar_w = w
        .saturating_sub(2 /* gutter */ + 1 /* gap */ + pct_label.len())
        .clamp(6, 28);
    let filled = ((pct as usize) * bar_w) / 100;
    let empty = bar_w.saturating_sub(filled);

    let mut lines = Vec::new();

    // Row 1: continuous bar + percent (color shifts with pressure).
    lines.push(Line::from(vec![
        Span::styled("  ".to_string(), Style::default()),
        Span::styled("━".repeat(filled), Style::default().fg(bar_fg)),
        Span::styled("─".repeat(empty), Style::default().fg(t.border)),
        Span::styled(" ".to_string(), Style::default()),
        Span::styled(
            pct_label,
            Style::default().fg(bar_fg).add_modifier(Modifier::BOLD),
        ),
    ]));

    // Row 2: used / max  ·  free left
    let used_s = format_tokens(ctx);
    let max_s = format_tokens(max);
    let free_s = format_tokens(free);
    let used_line = format!("{used_s}/{max_s}");
    // Prefer "used/max · free left" when it fits; otherwise stack free.
    let free_part = format!(" · {free_s} free");
    if used_line.chars().count() + free_part.chars().count() + 2 <= w {
        lines.push(Line::from(vec![
            Span::styled("  ".to_string(), Style::default()),
            Span::styled(used_line, Style::default().fg(t.text_bright)),
            Span::styled(free_part, Style::default().fg(t.text_dim)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("  ".to_string(), Style::default()),
            Span::styled(used_line, Style::default().fg(t.text_bright)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  ".to_string(), Style::default()),
            Span::styled(format!("{free_s} free"), Style::default().fg(t.text_dim)),
        ]));
    }

    // Row 3: session totals (in/out) — compact chip style.
    let sess_in = format_tokens(app.session_usage.input_tokens);
    let sess_out = format_tokens(app.session_usage.output_tokens);
    let sess_total = format_tokens(app.session_usage.total());
    lines.push(Line::from(vec![
        Span::styled("  in ".to_string(), Style::default().fg(t.text_dim)),
        Span::styled(sess_in, Style::default().fg(t.accent)),
        Span::styled("  out ".to_string(), Style::default().fg(t.text_dim)),
        Span::styled(sess_out, Style::default().fg(t.accent2)),
        Span::styled("  Σ ".to_string(), Style::default().fg(t.text_dim)),
        Span::styled(
            sess_total,
            Style::default()
                .fg(t.text_bright)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Row 4 (optional): last turn usage + iterations.
    if app.last_iterations > 0 || app.last_usage.total() > 0 {
        let last = format!(
            "{} · {}it",
            format_tokens(app.last_usage.total()),
            app.last_iterations
        );
        lines.push(kv_inline("last", last, t, w));
    }

    // Pressure hint when context is getting tight.
    if pct >= 90 {
        lines.push(Line::from(Span::styled(
            "  ⚠ near limit".to_string(),
            Style::default().fg(t.error).add_modifier(Modifier::BOLD),
        )));
    } else if pct >= 70 {
        lines.push(Line::from(Span::styled(
            "  · compaction soon".to_string(),
            Style::default().fg(t.warn),
        )));
    }

    lines
}

fn context_color(pct: u16, t: &Theme) -> ratatui::style::Color {
    if pct >= 90 {
        t.error
    } else if pct >= 70 {
        t.warn
    } else if pct >= 40 {
        t.info
    } else {
        t.success
    }
}

fn skills_block(app: &App, t: &Theme, w: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // Prefer the live prompt toggles (per-turn) when present.
    if let Some(toggles) = &app.prompt_toggles {
        for sk in toggles.iter().take(6) {
            let mark = if sk.include { "✓" } else { "·" };
            let fg = if sk.include { t.success } else { t.text_dim };
            lines.push(Line::from(vec![
                Span::styled(format!("  {mark} "), Style::default().fg(fg)),
                Span::styled(
                    truncate(&sk.skill_id, w.saturating_sub(4)),
                    Style::default().fg(if sk.include {
                        t.text_bright
                    } else {
                        t.text_dim
                    }),
                ),
            ]));
        }
        if toggles.len() > 6 {
            lines.push(Line::from(Span::styled(
                format!("  +{} more", toggles.len() - 6),
                Style::default().fg(t.text_dim),
            )));
        }
    } else {
        for sk in app.session.skills.iter().take(6) {
            lines.push(Line::from(vec![
                Span::styled("  · ".to_string(), Style::default().fg(t.accent3)),
                Span::styled(
                    truncate(&sk.skill_id, w.saturating_sub(4)),
                    Style::default().fg(t.text_bright),
                ),
            ]));
        }
    }
    lines
}

fn todos_block(app: &App, t: &Theme, w: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // Show in-progress first, then pending, then a short completed count.
    let mut items = app.session.todos.clone();
    items.sort_by_key(|td| match td.status {
        TodoStatus::InProgress => 0,
        TodoStatus::Pending => 1,
        TodoStatus::Completed => 2,
        TodoStatus::Cancelled => 3,
    });
    let mut shown = 0usize;
    let mut hidden_done = 0usize;
    for td in &items {
        if shown >= 5 {
            if matches!(td.status, TodoStatus::Completed | TodoStatus::Cancelled) {
                hidden_done += 1;
            }
            continue;
        }
        let (mark, fg) = match td.status {
            TodoStatus::InProgress => ("▸", t.warn),
            TodoStatus::Pending => ("○", t.text_dim),
            TodoStatus::Completed => ("✓", t.success),
            TodoStatus::Cancelled => ("✗", t.error),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {mark} "), Style::default().fg(fg)),
            Span::styled(
                truncate(&td.content, w.saturating_sub(4)),
                Style::default().fg(t.text_bright),
            ),
        ]));
        shown += 1;
    }
    let remaining = items.len().saturating_sub(shown);
    if remaining > 0 {
        lines.push(Line::from(Span::styled(
            format!(
                "  +{} more{}",
                remaining,
                if hidden_done > 0 {
                    format!(" ({hidden_done} done)")
                } else {
                    String::new()
                }
            ),
            Style::default().fg(t.text_dim),
        )));
    }
    lines
}

// ─── helpers ───────────────────────────────────────────────────────────────

fn section(label: &str, fg: ratatui::style::Color) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {}", label.to_uppercase()),
        Style::default().fg(fg).add_modifier(Modifier::BOLD),
    ))
}

fn blank() -> Line<'static> {
    Line::from("")
}

/// Key + value on one line when it fits; otherwise value on the next line.
fn kv_block(key: &str, value: &str, t: &Theme, w: usize) -> Vec<Line<'static>> {
    let key_col = 10usize.min(w.saturating_sub(2));
    let key_s = format!("  {key:<key_col$}");
    let avail = w.saturating_sub(key_s.chars().count());
    if avail >= 4 && value.chars().count() <= avail {
        vec![Line::from(vec![
            Span::styled(key_s, Style::default().fg(t.text_dim)),
            Span::styled(value.to_string(), Style::default().fg(t.text_bright)),
        ])]
    } else {
        // Stacked: label, then indented value (may still truncate).
        vec![
            Line::from(Span::styled(
                format!("  {key}"),
                Style::default().fg(t.text_dim),
            )),
            Line::from(Span::styled(
                format!("    {}", truncate(value, w.saturating_sub(4))),
                Style::default().fg(t.text_bright),
            )),
        ]
    }
}

fn kv_inline(key: &str, value: String, t: &Theme, w: usize) -> Line<'static> {
    let key_col = 10usize.min(w.saturating_sub(2));
    let key_s = format!("  {key:<key_col$}");
    let avail = w.saturating_sub(key_s.chars().count());
    Line::from(vec![
        Span::styled(key_s, Style::default().fg(t.text_dim)),
        Span::styled(truncate(&value, avail), Style::default().fg(t.text_bright)),
    ])
}

fn session_title(app: &App) -> String {
    app.session.display_title()
}

fn project_name(app: &App) -> String {
    app.cwd
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| app.cwd.display().to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_ascii() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 8), "hello w…");
        assert_eq!(truncate("x", 0), "");
        assert_eq!(truncate("xy", 1), "…");
    }

    #[test]
    fn test_truncate_unicode() {
        assert_eq!(truncate("ação", 10), "ação");
        assert_eq!(truncate("ação longa", 5), "ação…");
    }
}
