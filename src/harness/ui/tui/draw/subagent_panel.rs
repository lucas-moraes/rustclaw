//! Horizontal strip of live subagent panels (Feature C).
//!
//! Rendered as a dedicated band between the transcript and the status line:
//! one rounded box per active subagent, side by side, each `1/n` of the width.
//! The band only takes vertical space while at least one panel is live, so it
//! never steals rows from the transcript when no subagent is running.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::harness::ui::tui::anim::spinner_frame;
use crate::harness::ui::tui::app::App;
use crate::harness::ui::tui::subagent::SubagentPanel;
use crate::harness::ui::tui::text::truncate_to_width;
use crate::harness::ui::tui::theme::Theme;

/// Maximum number of panels shown side by side; extra ones are dropped.
pub const MAX_PANELS: usize = 3;

/// Height of the band: rounded border (2) + status line + 2 activity lines.
pub const BAND_HEIGHT: u16 = 5;

/// Number of activity lines rendered inside each box.
const ACTIVITY_LINES: usize = 2;

/// Panels that should be visible right now (newest first, capped).
pub fn visible_panels(app: &App) -> Vec<&SubagentPanel> {
    app.subagent_panels
        .iter()
        .rev()
        .take(MAX_PANELS)
        .map(|(_, p)| p)
        .collect()
}

/// Height the band needs for the current app state (0 = hidden).
pub fn band_height(app: &App) -> u16 {
    if visible_panels(app).is_empty() {
        0
    } else {
        BAND_HEIGHT
    }
}

/// Draws the subagent band into `area`.
pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let panels = visible_panels(app);
    if panels.is_empty() || area.height == 0 || area.width == 0 {
        return;
    }
    let n = panels.len() as u32;
    let constraints: Vec<Constraint> = (0..n).map(|_| Constraint::Ratio(1, n)).collect();
    let cols = Layout::horizontal(constraints).split(area);
    for (panel, col) in panels.iter().zip(cols.iter()) {
        draw_panel(frame, app, panel, *col);
    }
}

/// Draws a single subagent box.
fn draw_panel(frame: &mut Frame, app: &App, panel: &SubagentPanel, area: Rect) {
    let theme = &app.theme;
    let (border, mark) = if panel.failed > 0 {
        (theme.error, "✗")
    } else if panel.finished {
        (theme.success, "✓")
    } else {
        (theme.accent, spinner_frame(app.tick))
    };

    // The label already carries the agent name, the depth badge and the tool
    // counters; the live mark (spinner / ✓ / ✗) is rendered on the status line
    // below, so strip the label's own static mark to avoid showing it twice.
    let label = App::subagent_panel_label(panel);
    let label = strip_label_mark(&label);
    // Title must fit between the two corners, otherwise ratatui clips it
    // mid-glyph and the right border drifts.
    let title = format!(
        " {} ",
        truncate_to_width(&label, area.width.saturating_sub(4) as usize)
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Span::styled(
            title,
            Style::default().fg(border).add_modifier(Modifier::BOLD),
        ));
    // Transparent background: inherit the frame fill (no `bg` set).
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::with_capacity(ACTIVITY_LINES + 1);
    lines.push(status_line(theme, panel, mark, border));

    // Conversation beats noise: when the subagent has exchanged messages,
    // the newest ones take the activity slots (they are what the user is
    // waiting to see), and tool lines fill whatever is left.
    let msgs = &panel.messages;
    let shown_msgs = msgs.len().min(ACTIVITY_LINES);
    let start_msg = msgs.len() - shown_msgs;
    for line in &msgs[start_msg..] {
        lines.push(Line::from(Span::styled(
            truncate_to_width(line, inner.width as usize),
            Style::default().fg(theme.accent2),
        )));
    }
    let tool_slots = ACTIVITY_LINES - shown_msgs;
    if tool_slots > 0 {
        let start = panel.lines.len().saturating_sub(tool_slots);
        for line in &panel.lines[start..] {
            lines.push(Line::from(Span::styled(
                truncate_to_width(line, inner.width as usize),
                Style::default().fg(theme.text_dim),
            )));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Removes the static status mark (`⏳` / `✓` / `✗`) that
/// `subagent_panel_label` prefixes, since the live mark is drawn separately.
fn strip_label_mark(label: &str) -> String {
    let trimmed = label.trim_start();
    for mark in ["⏳", "✓", "✗"] {
        if let Some(rest) = trimmed.strip_prefix(mark) {
            return rest.trim_start().to_string();
        }
    }
    trimmed.to_string()
}

/// Status line: spinner/check mark, agent name and tool counters.
fn status_line<'a>(
    theme: &Theme,
    panel: &SubagentPanel,
    mark: &'a str,
    color: ratatui::style::Color,
) -> Line<'a> {
    let mut spans = vec![
        Span::styled(format!("{mark} "), Style::default().fg(color)),
        Span::styled(
            panel.agent.clone(),
            Style::default()
                .fg(theme.text_bright)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} ok", panel.done),
            Style::default().fg(theme.success),
        ),
    ];
    if panel.failed > 0 {
        spans.push(Span::styled(
            format!(" · {} fail", panel.failed),
            Style::default().fg(theme.error),
        ));
    }
    // A subagent blocked on a question looks frozen otherwise; say why.
    if !panel.finished && panel.messages.last().is_some_and(|m| m.starts_with('?')) {
        spans.push(Span::styled(
            " · aguardando resposta",
            Style::default().fg(theme.accent2),
        ));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::ui::tui::app::App;
    use crate::harness::ui::tui::draw::render_to_buffer;

    fn panel(agent: &str, finished: bool) -> SubagentPanel {
        SubagentPanel {
            child_session_id: format!("c-{agent}"),
            agent: agent.into(),
            depth: 0,
            lines: vec!["· grep".into(), "✓ grep: 2".into()],
            done: 1,
            failed: 0,
            finished,
            finished_at: if finished { Some(0) } else { None },
            summary: None,
            messages: Vec::new(),
        }
    }

    fn buffer_rows(app: &mut App) -> Vec<String> {
        let buf = render_to_buffer(app, 100, 40);
        let width = buf.area.width as usize;
        let symbols: Vec<String> = buf
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect();
        symbols.chunks(width).map(|row| row.concat()).collect()
    }

    fn buffer_text(app: &mut App) -> String {
        buffer_rows(app).join("\n")
    }

    /// Counts rows that contain at least two top-left rounded corners, i.e.
    /// two boxes starting on the same terminal row.
    fn rows_with_two_boxes(rows: &[String]) -> usize {
        rows.iter().filter(|r| r.matches('╭').count() >= 2).count()
    }

    #[test]
    fn two_panels_render_side_by_side() {
        let mut app = App::inline_for_tests("");
        app.splash = None;
        app.subagent_panels = vec![
            ("a".into(), panel("explore", false)),
            ("b".into(), panel("build", false)),
        ];
        let rows = buffer_rows(&mut app);
        let text = rows.join("\n");
        assert_eq!(
            rows_with_two_boxes(&rows),
            1,
            "expected two boxes on the same row:\n{text}"
        );
        assert!(text.contains("explore"));
        assert!(text.contains("build"));
    }

    #[test]
    fn band_is_hidden_without_panels() {
        let mut app = App::inline_for_tests("");
        app.splash = None;
        assert_eq!(band_height(&app), 0);
        let rows = buffer_rows(&mut app);
        assert_eq!(rows_with_two_boxes(&rows), 0);
    }

    #[test]
    fn band_takes_rows_only_when_panels_are_live() {
        let mut app = App::inline_for_tests("");
        app.splash = None;
        app.subagent_panels = vec![("a".into(), panel("explore", false))];
        assert_eq!(band_height(&app), BAND_HEIGHT);
    }

    #[test]
    fn spinner_only_while_running() {
        let mut app = App::inline_for_tests("");
        app.splash = None;
        app.tick = 0;
        app.subagent_panels = vec![("a".into(), panel("explore", false))];
        let running = buffer_text(&mut app);
        assert!(running.contains(spinner_frame(0)));

        app.subagent_panels = vec![("a".into(), panel("explore", true))];
        let done = buffer_text(&mut app);
        assert!(done.contains('✓'));
        assert!(!done.contains(spinner_frame(0)));
    }

    #[test]
    fn long_title_does_not_break_the_border() {
        let mut app = App::inline_for_tests("");
        app.splash = None;
        app.subagent_panels = (0..3)
            .map(|i| {
                let mut p = panel("a-very-long-agent-name-that-would-overflow", false);
                p.summary = Some("x".repeat(80));
                (format!("c{i}"), p)
            })
            .collect();
        let rows = buffer_rows(&mut app);
        // Every box row must keep the same width: a clipped title would push
        // the right corner out of the column.
        for row in rows.iter().filter(|r| r.contains('╭') || r.contains('╰')) {
            assert_eq!(row.chars().count(), 100, "ragged row: {row}");
        }
    }

    #[test]
    fn conversation_lines_take_the_activity_slots() {
        let mut app = App::inline_for_tests("");
        app.splash = None;
        let mut p = panel("build", false);
        p.push_message("? pai: qual schema usar?".into());
        app.subagent_panels = vec![("a".into(), p)];
        let text = buffer_text(&mut app);
        assert!(
            text.contains("qual schema usar?"),
            "panel should show the question:\n{text}"
        );
    }

    #[test]
    fn at_most_three_panels_are_visible() {
        let mut app = App::inline_for_tests("");
        app.splash = None;
        app.subagent_panels = (0..5)
            .map(|i| (format!("c{i}"), panel(&format!("agent{i}"), false)))
            .collect();
        assert_eq!(visible_panels(&app).len(), MAX_PANELS);
    }
}
