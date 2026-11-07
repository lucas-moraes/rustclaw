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
use super::pickers_state::AuthPromptState;
use super::state::{App, Modal};

pub async fn run_tui(
    runtime: SessionRuntime,
    session: Session,
    cwd: std::path::PathBuf,
    permission_rx: mpsc::UnboundedReceiver<PermissionRequest>,
    question_rx: mpsc::UnboundedReceiver<QuestionRequest>,
    swarm_ask_rx: mpsc::UnboundedReceiver<crate::harness::ui::tui::swarm_asks::SwarmAsk>,
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
    // Hide the terminal's own cursor for the whole session: the input editor
    // draws its own blinking glyph (`anim::CURSOR_ON`), and leaving the
    // hardware cursor visible put two cursors in the same cell with
    // independent blink phases.
    execute!(stdout, EnterAlternateScreen, crossterm::cursor::Hide)?;
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
    // Physical clear before any draw or async boot work so shell scrollback
    // does not bleed through the alternate screen.
    terminal.clear()?;
    let _guard = TerminalGuard;

    let prev_hook = std::sync::Arc::new(std::panic::take_hook());
    {
        let prev = prev_hook.clone();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal();
            prev(info);
        }));
    }

    let mut app = App::new(
        runtime,
        session,
        cwd,
        permission_rx,
        question_rx,
        swarm_ask_rx,
    );

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

    // Warn (non-fatally) when the tavily tool service has no token: web_search
    // still works via the HTML fallback, but with lower quality. `/auth` sets it.
    if let Some(hint) = crate::harness::tool::web_search::tavily_status_hint() {
        app.add_system(hint);
    }

    // Auto-compact oversized sessions on open so the first turn doesn't start
    // already over the context budget (and so resume stays snappy).
    if app.runtime.config.is_configured() {
        match app
            .runtime
            .maybe_compact(&mut app.session, false, None)
            .await
        {
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
        // Coalescing is keyed by (session_id, message_id, parent_session_id)
        // so subagent text keeps its routing and reaches the right panel.
        let mut raw = Vec::new();
        while let Ok(ev) = app.events_rx.try_recv() {
            app.runtime.event_recorder.record(&ev);
            raw.push(ev);
        }
        for ev in coalesce_text_deltas(raw.into_iter()) {
            app.apply_event(ev);
        }
        app.flush_pending_stream_if_due();
        // Drain voice transcription results (push-to-talk).
        #[cfg(feature = "voice")]
        {
            while let Ok(ev) = app.voice_rx.try_recv() {
                app.transcribing = false;
                match ev {
                    super::state::VoiceEvent::Transcribed(text) => {
                        app.paste_text(&text);
                    }
                    super::state::VoiceEvent::Failed(msg) => {
                        app.push_toast_kind(
                            msg,
                            crate::harness::ui::tui::draw::toast::ToastKind::Error,
                        );
                    }
                }
            }
            // Auto-stop at the max recording length.
            if app
                .recording
                .as_ref()
                .is_some_and(|r| r.recorder.exceeded_max())
            {
                super::keys::stop_and_transcribe_pub(&mut app);
            }
        }
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
        // A subagent asked something while we were idle: show it as a normal
        // question modal, but route the answer back over the message bus.
        while let Ok(ask) = app.swarm_ask_rx.try_recv() {
            app.flush_stream_now();
            app.push(
                LineKind::System,
                format!(
                    "[swarm] {} pergunta: {}",
                    ask.from,
                    preview(&ask.question, 120)
                ),
            );
            // During a turn the parent agent answers from its own loop, so
            // opening a modal would let the user answer twice. Just show it.
            if app.running {
                continue;
            }
            let (reply, _rx) = tokio::sync::oneshot::channel();
            app.enqueue_modal(Modal::Question {
                req: QuestionRequest {
                    question: format!("[{}] {}", ask.from, ask.question),
                    options: Vec::new(),
                    reply,
                },
                swarm_ask: Some(ask),
                draft: String::new(),
                cursor: 0,
            });
        }
        while let Ok(req) = app.question_rx.try_recv() {
            app.flush_stream_now();
            app.push(LineKind::System, format!("[question] {}", req.question));
            app.enqueue_modal(Modal::Question {
                req,
                swarm_ask: None,
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

        app.tick_toast();
        app.expire_finished_subagent_panels();
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

/// Coalesces consecutive `TextDelta` events, preserving the routing key
/// `(session_id, message_id, parent_session_id)` so subagent text still
/// reaches the correct panel. A run of deltas with the same key becomes one
/// event; a key change flushes the previous run first.
pub(crate) fn coalesce_text_deltas(
    events: impl Iterator<Item = HarnessEvent>,
) -> impl Iterator<Item = HarnessEvent> {
    #[derive(PartialEq, Eq, Clone)]
    struct Key {
        session_id: String,
        message_id: String,
        parent_session_id: Option<String>,
    }

    let mut pending: Option<(Key, String)> = None;
    let mut out: Vec<HarnessEvent> = Vec::new();
    for ev in events {
        match ev {
            HarnessEvent::TextDelta {
                delta,
                session_id,
                message_id,
                parent_session_id,
            } => {
                let key = Key {
                    session_id: session_id.clone(),
                    message_id: message_id.clone(),
                    parent_session_id: parent_session_id.clone(),
                };
                match &mut pending {
                    Some((k, buf)) if *k == key => buf.push_str(&delta),
                    Some(_) => {
                        let (k, buf) = pending.take().expect("checked");
                        out.push(HarnessEvent::TextDelta {
                            delta: buf,
                            session_id: k.session_id,
                            message_id: k.message_id,
                            parent_session_id: k.parent_session_id,
                        });
                        pending = Some((key, delta));
                    }
                    None => pending = Some((key, delta)),
                }
            }
            other => {
                if let Some((k, buf)) = pending.take() {
                    out.push(HarnessEvent::TextDelta {
                        delta: buf,
                        session_id: k.session_id,
                        message_id: k.message_id,
                        parent_session_id: k.parent_session_id,
                    });
                }
                out.push(other);
            }
        }
    }
    if let Some((k, buf)) = pending.take() {
        out.push(HarnessEvent::TextDelta {
            delta: buf,
            session_id: k.session_id,
            message_id: k.message_id,
            parent_session_id: k.parent_session_id,
        });
    }
    out.into_iter()
}

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
    // Bring the terminal cursor back: it was hidden on entry and the user is
    // returning to a normal shell prompt.
    let _ = execute!(std::io::stdout(), crossterm::cursor::Show);
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

    fn delta(session: &str, msg: &str, parent: Option<&str>, text: &str) -> HarnessEvent {
        HarnessEvent::TextDelta {
            delta: text.to_string(),
            session_id: session.to_string(),
            message_id: msg.to_string(),
            parent_session_id: parent.map(|p| p.to_string()),
        }
    }

    /// Consecutive deltas with the same routing key merge into one event
    /// carrying the full text.
    #[test]
    fn coalescing_merges_same_key_deltas() {
        let events = vec![
            delta("s", "m", None, "hel"),
            delta("s", "m", None, "lo "),
            delta("s", "m", None, "world"),
        ];
        let out: Vec<HarnessEvent> = coalesce_text_deltas(events.into_iter()).collect();
        assert_eq!(out.len(), 1);
        match &out[0] {
            HarnessEvent::TextDelta {
                delta,
                session_id,
                message_id,
                parent_session_id,
            } => {
                assert_eq!(delta, "hello world");
                assert_eq!(session_id, "s");
                assert_eq!(message_id, "m");
                assert!(parent_session_id.is_none());
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    /// Deltas from different (session, message, parent) keys are never
    /// merged across keys — subagent text keeps its routing.
    #[test]
    fn coalescing_preserves_routing_keys() {
        let events = vec![
            delta("root", "m1", None, "root "),
            delta("root", "m1", None, "more"),
            delta("child", "m2", Some("root"), "child "),
            delta("child", "m2", Some("root"), "text"),
        ];
        let out: Vec<HarnessEvent> = coalesce_text_deltas(events.into_iter()).collect();
        let texts: Vec<(String, Option<String>)> = out
            .iter()
            .filter_map(|ev| match ev {
                HarnessEvent::TextDelta {
                    delta,
                    parent_session_id,
                    ..
                } => Some((delta.clone(), parent_session_id.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            texts,
            vec![
                ("root more".to_string(), None),
                ("child text".to_string(), Some("root".to_string())),
            ]
        );
    }

    /// A non-delta event flushes the pending run before itself, preserving
    /// order.
    #[test]
    fn coalescing_flushes_on_non_delta_event() {
        let events = vec![
            delta("s", "m", None, "part"),
            HarnessEvent::RunFinished {
                session_id: "s".into(),
                parent_session_id: None,
            },
            delta("s", "m", None, "next"),
        ];
        let out: Vec<HarnessEvent> = coalesce_text_deltas(events.into_iter()).collect();
        assert_eq!(out.len(), 3);
        assert!(matches!(out[0], HarnessEvent::TextDelta { .. }));
        assert!(matches!(out[1], HarnessEvent::RunFinished { .. }));
        assert!(matches!(out[2], HarnessEvent::TextDelta { .. }));
    }
}
