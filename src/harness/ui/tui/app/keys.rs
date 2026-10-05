//! Keyboard dispatch for the TUI (`handle_key`, modal keys, clipboard paste).

use crate::harness::runtime::PromptResult;
use crate::harness::session::Session;
use crate::harness::tool::context::AbortSignal;
use crate::harness::ui::commands::CommandOutcome;
use crate::harness::ui::tui::askers::QuestionRequest;
use crate::harness::ui::tui::draw::toast::ToastKind;
use crate::harness::ui::tui::palette::{AutoComplete, PaletteState};
use crate::harness::ui::tui::theme::Theme;
use crate::harness::ui::tui::transcript::{preview, LineKind};
use anyhow::Result;
use crossterm::event::KeyEvent;
#[cfg(feature = "voice")]
use crossterm::event::{KeyCode, KeyModifiers};

use super::pickers::{
    handle_auth_picker_key, handle_auth_prompt_key, handle_cursor_command, handle_model_picker_key,
    handle_resume_picker_key, handle_settings_command, handle_skill_picker_key,
    handle_theme_picker_key,
};
use super::pickers_state::{AuthPickerState, AuthPromptState, ResumePickerState};
#[allow(unused_imports)]
use super::settings_keys::{
    audio_custom_input_key, audio_nav_key, cursor_model_target, handle_audio_settings_key,
    handle_settings_like_key, list_cursor_models, parse_model_list, toggle_setting,
};
use super::state::{App, CursorModelTarget, Modal};
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
        && app.auth_picker.is_none()
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

    // Push-to-talk recording mode: only Ctrl+R (stop+transcribe) and Esc
    // (cancel) act; everything else is ignored so the user can't type while
    // recording.
    #[cfg(feature = "voice")]
    if app.recording.is_some() {
        return Ok(handle_recording_key(app, key));
    }

    if app.skill_picker.is_some() {
        return handle_skill_picker_key(app, key);
    }

    if app.model_picker.is_some() {
        return handle_model_picker_key(app, key);
    }

    if app.auth_picker.is_some() {
        return handle_auth_picker_key(app, key);
    }

    if app.auth_prompt.is_some() {
        return handle_auth_prompt_key(app, key);
    }

    if app.resume_picker.is_some() {
        return handle_resume_picker_key(app, key).await;
    }

    if app.theme_picker.is_some() {
        return handle_theme_picker_key(app, key);
    }

    if app.search.is_some() {
        return handle_search_key(app, key);
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
            // Respond to any pending permission/question modal so the awaiting
            // tool doesn't hang or get a stale reply after we quit.
            if let Some(modal) = app.modal.take() {
                match modal {
                    Modal::Permission(req) => {
                        let _ = req.reply.send(false);
                        app.push(LineKind::System, "[permission] denied".to_string());
                    }
                    Modal::Question { req, .. } => {
                        let _ = req.reply.send(None);
                        app.push(LineKind::System, "[question] no answer".to_string());
                    }
                    Modal::UserPrompt { .. } => {}
                    Modal::Settings { .. } => {}
                    Modal::Cursor { .. } => {}
                    Modal::CursorModel { .. } => {}
                    Modal::AudioSettings { .. } => {}
                }
            }
            // Drain any queued asks too, so no oneshot is left dangling.
            while let Some(modal) = app.modal_queue.pop_front() {
                match modal {
                    Modal::Permission(req) => {
                        let _ = req.reply.send(false);
                    }
                    Modal::Question { req, .. } => {
                        let _ = req.reply.send(None);
                    }
                    Modal::UserPrompt { .. } => {}
                    Modal::Settings { .. } => {}
                    Modal::Cursor { .. } => {}
                    Modal::CursorModel { .. } => {}
                    Modal::AudioSettings { .. } => {}
                }
            }
            if app.running {
                app.abort.abort();
            }
            return Ok(true);
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.palette = Some(PaletteState::open_with("", &app.custom_agent_items()));
            app.autocomplete = None;
            return Ok(false);
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.open_theme_picker();
            return Ok(false);
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.clear_transcript();
            return Ok(false);
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.open_search();
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
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.kill_to_line_end();
            return Ok(false);
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::ALT) => {
            app.kill_word_forward();
            return Ok(false);
        }
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_word_left();
            return Ok(false);
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_word_right();
            return Ok(false);
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
            // Yank from the kill-ring when the editor has a kill and focus;
            // otherwise copy the last fenced code block (legacy Ctrl+Y).
            let has_kill = app.kill_ring.as_ref().is_some_and(|s| !s.is_empty());
            if has_kill {
                app.yank_kill_ring();
            } else {
                app.copy_last_code_block();
            }
            return Ok(false);
        }
        // Paste screenshot / image from clipboard (Ctrl+V or Cmd+V on macOS).
        // When no image is available, fall back to pasting clipboard text.
        KeyCode::Char('v')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::SUPER) =>
        {
            match app.try_paste_clipboard_image() {
                Ok(true) => return Ok(false),
                Ok(false) => return Ok(false),
                Err(_) => {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        if let Ok(text) = cb.get_text() {
                            app.paste_text(&text);
                            return Ok(false);
                        }
                    }
                    app.push_toast_kind(
                        "clipboard has no image or text to paste",
                        ToastKind::Error,
                    );
                    return Ok(false);
                }
            }
        }
        // Toggle collapsible thinking (reasoning) blocks — only when the
        // prompt is empty so typing is never swallowed.
        KeyCode::Char('x') if app.input.is_empty() => {
            app.thinking_expanded = !app.thinking_expanded;
            app.status_msg = Some(if app.thinking_expanded {
                "thinking blocks expanded (x to collapse)".to_string()
            } else {
                "thinking blocks collapsed (x to expand)".to_string()
            });
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
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            #[cfg(feature = "voice")]
            start_recording(app);
        }
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
        KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
            app.kill_word_back();
        }
        KeyCode::Backspace => app.backspace(),
        KeyCode::Delete => app.delete_forward(),
        KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_word_left();
        }
        KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_word_right();
        }
        KeyCode::Left => app.cursor_left(),
        KeyCode::Right => app.cursor_right(),
        // Home/End (and Ctrl+A/E) act inside the current logical line.
        KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.scroll_to_top();
        }
        KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.scroll_to_end();
        }
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
            // Active history navigation wins over `/` autocomplete, so a
            // recalled prompt starting with `/` can still be browsed.
            if app.history_pos.is_some() {
                app.history_up();
            } else if let Some(ac) = app.autocomplete.as_mut() {
                ac.move_sel(-1);
            } else if app.input.contains('\n') {
                // Multi-line input → move inside the editor; single-line falls
                // back to prompt history (opencode behavior).
                app.cursor_visual_up();
            } else if !app.running {
                app.history_up();
            }
        }
        KeyCode::Down => {
            if app.history_pos.is_some() {
                app.history_down();
            } else if let Some(ac) = app.autocomplete.as_mut() {
                ac.move_sel(1);
            } else if app.input.contains('\n') {
                app.cursor_visual_down();
            } else if !app.running {
                app.history_down();
            }
        }
        KeyCode::PageUp => {
            let d = app.page_scroll_delta();
            app.scroll_by(-d);
        }
        KeyCode::PageDown => {
            let d = app.page_scroll_delta();
            app.scroll_by(d);
        }
        KeyCode::Esc => {
            // Priority: cancel whatever is in flight first.
            // 1) running turn / streaming → abort
            // 2) overlays / selection / autocomplete → close
            // 3) pending image attachments → clear
            // 4) draft prompt text → clear
            if app.running {
                app.cancel_running_turn();
            } else if app.selection.is_some() || app.pending_click.is_some() {
                app.clear_selection();
            } else if app.autocomplete.is_some() {
                app.autocomplete = None;
            } else if !app.pending_images.is_empty() {
                app.clear_pending_images();
                app.push_toast("cleared pending images");
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
            "__theme_picker__" => app.open_theme_picker(),
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
                    app.close_modal();
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
            app.close_modal();
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
                app.close_modal();
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
                app.close_modal();
            }
        }
        Some(Modal::Settings { selected }) => {
            let rows = crate::harness::ui::tui::draw::modal::settings_rows(&app.runtime.config);
            handle_settings_like_key(app, key, rows, selected, Modal::Settings { selected: 0 });
        }
        Some(Modal::Cursor { selected }) => {
            let rows = crate::harness::ui::tui::draw::modal::cursor_rows(&app.runtime.config);
            handle_settings_like_key(app, key, rows, selected, Modal::Cursor { selected: 0 });
        }
        Some(Modal::AudioSettings {
            selected,
            custom_input,
        }) => {
            handle_audio_settings_key(app, key, selected, custom_input);
        }
        Some(Modal::CursorModel {
            selected,
            scroll_offset,
            filter,
            models,
            target,
        }) => {
            use crate::harness::ui::tui::app::state::cursor_model_filtered_indices;
            use crossterm::event::KeyModifiers;

            const PAGE_STEP: usize = 10;
            let indices = cursor_model_filtered_indices(&models, &filter);
            let n = indices.len();
            let mut sel = selected.min(n.saturating_sub(1));
            let mut scroll = scroll_offset;
            let mut filt = filter;
            let mut close = false;
            match key.code {
                KeyCode::Esc => {
                    if filt.is_empty() {
                        close = true;
                    } else {
                        filt.clear();
                        sel = 0;
                        scroll = 0;
                    }
                }
                KeyCode::Char('q') if filt.is_empty() => close = true,
                KeyCode::Up | KeyCode::Char('k') => {
                    sel = sel.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if sel + 1 < n {
                        sel += 1;
                    }
                }
                KeyCode::PageUp => {
                    sel = sel.saturating_sub(PAGE_STEP);
                }
                KeyCode::PageDown => {
                    sel = (sel + PAGE_STEP).min(n.saturating_sub(1));
                }
                KeyCode::Home => {
                    sel = 0;
                }
                KeyCode::End => {
                    sel = n.saturating_sub(1);
                }
                KeyCode::Backspace => {
                    filt.pop();
                    sel = 0;
                    scroll = 0;
                }
                KeyCode::Enter => {
                    if let Some(&orig) = indices.get(sel) {
                        if let Some((model, _)) = models.get(orig).cloned() {
                            // "auto" is stored as an empty string (no --model flag).
                            let stored = if model == "auto" {
                                String::new()
                            } else {
                                model
                            };
                            let saved = match target {
                                CursorModelTarget::Build => {
                                    app.runtime.set_cursor_model(stored.clone())
                                }
                                CursorModelTarget::Plan => {
                                    app.runtime.set_cursor_plan_model(stored.clone())
                                }
                            };
                            match saved {
                                Ok(()) => app.add_system(&format!(
                                    "{} = {} (Cursor CLI --model)",
                                    target.label(),
                                    if stored.is_empty() { "auto" } else { &stored }
                                )),
                                Err(e) => app
                                    .add_system(&format!("[error] failed to save settings: {}", e)),
                            }
                        }
                    }
                    close = true;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    filt.push(c);
                    sel = 0;
                    scroll = 0;
                }
                _ => {}
            }
            if close {
                app.close_modal();
            } else {
                app.modal = Some(Modal::CursorModel {
                    selected: sel,
                    scroll_offset: scroll,
                    filter: filt,
                    models,
                    target,
                });
            }
        }
        None => {}
    }
    Ok(false)
}

pub(crate) fn handle_search_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use crossterm::event::{KeyCode, KeyModifiers};

    let Some(mut st) = app.search.take() else {
        return Ok(false);
    };
    match key.code {
        KeyCode::Esc => {
            app.search = None;
        }
        KeyCode::Enter => {
            if let Some(idx) = st.current() {
                app.jump_to_line(idx);
            }
            app.search = None;
        }
        KeyCode::Up => {
            st.move_sel(-1);
            app.search = Some(st);
        }
        KeyCode::Down => {
            st.move_sel(1);
            app.search = Some(st);
        }
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            st.move_sel(1);
            if let Some(idx) = st.current() {
                app.jump_to_line(idx);
            }
            app.search = Some(st);
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            st.move_sel(-1);
            if let Some(idx) = st.current() {
                app.jump_to_line(idx);
            }
            app.search = Some(st);
        }
        KeyCode::Backspace => {
            st.backspace();
            st.refresh(&app.lines);
            app.search = Some(st);
        }
        KeyCode::Char(c)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            st.push_char(c);
            st.refresh(&app.lines);
            app.search = Some(st);
        }
        _ => {
            app.search = Some(st);
        }
    }
    Ok(false)
}

pub(crate) async fn submit_input(
    app: &mut App,
    prompt_task: &mut Option<tokio::task::JoinHandle<Result<(PromptResult, Session)>>>,
) -> Result<bool> {
    let text = app.input.trim().to_string();
    let has_images = !app.pending_images.is_empty();
    if text.is_empty() && !has_images {
        return Ok(false);
    }
    app.flush_streaming();
    app.input.clear();
    app.input_cursor = 0;
    app.autocomplete = None;
    if !text.is_empty() {
        app.history.push(text.clone());
    }
    app.history_pos = None;

    // /image [path] — queue attachment (native picker when path omitted).
    if text == "/image" || text.starts_with("/image ") {
        let arg = text.strip_prefix("/image").unwrap_or("").trim();
        match app.attach_image_from_arg_or_picker(arg) {
            Ok(()) => {}
            Err(e) => app.add_system(&format!("[error] {e}")),
        }
        return Ok(false);
    }

    if text.starts_with('/') {
        if text == "/copy-code" {
            app.copy_last_code_block();
            return Ok(false);
        }
        if text == "/save-code" {
            app.save_last_code_block();
            return Ok(false);
        }

        if text == "/theme" || text.starts_with("/theme ") {
            let arg = text.strip_prefix("/theme").unwrap_or("").trim();
            if arg.is_empty() {
                app.open_theme_picker();
            } else if arg == "list" {
                app.add_system(&format!("themes: {}", Theme::names().join(", ")));
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
        // /cursor: Cursor CLI toggle + model (modal, or on/off/model args).
        if text == "/cursor" || text.starts_with("/cursor ") {
            handle_cursor_command(app, &text);
            return Ok(false);
        }
        // /audio-settings: push-to-talk toggle + STT model (modal).
        if text == "/audio-settings" || text.starts_with("/audio-settings ") {
            app.modal = Some(Modal::AudioSettings {
                selected: 0,
                custom_input: None,
            });
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
            app.open_model_picker_for_current();
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
            if arg.is_empty() {
                // No argument → open a picker to choose which token to set.
                if app.running {
                    app.add_system("[busy] cannot run /auth while a turn is running");
                } else {
                    app.auth_picker = Some(AuthPickerState::new());
                }
            } else if app.running {
                app.add_system("[busy] cannot run /auth while a turn is running");
            } else {
                app.auth_prompt = Some(AuthPromptState::new(arg));
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

    // Build multimodal parts: pending images + user text (default caption when
    // the user only attached screenshots and hit Enter).
    let images = std::mem::take(&mut app.pending_images);
    let prompt_text = if text.is_empty() {
        if images.len() == 1 {
            "describe this image".to_string()
        } else {
            "describe these images".to_string()
        }
    } else {
        text.clone()
    };

    let mut display = prompt_text.clone();
    if !images.is_empty() {
        let names: Vec<String> = images
            .iter()
            .map(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("image")
                    .to_string()
            })
            .collect();
        display = format!("[{}] {}", names.join(", "), prompt_text);
    }
    app.add_user_prompt(&display);
    app.running = true;
    app.turn_started_at = Some(std::time::Instant::now());
    app.abort = AbortSignal::new();
    app.last_iterations = 0;

    let abort = app.abort.clone();
    let runtime = app.runtime.clone_shareable();
    let mut task_session = app.session.clone();
    let tx = app.events_tx.clone();
    let enabled_skills = app.enabled_skill_ids();
    app.skills_focused = false;

    let mut parts: Vec<crate::harness::session::Part> = images
        .into_iter()
        .map(|p| crate::harness::session::Part::image(p.to_string_lossy().into_owned()))
        .collect();
    parts.push(crate::harness::session::Part::text(prompt_text));

    let handle = tokio::spawn(async move {
        let result = runtime
            .prompt_with_parts(&mut task_session, &tx, parts, abort, Some(&enabled_skills))
            .await;
        result.map(|r| (r, task_session))
    });
    *prompt_task = Some(handle);
    Ok(false)
}

/// Starts push-to-talk recording (`Ctrl+R` when idle).
#[cfg(feature = "voice")]
fn start_recording(app: &mut App) {
    if app.recording.is_some() || app.transcribing {
        return;
    }
    if !app.runtime.config.voice_enabled {
        app.push_toast_kind(
            "voice is off — enable it in /audio-settings".to_string(),
            ToastKind::Error,
        );
        return;
    }
    match crate::harness::voice::Recorder::start() {
        Ok(recorder) => {
            app.recording = Some(super::state::RecordingState {
                recorder,
                started_at: std::time::Instant::now(),
            });
            app.push_toast("🎙 recording… (Ctrl+R to stop, Esc to cancel)");
        }
        Err(e) => {
            app.push_toast_kind(format!("voice: {e}"), ToastKind::Error);
        }
    }
}

/// Key handling while a recording is active: only Ctrl+R (stop + transcribe)
/// and Esc (cancel) act; everything else is swallowed.
#[cfg(feature = "voice")]
fn handle_recording_key(app: &mut App, key: KeyEvent) -> bool {
    match (key.code, key.modifiers.contains(KeyModifiers::CONTROL)) {
        (KeyCode::Char('r'), true) => stop_and_transcribe(app),
        (KeyCode::Esc, _) => {
            app.recording = None;
            app.push_toast("recording cancelled");
            false
        }
        _ => false, // swallow all other keys while recording (false = don't quit)
    }
}

/// Stops the recorder and spawns the async transcription task.
#[cfg(feature = "voice")]
pub(crate) fn stop_and_transcribe_pub(app: &mut App) -> bool {
    stop_and_transcribe(app)
}

/// Stops the recorder and spawns the async transcription task.
#[cfg(feature = "voice")]
fn stop_and_transcribe(app: &mut App) -> bool {
    let Some(state) = app.recording.take() else {
        return false;
    };
    let (samples, sample_rate) = state.recorder.stop();
    app.transcribing = true;
    app.push_toast("transcribing…");
    let tx = app.voice_tx.clone();
    let model = {
        let m = app.runtime.config.stt_model.clone();
        if m.is_empty() {
            crate::harness::voice::DEFAULT_STT_MODEL.to_string()
        } else {
            m
        }
    };
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            let samples = crate::harness::voice::resample(&samples, sample_rate, 16_000);
            crate::harness::voice::encode_wav(&samples, 16_000)
        })
        .await
        .map_err(|e| anyhow::anyhow!("voice: encode task failed: {e}"))
        .and_then(|wav| {
            let store = crate::harness::auth::AuthStore::load();
            store
                .get_key("deepinfra")
                .map(|k| (wav, k))
                .ok_or_else(|| anyhow::anyhow!("voice: no deepinfra token — run /auth to set it"))
        });
        match result {
            Ok((wav, key)) => {
                let http = reqwest::Client::new();
                match crate::harness::voice::transcribe(&http, &key, &wav, &model).await {
                    Ok(text) => {
                        let _ = tx.send(super::state::VoiceEvent::Transcribed(text));
                    }
                    Err(e) => {
                        let _ = tx.send(super::state::VoiceEvent::Failed(format!("{e:#}")));
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(super::state::VoiceEvent::Failed(format!("{e:#}")));
            }
        }
    });
    false
}

#[cfg(test)]
mod cursor_model_tests {
    use super::parse_model_list;

    /// The parser must skip the `Available models` header and the blank line,
    /// keep the ids (first token) with their descriptions, and dedup while
    /// preserving order.
    #[test]
    fn test_parse_model_list_real_output() {
        let out = "Available models\n\n\
auto - Auto (current, default)\n\
gpt-5.3-codex-low - Codex 5.3 Low\n\
gpt-5.3-codex - Codex 5.3\n\
claude-opus-5-5-high - Claude Opus 5.5 1M High\n";
        let models = parse_model_list(out);
        assert_eq!(
            models,
            vec![
                ("auto".to_string(), "Auto (current, default)".to_string()),
                ("gpt-5.3-codex-low".to_string(), "Codex 5.3 Low".to_string()),
                ("gpt-5.3-codex".to_string(), "Codex 5.3".to_string()),
                (
                    "claude-opus-5-5-high".to_string(),
                    "Claude Opus 5.5 1M High".to_string()
                ),
            ]
        );
    }

    /// Bare ids (no description) are kept with an empty description;
    /// duplicates are dropped.
    #[test]
    fn test_parse_model_list_bare_and_dedup() {
        let out = "Models:\nsonnet\nsonnet\nopus\n";
        assert_eq!(
            parse_model_list(out),
            vec![
                ("sonnet".to_string(), String::new()),
                ("opus".to_string(), String::new()),
            ]
        );
    }

    /// Empty / header-only output yields no models.
    #[test]
    fn test_parse_model_list_empty() {
        assert!(parse_model_list("").is_empty());
        assert!(parse_model_list("Available models\n").is_empty());
    }
}

#[cfg(test)]
mod audio_settings_tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn open_audio(app: &mut App, selected: usize) {
        app.modal = Some(Modal::AudioSettings {
            selected,
            custom_input: None,
        });
    }

    fn selected_of(app: &App) -> usize {
        match &app.modal {
            Some(Modal::AudioSettings { selected, .. }) => *selected,
            other => panic!("expected AudioSettings modal, got {:?}", other.is_some()),
        }
    }

    /// Down moves the highlight forward; Up moves it back (clamped at 0).
    #[test]
    fn audio_nav_moves_selection() {
        let mut app = App::inline_for_tests("test");
        app.splash = None;
        open_audio(&mut app, 0);

        handle_audio_settings_key(&mut app, key(KeyCode::Down), 0, None);
        assert_eq!(selected_of(&app), 1);

        handle_audio_settings_key(&mut app, key(KeyCode::Up), 1, None);
        assert_eq!(selected_of(&app), 0);

        // Up at the top stays at 0.
        handle_audio_settings_key(&mut app, key(KeyCode::Up), 0, None);
        assert_eq!(selected_of(&app), 0);
    }

    /// Down is clamped at the last row (2 rows: voice_enabled, stt_model).
    #[test]
    fn audio_nav_clamps_at_last_row() {
        let mut app = App::inline_for_tests("test");
        app.splash = None;
        open_audio(&mut app, 1);
        handle_audio_settings_key(&mut app, key(KeyCode::Down), 1, None);
        assert_eq!(selected_of(&app), 1);
    }

    /// Esc closes the modal.
    #[test]
    fn audio_esc_closes_modal() {
        let mut app = App::inline_for_tests("test");
        app.splash = None;
        open_audio(&mut app, 0);
        handle_audio_settings_key(&mut app, key(KeyCode::Esc), 0, None);
        assert!(app.modal.is_none());
    }

    /// Enter on the `stt_model` row (index 1) opens the inline text prompt.
    #[test]
    fn audio_enter_on_stt_model_opens_input() {
        let mut app = App::inline_for_tests("test");
        app.splash = None;
        open_audio(&mut app, 1);
        handle_audio_settings_key(&mut app, key(KeyCode::Enter), 1, None);
        match &app.modal {
            Some(Modal::AudioSettings { custom_input, .. }) => {
                assert_eq!(custom_input.as_deref(), Some(""));
            }
            other => panic!("expected inline input, got {:?}", other.is_some()),
        }
    }

    /// Enter on the `voice_enabled` row (index 0) does not open the prompt.
    #[test]
    fn audio_enter_on_toggle_row_does_not_open_input() {
        let mut app = App::inline_for_tests("test");
        app.splash = None;
        open_audio(&mut app, 0);
        handle_audio_settings_key(&mut app, key(KeyCode::Enter), 0, None);
        match &app.modal {
            Some(Modal::AudioSettings { custom_input, .. }) => {
                assert!(custom_input.is_none());
            }
            other => panic!("expected AudioSettings modal, got {:?}", other.is_some()),
        }
    }

    /// Typing appends to the inline buffer; Backspace removes the last char.
    #[test]
    fn audio_input_edits_buffer() {
        let mut app = App::inline_for_tests("test");
        app.splash = None;
        open_audio(&mut app, 1);

        handle_audio_settings_key(&mut app, key(KeyCode::Char('a')), 1, Some("".into()));
        handle_audio_settings_key(&mut app, key(KeyCode::Char('b')), 1, Some("a".into()));
        match &app.modal {
            Some(Modal::AudioSettings { custom_input, .. }) => {
                assert_eq!(custom_input.as_deref(), Some("ab"));
            }
            other => panic!("expected inline input, got {:?}", other.is_some()),
        }

        handle_audio_settings_key(&mut app, key(KeyCode::Backspace), 1, Some("ab".into()));
        match &app.modal {
            Some(Modal::AudioSettings { custom_input, .. }) => {
                assert_eq!(custom_input.as_deref(), Some("a"));
            }
            other => panic!("expected inline input, got {:?}", other.is_some()),
        }
    }

    /// Esc while typing closes the prompt (and the modal).
    #[test]
    fn audio_input_esc_closes_prompt() {
        let mut app = App::inline_for_tests("test");
        app.splash = None;
        open_audio(&mut app, 1);
        handle_audio_settings_key(&mut app, key(KeyCode::Esc), 1, Some("abc".into()));
        match &app.modal {
            Some(Modal::AudioSettings { custom_input, .. }) => {
                assert!(custom_input.is_none(), "prompt must close on Esc");
            }
            other => panic!("expected AudioSettings modal, got {:?}", other.is_some()),
        }
    }
}
