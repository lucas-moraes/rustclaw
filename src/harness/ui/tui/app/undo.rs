//! Undo / revert helpers for the TUI transcript.
//!
//! // TODO(service-layer): UI não deveria falar com SessionStore

use crate::harness::ui::tui::transcript::LineKind;
use anyhow::Result;

use super::state::App;

pub(crate) fn user_prompt_text(app: &App, line_idx: usize) -> String {
    app.lines
        .get(line_idx)
        .map(|l| l.text.clone())
        .unwrap_or_default()
}

pub fn copy_to_clipboard(text: &str) -> bool {
    match arboard::Clipboard::new() {
        Ok(mut cb) => cb.set_text(text.to_string()).is_ok(),
        Err(_) => false,
    }
}

/// Returns the status mark for a tool line kind (✓ for ok, ✗ for error).
pub(crate) fn mark_for(kind: LineKind) -> &'static str {
    match kind {
        LineKind::ToolOk => "✓",
        LineKind::ToolError => "✗",
        _ => "·",
    }
}

/// Reverts the last user prompt and everything after it (TUI `/undo`).
/// Returns a user-facing message describing the outcome.
pub(crate) fn undo_last_turn(app: &mut App) -> String {
    let last_user = app
        .session
        .messages
        .iter()
        .rposition(|m| m.role.as_str() == "user");
    let Some(idx) = last_user else {
        return "nothing to undo".to_string();
    };
    let msg_id = app.session.messages[idx].id.clone();
    match app
        .runtime
        .store
        .delete_messages_from(&app.session.id, &app.session.cwd, &msg_id)
    {
        Ok(()) => {
            app.session.messages.truncate(idx);
            app.session.invalidate_messages_cache();
            match app.runtime.store.save_session(&app.session) {
                Ok(()) => {
                    app.tool_status = None;
                    app.rebuild_transcript_from_session();
                    "session reverted to before last prompt".to_string()
                }
                Err(e) => format!("[error] failed to save: {}", e),
            }
        }
        Err(e) => format!("[error] failed to revert: {}", e),
    }
}

/// Removes the clicked user prompt and everything after it (memory + SQLite).
pub(crate) fn revert_to_prompt(app: &mut App, line_idx: usize) -> Result<()> {
    // The clicked User bubble is the nth user prompt in the transcript.
    let nth = app
        .lines
        .iter()
        .take(line_idx + 1)
        .filter(|l| l.kind == LineKind::User)
        .count()
        .saturating_sub(1);
    let mut user_seen = 0usize;
    let mut cut_msg_index = None;
    for (i, m) in app.session.messages.iter().enumerate() {
        if m.role.as_str() == "user" {
            if user_seen == nth {
                cut_msg_index = Some(i);
                break;
            }
            user_seen += 1;
        }
    }
    let Some(msg_index) = cut_msg_index else {
        anyhow::bail!("user prompt not found");
    };
    let msg_id = app.session.messages[msg_index].id.clone();
    // DB: drop rows with ordinal >= this message.
    app.runtime
        .store
        .delete_messages_from(&app.session.id, &app.session.cwd, &msg_id)?;
    // Memory: drop messages from this prompt on.
    app.session.messages.truncate(msg_index);
    app.session.invalidate_messages_cache();
    app.runtime.store.save_session(&app.session)?;
    app.tool_status = None;
    // Rebuild the on-screen transcript so removed messages disappear.
    app.rebuild_transcript_from_session();
    Ok(())
}

/// Reads the system clipboard (best effort). Used for Ctrl+V in masked inputs.
pub(crate) fn paste_clipboard() -> Option<String> {
    arboard::Clipboard::new()
        .ok()
        .and_then(|mut cb| cb.get_text().ok())
}
