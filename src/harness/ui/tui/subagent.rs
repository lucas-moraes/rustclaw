//! Live subagent panels shown in the transcript sidebar.
//!
//! When the `task` tool spawns a child session, its events are routed into a
//! compact panel (tool names + status marks) instead of the main transcript.
//! This module owns the panel state and the event routing / label helpers,
//! extracted from `app.rs`.

use crate::harness::event::{HarnessEvent, ToolStatus};
use crate::harness::ui::tui::app::App;

/// How long a finished panel lingers (in frame ticks) before it is dropped
/// from the band. The runner draws at ~30 fps (`MIN_FRAME_DT` = 33 ms), so
/// 90 ticks ≈ 3 s — enough to read the final `✓`/`✗` and the tool count.
pub const FINISHED_LINGER_TICKS: u64 = 90;

/// A live subagent panel (one per `task` tool call).
pub struct SubagentPanel {
    /// Child session id (events are routed by `parent_session_id`).
    pub child_session_id: String,
    /// Agent name shown in the header (parsed from the tool input).
    pub agent: String,
    /// Nesting depth of this subagent (root agent = 0, its subagents = 1, ...).
    /// Used to render the subagent tree in the panel.
    pub depth: usize,
    /// Compact activity lines (tool names + status marks).
    pub lines: Vec<String>,
    pub done: usize,
    pub failed: usize,
    pub finished: bool,
    /// Frame tick at which the panel finished; drives the linger timeout so
    /// the final `✓`/`✗` stays readable before the panel disappears.
    pub finished_at: Option<u64>,
    /// Final summary preview (set when the `task` tool completes).
    pub summary: Option<String>,
    /// Messages exchanged with this subagent (`message_send`/`message_ask`/
    /// `message_reply`), newest last. Shown in the panel so a conversation is
    /// visible instead of only the tool call names.
    pub messages: Vec<String>,
}

/// Cap on retained conversation lines per panel; the band only draws two.
const MAX_PANEL_MESSAGES: usize = 6;

impl SubagentPanel {
    /// Appends a conversation line, keeping only the most recent ones.
    pub fn push_message(&mut self, line: String) {
        self.messages.push(line);
        if self.messages.len() > MAX_PANEL_MESSAGES {
            self.messages.remove(0);
        }
    }
}

impl App {
    /// Drops finished panels whose `✓`/`✗` has been visible long enough.
    ///
    /// Called once per frame from the runner loop: without it the band never
    /// empties and permanently eats `BAND_HEIGHT` rows of the transcript.
    pub(crate) fn expire_finished_subagent_panels(&mut self) {
        let now = self.tick;
        self.subagent_panels.retain(|(_, p)| match p.finished_at {
            Some(at) => now.saturating_sub(at) < FINISHED_LINGER_TICKS,
            // Still running (or finished before the timestamp existed).
            None => true,
        });
    }

    /// Routes a child-session event into its subagent panel (never the transcript).
    pub(crate) fn apply_subagent_event(&mut self, ev: &HarnessEvent) {
        // A `task` call issued *by* a subagent opens a nested panel. The child
        // session id isn't known yet (it arrives with the grandchild's first
        // event), so the panel starts empty and is matched later.
        if let HarnessEvent::ToolStart {
            name, input, depth, ..
        } = ev
        {
            if name == "task" {
                let agent = input["agent"].as_str().unwrap_or("reason").to_string();
                self.subagent_panels.push((
                    String::new(),
                    SubagentPanel {
                        child_session_id: String::new(),
                        agent,
                        depth: depth + 1,
                        lines: Vec::new(),
                        done: 0,
                        failed: 0,
                        finished: false,
                        finished_at: None,
                        summary: None,
                        messages: Vec::new(),
                    },
                ));
                return;
            }
        }
        // A `task` ToolEnd from a subagent finalizes its nested panel (the
        // grandchild panel opened by the matching ToolStart above). This must
        // run *before* the child-session matching below, which would otherwise
        // claim the still-empty nested panel for the subagent's own session.
        if let HarnessEvent::ToolEnd {
            name,
            tool_id,
            output_preview,
            ..
        } = ev
        {
            if name == "task" {
                // Match by tool_call_id when known (robust to nested and
                // out-of-order task calls); fall back to the newest open
                // empty-child panel for legacy events without an id.
                let target = if tool_id.is_empty() {
                    self.subagent_panels
                        .iter()
                        .rposition(|(_, p)| !p.finished && p.child_session_id.is_empty())
                        .map(|i| self.subagent_panels[i].0.clone())
                } else {
                    self.subagent_panels
                        .iter()
                        .find(|(id, _)| *id == tool_id.as_str())
                        .map(|(id, _)| id.clone())
                        .or_else(|| {
                            // Panels are keyed by the child session id, which
                            // is unknown at ToolStart time (key stays empty
                            // until the grandchild's first event). Fall back
                            // to the newest open empty-child panel.
                            self.subagent_panels
                                .iter()
                                .rposition(|(_, p)| !p.finished && p.child_session_id.is_empty())
                                .map(|i| self.subagent_panels[i].0.clone())
                        })
                };
                if let Some(key) = target {
                    if let Some((_, nested)) =
                        self.subagent_panels.iter_mut().find(|(id, _)| *id == key)
                    {
                        nested.finished = true;
                        nested.finished_at = Some(self.tick);
                        nested.summary = Some(output_preview.clone());
                    }
                }
            }
        }
        let child = ev.session_id().unwrap_or("").to_string();
        // Find (or create) the open panel for this child session.
        let pos = self
            .subagent_panels
            .iter()
            .position(|(_, p)| p.child_session_id == child);
        let idx = match pos {
            Some(i) => i,
            None => {
                // Attach to the newest unfinished panel (task ToolStart arrives
                // before the child's first event).
                match self
                    .subagent_panels
                    .iter()
                    .rposition(|(_, p)| !p.finished && p.child_session_id.is_empty())
                {
                    Some(i) => {
                        self.subagent_panels[i].1.child_session_id = child.clone();
                        i
                    }
                    None => return,
                }
            }
        };
        let panel = &mut self.subagent_panels[idx].1;
        match ev {
            // Swarm traffic is the interesting part of a subagent's life: a
            // question it asked (blocked) or a note it sent. Record it so the
            // panel shows a conversation, not just tool names.
            HarnessEvent::ToolStart { name, input, .. }
                if matches!(
                    name.as_str(),
                    "message_send" | "message_ask" | "message_reply"
                ) =>
            {
                let body = input["body"]
                    .as_str()
                    .or_else(|| input["question"].as_str())
                    .or_else(|| input["answer"].as_str())
                    .unwrap_or("");
                let arrow = match name.as_str() {
                    "message_send" => "→",
                    "message_ask" => "?",
                    _ => "↩",
                };
                panel.push_message(format!(
                    "{} {}",
                    arrow,
                    crate::harness::session::preview(body, 60)
                ));
            }
            HarnessEvent::ToolStart { name, .. } => {
                panel.lines.push(format!("· {}", name));
            }
            HarnessEvent::ToolEnd {
                name,
                status,
                title,
                ..
            } => {
                let mark = match status {
                    ToolStatus::Completed => {
                        panel.done += 1;
                        "✓"
                    }
                    ToolStatus::Error => {
                        panel.failed += 1;
                        "✗"
                    }
                    _ => "·",
                };
                let label = if title.is_empty() { name } else { title };
                panel.lines.push(format!("{} {}", mark, label));
            }
            HarnessEvent::Error { message, .. } => {
                panel.lines.push(format!("✗ {}", message));
            }
            // Text/reasoning deltas are not streamed into the panel; the final
            // summary arrives via the parent's `task` ToolEnd.
            _ => {}
        }
    }

    /// Compact one-line status of a subagent panel: `⏳ explore — 3 tools`.
    ///
    /// Nested subagents (depth ≥ 1) are prefixed with a tree marker and a
    /// `L{n}` level tag so the user can see the composition at a glance.
    pub fn subagent_panel_label(panel: &SubagentPanel) -> String {
        let mark = if panel.finished { "✓" } else { "⏳" };
        let tools = panel.done + panel.failed;
        let level = if panel.depth > 0 {
            format!("L{}", panel.depth)
        } else {
            String::new()
        };
        match &panel.summary {
            Some(s) if !s.is_empty() => format!(
                "{}{}{} {} — {} tools · {}",
                Self::subagent_tree_prefix(panel.depth),
                mark,
                level,
                panel.agent,
                tools,
                crate::harness::session::preview(s, 80)
            ),
            _ => format!(
                "{}{}{} {} — {} tools",
                Self::subagent_tree_prefix(panel.depth),
                mark,
                level,
                panel.agent,
                tools
            ),
        }
    }

    /// Tree-drawing prefix for a subagent at `depth` (root = 0 → no prefix).
    /// Depth 1 → `└─ `, depth 2 → `│  └─ `, depth 3 → `│  │  └─ `.
    pub fn subagent_tree_prefix(depth: usize) -> String {
        if depth == 0 {
            return String::new();
        }
        let mut s = String::new();
        for _ in 1..depth {
            s.push_str("│  ");
        }
        s.push_str("└─ ");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::ui::tui::app::App;

    fn panel(finished_at: Option<u64>) -> SubagentPanel {
        SubagentPanel {
            child_session_id: "c".into(),
            agent: "explore".into(),
            depth: 0,
            lines: Vec::new(),
            done: 1,
            failed: 0,
            finished: finished_at.is_some(),
            finished_at,
            summary: None,
            messages: Vec::new(),
        }
    }

    #[test]
    fn running_panel_is_never_expired() {
        let mut app = App::inline_for_tests("");
        app.tick = 10_000;
        app.subagent_panels = vec![("a".into(), panel(None))];
        app.expire_finished_subagent_panels();
        assert_eq!(app.subagent_panels.len(), 1);
    }

    #[test]
    fn finished_panel_lingers_then_is_dropped() {
        let mut app = App::inline_for_tests("");
        app.tick = 100;
        app.subagent_panels = vec![("a".into(), panel(Some(100)))];

        app.tick = 100 + FINISHED_LINGER_TICKS - 1;
        app.expire_finished_subagent_panels();
        assert_eq!(app.subagent_panels.len(), 1, "must still be visible");

        app.tick = 100 + FINISHED_LINGER_TICKS;
        app.expire_finished_subagent_panels();
        assert!(app.subagent_panels.is_empty(), "must be gone after linger");
    }

    #[test]
    fn expiry_keeps_running_panels_alongside_finished_ones() {
        let mut app = App::inline_for_tests("");
        app.tick = 1_000;
        app.subagent_panels = vec![
            ("done".into(), panel(Some(1_000 - FINISHED_LINGER_TICKS))),
            ("live".into(), panel(None)),
        ];
        app.expire_finished_subagent_panels();
        assert_eq!(app.subagent_panels.len(), 1);
        assert_eq!(app.subagent_panels[0].0, "live");
    }

    #[test]
    fn band_forces_frames_so_the_linger_can_elapse() {
        let mut app = App::inline_for_tests("");
        app.splash = None;
        app.tick = 0;
        app.subagent_panels = vec![("a".into(), panel(Some(0)))];
        assert!(app.needs_anim());
    }
}
