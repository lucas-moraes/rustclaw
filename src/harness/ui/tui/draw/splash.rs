//! Boot splash with ASCII claw + particles.

use crate::harness::ui::tui::anim::{self, SplashState};
use crate::harness::ui::tui::theme::Theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, splash: &mut SplashState, theme: &Theme, area: Rect) {
    frame.render_widget(Clear, area);

    splash.advance();
    anim::step_particles(
        &mut splash.particles,
        area.width,
        area.height.saturating_sub(2),
        splash.frame,
        36,
    );

    // Paint particles first (background).
    for p in &splash.particles {
        let x = area.x.saturating_add(p.x as u16);
        let y = area.y.saturating_add(p.y as u16);
        if x < area.x + area.width && y < area.y + area.height {
            frame.render_widget(
                Paragraph::new(Span::styled(p.ch.to_string(), p.style(theme))),
                Rect {
                    x,
                    y,
                    width: 1,
                    height: 1,
                },
            );
        }
    }

    let logo = anim::claw_logo(splash.frame);
    let logo_h = logo.len() as u16 + 4;
    let logo_w = logo.iter().map(|l| l.chars().count()).max().unwrap_or(20) as u16;

    let v = Layout::vertical([
        Constraint::Percentage(
            ((100u16.saturating_sub(logo_h.saturating_mul(100) / area.height.max(1))) / 2).min(40),
        ),
        Constraint::Length(logo_h.min(area.height)),
        Constraint::Min(0),
    ])
    .split(area);

    let h = Layout::horizontal([
        Constraint::Percentage(
            ((100u16.saturating_sub(logo_w.saturating_mul(100) / area.width.max(1))) / 2).min(40),
        ),
        Constraint::Length(logo_w.min(area.width)),
        Constraint::Min(0),
    ])
    .split(v[1]);

    let pulse = anim::pulse_border(splash.frame, theme);
    let mut lines: Vec<Line> = logo
        .into_iter()
        .map(|row| {
            Line::from(Span::styled(
                row.to_string(),
                Style::default().fg(pulse).add_modifier(Modifier::BOLD),
            ))
        })
        .collect();

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "RUSTCLAW",
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )));

    let sub = anim::splash_subtitle(splash.frame, theme.name);
    if !sub.is_empty() {
        lines.push(Line::from(Span::styled(
            sub,
            Style::default().fg(theme.text_dim),
        )));
    }

    // Progress bar
    let prog_w = 24.min(area.width as usize);
    let filled = ((splash.frame as usize * prog_w) / splash.max_frames as usize).min(prog_w);
    let bar = format!(
        "{}{}",
        "█".repeat(filled),
        "░".repeat(prog_w.saturating_sub(filled))
    );
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        bar,
        Style::default().fg(theme.accent2),
    )));

    frame.render_widget(Paragraph::new(lines), h[1]);
}
