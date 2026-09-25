//! TUI main loop, input submission and terminal lifecycle.

use crate::harness::event::HarnessEvent;
use crate::harness::provider::format_tokens;
use crate::harness::runtime::{PromptResult, SessionRuntime};
use crate::harness::session::Session;
use crate::harness::ui::tui::askers::{PermissionRequest, QuestionRequest};
use crate::harness::ui::tui::selection::{PendingClick, TextSelection, DRAG_THRESHOLD};
use crate::harness::ui::tui::transcript::{preview, LineKind};
use anyhow::Result;
use tokio::sync::mpsc;

use super::keys::handle_key;
use super::state::{App, AuthPromptState, Modal};

pub async fn run_tui(
    runtime: SessionRuntime,
    session: Session,
    cwd: std::path::PathBuf,
    permission_rx: mpsc::UnboundedReceiver<PermissionRequest>,
    question_rx: mpsc::UnboundedReceiver<QuestionRequest>,
) -> Result<()> {
    use crossterm::event::{self, Event};
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use std::io::stdout;

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    crossterm::execute!(stdout, crossterm::event::EnableMouseCapture)?;
    let _ = crossterm::execute!(stdout, crossterm::event::EnableBracketedPaste);
    // Kitty keyboard protocol: lets Shift+Enter reach the app as a distinct
    // key so multi-line prompts work (terminals without support just stay
    // on plain Enter). DISAMBIGUATE is what disambiguates Shift+Enter;
    // REPORT_EVENT_TYPES is intentionally omitted — we only handle presses.
    let _ = crossterm::execute!(
        stdout,
        crossterm::event::PushKeyboardEnhancementFlags(
            crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | crossterm::event::KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        )
    );
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let _guard = TerminalGuard;

    let prev_hook = std::sync::Arc::new(std::panic::take_hook());
    {
        let prev = prev_hook.clone();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal();
            prev(info);
        }));
    }

    let mut app = App::new(runtime, session, cwd, permission_rx, question_rx);

    // Warn (non-fatally) about missing system dependencies, e.g. a Chrome
    // binary for `fetch_webpage`'s browser render mode. The harness still runs;
    // the affected feature degrades to its fallback. `/doctor` shows details.
    {
        let (lines, all_ok) = crate::harness::deps::render_report();
        if !all_ok {
            for line in lines
                .iter()
                .filter(|l| l.contains('✗') || l.contains("install:"))
            {
                app.add_system(&format!("[warn] {line}"));
            }
            app.add_system("[warn] run /doctor for the full dependency report");
        }
    }

    // Auto-compact oversized sessions on open so the first turn doesn't start
    // already over the context budget (and so resume stays snappy).
    if app.runtime.config.is_configured() {
        match app
            .runtime
            .maybe_compact(&mut app.session, false, None)
            .await
        {
            Ok(n) if n > 0 => {
                app.add_system(&format!("auto-compacted {n} message(s) on open"));
            }
            Ok(_) => {}
            Err(e) => app.add_system(&format!("[warn] auto-compact on open failed: {e}")),
        }
    }

    // Unconfigured boot → onboarding wizard: with no model selected yet, open
    // the /models picker first (the auth prompt follows automatically after
    // the model choice, see apply_model_choice). When a model is selected but
    // its provider has no stored token, the app still behaves as a first run:
    // open the provider/model picker instead of an auth prompt for a provider
    // the user may not want to keep.
    if !app.runtime.config.is_configured() {
        let settings = crate::config::GlobalSettings::load();
        let auth = crate::harness::auth::AuthStore::load();
        let token_len = auth
            .get_key(app.runtime.config.provider.trim())
            .map(|k| k.trim().len())
            .unwrap_or(0);
        if settings.provider.is_empty() && settings.model.is_empty() || token_len < 10 {
            app.open_models_picker();
            app.add_system("RustClaw needs a provider/model and an API token — configure now");
        } else {
            let provider = app.runtime.config.provider.clone();
            app.add_system(
                "model already selected — RustClaw just needs an API token for this provider",
            );
            app.auth_prompt = Some(AuthPromptState::new(&provider));
        }
    }

    let mut prompt_task: Option<tokio::task::JoinHandle<Result<(PromptResult, Session)>>> = None;
    let mut last_draw = std::time::Instant::now()
        .checked_sub(MIN_FRAME_DT)
        .unwrap_or_else(std::time::Instant::now);

    loop {
        // Splash finished?
        if let Some(s) = &app.splash {
            if s.done() {
                app.splash = None;
                app.mark_dirty();
                let fresh = app.session.messages.is_empty();
                if fresh && app.skill_picker.is_none() {
                    // New session → choose skills for this session's memory.
                    app.open_skill_picker();
                } else {
                    app.sync_prompt_toggles();
                }
            }
        }

        // Drain events, coalescing consecutive TextDelta events to reduce
        // the number of apply_event calls (and thus frame work) under burst.
        let mut coalesced_delta = String::new();
        while let Ok(ev) = app.events_rx.try_recv() {
            app.runtime.event_recorder.record(&ev);
            match ev {
                HarnessEvent::TextDelta { delta, .. } => {
                    coalesced_delta.push_str(&delta);
                }
                other => {
                    // Flush any coalesced delta before the non-delta event.
                    if !coalesced_delta.is_empty() {
                        let combined = std::mem::take(&mut coalesced_delta);
                        app.apply_event(HarnessEvent::TextDelta {
                            delta: combined,
                            session_id: String::new(),
                            message_id: String::new(),
                            parent_session_id: None,
                        });
                    }
                    app.apply_event(other);
                }
            }
        }
        // Flush any remaining coalesced delta.
        if !coalesced_delta.is_empty() {
            app.apply_event(HarnessEvent::TextDelta {
                delta: std::mem::take(&mut coalesced_delta),
                session_id: String::new(),
                message_id: String::new(),
                parent_session_id: None,
            });
        }
        app.flush_pending_stream_if_due();
        while let Ok(req) = app.permission_rx.try_recv() {
            app.flush_stream_now();
            app.push(
                LineKind::System,
                format!(
                    "[permission] {} {}",
                    req.input.tool,
                    preview(&req.input.args_summary, 120)
                ),
            );
            app.enqueue_modal(Modal::Permission(req));
        }
        while let Ok(req) = app.question_rx.try_recv() {
            app.flush_stream_now();
            app.push(LineKind::System, format!("[question] {}", req.question));
            app.enqueue_modal(Modal::Question {
                req,
                draft: String::new(),
                cursor: 0,
            });
        }

        if let Some(handle) = prompt_task.take() {
            if handle.is_finished() {
                match handle.await {
                    Ok(Ok((r, updated))) => {
                        let was_aborted = r.aborted;
                        app.session = updated;
                        app.record_usage(r.usage, r.iterations);
                        app.running = false;
                        app.turn_started_at = None;
                        app.status_msg = None;
                        app.active_tools.clear();
                        app.flush_stream_now();
                        app.mark_dirty();
                        if was_aborted {
                            // Accidental Enter recovery: put the last submitted
                            // prompt back into the editor so the user can edit
                            // or resend without retyping.
                            if let Some(last) = app.history.last().cloned() {
                                if app.input.is_empty() {
                                    app.input = last;
                                    app.input_cursor = app.input.chars().count();
                                    app.autocomplete = None;
                                }
                            }
                            app.add_system("turn cancelled — prompt restored to input");
                        } else if r.usage.total() > 0 || r.iterations > 0 {
                            app.add_system(&format!(
                                "turn · in {} · out {} · Σ {} · ctx ~{}/{} · {} iter(s)",
                                format_tokens(r.usage.input_tokens),
                                format_tokens(r.usage.output_tokens),
                                format_tokens(r.usage.total()),
                                format_tokens(app.context_tokens() as u64),
                                format_tokens(app.max_context_tokens() as u64),
                                r.iterations
                            ));
                        }
                    }
                    Ok(Err(e)) => {
                        // Show the full cause chain so the real provider error
                        // (e.g. xAI rejecting an image) is visible, not just
                        // the outer "agent turn failed" context.
                        let chain: Vec<String> = e.chain().map(|c| c.to_string()).collect();
                        let msg = if chain.len() <= 1 {
                            format!("[error] {}", chain[0])
                        } else {
                            format!(
                                "[error] {} — root cause: {}",
                                chain[0],
                                chain[chain.len() - 1]
                            )
                        };
                        app.push(LineKind::Error, msg);
                        app.running = false;
                        app.turn_started_at = None;
                        app.status_msg = None;
                        app.active_tools.clear();
                        app.flush_stream_now();
                        app.mark_dirty();
                    }
                    Err(e) => {
                        app.push(LineKind::Error, format!("[error] task: {}", e));
                        app.running = false;
                        app.turn_started_at = None;
                        app.active_tools.clear();
                    }
                }
            } else {
                prompt_task = Some(handle);
            }
        }

        let now = std::time::Instant::now();
        let anim = app.needs_anim();
        if should_draw(app.needs_redraw, anim, last_draw, now) {
            app.tick = app.tick.wrapping_add(1);
            terminal.draw(|frame| crate::harness::ui::tui::draw::draw(frame, &mut app))?;
            last_draw = std::time::Instant::now();
            app.clear_dirty_after_draw();
        }

        let timeout = poll_timeout(app.needs_redraw, app.needs_anim(), last_draw);
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    // Kitty protocol reports releases/repeats as well; only
                    // handle press events (otherwise Shift+Enter fires twice).
                    if key.kind != crossterm::event::KeyEventKind::Press {
                        continue;
                    }
                    // Skip splash on any key.
                    if app.splash.is_some() {
                        app.splash = None;
                        app.mark_dirty();
                        let fresh = app.session.messages.is_empty();
                        if fresh && app.skill_picker.is_none() {
                            app.open_skill_picker();
                        } else {
                            app.sync_prompt_toggles();
                        }
                        continue;
                    }
                    let quit = handle_key(&mut app, key, &mut prompt_task).await?;
                    app.mark_dirty();
                    if quit {
                        // Graceful shutdown: give the in-flight turn a moment to
                        // persist its state before we drop the task handle.
                        if let Some(handle) = prompt_task.take() {
                            let _ =
                                tokio::time::timeout(std::time::Duration::from_millis(500), handle)
                                    .await;
                        }
                        break;
                    }
                }
                Event::Mouse(m) => {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    if app.splash.is_some() {
                        continue;
                    }
                    // Overlays own the pointer — don't start transcript selection.
                    let overlays_open = app.modal.is_some()
                        || app.palette.is_some()
                        || app.show_help
                        || app.skill_picker.is_some()
                        || app.model_picker.is_some()
                        || app.auth_prompt.is_some()
                        || app.resume_picker.is_some()
                        || app.theme_picker.is_some();
                    app.mark_dirty();
                    match m.kind {
                        MouseEventKind::ScrollUp => {
                            // Scroll the active overlay (picker/modal) when one
                            // is open; otherwise scroll the transcript.
                            if !app.mouse_scroll(-3) {
                                app.scroll_by(-3);
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            if !app.mouse_scroll(3) {
                                app.scroll_by(3);
                            }
                        }
                        MouseEventKind::Down(MouseButton::Left) => {
                            if overlays_open {
                                continue;
                            }
                            if let Some(pos) = app.hit_test_transcript(m.column, m.row) {
                                let line_idx = app.line_idx_at_cell(pos);
                                app.pending_click = Some(PendingClick {
                                    pos,
                                    screen_col: m.column,
                                    screen_row: m.row,
                                    line_idx,
                                });
                                // Seed a collapsed selection at the click point; it
                                // only becomes visible once the drag threshold is met.
                                app.selection = Some(TextSelection::new(pos));
                            } else {
                                // Click outside the transcript clears selection.
                                app.clear_selection();
                            }
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            if overlays_open {
                                continue;
                            }
                            let Some(pos) = app.hit_test_transcript(m.column, m.row) else {
                                continue;
                            };
                            if let Some(pending) = &app.pending_click {
                                if pending.pos.manhattan(pos) >= DRAG_THRESHOLD
                                    || (m.column.abs_diff(pending.screen_col) as usize
                                        + m.row.abs_diff(pending.screen_row) as usize)
                                        >= DRAG_THRESHOLD
                                {
                                    if let Some(sel) = app.selection.as_mut() {
                                        sel.dragging = true;
                                        sel.set_head(pos);
                                    } else {
                                        let mut sel = TextSelection::new(pending.pos);
                                        sel.dragging = true;
                                        sel.set_head(pos);
                                        app.selection = Some(sel);
                                    }
                                }
                            } else if let Some(sel) = app.selection.as_mut() {
                                if sel.dragging {
                                    sel.set_head(pos);
                                }
                            }
                            // Drag past the viewport edge auto-scrolls.
                            if app.selection.as_ref().map(|s| s.dragging).unwrap_or(false) {
                                let area = app.transcript_area;
                                const EDGE: u16 = 1;
                                if m.row <= area.y.saturating_add(EDGE) {
                                    app.scroll_by(-1);
                                } else if area.height > 0
                                    && m.row + EDGE >= area.y.saturating_add(area.height)
                                {
                                    app.scroll_by(1);
                                }
                            }
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            if overlays_open {
                                app.pending_click = None;
                                continue;
                            }
                            let dragged =
                                app.selection.as_ref().map(|s| s.dragging).unwrap_or(false);
                            if dragged {
                                // Auto-copy on release when the selection has content.
                                if app.has_text_selection() {
                                    app.copy_selection_to_clipboard();
                                }
                                if let Some(sel) = app.selection.as_mut() {
                                    sel.dragging = false;
                                }
                                app.pending_click = None;
                            } else {
                                // Click without drag: preserve UserPrompt modal behavior.
                                let pending = app.pending_click.take();
                                app.selection = None;
                                if let Some(p) = pending {
                                    if let Some(li) = p.line_idx {
                                        if let Some(line) = app.lines.get(li) {
                                            if line.kind == LineKind::User && app.modal.is_none() {
                                                app.autocomplete = None;
                                                app.modal =
                                                    Some(Modal::UserPrompt { line_idx: li });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        MouseEventKind::Down(_) => {
                            // Other buttons clear selection.
                            app.clear_selection();
                        }
                        _ => {}
                    }
                }
                Event::Paste(text) => {
                    if app.splash.is_some() || app.modal.is_some() || app.palette.is_some() {
                        continue;
                    }
                    app.paste_text(&text);
                    app.mark_dirty();
                }
                Event::Resize(..) => {
                    // Keep selection; next draw reclamps scroll and remaps via
                    // transcript_row_map. Collapse only if the anchor is gone.
                    app.mark_dirty();
                }
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    drop(terminal);
    let mut stdout = std::io::stdout();
    execute!(stdout, crossterm::event::DisableMouseCapture)?;
    execute!(stdout, LeaveAlternateScreen)?;
    let _ = crossterm::execute!(stdout, crossterm::event::PopKeyboardEnhancementFlags);
    // Restore the panic hook we replaced so later panics use the default
    // reporter. Abort (`panic = abort`) never runs Drop or this path.
    let _ = std::panic::take_hook();
    let prev = prev_hook.clone();
    std::panic::set_hook(Box::new(move |info| prev(info)));
    Ok(())
}

/// Minimum time between draws (~30 fps). Animation still polls at 50 ms.
pub(crate) const MIN_FRAME_DT: std::time::Duration = std::time::Duration::from_millis(33);

pub(crate) fn should_draw(
    needs_redraw: bool,
    needs_anim: bool,
    last_draw: std::time::Instant,
    now: std::time::Instant,
) -> bool {
    if needs_anim {
        return true;
    }
    needs_redraw && now.duration_since(last_draw) >= MIN_FRAME_DT
}

pub(crate) fn poll_timeout(
    needs_redraw: bool,
    needs_anim: bool,
    last_draw: std::time::Instant,
) -> std::time::Duration {
    let base = if needs_anim {
        std::time::Duration::from_millis(50)
    } else {
        std::time::Duration::from_millis(120)
    };
    if needs_redraw && !needs_anim {
        let remaining = MIN_FRAME_DT.saturating_sub(last_draw.elapsed());
        remaining.min(base)
    } else {
        base
    }
}

/// Restores cooked mode, mouse, alternate screen, kitty flags. Idempotent.
/// Called from `TerminalGuard::drop` and the TUI panic hook. Does not run on
/// abort (`panic = abort`).
pub(crate) fn restore_terminal() {
    use crossterm::execute;
    use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::event::PopKeyboardEnhancementFlags
    );
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn test_idle_loop_skips_draw_when_clean() {
        let last = Instant::now();
        let now = last + Duration::from_millis(200);
        assert!(!should_draw(false, false, last, now));
        assert!(should_draw(true, false, last, now));
        assert!(should_draw(false, true, last, now));
        let too_soon = last + Duration::from_millis(10);
        assert!(!should_draw(true, false, last, too_soon));
    }

    #[test]
    fn test_terminal_guard_drop_is_idempotent() {
        restore_terminal();
        restore_terminal();
    }

    #[test]
    fn test_paste_preserves_newlines() {
        let mut app = App::inline_for_tests("");
        app.paste_text("a\r\nb\rc");
        assert_eq!(app.input, "a\nb\nc");
    }
}
