//! Harness event application and transcript rebuild.

use crate::harness::event::{HarnessEvent, ToolStatus};
use crate::harness::session::doom_loop::{DOOM_LOOP_STOP, DOOM_LOOP_WARN};
use crate::harness::ui::tui::subagent::SubagentPanel;
use crate::harness::ui::tui::transcript::{tool_arg_label, ActiveTool, LineKind, ToolBatch};

use super::state::{should_flush_stream, App, DoomLevel};
use super::undo::mark_for;

impl App {
    fn finish_tool_batch(&mut self) {
        if let Some(batch) = self.tool_status.take() {
            let (kind, mark) = if batch.failed == 0 {
                (LineKind::ToolOk, "✓")
            } else if batch.done == 0 {
                (LineKind::ToolError, "✗")
            } else {
                (LineKind::ToolError, "✓/✗")
            };
            self.push(kind, format!("  {} {}", mark, batch.summary()));
        }
        self.had_tool_batch = true;
    }

    /// Moves `pending_stream` into the visible `streaming` buffer.
    pub fn flush_pending_into_streaming(&mut self) {
        if self.pending_stream.is_empty() {
            return;
        }
        self.streaming
            .get_or_insert_with(String::new)
            .push_str(&self.pending_stream);
        self.pending_stream.clear();
        self.last_stream_flush = Some(std::time::Instant::now());
        self.mark_dirty();
    }

    /// Flush pending deltas when the timer or size cap says so.
    pub fn flush_pending_stream_if_due(&mut self) {
        if self.pending_stream.is_empty() {
            return;
        }
        let elapsed = self
            .last_stream_flush
            .map(|t| t.elapsed())
            .unwrap_or(std::time::Duration::MAX);
        let n = self.pending_stream.chars().count();
        if should_flush_stream(&self.pending_stream, elapsed, n) {
            self.flush_pending_into_streaming();
        }
    }

    /// Immediate flush of the temporal buffer (block boundaries).
    pub fn flush_stream_now(&mut self) {
        self.flush_pending_into_streaming();
        self.flush_streaming();
    }

    fn note_tool_signature(&mut self, name: &str, input: &serde_json::Value) {
        let sig = format!("{name}:{input}");
        if self.doom_sig.as_ref() == Some(&sig) {
            self.doom_streak = self.doom_streak.saturating_add(1);
        } else {
            self.doom_sig = Some(sig);
            self.doom_streak = 1;
        }
        self.doom_level = if self.doom_streak >= DOOM_LOOP_STOP {
            DoomLevel::Stop
        } else if self.doom_streak >= DOOM_LOOP_WARN {
            DoomLevel::Warn
        } else {
            DoomLevel::Ok
        };
        self.mark_dirty();
    }

    pub fn apply_event(&mut self, ev: HarnessEvent) {
        // Subagent (child) events are routed into their panel instead of the
        // main transcript, keeping the parent view clean.
        if ev.parent_session_id().is_some() {
            self.apply_subagent_event(&ev);
            self.mark_dirty();
            return;
        }
        match ev {
            HarnessEvent::TextDelta { delta, .. } => {
                if delta.is_empty() {
                    return;
                }
                if self.last_stream_flush.is_none() {
                    self.last_stream_flush = Some(std::time::Instant::now());
                }
                self.pending_stream.push_str(&delta);
                self.mark_dirty();
                let n = self.pending_stream.chars().count();
                let elapsed = self
                    .last_stream_flush
                    .map(|t| t.elapsed())
                    .unwrap_or_default();
                if should_flush_stream(&self.pending_stream, elapsed, n) {
                    self.flush_pending_into_streaming();
                }
            }
            HarnessEvent::ReasoningDelta { delta, .. } => {
                if delta.chars().count() >= 40 {
                    self.flush_stream_now();
                    self.push(LineKind::Reasoning, delta);
                }
            }
            HarnessEvent::MessageUpdated { .. } => {
                let had = self.streaming.is_some() || !self.pending_stream.is_empty();
                self.flush_stream_now();
                if had {
                    self.mark_dirty();
                }
            }
            HarnessEvent::ToolStart {
                name, input, depth, ..
            } => {
                self.flush_stream_now();
                let new_batch = self.tool_status.is_none();
                if new_batch && self.had_tool_batch {
                    self.current_iteration = self.current_iteration.saturating_add(1);
                }
                self.status_msg = Some(format!("running: {}", name));
                self.active_tools.push(ActiveTool { name: name.clone() });
                self.note_tool_signature(&name, &input);
                let batch = self.tool_status.get_or_insert_with(ToolBatch::default);
                batch.start(&name, tool_arg_label(&name, &input));
                // A `task` call opens a live subagent panel. The spawned
                // subagent runs one level deeper than the caller.
                if name == "task" {
                    let agent = input["agent"].as_str().unwrap_or("explore").to_string();
                    self.subagent_panels.push((
                        String::new(), // tool_id unknown here; matched on ToolEnd
                        SubagentPanel {
                            child_session_id: String::new(),
                            agent,
                            depth: depth + 1,
                            lines: Vec::new(),
                            done: 0,
                            failed: 0,
                            finished: false,
                            summary: None,
                        },
                    ));
                }
            }
            HarnessEvent::ToolEnd {
                name,
                status,
                diff,
                output_preview,
                ..
            } => {
                self.flush_stream_now();
                self.active_tools.retain(|t| t.name != name);
                if let Some(batch) = self.tool_status.as_mut() {
                    match status {
                        ToolStatus::Completed => batch.done += 1,
                        ToolStatus::Error => batch.failed += 1,
                        _ => {}
                    }
                    batch.pending = batch.pending.saturating_sub(1);
                    // Batch complete → emit a single summary line.
                    if batch.pending == 0 {
                        self.finish_tool_batch();
                    }
                }
                // A finished `task` call finalizes the newest open panel.
                if name == "task" {
                    if let Some((_, panel)) = self
                        .subagent_panels
                        .iter_mut()
                        .rev()
                        .find(|(_, p)| !p.finished)
                    {
                        panel.finished = true;
                        panel.summary = Some(if output_preview.is_empty() {
                            String::new()
                        } else {
                            output_preview.clone()
                        });
                    }
                }
                if let Some(d) = diff {
                    if !d.trim().is_empty() {
                        self.push(LineKind::Diff, d);
                    }
                }
                self.status_msg = None;
                self.mark_dirty();
            }
            HarnessEvent::CompactionStarted { .. } => {
                self.flush_stream_now();
                self.push(LineKind::System, "[compacting context…]".to_string());
            }
            HarnessEvent::CompactionFinished {
                summarized_messages,
                ..
            } => {
                self.push(
                    LineKind::System,
                    format!(
                        "[compaction: {} message(s) summarized]",
                        summarized_messages
                    ),
                );
            }
            HarnessEvent::AutoContinue {
                round,
                total,
                reason,
                ..
            } => {
                self.flush_stream_now();
                self.push(
                    LineKind::System,
                    format!("[auto-continue {}/{}] {}", round, total, reason),
                );
            }
            HarnessEvent::Error { message, .. } => {
                self.flush_stream_now();
                if message.contains("Stopped: the same tool")
                    || message.contains("repeating the same")
                {
                    self.doom_level = DoomLevel::Stop;
                }
                self.push(LineKind::Error, format!("[error] {}", message));
            }
            HarnessEvent::RunStarted { .. } => {
                self.running = true;
                self.current_iteration = 1;
                self.had_tool_batch = false;
                self.doom_level = DoomLevel::Ok;
                self.doom_sig = None;
                self.doom_streak = 0;
                if self.turn_started_at.is_none() {
                    self.turn_started_at = Some(std::time::Instant::now());
                }
                self.status_msg = Some("running…".to_string());
                self.mark_dirty();
            }
            HarnessEvent::RunFinished { .. } => {
                self.running = false;
                self.turn_started_at = None;
                self.status_msg = None;
                self.active_tools.clear();
                self.flush_stream_now();
                self.mark_dirty();
            }
            HarnessEvent::UserMessage { .. } => {}
            HarnessEvent::JobFinished {
                job_id, exit_code, ..
            } => {
                self.flush_stream_now();
                self.push(
                    LineKind::System,
                    format!(
                        "[background job {} finished · exit {}]",
                        job_id,
                        exit_code
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "signal".into())
                    ),
                );
            }
            HarnessEvent::PermissionAsk { .. } | HarnessEvent::PermissionResolved { .. } => {}
            HarnessEvent::BudgetWarn { message, .. } => {
                self.flush_stream_now();
                self.push(LineKind::System, format!("[budget] {}", message));
            }
            HarnessEvent::Rollback { paths, .. } => {
                self.flush_stream_now();
                self.push(
                    LineKind::System,
                    format!(
                        "[rollback] restored {} file(s): {}",
                        paths.len(),
                        paths.join(", ")
                    ),
                );
            }
        }
    }

    pub fn flush_streaming(&mut self) {
        if let Some(s) = self.streaming.take() {
            if !s.trim().is_empty() {
                self.push(LineKind::Assistant, s);
            }
        }
    }

    pub fn add_user_prompt(&mut self, text: &str) {
        self.push(LineKind::User, text.to_string());
    }

    pub fn add_system(&mut self, text: &str) {
        self.push(LineKind::System, text.to_string());
    }

    /// Esc while a turn is running: signal abort so streaming/tools stop.
    /// The prompt text is restored when the cancelled turn finishes (see
    /// the prompt_task completion handler) so an accidental Enter is recoverable.
    pub fn cancel_running_turn(&mut self) {
        if !self.running {
            return;
        }
        self.abort.abort();
        self.status_msg = Some("cancelling…".to_string());
        self.mark_dirty();
        // Avoid spamming the transcript if the user mashes Esc.
        let already = self
            .lines
            .last()
            .map(|l| l.text.contains("run cancelled by user"))
            .unwrap_or(false);
        if !already {
            self.push(LineKind::System, "[run cancelled by user]".to_string());
        }
    }

    pub fn scroll_by(&mut self, delta: i32) {
        self.stick_bottom = false;
        let new = (self.scroll as i32 + delta).max(0) as usize;
        self.scroll = new;
        self.mark_dirty();
    }

    /// Scrolls the active overlay (model/skill/resume/theme picker, palette,
    /// search) with the mouse wheel. Returns `true` when an overlay consumed
    /// the scroll (so the transcript is left untouched).
    pub fn mouse_scroll(&mut self, delta: i32) -> bool {
        if let Some(p) = self.model_picker.as_mut() {
            p.scroll_by(delta);
            self.mark_dirty();
            return true;
        }
        if let Some(p) = self.skill_picker.as_mut() {
            p.scroll_by(delta);
            self.mark_dirty();
            return true;
        }
        if let Some(p) = self.resume_picker.as_mut() {
            p.scroll_by(delta);
            self.mark_dirty();
            return true;
        }
        if let Some(p) = self.theme_picker.as_mut() {
            p.scroll_by(delta);
            self.mark_dirty();
            return true;
        }
        if let Some(p) = self.palette.as_mut() {
            p.move_sel(delta);
            self.mark_dirty();
            return true;
        }
        if let Some(s) = self.search.as_mut() {
            s.move_sel(delta);
            self.mark_dirty();
            return true;
        }
        false
    }

    pub fn clamp_scroll(&mut self, total: usize, view_height: usize) {
        let max = total.saturating_sub(view_height);
        if self.stick_bottom {
            self.scroll = max;
        } else {
            self.scroll = self.scroll.min(max);
            if self.scroll >= max {
                self.stick_bottom = true;
            }
        }
    }

    pub fn clear_transcript(&mut self) {
        self.lines.clear();
        self.streaming = None;
        self.tool_status = None;
        self.scroll = 0;
        self.stick_bottom = true;
        self.clear_selection();
        self.add_system("transcript cleared");
    }

    /// Rebuilds the transcript lines from the current session messages.
    /// Used after an undo/revert so the on-screen transcript matches the
    /// (truncated) persisted conversation instead of leaving stale lines.
    pub fn rebuild_transcript_from_session(&mut self) {
        let keep_scroll = !self.stick_bottom;
        let saved_scroll = self.scroll;
        self.lines.clear();
        self.streaming = None;
        self.pending_stream.clear();
        self.tool_status = None;
        self.active_tools.clear();
        self.clear_selection();
        if keep_scroll {
            self.scroll = saved_scroll;
            self.stick_bottom = false;
        } else {
            self.scroll = 0;
            self.stick_bottom = true;
        }
        self.mark_dirty();
        // Clone so we can push to self.lines while iterating.
        let messages = self.session.messages.clone();
        for msg in &messages {
            match msg.role.as_str() {
                "user" => {
                    for part in &msg.parts {
                        if let crate::harness::session::Part::Image { path } = part {
                            self.push(LineKind::System, format!("[image: {}]", path));
                        }
                    }
                    // Skip runtime-injected `<project-memory>` parts; keep the
                    // real user prompt (may live in a later text part).
                    let text = msg
                        .parts
                        .iter()
                        .find_map(|p| {
                            let raw = p.as_text()?;
                            let cleaned = crate::harness::project::memory::strip_memory_blocks(raw);
                            let cleaned = cleaned.trim();
                            if cleaned.is_empty() {
                                None
                            } else {
                                Some(cleaned.to_string())
                            }
                        })
                        .unwrap_or_default();
                    if !text.is_empty() {
                        self.push(LineKind::User, text);
                    }
                }
                "assistant" => {
                    for part in &msg.parts {
                        match part {
                            crate::harness::session::Part::Text { text } => {
                                if !text.trim().is_empty() {
                                    self.push(LineKind::Assistant, text.clone());
                                }
                            }
                            crate::harness::session::Part::Tool(tp) => {
                                let kind = match tp.status {
                                    crate::harness::session::ToolStatus::Error => {
                                        LineKind::ToolError
                                    }
                                    _ => LineKind::ToolOk,
                                };
                                let label = if tp.title.is_empty() {
                                    tp.name.clone()
                                } else {
                                    tp.title.clone()
                                };
                                self.push(kind, format!("  {} {}", mark_for(kind), label));
                            }
                            crate::harness::session::Part::Reasoning { text } => {
                                if !text.trim().is_empty() {
                                    self.push(LineKind::Reasoning, text.clone());
                                }
                            }
                            crate::harness::session::Part::Image { .. } => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
