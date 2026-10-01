//! TUI rendering (ratatui widgets) — Cyberclaw chrome.

mod auth_picker;
mod fuzzy_list;
pub mod help;
mod input;
pub mod modal;
mod model_picker;
mod palette_view;
mod resume_picker;
mod search;
mod sidebar;
mod skill_picker;
mod splash;
mod status;
mod theme_picker;
pub(crate) mod toast;
pub(crate) mod transcript;

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

    let has_skill_chips = app
        .prompt_toggles
        .as_ref()
        .map(|t| !t.is_empty())
        .unwrap_or(false);
    let has_image_chip = !app.pending_images.is_empty();
    let has_chips = has_skill_chips || has_image_chip;

    // Top navbar (the old left panel, laid out horizontally) so the
    // transcript gets the full width. Hidden only when the terminal is
    // too short to keep a transcript under it.
    let nav_h = if area.height >= sidebar::MIN_TERMINAL_HEIGHT {
        sidebar::height(app)
    } else {
        0
    };
    let nav_rows = if nav_h == 0 {
        Layout::vertical([Constraint::Percentage(100)]).split(area)
    } else {
        Layout::vertical([Constraint::Length(nav_h), Constraint::Min(0)]).split(area)
    };
    let content = if nav_h == 0 { nav_rows[0] } else { nav_rows[1] };
    if nav_h > 0 {
        sidebar::draw(frame, app, nav_rows[0]);
    }

    let footer_h: u16 = 1;
    let status_h: u16 = 1;
    let chips_h: u16 = match (has_skill_chips, has_image_chip) {
        (true, true) => 2,
        (true, false) | (false, true) => 1,
        (false, false) => 0,
    };
    // The input box grows with soft-wrapped visual rows (1 text row + up to
    // 9 extra wrapped/newline rows), cap at 10 total, plus 2 rows for the
    // rounded border (top + bottom). The border costs 2 columns and the prompt
    // adds 1 column of padding inside it, so wrapping sees width - 4.
    let est_inner = content.width.saturating_sub(4).max(1) as usize;
    let vis_rows = crate::harness::ui::tui::input::wrap_input_rows(&app.input, est_inner).len();
    let input_h: u16 = 3 + (vis_rows_extra(vis_rows)).min(9);

    let rows = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(status_h),
        Constraint::Length(chips_h),
        Constraint::Length(input_h),
        Constraint::Length(footer_h),
    ])
    .split(content);

    transcript::draw(frame, app, rows[0]);
    status::draw(frame, app, rows[1], app.tick);
    let chips_row = rows[2];
    if has_chips {
        if has_skill_chips && has_image_chip {
            let chip_rows =
                Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(chips_row);
            skill_picker::draw_chips(frame, app, chip_rows[0]);
            input::draw_pending_images(frame, app, chip_rows[1]);
        } else if has_skill_chips {
            skill_picker::draw_chips(frame, app, chips_row);
        } else {
            input::draw_pending_images(frame, app, chips_row);
        }
    }
    if app.runtime.config.is_configured() {
        input::draw(frame, app, rows[3]);
    } else {
        draw_unconfigured_hint(frame, app, rows[3]);
    }
    draw_footer(frame, app, rows[4]);

    // Autocomplete dropdown above input.
    if app.runtime.config.is_configured()
        && app.palette.is_none()
        && app.modal.is_none()
        && !app.show_help
    {
        if let Some(ac) = &app.autocomplete {
            palette_view::draw_autocomplete(frame, ac, &app.theme, rows[3], area);
        }
    }

    // Floating toast: drawn over the transcript but under any modal, so a
    // permission prompt always wins visually.
    toast::draw(frame, app, content);

    if let Some(modal) = &app.modal {
        modal::draw(
            frame,
            modal,
            &app.theme,
            app.tick,
            area,
            &app.runtime.config,
        );
    } else if app.show_help {
        help::draw(frame, app, area);
    } else if let Some(pal) = &app.palette {
        palette_view::draw_palette(frame, pal, &app.theme, area);
    } else if app.model_picker.is_some() {
        model_picker::draw_picker(frame, app, area);
    } else if app.auth_picker.is_some() {
        auth_picker::draw(frame, app, area);
    } else if let Some(auth) = &app.auth_prompt {
        model_picker::draw_auth(frame, app, auth, area);
    } else if app.resume_picker.is_some() {
        resume_picker::draw(frame, app, area);
    } else if app.theme_picker.is_some() {
        theme_picker::draw(frame, app, area);
    } else if app.search.is_some() {
        search::draw(frame, app, area);
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
        Span::styled(" ", Style::default().fg(t.border)),
        Span::styled("?", Style::default().fg(t.accent)),
        Span::styled(" help", Style::default().fg(t.text_dim)),
        Span::styled("  ·  ", Style::default().fg(t.border)),
        Span::styled("⌃P", Style::default().fg(t.accent)),
        Span::styled(" palette", Style::default().fg(t.text_dim)),
        Span::styled("  ·  ", Style::default().fg(t.border)),
        Span::styled("⌃T", Style::default().fg(t.accent)),
        Span::styled(" theme", Style::default().fg(t.text_dim)),
        Span::styled("  ·  ", Style::default().fg(t.border)),
        Span::styled("/", Style::default().fg(t.accent2)),
        Span::styled(" commands", Style::default().fg(t.text_dim)),
        Span::styled("  ·  ", Style::default().fg(t.border)),
        Span::styled("Esc", Style::default().fg(t.accent3)),
        Span::styled(" back", Style::default().fg(t.text_dim)),
    ]);
    frame.render_widget(Paragraph::new(line).style(Style::default().bg(t.bg)), area);
}

/// Extra text rows needed beyond the first for `n` wrapped visual rows
/// (min 1: an empty/short input still uses a single row).
fn vis_rows_extra(n: usize) -> u16 {
    n.saturating_sub(1) as u16
}

/// Replaces the prompt input while no provider/model/token is configured.
fn draw_unconfigured_hint(frame: &mut Frame, app: &App, area: Rect) {
    use ratatui::style::Style;
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

    let t = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.warn))
        .title(Span::styled(
            " setup required ",
            Style::default()
                .fg(t.warn)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(Span::styled(
            "  Prompt disabled — RustClaw is not configured yet.",
            Style::default().fg(t.text_bright),
        )),
        Line::from(Span::styled(
            "  /models  pick provider + model     /auth <provider>  add token",
            Style::default().fg(t.text_dim),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
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

/// Renders the TUI into an in-memory buffer (no TTY). Used by snapshot tests.
#[cfg(test)]
pub fn render_to_buffer(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal.draw(|frame| draw(frame, app)).expect("test draw");
    terminal.backend().buffer().clone()
}

#[cfg(test)]
mod toast_tests {
    use super::*;
    use crate::harness::ui::tui::app::App;
    use crate::harness::ui::tui::draw::toast::{ToastKind, TOAST_TICKS};

    fn buffer_text(app: &mut App) -> String {
        let buf = render_to_buffer(app, 100, 40);
        buf.content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .replace('\u{0}', " ")
    }

    #[test]
    fn toast_is_rendered_over_the_transcript() {
        let mut app = App::inline_for_tests("");
        app.splash = None;
        app.push_toast_kind("copied 42 chars", ToastKind::Success);
        let text = buffer_text(&mut app);
        assert!(
            text.contains("copied 42 chars"),
            "toast text should be visible in the frame"
        );
    }

    #[test]
    fn expired_toast_disappears_after_tick() {
        let mut app = App::inline_for_tests("");
        app.splash = None;
        app.push_toast("hello toast");
        app.tick = app.tick.wrapping_add(TOAST_TICKS + 1);
        app.tick_toast();
        assert!(app.toast.is_none(), "toast should be dropped once expired");
        let text = buffer_text(&mut app);
        assert!(!text.contains("hello toast"), "expired toast must not draw");
    }
}
