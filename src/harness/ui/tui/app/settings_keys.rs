//! Settings/model/audio key handling, extracted from `keys.rs` (R6 split).

use crossterm::event::KeyEvent;

use super::state::{App, CursorModelTarget, Modal};

/// Runs `agent --list-models` and parses the model ids from its output.
/// Returns an empty vec if the CLI is missing or the output can't be parsed.
pub(super) fn list_cursor_models() -> Vec<(String, String)> {
    let out = match std::process::Command::new("agent")
        .arg("--list-models")
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    parse_model_list(&text)
}

/// Parses `agent --list-models` output into `(id, description)` pairs. Lines
/// look like `  gpt-5.3-codex - Codex 5.3` or a bare `  claude-4.5-sonnet`; the
/// id is the first whitespace-delimited token and the description is whatever
/// follows the ` - ` separator (empty when absent).
pub(super) fn parse_model_list(text: &str) -> Vec<(String, String)> {
    let mut models: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Skip obvious header lines ("Available models", "Models:", ...).
        if line.ends_with(':') && !line.contains(' ') {
            continue;
        }
        if line.eq_ignore_ascii_case("available models") || line.eq_ignore_ascii_case("models") {
            continue;
        }
        let id = line.split_whitespace().next().unwrap_or("");
        // Drop a trailing dash separator ("gpt-5 - desc").
        let id = id.trim_end_matches('-').trim();
        if id.is_empty() || id.eq_ignore_ascii_case("model") || id.eq_ignore_ascii_case("models") {
            continue;
        }
        // Description = text after the first " - " separator, if any.
        let desc = line
            .split_once(" - ")
            .map(|(_, d)| d.trim().to_string())
            .unwrap_or_default();
        if !models.iter().any(|(m, _)| m == id) {
            models.push((id.to_string(), desc));
        }
    }
    models
}

/// Toggleable settings, in the same order as `settings_rows` in draw/modal.rs.
/// Flip a boolean setting, persist it, and re-sync the runtime.
/// Shared key handling for the settings-like modals (`/settings` and
/// `/cursor`): navigate over `rows`, flip toggleable rows with Space/Enter,
/// and open the model picker on the `cursor_model` row.
///
/// `template` is the modal to keep open while navigating (with its `selected`
/// field ignored); the picker, when opened, replaces it directly —
/// `close_modal()` would pop the queue and discard it.
pub(super) fn handle_settings_like_key(
    app: &mut App,
    key: KeyEvent,
    rows: Vec<(String, String, bool)>,
    selected: usize,
    template: Modal,
) {
    use crossterm::event::KeyCode;
    let n = rows.len();
    let mut sel = selected.min(n.saturating_sub(1));
    // `Keep` = stay on this modal; `Replace` = swap in another modal (the
    // picker); `Close` = dismiss entirely.
    enum Next {
        Keep,
        Replace(Modal),
        Close,
    }
    let mut next = Next::Keep;
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => next = Next::Close,
        KeyCode::Up | KeyCode::Char('k') => {
            sel = sel.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if sel + 1 < n {
                sel += 1;
            }
        }
        KeyCode::Char(' ') | KeyCode::Enter => {
            if let Some((label, _, toggleable)) = rows.get(sel) {
                if *toggleable {
                    // The row label *is* the setting key (cursor_agent /
                    // cursor_plan), so no per-row mapping is needed.
                    toggle_setting(app, label);
                } else if let Some(target) = cursor_model_target(label) {
                    let mut models = vec![("auto".to_string(), "CLI default".to_string())];
                    models.extend(list_cursor_models());
                    let current = match target {
                        CursorModelTarget::Build => app.runtime.config.cursor_model.clone(),
                        CursorModelTarget::Plan => app.runtime.config.cursor_plan_model.clone(),
                    };
                    let selected = models
                        .iter()
                        .position(|(m, _)| m == &current || (current.is_empty() && m == "auto"))
                        .unwrap_or(0);
                    next = Next::Replace(Modal::CursorModel {
                        selected,
                        scroll_offset: 0,
                        filter: String::new(),
                        models,
                        target,
                    });
                }
            }
        }
        _ => {}
    }
    match next {
        Next::Keep => {
            app.modal = Some(match template {
                Modal::Cursor { .. } => Modal::Cursor { selected: sel },
                _ => Modal::Settings { selected: sel },
            });
        }
        Next::Replace(modal) => app.modal = Some(modal),
        Next::Close => app.close_modal(),
    }
}

pub(super) fn toggle_setting(app: &mut App, field: &str) {
    let (value, saved, note) = match field {
        "cursor_agent" => {
            let v = !app.runtime.config.cursor_agent;
            (
                v,
                app.runtime.set_cursor_agent(v),
                "build mode now uses the Cursor CLI",
            )
        }
        "cursor_plan" => {
            let v = !app.runtime.config.cursor_plan;
            (
                v,
                app.runtime.set_cursor_plan(v),
                "plan mode now uses the Cursor CLI",
            )
        }
        "voice_enabled" => {
            let v = !app.runtime.config.voice_enabled;
            (
                v,
                app.runtime.set_voice_enabled(v),
                "push-to-talk (Ctrl+R) in the TUI",
            )
        }
        _ => return,
    };
    match saved {
        Ok(()) => app.add_system(&format!(
            "{} = {} ({})",
            field,
            if value { "on" } else { "off" },
            note
        )),
        Err(e) => app.add_system(&format!("[error] failed to save settings: {}", e)),
    }
}

/// Maps a Cursor modal row label to the model knob it edits, if any.
pub(super) fn cursor_model_target(label: &str) -> Option<CursorModelTarget> {
    match label {
        "cursor_model" => Some(CursorModelTarget::Build),
        "cursor_plan_model" => Some(CursorModelTarget::Plan),
        _ => None,
    }
}

/// Key handler for the `/audio-settings` modal. Space toggles the highlighted
/// boolean row; Enter on `stt_model` opens an inline text prompt; Esc closes.
pub(super) fn handle_audio_settings_key(
    app: &mut App,
    key: KeyEvent,
    selected: usize,
    custom_input: Option<String>,
) {
    if custom_input.is_some() {
        audio_custom_input_key(app, key, selected, custom_input);
    } else {
        audio_nav_key(app, key, selected);
    }
}

/// Inline text entry for the STT model name (the `stt_model` row).
pub(super) fn audio_custom_input_key(
    app: &mut App,
    key: KeyEvent,
    selected: usize,
    custom_input: Option<String>,
) {
    use crossterm::event::KeyCode;
    let sel = selected;
    let Some(mut inp) = custom_input else {
        return;
    };
    match key.code {
        KeyCode::Esc => {}
        KeyCode::Backspace => {
            inp.pop();
        }
        KeyCode::Enter => {
            let v = inp.trim().to_string();
            match app.runtime.set_stt_model(v.clone()) {
                Ok(()) => {
                    let shown = if v.is_empty() { "default".into() } else { v };
                    app.add_system(&format!("stt_model = {}", shown));
                }
                Err(e) => app.add_system(&format!("[error] failed to save: {}", e)),
            }
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            inp.push(c);
        }
        _ => {}
    }
    // Keep the modal open; re-open with the (possibly updated) input.
    let keep_input = !matches!(key.code, KeyCode::Esc | KeyCode::Enter);
    app.modal = Some(Modal::AudioSettings {
        selected: sel,
        custom_input: if keep_input { Some(inp) } else { None },
    });
}

/// Navigation/toggle phase of the `/audio-settings` modal.
pub(super) fn audio_nav_key(app: &mut App, key: KeyEvent, selected: usize) {
    use crossterm::event::KeyCode;
    let mut sel = selected;
    let rows = crate::harness::ui::tui::draw::modal::audio_rows(&app.runtime.config);
    let n = rows.len();
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.close_modal(),
        KeyCode::Up | KeyCode::Char('k') => {
            sel = sel.saturating_sub(1);
            app.modal = Some(Modal::AudioSettings {
                selected: sel,
                custom_input: None,
            });
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if sel + 1 < n {
                sel += 1;
            }
            app.modal = Some(Modal::AudioSettings {
                selected: sel,
                custom_input: None,
            });
        }
        KeyCode::Char(' ') => {
            if let Some((label, _, toggleable)) = rows.get(sel) {
                if *toggleable {
                    toggle_setting(app, label);
                }
            }
            app.modal = Some(Modal::AudioSettings {
                selected: sel,
                custom_input: None,
            });
        }
        KeyCode::Enter if rows.get(sel).map(|(l, _, _)| l.as_str()) == Some("stt_model") => {
            app.modal = Some(Modal::AudioSettings {
                selected: sel,
                custom_input: Some(String::new()),
            });
        }
        _ => {}
    }
}
