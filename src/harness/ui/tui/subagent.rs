//! Live subagent panels shown in the transcript sidebar.
//!
//! When the `task` tool spawns a child session, its events are routed into a
//! compact panel (tool names + status marks) instead of the main transcript.
//! This module owns the panel state and the event routing / label helpers,
//! extracted from `app.rs`.

use crate::harness::event::{HarnessEvent, ToolStatus};
use crate::harness::ui::tui::app::App;

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
    /// Final summary preview (set when the `task` tool completes).
    pub summary: Option<String>,
}

impl App {
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
                let agent = input["agent"].as_str().unwrap_or("explore").to_string();
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
                        summary: None,
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
            output_preview,
            ..
        } = ev
        {
            if name == "task" {
                if let Some((_, nested)) = self
                    .subagent_panels
                    .iter_mut()
                    .rev()
                    .find(|(_, p)| !p.finished && p.child_session_id.is_empty())
                {
                    nested.finished = true;
                    nested.summary = Some(output_preview.clone());
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
