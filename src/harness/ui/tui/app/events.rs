//! Harness event application and transcript rebuild.

use crate::harness::event::{HarnessEvent, ToolStatus};
use crate::harness::ui::tui::subagent::SubagentPanel;
use crate::harness::ui::tui::transcript::{tool_arg_label, ActiveTool, LineKind, ToolBatch};

use super::state::App;
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
    }

    pub fn apply_event(&mut self, ev: HarnessEvent) {
        // Subagent (child) events are routed into their panel instead of the
        // main transcript, keeping the parent view clean.
        if ev.parent_session_id().is_some() {
            self.apply_subagent_event(&ev);
            return;
        }
        match ev {
            HarnessEvent::TextDelta { delta, .. } => {
                self.streaming
                    .get_or_insert_with(String::new)
                    .push_str(&delta);
            }
            HarnessEvent::ReasoningDelta { delta, .. } => {
                if delta.chars().count() >= 40 {
                    self.flush_streaming();
                    self.push(LineKind::Reasoning, delta);
                }
            }
            HarnessEvent::MessageUpdated { .. } => {
                self.flush_streaming();
            }
            HarnessEvent::ToolStart { name, input, .. } => {
                self.flush_streaming();
                self.status_msg = Some(format!("running: {}", name));
                self.active_tools.push(ActiveTool { name: name.clone() });
                let batch = self.tool_status.get_or_insert_with(ToolBatch::default);
                batch.start(&name, tool_arg_label(&name, &input));
                // A `task` call opens a live subagent panel.
                if name == "task" {
                    let agent = input["agent"].as_str().unwrap_or("explore").to_string();
                    self.subagent_panels.push((
                        String::new(), // tool_id unknown here; matched on ToolEnd
                        SubagentPanel {
                            child_session_id: String::new(),
                            agent,
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
            }
            HarnessEvent::CompactionStarted { .. } => {
                self.flush_streaming();
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
                self.flush_streaming();
                self.push(
                    LineKind::System,
                    format!("[auto-continue {}/{}] {}", round, total, reason),
                );
            }
            HarnessEvent::Error { message, .. } => {
                self.flush_streaming();
                self.push(LineKind::Error, format!("[error] {}", message));
            }
            HarnessEvent::RunStarted { .. } => {
                self.running = true;
                if self.turn_started_at.is_none() {
                    self.turn_started_at = Some(std::time::Instant::now());
                }
                self.status_msg = Some("running…".to_string());
            }
            HarnessEvent::RunFinished { .. } => {
                self.running = false;
                self.turn_started_at = None;
                self.status_msg = None;
                self.active_tools.clear();
                self.flush_streaming();
            }
            HarnessEvent::UserMessage { .. } => {}
            HarnessEvent::JobFinished {
                job_id, exit_code, ..
            } => {
                self.flush_streaming();
                self.push(
                    LineKind::System,
                    format!(
                        "[background job {} finished · exit {}] — /jobs {} for output",
                        job_id,
                        exit_code
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "signal".into()),
                        job_id
                    ),
                );
            }
            HarnessEvent::PermissionAsk { .. } | HarnessEvent::PermissionResolved { .. } => {}
            HarnessEvent::BudgetWarn { message, .. } => {
                self.flush_streaming();
                self.push(LineKind::System, format!("[budget] {}", message));
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
        self.lines.clear();
        self.streaming = None;
        self.tool_status = None;
        self.active_tools.clear();
        self.scroll = 0;
        self.stick_bottom = true;
        self.clear_selection();
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
