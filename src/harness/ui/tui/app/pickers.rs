//! Key handlers for skill/model/auth/resume pickers and settings helpers.

use crate::harness::ui::tui::theme::Theme;
use anyhow::Result;
use crossterm::event::KeyEvent;

use super::state::{AddProviderForm, App, ModelPickerState, ResumePickerState};
use super::undo::paste_clipboard;

pub(crate) fn handle_skill_picker_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use crossterm::event::KeyCode;
    let Some(picker) = app.skill_picker.as_mut() else {
        return Ok(false);
    };
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => picker.move_sel(-1),
        KeyCode::Down | KeyCode::Char('j') => picker.move_sel(1),
        KeyCode::Char(' ') => picker.toggle(),
        KeyCode::Char('a') => picker.toggle_all(),
        KeyCode::Enter => {
            app.apply_skill_picker()?;
            return Ok(false);
        }
        KeyCode::Esc => {
            app.apply_skill_picker()?;
            return Ok(false);
        }
        _ => {}
    }
    Ok(false)
}

/// `/settings` — show/update global limits (config.json), no modal needed.
pub(crate) fn handle_settings_command(app: &mut App, text: &str) {
    let rest = text.strip_prefix("/settings").unwrap_or("").trim();
    let mut parts = rest.split_whitespace();
    match parts.next() {
        None => {
            let c = &app.runtime.config;
            app.add_system(&format!(
                "settings · iterations {} · context {} · turn_timeout {}s · theme {} · provider {} · model {}",
                c.max_iterations, c.max_context_tokens, c.turn_timeout_secs, app.theme.name, c.provider, c.model
            ));
            app.add_system("usage: /settings iterations <n> · context <n> · turn_timeout <secs> · theme <name>");
        }
        Some("iterations") => {
            let Some(n) = parts.next().and_then(|v| v.parse::<usize>().ok()) else {
                app.add_system("usage: /settings iterations <n> (e.g. 50)");
                return;
            };
            match app.runtime.update_settings(Some(n), None, None) {
                Ok(()) => app.add_system(&format!("settings · max_iterations = {}", n)),
                Err(e) => app.add_system(&format!("[error] {}", e)),
            }
        }
        Some("context") => {
            let Some(n) = parts.next().and_then(|v| v.parse::<usize>().ok()) else {
                app.add_system("usage: /settings context <tokens> (e.g. 100000)");
                return;
            };
            match app.runtime.update_settings(None, Some(n), None) {
                Ok(()) => app.add_system(&format!("settings · max_context_tokens = {}", n)),
                Err(e) => app.add_system(&format!("[error] {}", e)),
            }
        }
        Some("turn_timeout") => {
            let Some(n) = parts.next().and_then(|v| v.parse::<u64>().ok()) else {
                app.add_system("usage: /settings turn_timeout <secs> (e.g. 1200)");
                return;
            };
            match app.runtime.update_settings(None, None, Some(n)) {
                Ok(()) => app.add_system(&format!("settings · turn_timeout_secs = {}", n)),
                Err(e) => app.add_system(&format!("[error] {}", e)),
            }
        }
        Some("theme") => match parts.next() {
            Some(name) if app.set_theme(name) => {
                app.add_system(&format!(
                    "theme → {} (saved to config.json)",
                    app.theme.name
                ));
            }
            Some(_) => app.add_system(&format!(
                "unknown theme (options: {})",
                Theme::names().join(", ")
            )),
            None => app.add_system(&format!("current theme: {}", app.theme.name)),
        },
        Some(other) => app.add_system(&format!(
            "unknown setting: {} (iterations · context · theme)",
            other
        )),
    }
}

/// Persists a model in the user store (creating a builtin override when
/// needed) so custom models survive restarts. Skips models already known.
pub(crate) fn persist_custom_model(provider: &str, model: &str) -> anyhow::Result<()> {
    use crate::harness::provider::user_store::UserProviders;
    if crate::harness::provider::catalog::models_for(provider).contains(&model.to_string()) {
        return Ok(()); // already listed — nothing to persist
    }
    let mut store = UserProviders::load();
    if store.add_model_anywhere(provider, model) {
        store.save()?;
    }
    Ok(())
}

/// Removes the selected provider (non-builtin) or user-added model from
/// `providers.json`. Returns `Err(reason)` when the item cannot be removed
/// (builtin entry) and `Ok(false)` when nothing has to change.
pub(crate) fn remove_from_user_store(
    picker: &ModelPickerState,
    item: &str,
) -> anyhow::Result<bool> {
    use crate::harness::provider::user_store::UserProviders;
    if !picker.stage_models {
        if item == "add provider…" {
            return Ok(false);
        }
        crate::harness::provider::catalog::find_provider(item)
            .ok_or_else(|| anyhow::anyhow!("provider `{}` not found", item))?;
        let mut store = UserProviders::load();
        let removed = if store.find(item).is_some_and(|p| p.removed) {
            false // already hidden
        } else if store.remove(item) || store.hide_builtin(item) {
            // user-defined provider deleted outright, or builtin hidden via tombstone
            true
        } else {
            false
        };
        store.save()?;
        return Ok(removed);
    }
    let mut store = UserProviders::load();
    if !store.remove_model(&picker.provider, item) {
        return Err(anyhow::anyhow!(
            "model `{}` is builtin for `{}` — only user-added models can be removed",
            item,
            picker.provider
        ));
    }
    store.save()?;
    Ok(true)
}

/// Handles a key while the `/models` picker is open.
pub(crate) fn handle_model_picker_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let Some(picker) = app.model_picker.as_mut() else {
        return Ok(false);
    };

    // Adding a provider: multi-field form (name, base_url, default_model).
    if let Some(form) = picker.add_provider.as_mut() {
        match key.code {
            KeyCode::Esc => {
                picker.add_provider = None;
            }
            KeyCode::Tab | KeyCode::Down | KeyCode::Char('j') => {
                form.field = (form.field + 1) % 3;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                form.field = (form.field + 2) % 3;
            }
            KeyCode::Backspace => {
                form.fields[form.field].pop();
            }
            KeyCode::Enter => {
                let name = form.fields[0].trim().to_string();
                let base_url = form.fields[1].trim().to_string();
                let default_model = form.fields[2].trim().to_string();
                if name.is_empty() || base_url.is_empty() {
                    app.add_system("[error] provider needs a name and base_url");
                } else {
                    use crate::harness::provider::user_store::{UserProvider, UserProviders};
                    let mut store = UserProviders::load();
                    let replaced = store.upsert(UserProvider {
                        name: name.clone(),
                        base_url: base_url.clone(),
                        default_model: default_model.clone(),
                        models: if default_model.is_empty() {
                            Vec::new()
                        } else {
                            vec![default_model.clone()]
                        },
                        removed: false,
                        prompt_cache: None,
                    });
                    match store.save() {
                        Ok(()) => {
                            app.add_system(&format!(
                                "provider `{}` {} (providers.json)",
                                name,
                                if replaced { "updated" } else { "added" }
                            ));
                            // Rebuild the picker at the provider stage so the
                            // new provider is selectable.
                            app.model_picker = Some(ModelPickerState::new());
                        }
                        Err(e) => app.add_system(&format!("[error] {}", e)),
                    }
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                form.fields[form.field].push(c);
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(pasted) = paste_clipboard() {
                    form.fields[form.field].push_str(&pasted);
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    // Typing a custom model name.
    if picker.custom_input.is_some() {
        match key.code {
            KeyCode::Esc => {
                picker.custom_input = None;
            }
            KeyCode::Backspace => {
                if let Some(inp) = picker.custom_input.as_mut() {
                    inp.pop();
                }
            }
            KeyCode::Enter => {
                if let Some(model) = picker.pick_model() {
                    let provider = picker.provider.clone();
                    app.model_picker = None;
                    app.apply_model_choice(&provider, &model)?;
                } else if picker.custom_input.is_none() {
                    // empty input → back to the model list
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(inp) = picker.custom_input.as_mut() {
                    inp.push(c);
                }
            }
            // Ctrl+V pastes from the system clipboard.
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(pasted) = paste_clipboard() {
                    if let Some(inp) = picker.custom_input.as_mut() {
                        inp.push_str(&pasted);
                    }
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => picker.move_sel(-1),
        KeyCode::Down | KeyCode::Char('j') => picker.move_sel(1),
        KeyCode::Esc => {
            if picker.stage_models {
                // Back to the provider stage.
                picker.stage_models = false;
                picker.selected = 0;
            } else {
                app.model_picker = None;
            }
        }
        KeyCode::Char('x') => {
            let Some(item) = picker.items().get(picker.selected).cloned() else {
                return Ok(false);
            };
            match remove_from_user_store(picker, &item) {
                Ok(true) => {
                    let len = picker.items().len();
                    picker.selected = picker.selected.min(len.saturating_sub(1));
                    app.add_system(&format!("removed `{item}` from the provider list"));
                }
                Ok(false) => {}
                Err(reason) => app.add_system(&format!("[error] {}", reason)),
            }
        }
        KeyCode::Enter => {
            if !picker.stage_models {
                if let Some(name) = picker.items().get(picker.selected).cloned() {
                    if name == "add provider…" {
                        picker.add_provider = Some(AddProviderForm::new());
                    } else {
                        picker.pick_provider(name);
                    }
                }
            } else if let Some(model) = picker.pick_model() {
                let provider = picker.provider.clone();
                app.model_picker = None;
                app.apply_model_choice(&provider, &model)?;
            }
        }
        _ => {}
    }
    Ok(false)
}

/// Handles a key while the session manager picker (/sessions) is open.
pub(crate) async fn handle_resume_picker_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let Some(picker) = app.resume_picker.as_mut() else {
        return Ok(false);
    };
    if picker.sessions.is_empty() {
        app.resume_picker = None;
        return Ok(false);
    }

    // Inline rename editing.
    if let Some(text) = picker.rename_input.as_mut() {
        match key.code {
            KeyCode::Esc => {
                picker.rename_input = None;
            }
            KeyCode::Enter => {
                let new_title = text.trim().to_string();
                picker.rename_input = None;
                if !new_title.is_empty() {
                    let id = picker.sessions[picker.selected].id.clone();
                    app.runtime.set_session_title(&id, &new_title)?;
                    // Keep the live session in sync so the sidebar updates
                    // immediately when renaming the currently open session.
                    if app.session.id == id {
                        app.session.title = Some(new_title.clone());
                    }
                    app.resume_picker = Some(ResumePickerState::new(app)?);
                    app.add_system("session renamed");
                }
            }
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                text.push(c);
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(pasted) = paste_clipboard() {
                    text.push_str(&pasted);
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => picker.move_sel(-1),
        KeyCode::Down | KeyCode::Char('j') => picker.move_sel(1),
        KeyCode::PageUp => picker.move_sel(-10),
        KeyCode::PageDown => picker.move_sel(10),
        KeyCode::Esc => {
            app.resume_picker = None;
        }
        KeyCode::Enter => {
            let id = picker.sessions[picker.selected].id.clone();
            app.resume_picker = None;
            if let Some(mut loaded) = app.runtime.load_session(&id)? {
                // Auto-compact oversized sessions when switching via /sessions.
                if app.runtime.config.is_configured() {
                    match app.runtime.maybe_compact(&mut loaded, false, None).await {
                        Ok(n) if n > 0 => {
                            app.add_system(&format!(
                                "session selected · auto-compacted {n} message(s)"
                            ));
                        }
                        Ok(_) => app.add_system("session selected"),
                        Err(e) => {
                            app.add_system(&format!("session selected · auto-compact failed: {e}"));
                        }
                    }
                } else {
                    app.add_system("session selected");
                }
                app.session = loaded;
                app.rebuild_transcript_from_session();
                app.reset_usage();
                app.sync_prompt_toggles();
            } else {
                app.add_system("session not found");
            }
        }
        // Delete the selected session.
        KeyCode::Char('d') | KeyCode::Delete => {
            let id = picker.sessions[picker.selected].id.clone();
            let keep = picker.selected;
            app.resume_picker = None;
            app.runtime.delete_session(&id)?;
            app.add_system("session deleted");
            match ResumePickerState::new(app) {
                Ok(mut next) if !next.sessions.is_empty() => {
                    next.selected = keep.min(next.sessions.len() - 1);
                    app.resume_picker = Some(next);
                }
                Ok(_) => {}
                Err(e) => app.add_system(&format!("[error] failed to refresh sessions: {e}")),
            }
        }
        // Rename the selected session.
        KeyCode::Char('r') => {
            picker.rename_input = Some(picker.selected_title());
        }
        _ => {}
    }
    Ok(false)
}

/// Handles a key while the `/auth` token prompt is open.
pub(crate) fn handle_auth_prompt_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let Some(prompt) = app.auth_prompt.as_mut() else {
        return Ok(false);
    };
    match key.code {
        KeyCode::Esc => {
            app.auth_prompt = None;
            app.add_system("auth cancelled");
        }
        KeyCode::Enter => {
            let provider = prompt.provider.clone();
            let token = prompt.input.trim().to_string();
            app.auth_prompt = None;
            if token.is_empty() {
                app.add_system("[error] empty token — auth cancelled");
            } else {
                let mut store = crate::harness::auth::AuthStore::load();
                store.set_key(&provider, token.clone());
                match store.save() {
                    Ok(()) => {
                        // Point the runtime at this provider and rebuild the
                        // provider so the freshly saved token is live. This
                        // also force-enables the prompt (`is_configured`).
                        let model = if app.runtime.config.provider == provider {
                            app.runtime.config.model.clone()
                        } else {
                            crate::harness::provider::catalog::default_model(&provider)
                                .unwrap_or_default()
                        };
                        if let Err(e) = app.runtime.switch_model(&provider, &model) {
                            app.add_system(&format!("[error] applying provider: {}", e));
                        }
                        app.runtime.config.api_key = token.clone();
                        app.add_system(&format!(
                            "token saved for provider `{}` (auth.json, 0600){}",
                            provider,
                            if app.runtime.config.is_configured() {
                                " — ready to go"
                            } else {
                                ""
                            }
                        ));
                    }
                    Err(e) => app.add_system(&format!("[error] failed to save token: {}", e)),
                }
            }
        }
        KeyCode::Backspace => prompt.backspace(),
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => prompt.push_char(c),
        // Ctrl+V pastes from the system clipboard.
        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(pasted) = paste_clipboard() {
                prompt.input.push_str(&pasted);
            }
        }
        _ => {}
    }
    Ok(false)
}
