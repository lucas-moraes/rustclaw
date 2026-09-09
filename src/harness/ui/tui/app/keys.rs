//! Keyboard dispatch for the TUI (`handle_key`, modal keys, clipboard paste).

use crate::harness::runtime::PromptResult;
use crate::harness::session::Session;
use crate::harness::tool::context::AbortSignal;
use crate::harness::ui::commands::CommandOutcome;
use crate::harness::ui::tui::askers::QuestionRequest;
use crate::harness::ui::tui::palette::{AutoComplete, PaletteState};
use crate::harness::ui::tui::theme::Theme;
use crate::harness::ui::tui::transcript::{preview, LineKind};
use anyhow::Result;
use crossterm::event::KeyEvent;

use super::pickers::{
    handle_auth_prompt_key, handle_model_picker_key, handle_resume_picker_key,
    handle_settings_command, handle_skill_picker_key,
};
use super::state::{App, AuthPromptState, Modal, ResumePickerState};
use super::undo::{copy_to_clipboard, revert_to_prompt, undo_last_turn, user_prompt_text};

pub(crate) async fn handle_key(
    app: &mut App,
    key: KeyEvent,
    prompt_task: &mut Option<tokio::task::JoinHandle<Result<(PromptResult, Session)>>>,
) -> Result<bool> {
    use crossterm::event::{KeyCode, KeyModifiers};

    // Esc while a turn is streaming/running always cancels first — even if an
    // overlay (help/palette/selection) is open. Permission/question modals are
    // handled below so they can deny the ask and then abort.
    if matches!(key.code, KeyCode::Esc)
        && app.running
        && app.modal.is_none()
        && app.skill_picker.is_none()
        && app.model_picker.is_none()
        && app.auth_prompt.is_none()
        && app.resume_picker.is_none()
    {
        app.show_help = false;
        app.palette = None;
        app.autocomplete = None;
        if app.selection.is_some() || app.pending_click.is_some() {
            app.clear_selection();
        }
        app.cancel_running_turn();
        return Ok(false);
    }

    if app.skill_picker.is_some() {
        return handle_skill_picker_key(app, key);
    }

    if app.model_picker.is_some() {
        return handle_model_picker_key(app, key);
    }

    if app.auth_prompt.is_some() {
        return handle_auth_prompt_key(app, key);
    }

    if app.resume_picker.is_some() {
        return handle_resume_picker_key(app, key).await;
    }

    if app.modal.is_some() {
        return handle_modal_key(app, key);
    }

    if app.palette.is_some() {
        return handle_palette_key(app, key, prompt_task).await;
    }

    if app.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::F(1) => {
                app.show_help = false;
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                app.help_section =
                    (app.help_section + 1) % crate::harness::ui::tui::draw::help::section_count();
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                let n = crate::harness::ui::tui::draw::help::section_count();
                app.help_section = (app.help_section + n - 1) % n;
            }
            _ => {}
        }
        return Ok(false);
    }

    // Global shortcuts
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.has_text_selection() {
                app.copy_selection_to_clipboard();
                return Ok(false);
            }
            if app.running {
                app.abort.abort();
            }
            return Ok(true);
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.palette = Some(PaletteState::open(""));
            app.autocomplete = None;
            return Ok(false);
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cycle_theme();
            return Ok(false);
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.clear_transcript();
            return Ok(false);
        }
        // Opencode-style line editing (prompt editor).
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_line_start();
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_line_end();
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.kill_to_line_start();
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.kill_word_back();
        }
        KeyCode::F(1) => {
            app.show_help = true;
            return Ok(false);
        }
        KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.session.skills.is_empty() {
                return Ok(false);
            }
            app.cycle_focus();
            return Ok(false);
        }
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Save the last code block to a file.
            app.save_last_code_block();
            return Ok(false);
        }
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Copy the last code block to the clipboard.
            app.copy_last_code_block();
            return Ok(false);
        }
        _ => {}
    }

    // Autocomplete navigation
    if let Some(ref mut ac) = app.autocomplete {
        match key.code {
            KeyCode::Up => {
                ac.move_sel(-1);
                return Ok(false);
            }
            KeyCode::Down => {
                ac.move_sel(1);
                return Ok(false);
            }
            KeyCode::Tab => {
                if let Some(item) = ac.current().cloned() {
                    app.input = item.payload;
                    app.input_cursor = app.input.chars().count();
                    app.autocomplete = AutoComplete::from_input(&app.input);
                }
                return Ok(false);
            }
            _ => {}
        }
    }

    // Skill chips focused: navigate/toggle chips instead of input.
    if app.skills_focused {
        match key.code {
            KeyCode::Up | KeyCode::Down => app.cycle_focus(),
            KeyCode::Char(' ') => app.toggle_prompt_skill(app.skills_idx),
            KeyCode::Left => {
                let n = app.prompt_toggles.as_ref().map(|t| t.len()).unwrap_or(0);
                if n > 0 {
                    app.skills_idx = (app.skills_idx + n - 1) % n;
                }
            }
            KeyCode::Right => {
                let n = app.prompt_toggles.as_ref().map(|t| t.len()).unwrap_or(0);
                if n > 0 {
                    app.skills_idx = (app.skills_idx + 1) % n;
                }
            }
            KeyCode::Enter => app.cycle_focus(),
            _ => return Ok(false),
        }
        return Ok(false);
    }

    match key.code {
        // Enter + any modifier inserts a newline (multi-line prompt / list
        // building): Shift+Enter, Shift+Return, Alt+Enter and Ctrl+Enter all
        // work here. On macOS terminals without the kitty keyboard protocol
        // (default Terminal.app), Ctrl+J is the reliable fallback.
        KeyCode::Enter if !key.modifiers.is_empty() => {
            app.insert_char_fixed('\n');
        }
        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.insert_char_fixed('\n');
        }
        KeyCode::Enter => {
            if submit_input(app, prompt_task).await? {
                return Ok(true);
            }
        }
        KeyCode::Char(c) => {
            if c == '?' && app.input.is_empty() {
                app.show_help = !app.show_help;
            } else {
                app.insert_char_fixed(c);
            }
        }
        KeyCode::Backspace => app.backspace(),
        KeyCode::Delete => app.delete_forward(),
        KeyCode::Left => app.cursor_left(),
        KeyCode::Right => app.cursor_right(),
        // Home/End (and Ctrl+A/E) act inside the current logical line.
        KeyCode::Home => app.cursor_line_start(),
        KeyCode::End => app.cursor_line_end(),
        KeyCode::Tab => {
            // Tab cycles the active mode when not navigating `/` autocomplete
            // (which is handled above when autocomplete is Some).
            if app.autocomplete.is_none() && !app.running {
                app.cycle_mode();
            }
        }
        KeyCode::Up => {
            if app.autocomplete.is_none() {
                // Multi-line input → move inside the editor; single-line falls
                // back to prompt history (opencode behavior).
                if app.input.contains('\n') {
                    app.cursor_visual_up();
                } else if !app.running {
                    app.history_up();
                }
            }
        }
        KeyCode::Down => {
            if app.autocomplete.is_none() {
                if app.input.contains('\n') {
                    app.cursor_visual_down();
                } else if !app.running {
                    app.history_down();
                }
            }
        }
        KeyCode::PageUp => app.scroll_by(-8),
        KeyCode::PageDown => app.scroll_by(8),
        KeyCode::Esc => {
            // Priority: cancel whatever is in flight first.
            // 1) running turn / streaming → abort
            // 2) overlays / selection / autocomplete → close
            // 3) draft prompt text → clear
            if app.running {
                app.cancel_running_turn();
            } else if app.selection.is_some() || app.pending_click.is_some() {
                app.clear_selection();
            } else if app.autocomplete.is_some() {
                app.autocomplete = None;
            } else if !app.input.is_empty() {
                app.clear_prompt_input();
            }
        }
        _ => {}
    }
    Ok(false)
}

pub(crate) async fn handle_palette_key(
    app: &mut App,
    key: KeyEvent,
    prompt_task: &mut Option<tokio::task::JoinHandle<Result<(PromptResult, Session)>>>,
) -> Result<bool> {
    use crossterm::event::KeyCode;

    let action = {
        let pal = match app.palette.as_mut() {
            Some(p) => p,
            None => return Ok(false),
        };
        match key.code {
            KeyCode::Esc => {
                app.palette = None;
                return Ok(false);
            }
            KeyCode::Up => {
                pal.move_sel(-1);
                return Ok(false);
            }
            KeyCode::Down => {
                pal.move_sel(1);
                return Ok(false);
            }
            KeyCode::Backspace => {
                pal.backspace();
                return Ok(false);
            }
            KeyCode::Char(c) => {
                pal.push_char(c);
                return Ok(false);
            }
            KeyCode::Enter => pal.current().map(|i| i.payload.clone()),
            _ => return Ok(false),
        }
    };

    app.palette = None;
    if let Some(payload) = action {
        match payload.as_str() {
            "__help__" => app.show_help = true,
            "__clear__" => app.clear_transcript(),
            "__theme_cycle__" => app.cycle_theme(),
            "__skills__" => {
                app.open_skill_picker();
            }
            "__quit__" => return Ok(true),
            other if other.starts_with('/') => {
                app.input = other.to_string();
                app.input_cursor = app.input.chars().count();
                // If payload ends with space, leave for args; else submit.
                if !other.ends_with(' ') && submit_input(app, prompt_task).await? {
                    return Ok(true);
                }
            }
            _ => {}
        }
    }
    Ok(false)
}

pub(crate) fn handle_modal_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use crossterm::event::KeyCode;
    match app.modal.take() {
        Some(Modal::Permission(req)) => {
            let reply = match key.code {
                KeyCode::Char('y') | KeyCode::Enter => true,
                KeyCode::Char('a') => {
                    app.runtime.permission.set_always_allow(&req.input.tool);
                    true
                }
                KeyCode::Char('n') => false,
                KeyCode::Esc => {
                    // Esc cancels the whole in-flight turn, not just this ask.
                    let _ = req.reply.send(false);
                    app.push(LineKind::System, "[permission] denied".to_string());
                    if app.running {
                        app.cancel_running_turn();
                    }
                    return Ok(false);
                }
                _ => {
                    app.modal = Some(Modal::Permission(req));
                    return Ok(false);
                }
            };
            let _ = req.reply.send(reply);
            app.push(
                LineKind::System,
                format!("[permission] {}", if reply { "allowed" } else { "denied" }),
            );
        }
        Some(Modal::Question {
            req,
            mut draft,
            mut cursor,
        }) => {
            use crossterm::event::KeyModifiers;

            let put_back = |app: &mut App, req: QuestionRequest, draft: String, cursor: usize| {
                app.modal = Some(Modal::Question { req, draft, cursor });
            };
            let finish = |app: &mut App, req: QuestionRequest, answer: Option<String>| {
                let _ = req.reply.send(answer.clone());
                match answer {
                    Some(a) => app.push(
                        LineKind::System,
                        format!("[question] answered: {}", preview(&a, 80)),
                    ),
                    None => app.push(LineKind::System, "[question] no answer".to_string()),
                }
            };
            let insert_char = |draft: &mut String, cursor: &mut usize, c: char| {
                let mut chars: Vec<char> = draft.chars().collect();
                let at = (*cursor).min(chars.len());
                chars.insert(at, c);
                *draft = chars.into_iter().collect();
                *cursor = at + 1;
            };

            match key.code {
                KeyCode::Esc => {
                    finish(app, req, None);
                    if app.running {
                        app.cancel_running_turn();
                    }
                }
                KeyCode::Enter => {
                    let trimmed = draft.trim();
                    if trimmed.is_empty() {
                        // Keep the modal open until the user types something or Esc.
                        put_back(app, req, draft, cursor);
                        return Ok(false);
                    }
                    let answer = if let Ok(num) = trimmed.parse::<usize>() {
                        // Bare number selects a predefined option when present.
                        if num >= 1 && num <= req.options.len() {
                            req.options[num - 1].clone()
                        } else {
                            trimmed.to_string()
                        }
                    } else {
                        trimmed.to_string()
                    };
                    finish(app, req, Some(answer));
                }
                // Every printable character goes into the free-text draft.
                // Option shortcuts still work: type "1" + Enter, or just "1" + Enter.
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT)
                        && !key.modifiers.contains(KeyModifiers::SUPER) =>
                {
                    insert_char(&mut draft, &mut cursor, c);
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Backspace => {
                    if cursor > 0 {
                        let mut chars: Vec<char> = draft.chars().collect();
                        chars.remove(cursor - 1);
                        draft = chars.into_iter().collect();
                        cursor -= 1;
                    }
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Delete => {
                    let mut chars: Vec<char> = draft.chars().collect();
                    if cursor < chars.len() {
                        chars.remove(cursor);
                        draft = chars.into_iter().collect();
                    }
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Left => {
                    cursor = cursor.saturating_sub(1);
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Right => {
                    let len = draft.chars().count();
                    if cursor < len {
                        cursor += 1;
                    }
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Home => {
                    cursor = 0;
                    put_back(app, req, draft, cursor);
                }
                KeyCode::End => {
                    cursor = draft.chars().count();
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    draft.clear();
                    cursor = 0;
                    put_back(app, req, draft, cursor);
                }
                KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let chars: Vec<char> = draft.chars().collect();
                    let mut i = cursor.min(chars.len());
                    while i > 0 && chars[i - 1].is_whitespace() {
                        i -= 1;
                    }
                    while i > 0 && !chars[i - 1].is_whitespace() {
                        i -= 1;
                    }
                    let mut kept = chars;
                    kept.drain(i..cursor.min(kept.len()));
                    draft = kept.into_iter().collect();
                    cursor = i;
                    put_back(app, req, draft, cursor);
                }
                _ => {
                    put_back(app, req, draft, cursor);
                }
            }
        }
        Some(Modal::UserPrompt { line_idx }) => {
            let mut continue_modal = true;
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    app.add_system("prompt action dismissed");
                }
                KeyCode::Char('c') => {
                    let text = user_prompt_text(app, line_idx);
                    if copy_to_clipboard(&text) {
                        app.push(LineKind::System, "prompt copied to clipboard".to_string());
                    } else {
                        app.push(LineKind::Error, "[error] clipboard unavailable".to_string());
                    }
                    // One-shot action: the popup closes after executing.
                    continue_modal = false;
                }
                KeyCode::Char('r') => {
                    match revert_to_prompt(app, line_idx) {
                        Ok(()) => app.add_system("session reverted to before this prompt"),
                        Err(e) => app.add_system(&format!("[error] revert failed: {}", e)),
                    }
                    continue_modal = false;
                }
                _ => {
                    app.modal = Some(Modal::UserPrompt { line_idx });
                }
            }
            if !continue_modal {
                app.modal = None;
            }
        }
        None => {}
    }
    Ok(false)
}

pub(crate) async fn submit_input(
    app: &mut App,
    prompt_task: &mut Option<tokio::task::JoinHandle<Result<(PromptResult, Session)>>>,
) -> Result<bool> {
    let text = app.input.trim().to_string();
    if text.is_empty() {
        return Ok(false);
    }
    app.flush_streaming();
    app.input.clear();
    app.input_cursor = 0;
    app.autocomplete = None;
    app.history.push(text.clone());
    app.history_pos = None;

    if text.starts_with('/') {
        if text == "/theme" || text.starts_with("/theme ") {
            let arg = text.strip_prefix("/theme").unwrap_or("").trim();
            if arg.is_empty() {
                app.add_system(&format!(
                    "themes: {}  (current: {})",
                    Theme::names().join(", "),
                    app.theme.name
                ));
            } else if app.set_theme(arg) {
                app.add_system(&format!("theme → {}", app.theme.name));
            } else {
                app.add_system(&format!(
                    "unknown theme: {} (try {})",
                    arg,
                    Theme::names().join(", ")
                ));
            }
            return Ok(false);
        }

        if text == "/usage" || text == "/tokens" {
            for line in app.usage_report() {
                app.add_system(&line);
            }
            return Ok(false);
        }

        if text == "/skills" || text.starts_with("/skills ") {
            app.handle_skills_command(&text)?;
            return Ok(false);
        }

        // /models picker, /model and /provider direct switch, /auth token input.
        // /settings: show or update global limits/theme (persisted in config.json).
        if text == "/settings" || text.starts_with("/settings ") {
            handle_settings_command(app, &text);
            return Ok(false);
        }
        if text == "/models" || text.starts_with("/models ") {
            let arg = text.strip_prefix("/models").unwrap_or("").trim();
            if arg.is_empty() {
                app.open_models_picker();
            } else {
                // List the models of a provider in the transcript.
                let models = crate::harness::provider::catalog::models_for(arg);
                if models.is_empty() {
                    app.add_system(&format!("unknown provider: {} (try /models)", arg));
                } else {
                    app.add_system(&format!("models for {} ({}):", arg, models.len()));
                    for m in models {
                        app.add_system(&format!("  {}", m));
                    }
                }
            }
            return Ok(false);
        }
        if let Some(rest) = text.strip_prefix("/model ") {
            let model = rest.trim();
            if model.is_empty() {
                app.add_system(&format!(
                    "current model: {} (provider {})",
                    app.runtime.config.model, app.runtime.config.provider
                ));
            } else if app.running {
                app.add_system("[busy] cannot switch model while a turn is running");
            } else {
                let provider = app.runtime.config.provider.clone();
                app.apply_model_choice(&provider, model)?;
            }
            return Ok(false);
        }
        if text == "/model" {
            app.add_system(&format!(
                "current model: {} (provider {}) · usage: /model <name>",
                app.runtime.config.model, app.runtime.config.provider
            ));
            return Ok(false);
        }
        if let Some(rest) = text.strip_prefix("/provider ") {
            let provider = rest.trim();
            if provider.is_empty() {
                app.add_system(&format!(
                    "current provider: {}",
                    app.runtime.config.provider
                ));
            } else if app.running {
                app.add_system("[busy] cannot switch provider while a turn is running");
            } else if let Some(default_model) =
                crate::harness::provider::catalog::default_model(provider)
            {
                app.apply_model_choice(provider, &default_model)?;
            } else {
                app.add_system(&format!(
                    "unknown provider: {} (options: {})",
                    provider,
                    crate::harness::provider::catalog::provider_names().join(", ")
                ));
            }
            return Ok(false);
        }
        if text == "/provider" {
            app.add_system(&format!(
                "current provider: {} (model {}) · usage: /provider <name>",
                app.runtime.config.provider, app.runtime.config.model
            ));
            return Ok(false);
        }
        if text == "/auth" || text.starts_with("/auth ") {
            let arg = text.strip_prefix("/auth").unwrap_or("").trim();
            // No argument → update the token for the current provider/model.
            let provider = if arg.is_empty() {
                app.runtime.config.provider.clone()
            } else {
                arg.to_string()
            };
            if provider.is_empty() {
                let store = crate::harness::auth::AuthStore::load();
                app.add_system("usage: /auth <provider> — stored providers:");
                let names: Vec<&str> = store.entries.keys().map(|s| s.as_str()).collect();
                if names.is_empty() {
                    app.add_system("  (none)");
                } else {
                    app.add_system(&format!("  {}", names.join(", ")));
                }
            } else if app.running {
                app.add_system("[busy] cannot run /auth while a turn is running");
            } else {
                app.auth_prompt = Some(AuthPromptState::new(provider));
            }
            return Ok(false);
        }
        if text == "/sessions" {
            if app.running {
                app.add_system("[busy] cannot switch session while a turn is running");
            } else {
                let picker = ResumePickerState::new(app)?;
                if picker.sessions.is_empty() {
                    app.add_system("no sessions to manage");
                } else {
                    app.resume_picker = Some(picker);
                }
            }
            return Ok(false);
        }
        if text == "/undo" {
            if app.running {
                app.add_system("[busy] cannot undo while a turn is running");
            } else {
                let msg = undo_last_turn(app);
                app.add_system(&msg);
            }
            return Ok(false);
        }

        match crate::harness::ui::commands::handle(&mut app.runtime, &mut app.session, &text)
            .await?
        {
            CommandOutcome::Exit => return Ok(true),
            CommandOutcome::Continue(lines) => {
                for l in lines {
                    app.add_system(&l);
                }
            }
        }
        // Fresh session counters + skill picker when switching sessions.
        if text == "/new" {
            app.reset_usage();
            app.open_skill_picker();
        } else if text == "/sessions" || text.starts_with("/sessions select ") {
            app.reset_usage();
            app.sync_prompt_toggles();
        }
        return Ok(false);
    }

    // Prompt input is hidden until a provider/model/token is configured.
    if !app.runtime.config.is_configured() {
        app.add_system("configure provider/model + token first — use /models and /auth");
        return Ok(false);
    }

    if app.running {
        app.add_system("[busy] still running a turn");
        return Ok(false);
    }

    app.add_user_prompt(&text);
    app.running = true;
    app.abort = AbortSignal::new();
    app.last_iterations = 0;

    let abort = app.abort.clone();
    let runtime = app.runtime.clone_shareable();
    let mut task_session = app.session.clone();
    let tx = app.events_tx.clone();
    let enabled_skills = app.enabled_skill_ids();
    app.skills_focused = false;

    let handle = tokio::spawn(async move {
        let result = runtime
            .prompt(&mut task_session, &tx, &text, abort, Some(&enabled_skills))
            .await;
        result.map(|r| (r, task_session))
    });
    *prompt_task = Some(handle);
    Ok(false)
}
