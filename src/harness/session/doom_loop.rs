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
}
