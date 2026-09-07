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
    pub fn subagent_panel_label(panel: &SubagentPanel) -> String {
        let mark = if panel.finished { "✓" } else { "⏳" };
        let tools = panel.done + panel.failed;
        match &panel.summary {
            Some(s) if !s.is_empty() => format!(
                "{} {} — {} tools · {}",
                mark,
                panel.agent,
                tools,
                crate::harness::session::preview(s, 80)
            ),
            _ => format!("{} {} — {} tools", mark, panel.agent, tools),
        }
    }
}
