//! Multi-section help overlay.

use crate::harness::ui::tui::app::App;
use crate::harness::ui::tui::draw::centered_rect;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

const SECTIONS: &[&str] = &["keys", "commands", "agents", "tips"];

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let help_area = centered_rect(72, 70, area);
    frame.render_widget(Clear, help_area);

    let sec = app.help_section % SECTIONS.len();
    let tabs: Vec<Span> = SECTIONS
        .iter()
        .enumerate()
        .flat_map(|(i, name)| {
            let style = if i == sec {
                Style::default()
                    .fg(t.bg)
                    .bg(t.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.text_dim)
            };
            vec![Span::styled(format!(" {} ", name), style), Span::raw(" ")]
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent))
        .title(Span::styled(
            " help · Tab sections · Esc close ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface).fg(t.text));

    let mut lines = vec![Line::from(tabs), Line::from("")];
    lines.extend(section_body(sec, t));

    let inner = block.inner(help_area);
    frame.render_widget(block, help_area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn section_body(sec: usize, t: &crate::harness::ui::tui::theme::Theme) -> Vec<Line<'static>> {
    match sec {
        0 => keys(t),
        1 => commands(t),
        2 => agents(t),
        _ => tips(t),
    }
}

fn kv(key: &str, desc: &str, t: &crate::harness::ui::tui::theme::Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<14}", key),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc.to_string(), Style::default().fg(t.text)),
    ])
}

fn keys(t: &crate::harness::ui::tui::theme::Theme) -> Vec<Line<'static>> {
    vec![
        kv("Ctrl+C", "quit (exit the project)", t),
        kv("Enter", "send prompt (Shift/Alt+Enter new line)", t),
        kv("Ctrl+J", "insert line break (macOS fallback)", t),
        kv("Ctrl+A / E", "line start / end", t),
        kv("Ctrl+U / W", "kill to line start / kill word", t),
        kv("Ctrl+Z", "reset prompt input", t),
        kv(
            "Esc",
            "cancel streaming/run · close overlay · clear draft",
            t,
        ),
        kv("Up/Down", "prompt history", t),
        kv("PgUp/PgDn", "scroll transcript", t),
        kv("Drag", "select transcript text (auto-copy)", t),
        kv("Ctrl+C", "copy selection · quit if none", t),
        kv("Ctrl+P", "command palette", t),
        kv("Ctrl+T", "cycle color theme", t),
        kv("Ctrl+L", "clear local transcript", t),
        kv("? / F1", "this help", t),
        kv("Tab", "autocomplete (in /) · cycle mode", t),
        kv("y / n / a", "permission allow · deny · always", t),
        kv("1..n / type", "answer question (option or free text)", t),
    ]
}

fn commands(t: &crate::harness::ui::tui::theme::Theme) -> Vec<Line<'static>> {
    vec![
        kv("/help", "list commands", t),
        kv("/new", "fresh session", t),
        kv("/sessions", "manage sessions (picker)", t),
        kv("/sessions select <id>", "load a session by id", t),
        kv("/agent name", "switch agent", t),
        kv("/compact", "summarize old messages (also auto on open)", t),
        kv("/theme name", "cyberclaw · aurora · ember · mono", t),
        kv("/usage", "tokens in/out + context window", t),
        kv("/memory", "list · rm <id> · clear project memory", t),
        kv("/models", "switch provider/model (picker)", t),
        kv("/model name", "set model for this project", t),
        kv("/provider name", "set provider (default model)", t),
        kv("/auth provider", "save API token (auth.json)", t),
        kv("/settings", "view · set iterations/context/theme", t),
        kv("/skills", "manage session skill memory", t),
        kv("/exit", "quit", t),
    ]
}

fn agents(t: &crate::harness::ui::tui::theme::Theme) -> Vec<Line<'static>> {
    vec![
        kv("build", "implement features and fix bugs", t),
        kv("plan", "design approach before coding", t),
        kv("explore", "read-only codebase exploration", t),
        kv("general", "multi-purpose assistant", t),
    ]
}

fn tips(t: &crate::harness::ui::tui::theme::Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "  · Start a line with / for slash autocomplete",
            Style::default().fg(t.text),
        )),
        Line::from(Span::styled(
            "  · Ctrl+P searches commands, agents, themes, actions",
            Style::default().fg(t.text),
        )),
        Line::from(Span::styled(
            "  · RUSTCLAW_THEME=aurora|ember|mono overrides default",
            Style::default().fg(t.text),
        )),
        Line::from(Span::styled(
            "  · NO_COLOR forces the mono theme",
            Style::default().fg(t.text),
        )),
        Line::from(Span::styled(
            "  · Mouse wheel scrolls the transcript",
            Style::default().fg(t.text),
        )),
        Line::from(Span::styled(
            "  · Drag to select text · release auto-copies · Esc clears",
            Style::default().fg(t.text),
        )),
        Line::from(Span::styled(
            "  · Outside-CWD tool paths ask permission",
            Style::default().fg(t.text),
        )),
    ]
}

pub fn section_count() -> usize {
    SECTIONS.len()
}
