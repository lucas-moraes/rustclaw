//! TUI rendering (ratatui widgets) — Cyberclaw chrome.

pub mod help;
mod input;
mod modal;
mod model_picker;
mod palette_view;
mod sidebar;
mod skill_picker;
mod splash;
mod status;
mod transcript;

use crate::harness::ui::tui::app::App;
use crate::harness::ui::tui::theme::Theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::Clear;
use ratatui::Frame;

/// Draws the full TUI.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Full-screen background clear with theme bg via empty block feel.
    frame.render_widget(Clear, area);

    if let Some(splash) = app.splash.as_mut() {
        splash::draw(frame, splash, &app.theme, area);
        return;
    }

    // Re-derive the theme from the base palette each frame, tinted by the
    // active agent mode (build=blue, plan=yellow, explore=orange, purple=general).
    app.theme = Theme::from_index(app.theme_id).with_mode(&app.session.agent);

    let has_chips = app
        .prompt_toggles
        .as_ref()
        .map(|t| !t.is_empty())
        .unwrap_or(false);

    // Left info panel + main content column.
    let cols =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)]).split(area);
    let content = cols[1];
    sidebar::draw(frame, app, cols[0]);

    let footer_h: u16 = 1;
    let status_h: u16 = 1;
    let chips_h: u16 = if has_chips { 1 } else { 0 };
    let input_h: u16 = 3;

    let rows = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(status_h),
        Constraint::Length(chips_h),
        Constraint::Length(input_h),
        Constraint::Length(footer_h),
    ])
    .split(content);

    transcript::draw(frame, app, rows[0]);
    status::draw(frame, app, rows[1]);
    if has_chips {
        skill_picker::draw_chips(frame, app, rows[2]);
    }
    input::draw(frame, app, rows[3]);
    draw_footer(frame, app, rows[4]);

    // Autocomplete dropdown above input.
    if app.palette.is_none() && app.modal.is_none() && !app.show_help {
        if let Some(ac) = &app.autocomplete {
            palette_view::draw_autocomplete(frame, ac, &app.theme, rows[3], area);
        }
    }

    if let Some(modal) = &app.modal {
        modal::draw(frame, modal, &app.theme, app.tick, area);
    } else if app.show_help {
        help::draw(frame, app, area);
    } else if let Some(pal) = &app.palette {
        palette_view::draw_palette(frame, pal, &app.theme, area);
    } else if let Some(picker) = &app.model_picker {
        model_picker::draw_picker(frame, app, picker, area);
    } else if let Some(auth) = &app.auth_prompt {
        model_picker::draw_auth(frame, app, auth, area);
    } else if app.skill_picker.is_some() {
        skill_picker::draw(frame, app, area);
    }
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    use ratatui::style::Style;
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Paragraph;

    let t = &app.theme;
    let line = Line::from(vec![
        Span::styled(" ? ", Style::default().fg(t.accent)),
        Span::styled("help  ", Style::default().fg(t.text_dim)),
        Span::styled("Ctrl+P ", Style::default().fg(t.accent)),
        Span::styled("palette  ", Style::default().fg(t.text_dim)),
        Span::styled("Ctrl+T ", Style::default().fg(t.accent)),
        Span::styled("theme  ", Style::default().fg(t.text_dim)),
        Span::styled("/ ", Style::default().fg(t.accent2)),
        Span::styled("commands  ", Style::default().fg(t.text_dim)),
        Span::styled("Esc ", Style::default().fg(t.accent3)),
        Span::styled("back", Style::default().fg(t.text_dim)),
    ]);
    frame.render_widget(Paragraph::new(line).style(Style::default().bg(t.bg)), area);
}

/// Centered rect helper shared by overlays.
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}

pub fn centered_rect_fixed(width: u16, height: u16, r: Rect) -> Rect {
    let width = width.min(r.width);
    let height = height.min(r.height);
    let x = r.x + (r.width.saturating_sub(width)) / 2;
    let y = r.y + (r.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}
