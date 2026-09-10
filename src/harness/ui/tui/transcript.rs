//! Transcript line types and tool-batch status helpers.
//!
//! These are the data types backing the transcript view (line kinds, tool
//! batch transient state, tool-call labels), extracted from `app.rs` so the
//! draw layer and the app state share a single definition.

/// Kind of a transcript line, used to pick colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    User,
    Assistant,
    Reasoning,
    ToolStart,
    ToolOk,
    ToolError,
    System,
    Error,
    Diff,
}

#[derive(Clone, Debug)]
pub struct TranscriptLine {
    pub kind: LineKind,
    pub text: String,
}

/// Marker appended to the collapsed thinking summary line; the draw layer
/// uses it to render the compact one-line panel instead of the full box.
pub const THINKING_COLLAPSED_MARKER: &str = "— press x to expand)";

/// Groups consecutive [`LineKind::Reasoning`] lines into a collapsible block.
///
/// When `expanded` is `false`, each run of consecutive reasoning lines is
/// replaced by a single line: a truncated preview of the first line plus a
/// char counter and the expand hint. When `true`, all lines are kept as-is.
/// Non-reasoning lines are always preserved.
pub fn collapse_thinking(lines: &[TranscriptLine], expanded: bool) -> Vec<TranscriptLine> {
    if expanded {
        return lines.to_vec();
    }
    let mut out: Vec<TranscriptLine> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Reasoning {
            out.push(lines[i].clone());
            i += 1;
            continue;
        }
        // Find the end of the consecutive reasoning run.
        let start = i;
        let mut total_chars = 0usize;
        while i < lines.len() && lines[i].kind == LineKind::Reasoning {
            total_chars += lines[i].text.chars().count();
            i += 1;
        }
        let first = &lines[start].text;
        let preview: String = first.chars().take(80).collect();
        let ellipsis = if first.chars().count() > 80 {
            "…"
        } else {
            ""
        };
        out.push(TranscriptLine {
            kind: LineKind::Reasoning,
            text: format!(
                "{}{} ({} chars thinking — {}",
                preview, ellipsis, total_chars, THINKING_COLLAPSED_MARKER
            ),
        });
    }
    out
}

#[derive(Clone, Debug)]
pub struct ActiveTool {
    pub name: String,
}

/// Transient state of the current parallel tool batch. Instead of pushing one
/// line per tool call, a single status line is overwritten while the batch is
/// running and a unique summary line is emitted when the batch completes.
#[derive(Clone, Debug, Default)]
pub struct ToolBatch {
    /// Per-tool completion counts, e.g. [("read", 3), ("bash", 1)].
    pub counts: Vec<(String, usize)>,
    /// Path/args preview of the most recently started tool.
    pub last_path: String,
    /// Last started tool name (used for the transient "running" line).
    pub last_name: String,
    pub done: usize,
    pub failed: usize,
    pub pending: usize,
}

impl ToolBatch {
    pub fn start(&mut self, name: &str, path: String) {
        self.last_name = name.to_string();
        self.last_path = path;
        self.pending += 1;
        if let Some(e) = self.counts.iter_mut().find(|(n, _)| n == name) {
            e.1 += 1;
        } else {
            self.counts.push((name.to_string(), 1));
        }
    }

    /// Summary like `read ×3 · bash ×1 (cargo test …)` — the trailing
    /// parentheses show the most recent call label so the user can see what
    /// the tools were actually doing.
    pub fn summary(&self) -> String {
        let mut s = self
            .counts
            .iter()
            .map(|(n, c)| format!("{} ×{}", n, c))
            .collect::<Vec<_>>()
            .join(" · ");
        if !self.last_path.trim().is_empty() && s.chars().count() < 60 {
            s.push_str(&format!(" ({})", self.last_path));
        }
        s
    }

    /// Transient label while running: `bash cargo test … (2/4)`.
    pub fn live_label(&self) -> String {
        format!(
            "{} {} ({}/{})",
            self.last_name,
            self.last_path,
            self.done + self.failed,
            self.pending + self.done + self.failed
        )
    }
}

/// Human-friendly one-line label of a tool call input (opencode-style):
/// key fields first (`command`/`path`/`query`…), falling back to a compact
/// JSON preview. Used on the transcript tool status lines.
pub fn tool_arg_label(name: &str, input: &serde_json::Value) -> String {
    let _ = name;
    for key in [
        "command",
        "cmd",
        "path",
        "file_path",
        "query",
        "pattern",
        "url",
        "text",
    ] {
        if let Some(s) = input.get(key).and_then(|v| v.as_str()) {
            if !s.trim().is_empty() {
                return preview(s, 60);
            }
        }
    }
    preview(&input.to_string(), 60)
}

pub fn preview(s: &str, max: usize) -> String {
    crate::harness::session::preview(s, max)
}
