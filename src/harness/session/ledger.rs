//! Structured context ledger: a compact, durable record of what the agent has
//! done in a session (files touched, commands run, decisions, subagents).
//!
//! Unlike the free-text compaction summary, the ledger lives on the [`Session`]
//! itself, so it **survives compaction** and is re-injected as a structured
//! block after each summary. This preserves the "hard facts" (paths, commands,
//! decisions) that a lossy LLM summary tends to drop.
//!
//! [`Session`]: crate::harness::session::Session

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Max distinct commands retained (oldest dropped first).
const MAX_COMMANDS: usize = 40;
/// Max decisions retained (oldest dropped first).
const MAX_DECISIONS: usize = 20;
/// Max subagent spawns retained (oldest dropped first).
const MAX_SUBAGENTS: usize = 20;
/// Max chars per recorded decision (keeps the ledger small).
const DECISION_CHARS: usize = 240;

/// How a file was touched by the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileOp {
    Read,
    Write,
}

/// Per-file touch record: which operations happened and how many times.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FileTouch {
    pub reads: usize,
    pub writes: usize,
}

impl FileTouch {
    /// Compact marker, e.g. `w` (written), `r` (read only), `rw` (both).
    fn marker(&self) -> String {
        let mut s = String::new();
        if self.writes > 0 {
            s.push('w');
        }
        if self.reads > 0 {
            s.push('r');
        }
        if s.is_empty() {
            s.push('?');
        }
        s
    }
}

/// A durable, structured record of the session's activity.
///
/// Updated incrementally as tools run; rendered into the context after each
/// compaction so key facts are never lost to a lossy summary.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContextLedger {
    /// Files touched, keyed by path (BTreeMap → stable, sorted rendering).
    #[serde(default)]
    pub files: BTreeMap<String, FileTouch>,
    /// Commands run via `bash` (deduplicated, insertion order, capped).
    #[serde(default)]
    pub commands: Vec<String>,
    /// Key decisions/notes extracted from assistant text (capped).
    #[serde(default)]
    pub decisions: Vec<String>,
    /// Subagents spawned via `task` (e.g. `explore`, `build`), capped.
    #[serde(default)]
    pub subagents: Vec<String>,
}

impl ContextLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a file read/write. `path` is stored as given (callers pass a
    /// workspace-relative path when possible).
    pub fn touch_file(&mut self, path: &str, op: FileOp) {
        if path.is_empty() {
            return;
        }
        let entry = self.files.entry(path.to_string()).or_default();
        match op {
            FileOp::Read => entry.reads += 1,
            FileOp::Write => entry.writes += 1,
        }
    }

    /// Records a command, deduplicating and capping the list.
    pub fn record_command(&mut self, cmd: &str) {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return;
        }
        if self.commands.iter().any(|c| c == cmd) {
            return;
        }
        self.commands.push(cmd.to_string());
        if self.commands.len() > MAX_COMMANDS {
            let excess = self.commands.len() - MAX_COMMANDS;
            self.commands.drain(0..excess);
        }
    }

    /// Records a decision/note, deduplicating and capping the list.
    pub fn record_decision(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let text = crate::harness::session::preview(text, DECISION_CHARS);
        if self.decisions.contains(&text) {
            return;
        }
        self.decisions.push(text);
        if self.decisions.len() > MAX_DECISIONS {
            let excess = self.decisions.len() - MAX_DECISIONS;
            self.decisions.drain(0..excess);
        }
    }

    /// Records a subagent spawn (agent name), deduplicating and capping.
    pub fn record_subagent(&mut self, agent: &str) {
        let agent = agent.trim();
        if agent.is_empty() {
            return;
        }
        if self.subagents.iter().any(|a| a == agent) {
            return;
        }
        self.subagents.push(agent.to_string());
        if self.subagents.len() > MAX_SUBAGENTS {
            let excess = self.subagents.len() - MAX_SUBAGENTS;
            self.subagents.drain(0..excess);
        }
    }

    /// Whether the ledger holds anything worth rendering.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
            && self.commands.is_empty()
            && self.decisions.is_empty()
            && self.subagents.is_empty()
    }

    /// Renders the ledger as a compact, structured Markdown block for the
    /// context. Returns `None` when empty (nothing to inject).
    pub fn render(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut out = String::from("[Session ledger — durable facts, survives compaction]\n");
        if !self.files.is_empty() {
            out.push_str("Files touched:\n");
            for (path, touch) in &self.files {
                out.push_str(&format!("- {} ({})\n", path, touch.marker()));
            }
        }
        if !self.commands.is_empty() {
            out.push_str("Commands run:\n");
            for c in &self.commands {
                out.push_str(&format!("- `{}`\n", c));
            }
        }
        if !self.subagents.is_empty() {
            out.push_str(&format!(
                "Subagents spawned: {}\n",
                self.subagents.join(", ")
            ));
        }
        if !self.decisions.is_empty() {
            out.push_str("Decisions:\n");
            for d in &self.decisions {
                out.push_str(&format!("- {}\n", d));
            }
        }
        Some(out)
    }
}

/// Extracts the workspace-relative path from a tool call, if any.
///
/// Handles the common file tools (`read`/`write`/`edit`) and `bash` (whose
/// command is recorded separately). Returns `None` for tools without a path.
pub fn file_path_from_tool(name: &str, input: &serde_json::Value) -> Option<String> {
    let key = match name {
        "read" | "write" | "edit" => "file_path",
        _ => return None,
    };
    input
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Maps a tool name to the file operation it performs, if any.
pub fn file_op_from_tool(name: &str) -> Option<FileOp> {
    match name {
        "read" => Some(FileOp::Read),
        "write" | "edit" => Some(FileOp::Write),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_touch_file_accumulates_ops() {
        let mut l = ContextLedger::new();
        l.touch_file("src/a.rs", FileOp::Read);
        l.touch_file("src/a.rs", FileOp::Read);
        l.touch_file("src/a.rs", FileOp::Write);
        let t = &l.files["src/a.rs"];
        assert_eq!(t.reads, 2);
        assert_eq!(t.writes, 1);
        assert_eq!(t.marker(), "wr");
    }

    #[test]
    fn test_touch_file_ignores_empty_path() {
        let mut l = ContextLedger::new();
        l.touch_file("", FileOp::Read);
        assert!(l.files.is_empty());
    }

    #[test]
    fn test_record_command_dedups_and_caps() {
        let mut l = ContextLedger::new();
        l.record_command("cargo build");
        l.record_command("cargo build");
        assert_eq!(l.commands.len(), 1);
        for i in 0..(MAX_COMMANDS + 5) {
            l.record_command(&format!("cmd {i}"));
        }
        assert_eq!(l.commands.len(), MAX_COMMANDS);
        // Oldest dropped: the first command is gone.
        assert!(!l.commands.iter().any(|c| c == "cargo build"));
    }

    #[test]
    fn test_record_decision_truncates_and_dedups() {
        let mut l = ContextLedger::new();
        let long = "x".repeat(DECISION_CHARS * 2);
        l.record_decision(&long);
        assert_eq!(l.decisions.len(), 1);
        // `preview` truncates to DECISION_CHARS and appends a single `…`.
        assert!(l.decisions[0].chars().count() <= DECISION_CHARS + 1);
        l.record_decision(&long);
        assert_eq!(l.decisions.len(), 1, "duplicate decision must be ignored");
    }

    #[test]
    fn test_record_subagent_dedups() {
        let mut l = ContextLedger::new();
        l.record_subagent("explore");
        l.record_subagent("explore");
        l.record_subagent("build");
        assert_eq!(l.subagents, vec!["explore", "build"]);
    }

    #[test]
    fn test_render_empty_is_none() {
        assert!(ContextLedger::new().render().is_none());
    }

    #[test]
    fn test_render_includes_all_sections() {
        let mut l = ContextLedger::new();
        l.touch_file("src/a.rs", FileOp::Write);
        l.record_command("cargo test");
        l.record_subagent("explore");
        l.record_decision("chose BTreeMap for stable order");
        let r = l.render().unwrap();
        assert!(r.contains("src/a.rs (w)"));
        assert!(r.contains("`cargo test`"));
        assert!(r.contains("Subagents spawned: explore"));
        assert!(r.contains("chose BTreeMap"));
    }

    #[test]
    fn test_file_path_and_op_from_tool() {
        let input = serde_json::json!({"file_path": "src/x.rs"});
        assert_eq!(
            file_path_from_tool("read", &input).as_deref(),
            Some("src/x.rs")
        );
        assert_eq!(file_op_from_tool("read"), Some(FileOp::Read));
        assert_eq!(file_op_from_tool("edit"), Some(FileOp::Write));
        assert_eq!(file_op_from_tool("bash"), None);
        assert_eq!(file_path_from_tool("bash", &input), None);
    }
}
