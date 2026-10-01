//! Doom-loop detection for the session processor.
//!
//! Detects when the model repeats the same tool call(s) across iterations,
//! including multi-call cycles (A,B,A,B) — not just a single repeated call.
//! The detector compares the *set* of tool-call signatures of the current
//! turn against the previous turns' sets, warns once at the warn threshold,
//! and signals a stop at the stop threshold. A broken pattern resets the
//! streak and the warn flag so a later distinct loop gets fresh feedback.

use std::collections::{HashMap, VecDeque};

/// Warn threshold: repeated the same call(s) this many times.
pub const DOOM_LOOP_WARN: usize = 3;
/// Stop threshold: repeated the same call(s) this many times.
pub const DOOM_LOOP_STOP: usize = 5;
/// How many recent turns' signature sets to keep for cycle detection.
const DOOM_LOOP_WINDOW: usize = 5;

/// Warn threshold: the same assistant text seen this many times in the window.
pub const TEXT_LOOP_WARN: usize = 3;
/// Stop threshold: the same assistant text seen this many times in the window.
pub const TEXT_LOOP_STOP: usize = 5;

/// Normalizes assistant text into a loop-comparison signature: whitespace is
/// collapsed and case is folded so trivial variations still match.
pub fn normalize_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Action the processor should take after feeding a signature set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoomAction {
    /// No loop; keep going.
    Continue,
    /// First crossing of the warn threshold (emit a warning to the model).
    Warn,
    /// Crossed the stop threshold (abort the turn).
    Stop,
}

/// Tracks repeated tool-call signature sets across iterations.
#[derive(Default)]
pub struct DoomLoopDetector {
    recent_sigs: VecDeque<Vec<String>>,
    streak: usize,
    warned: bool,
}

impl DoomLoopDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds the current turn's tool-call signatures and returns the action
    /// to take. An empty signature set (no tool calls) resets the detector.
    pub fn record(&mut self, sigs: Vec<String>) -> DoomAction {
        if sigs.is_empty() {
            self.recent_sigs.clear();
            self.streak = 0;
            self.warned = false;
            return DoomAction::Continue;
        }

        let is_repeat = self
            .recent_sigs
            .back()
            .map(|prev| prev == &sigs)
            .unwrap_or(false);
        if is_repeat {
            self.streak += 1;
        } else {
            // Pattern broke → reset the streak and the warn flag so a later
            // distinct loop gets fresh feedback.
            self.streak = 1;
            self.warned = false;
        }
        self.recent_sigs.push_back(sigs);
        if self.recent_sigs.len() > DOOM_LOOP_WINDOW {
            self.recent_sigs.pop_front();
        }

        if self.streak >= DOOM_LOOP_STOP {
            DoomAction::Stop
        } else if self.streak == DOOM_LOOP_WARN && !self.warned {
            self.warned = true;
            DoomAction::Warn
        } else {
            DoomAction::Continue
        }
    }
}

/// Tracks repeated assistant **text** across iterations. Feeds a normalized
/// text signature per iteration; per-signature counts are cumulative for the
/// whole turn, so long cycles (A,B,C,A,B,C with period 3..6) are also caught —
/// a short bounded window would cap counts below the stop threshold for any
/// cycle longer than the window.
#[derive(Default)]
pub struct TextLoopDetector {
    counts: HashMap<String, usize>,
    warned: bool,
}

impl TextLoopDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds a normalized assistant-text signature (`normalize_text`) and
    /// returns the action. An empty signature (no text) is ignored so
    /// tool-only or reasoning-only iterations don't reset the counts.
    pub fn record(&mut self, sig: String) -> DoomAction {
        if sig.is_empty() {
            return DoomAction::Continue;
        }

        let seen = self.counts.entry(sig).or_insert(0);
        *seen += 1;
        let seen = *seen;

        if seen >= TEXT_LOOP_STOP {
            DoomAction::Stop
        } else if seen == TEXT_LOOP_WARN && !self.warned {
            self.warned = true;
            DoomAction::Warn
        } else {
            DoomAction::Continue
        }
    }
}

/// Extracts the *primary target* of a tool call for semantic loop detection:
/// the path for file tools, the first token for `bash`, the query for
/// `grep`/`web_search`, etc. Returns an empty string when the tool has no
/// meaningful target (then the detector falls back to the tool name alone).
///
/// The target is normalized (trimmed, `./`/trailing-slash collapsed) so that
/// `ls`, `ls .` and `ls ./` map to the same target.
pub fn tool_target(name: &str, input: &serde_json::Value) -> String {
    let raw = match name {
        "read" | "write" | "edit" | "ast_search" | "diagnostics" => input
            .get("path")
            .or_else(|| input.get("file"))
            .and_then(|v| v.as_str()),
        "glob" => input.get("pattern").and_then(|v| v.as_str()),
        "grep" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .or_else(|| input.get("path").and_then(|v| v.as_str())),
        "bash" => input.get("command").and_then(|v| v.as_str()),
        "web_search" => input.get("query").and_then(|v| v.as_str()),
        "fetch_webpage" => input.get("url").and_then(|v| v.as_str()),
        _ => None,
    };
    let Some(raw) = raw else {
        return String::new();
    };
    normalize_target(raw)
}

/// Normalizes a target string: trims, collapses whitespace, strips a leading
/// `./` and trailing `/`, and (for commands) keeps only the first token so
/// `ls -la` and `ls` share a target.
fn normalize_target(raw: &str) -> String {
    let trimmed = raw.trim();
    // For shell commands, the first token is the program — the meaningful
    // "target" for loop detection (flags/args vary harmlessly).
    let first = trimmed.split_whitespace().next().unwrap_or("");
    let mut t = first
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string();
    if t.is_empty() {
        // A bare `.`/`./`/`/` command (e.g. `ls .`) normalizes to ".".
        t = ".".to_string();
    }
    t
}

/// A normalized signature of a tool output, used to tell "same result again"
/// from "real progress". Empty/whitespace-only outputs collapse to `""`.
fn output_signature(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Cap the length: only the head matters for equality, and this keeps the
    // per-target state small.
    let head: String = trimmed.chars().take(200).collect();
    normalize_text(&head)
}

/// Per-target state for the semantic detector.
#[derive(Default)]
struct TargetState {
    /// How many consecutive iterations this target produced no progress.
    streak: usize,
    /// Signature of the last output seen for this target.
    last_output: String,
}

/// Detects *semantic* loops: the same tool aimed at the same target, repeated
/// with no progress (empty output, an error, or the identical output). Unlike
/// [`DoomLoopDetector`] (byte-identical signatures), this catches inputs that
/// vary harmlessly — `ls`, `ls .`, `ls ./` — and repeated failures.
///
/// Progress (a different, non-empty output) resets the target's streak, so
/// legitimate exploration is not flagged.
#[derive(Default)]
pub struct SemanticLoopDetector {
    targets: HashMap<(String, String), TargetState>,
    warned: bool,
}

/// One tool call's contribution to a semantic-loop check.
pub struct ToolOutcome {
    pub name: String,
    pub target: String,
    /// The tool's output text.
    pub output: String,
    /// Whether the tool call failed.
    pub is_error: bool,
}

impl SemanticLoopDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one iteration's tool outcomes and returns the action to take.
    /// An empty list resets the detector (no tool calls = no loop).
    pub fn record(&mut self, outcomes: Vec<ToolOutcome>) -> DoomAction {
        if outcomes.is_empty() {
            self.targets.clear();
            self.warned = false;
            return DoomAction::Continue;
        }

        let mut max_streak = 0usize;
        for o in outcomes {
            // Only tools with a meaningful target participate: a tool with no
            // path/query (target empty) has no "same target" notion, so it is
            // left to the byte-identical detector instead.
            if o.target.is_empty() {
                continue;
            }
            let key = (o.name.clone(), o.target.clone());
            let sig = output_signature(&o.output);
            let state = self.targets.entry(key).or_default();

            // Progress = a non-empty, non-error output that differs from the
            // last one. Anything else (empty, error, identical) is a repeat.
            let progressed = !o.is_error && !sig.is_empty() && sig != state.last_output;
            if progressed {
                state.streak = 1;
            } else {
                state.streak += 1;
            }
            state.last_output = sig;
            max_streak = max_streak.max(state.streak);
        }

        if max_streak >= DOOM_LOOP_STOP {
            DoomAction::Stop
        } else if max_streak >= DOOM_LOOP_WARN && !self.warned {
            self.warned = true;
            DoomAction::Warn
        } else {
            DoomAction::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_repeated_call_warns_then_stops() {
        let mut d = DoomLoopDetector::new();
        let sig = vec!["bash:ls".to_string()];
        assert_eq!(d.record(sig.clone()), DoomAction::Continue);
        assert_eq!(d.record(sig.clone()), DoomAction::Continue);
        assert_eq!(d.record(sig.clone()), DoomAction::Warn); // 3rd
        assert_eq!(d.record(sig.clone()), DoomAction::Continue); // 4th (already warned)
        assert_eq!(d.record(sig.clone()), DoomAction::Stop); // 5th
    }

    #[test]
    fn test_multi_call_cycle_detected() {
        let mut d = DoomLoopDetector::new();
        let ab = vec!["a:1".to_string(), "b:2".to_string()];
        // A,B,A,B cycle: the set is identical each time.
        assert_eq!(d.record(ab.clone()), DoomAction::Continue);
        assert_eq!(d.record(ab.clone()), DoomAction::Continue);
        assert_eq!(d.record(ab.clone()), DoomAction::Warn);
        assert_eq!(d.record(ab.clone()), DoomAction::Continue);
        assert_eq!(d.record(ab.clone()), DoomAction::Stop);
    }

    #[test]
    fn test_broken_pattern_resets_and_re_warns() {
        let mut d = DoomLoopDetector::new();
        let a = vec!["a:1".to_string()];
        let b = vec!["b:2".to_string()];
        assert_eq!(d.record(a.clone()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Warn);
        // Pattern breaks → streak resets.
        assert_eq!(d.record(b.clone()), DoomAction::Continue);
        // A fresh loop gets a fresh warn.
        assert_eq!(d.record(a.clone()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Warn);
    }

    #[test]
    fn test_empty_signatures_reset() {
        let mut d = DoomLoopDetector::new();
        let a = vec!["a:1".to_string()];
        assert_eq!(d.record(a.clone()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Warn);
        // No tool calls → reset.
        assert_eq!(d.record(vec![]), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Warn);
    }

    fn sig(s: &str) -> String {
        normalize_text(s)
    }

    #[test]
    fn test_text_loop_direct_warn_then_hard_stop() {
        let mut d = TextLoopDetector::new();
        let s = sig("Let me check the session/mod.rs:");
        assert_eq!(d.record(s.clone()), DoomAction::Continue);
        assert_eq!(d.record(s.clone()), DoomAction::Continue);
        assert_eq!(d.record(s.clone()), DoomAction::Warn);
        assert_eq!(d.record(s.clone()), DoomAction::Continue);
        assert_eq!(d.record(s.clone()), DoomAction::Stop);
    }

    #[test]
    fn test_text_loop_alternation_detected() {
        let mut d = TextLoopDetector::new();
        let a = sig("Let me check the session/mod.rs:");
        let b = sig("Let me grep for 'pub fn preview'");
        // A,B,A,B cycle: A's count grows with every cycle.
        assert_eq!(d.record(a.clone()), DoomAction::Continue); // A1
        assert_eq!(d.record(b.clone()), DoomAction::Continue); // B1
        assert_eq!(d.record(a.clone()), DoomAction::Continue); // A2
        assert_eq!(d.record(b.clone()), DoomAction::Continue); // B2
        assert_eq!(d.record(a.clone()), DoomAction::Warn); // A3
        assert_eq!(d.record(b.clone()), DoomAction::Continue); // B3 (warned already)
        assert_eq!(d.record(a.clone()), DoomAction::Continue); // A4
        assert_eq!(d.record(b.clone()), DoomAction::Continue); // B4
        assert_eq!(d.record(a.clone()), DoomAction::Stop); // A5
    }

    #[test]
    fn test_text_loop_new_text_resets_warn() {
        let mut d = TextLoopDetector::new();
        let a = sig("repeat me");
        assert_eq!(d.record(a.clone()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Warn);
        // Fresh text appears → warn flag resets; a later A-loop still counts
        // because the window keeps the earlier repetitions.
        assert_eq!(d.record(sig("totally different")), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Continue); // A4th overall
        assert_eq!(d.record(a.clone()), DoomAction::Stop); // 5th occurrence
    }

    #[test]
    fn test_text_loop_whitespace_case_normalized() {
        let mut d = TextLoopDetector::new();
        assert_eq!(
            d.record(sig("Let  me  CHECK   the   File")),
            DoomAction::Continue
        );
        assert_eq!(
            d.record(sig("let  me check the file")),
            DoomAction::Continue
        );
        assert_eq!(d.record(sig("let me  check the file")), DoomAction::Warn);
    }

    #[test]
    fn test_text_loop_empty_sig_ignored() {
        let mut d = TextLoopDetector::new();
        let a = sig("loop text");
        assert_eq!(d.record(a.clone()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Warn);
        // Tool-only iteration (no text) does not reset the counts.
        assert_eq!(d.record(String::new()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Continue); // 4th
        assert_eq!(d.record(a.clone()), DoomAction::Stop); // 5th
    }

    #[test]
    fn test_text_loop_long_cycle_period3_stops() {
        // Long cycle A,B,C,A,B,C... — a short window would miss it.
        let mut d = TextLoopDetector::new();
        let a = sig("check header version");
        let b = sig("get committed module");
        let c = sig("run git show");
        for _ in 0..2 {
            assert_eq!(d.record(a.clone()), DoomAction::Continue);
            assert_eq!(d.record(b.clone()), DoomAction::Continue);
            assert_eq!(d.record(c.clone()), DoomAction::Continue);
        }
        // 3rd A → Warn; count keeps climbing per signature.
        assert_eq!(d.record(a.clone()), DoomAction::Warn);
        assert_eq!(d.record(b.clone()), DoomAction::Continue); // B3 (warned)
        assert_eq!(d.record(c.clone()), DoomAction::Continue);
        assert_eq!(d.record(a.clone()), DoomAction::Continue); // A4
                                                               // 5th A overall → Stop even though B and C only saw 4 rounds.
        assert_eq!(d.record(a.clone()), DoomAction::Stop);
    }

    // --- R5: semantic loop detection ---

    fn outcome(name: &str, target: &str, output: &str, is_error: bool) -> ToolOutcome {
        ToolOutcome {
            name: name.to_string(),
            target: target.to_string(),
            output: output.to_string(),
            is_error,
        }
    }

    #[test]
    fn test_tool_target_normalizes_paths_and_commands() {
        use serde_json::json;
        // `ls`, `ls .`, `ls ./` all normalize to the same target.
        assert_eq!(tool_target("bash", &json!({"command": "ls"})), "ls");
        assert_eq!(tool_target("bash", &json!({"command": "ls ."})), "ls");
        assert_eq!(tool_target("bash", &json!({"command": "ls ./"})), "ls");
        assert_eq!(tool_target("bash", &json!({"command": "ls -la"})), "ls");
        // Paths: leading `./` and trailing `/` collapse.
        assert_eq!(
            tool_target("read", &json!({"path": "./src/a.rs"})),
            "src/a.rs"
        );
        assert_eq!(
            tool_target("read", &json!({"path": "src/a.rs/"})),
            "src/a.rs"
        );
        // grep uses the pattern; web_search the query.
        assert_eq!(tool_target("grep", &json!({"pattern": "foo"})), "foo");
        assert_eq!(tool_target("web_search", &json!({"query": "rust"})), "rust");
        // Unknown tool / missing field → empty target.
        assert_eq!(tool_target("mystery", &json!({})), "");
    }

    #[test]
    fn test_semantic_loop_detects_varying_ls() {
        // `ls`, `ls .`, `ls ./` are byte-different but semantically identical
        // and produce the same output → detected as a loop.
        let mut d = SemanticLoopDetector::new();
        let o = |cmd: &str| {
            outcome(
                "bash",
                &tool_target("bash", &serde_json::json!({"command": cmd})),
                "file1\nfile2",
                false,
            )
        };
        assert_eq!(d.record(vec![o("ls")]), DoomAction::Continue);
        assert_eq!(d.record(vec![o("ls .")]), DoomAction::Continue);
        assert_eq!(d.record(vec![o("ls ./")]), DoomAction::Warn); // 3rd
        assert_eq!(d.record(vec![o("ls")]), DoomAction::Continue); // 4th
        assert_eq!(d.record(vec![o("ls .")]), DoomAction::Stop); // 5th
    }

    #[test]
    fn test_semantic_loop_progress_resets() {
        // `read a.rs` with *different* outputs is real progress → no loop.
        let mut d = SemanticLoopDetector::new();
        for i in 0..10 {
            let out = format!("content version {i}");
            let action = d.record(vec![outcome("read", "a.rs", &out, false)]);
            assert_eq!(action, DoomAction::Continue, "progress must not loop");
        }
    }

    #[test]
    fn test_semantic_loop_repeated_error_stops() {
        // `bash` returning the same error 5× → stop.
        let mut d = SemanticLoopDetector::new();
        let err = || outcome("bash", "make", "error: build failed", true);
        assert_eq!(d.record(vec![err()]), DoomAction::Continue);
        assert_eq!(d.record(vec![err()]), DoomAction::Continue);
        assert_eq!(d.record(vec![err()]), DoomAction::Warn);
        assert_eq!(d.record(vec![err()]), DoomAction::Continue);
        assert_eq!(d.record(vec![err()]), DoomAction::Stop);
    }

    #[test]
    fn test_semantic_loop_empty_output_counts_as_no_progress() {
        // Empty output (e.g. a no-op grep) repeated → loop.
        let mut d = SemanticLoopDetector::new();
        let o = || outcome("grep", "needle", "", false);
        assert_eq!(d.record(vec![o()]), DoomAction::Continue);
        assert_eq!(d.record(vec![o()]), DoomAction::Continue);
        assert_eq!(d.record(vec![o()]), DoomAction::Warn);
        assert_eq!(d.record(vec![o()]), DoomAction::Continue);
        assert_eq!(d.record(vec![o()]), DoomAction::Stop);
    }

    #[test]
    fn test_semantic_loop_empty_outcomes_reset() {
        let mut d = SemanticLoopDetector::new();
        let o = || outcome("bash", "ls", "same", false);
        assert_eq!(d.record(vec![o()]), DoomAction::Continue);
        assert_eq!(d.record(vec![o()]), DoomAction::Continue);
        assert_eq!(d.record(vec![o()]), DoomAction::Warn);
        // No tool calls → reset.
        assert_eq!(d.record(vec![]), DoomAction::Continue);
        assert_eq!(d.record(vec![o()]), DoomAction::Continue);
        assert_eq!(d.record(vec![o()]), DoomAction::Continue);
        assert_eq!(d.record(vec![o()]), DoomAction::Warn);
    }

    #[test]
    fn test_semantic_loop_different_targets_do_not_loop() {
        // Same tool, different targets → not a loop.
        let mut d = SemanticLoopDetector::new();
        for i in 0..10 {
            let target = format!("file{i}.rs");
            let action = d.record(vec![outcome("read", &target, "same output", false)]);
            assert_eq!(action, DoomAction::Continue);
        }
    }
}
