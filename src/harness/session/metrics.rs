//! Per-session metrics: a durable, structured record of how much work a
//! session did (turns, iterations, tool calls, tokens, cost, wall time).
//!
//! Like the [`ContextLedger`], the metrics live on the [`Session`] itself and
//! are persisted in SQLite, so they survive restarts and can be aggregated
//! across a project (see `/stats`). They are the raw material for evals and
//! regression tracking: without measurement there is no way to tell whether a
//! prompt/loop change helped or hurt.
//!
//! [`ContextLedger`]: crate::harness::session::ledger::ContextLedger
//! [`Session`]: crate::harness::session::Session

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Max distinct tool names tracked (defensive; the registry is far smaller).
const MAX_TOOL_NAMES: usize = 64;

/// Cumulative metrics for a single session.
///
/// All counters are monotonic within a session; `duration_ms` accumulates the
/// wall time of every turn. Cost is an estimate (see
/// [`crate::harness::budget::turn_cost`]) and is stored in USD.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionMetrics {
    /// Number of user turns processed.
    #[serde(default)]
    pub turns: usize,
    /// Total loop iterations across all turns (one per LLM request).
    #[serde(default)]
    pub iterations: usize,
    /// Total tool calls executed, keyed by tool name (BTreeMap → stable order).
    #[serde(default)]
    pub tool_calls: BTreeMap<String, usize>,
    /// Total tool calls that ended in an error status.
    #[serde(default)]
    pub tool_errors: usize,
    /// Input tokens billed (excluding cache reads/writes).
    #[serde(default)]
    pub input_tokens: u64,
    /// Output tokens billed.
    #[serde(default)]
    pub output_tokens: u64,
    /// Cache-read tokens (billed at a discount).
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Cache-write tokens (billed at a premium).
    #[serde(default)]
    pub cache_write_tokens: u64,
    /// Estimated cumulative cost in USD.
    #[serde(default)]
    pub cost_usd: f64,
    /// Cumulative wall time spent inside turns, in milliseconds.
    #[serde(default)]
    pub duration_ms: u64,
    /// Number of context compactions performed.
    #[serde(default)]
    pub compactions: usize,
    /// Number of subagents spawned via `task`.
    #[serde(default)]
    pub subagents: usize,
}

/// Token usage for a single turn, grouped to keep `record_turn` readable.
#[derive(Clone, Copy, Debug, Default)]
pub struct TurnTokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl SessionMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one completed turn: its iteration count, token usage, estimated
    /// cost and wall time.
    pub fn record_turn(
        &mut self,
        iterations: usize,
        tokens: TurnTokens,
        cost_usd: f64,
        duration_ms: u64,
    ) {
        self.turns += 1;
        self.iterations += iterations;
        self.input_tokens += tokens.input;
        self.output_tokens += tokens.output;
        self.cache_read_tokens += tokens.cache_read;
        self.cache_write_tokens += tokens.cache_write;
        self.cost_usd += cost_usd;
        self.duration_ms += duration_ms;
    }

    /// Records a single tool call by name, and whether it errored.
    pub fn record_tool_call(&mut self, name: &str, is_error: bool) {
        if name.is_empty() {
            return;
        }
        // Defensive cap: never let a hostile/buggy tool name blow up the map.
        if !self.tool_calls.contains_key(name) && self.tool_calls.len() >= MAX_TOOL_NAMES {
            return;
        }
        *self.tool_calls.entry(name.to_string()).or_insert(0) += 1;
        if is_error {
            self.tool_errors += 1;
        }
    }

    /// Records a context compaction.
    pub fn record_compaction(&mut self) {
        self.compactions += 1;
    }

    /// Records a subagent spawn.
    pub fn record_subagent(&mut self) {
        self.subagents += 1;
    }

    /// Total tokens billed (input + output + cache read + cache write).
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }

    /// Total number of tool calls across all tools.
    pub fn total_tool_calls(&self) -> usize {
        self.tool_calls.values().sum()
    }

    /// Whether any metric has been recorded.
    #[allow(dead_code)] // used by tests; kept for API completeness
    pub fn is_empty(&self) -> bool {
        self.turns == 0
            && self.iterations == 0
            && self.tool_calls.is_empty()
            && self.total_tokens() == 0
    }

    /// Renders a compact, human-readable report of the session metrics.
    pub fn render(&self) -> String {
        let mut out = String::from("Session metrics\n");
        out.push_str(&format!("  turns:        {}\n", self.turns));
        out.push_str(&format!("  iterations:   {}\n", self.iterations));
        out.push_str(&format!(
            "  tool calls:   {} ({} errors)\n",
            self.total_tool_calls(),
            self.tool_errors
        ));
        if !self.tool_calls.is_empty() {
            let mut tools: Vec<(&String, &usize)> = self.tool_calls.iter().collect();
            // Most-used first, then alphabetical for stable output.
            tools.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
            let rendered = tools
                .iter()
                .map(|(name, n)| format!("{name}×{n}"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("  by tool:      {rendered}\n"));
        }
        out.push_str(&format!(
            "  tokens:       {} in / {} out / {} cache-read / {} cache-write\n",
            self.input_tokens, self.output_tokens, self.cache_read_tokens, self.cache_write_tokens
        ));
        out.push_str(&format!("  cost:         ${:.4}\n", self.cost_usd));
        out.push_str(&format!(
            "  wall time:    {:.1}s\n",
            self.duration_ms as f64 / 1000.0
        ));
        out.push_str(&format!("  compactions:  {}\n", self.compactions));
        out.push_str(&format!("  subagents:    {}\n", self.subagents));
        out
    }
}

/// Aggregated metrics across many sessions of a project (for `/stats`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProjectMetrics {
    pub sessions: usize,
    pub turns: usize,
    pub iterations: usize,
    pub tool_calls: usize,
    pub tool_errors: usize,
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub duration_ms: u64,
    pub compactions: usize,
    pub subagents: usize,
}

impl ProjectMetrics {
    /// Folds one session's metrics into the aggregate.
    pub fn add(&mut self, m: &SessionMetrics) {
        self.sessions += 1;
        self.turns += m.turns;
        self.iterations += m.iterations;
        self.tool_calls += m.total_tool_calls();
        self.tool_errors += m.tool_errors;
        self.total_tokens += m.total_tokens();
        self.cost_usd += m.cost_usd;
        self.duration_ms += m.duration_ms;
        self.compactions += m.compactions;
        self.subagents += m.subagents;
    }

    /// Renders a compact, human-readable report of the project aggregate.
    pub fn render(&self) -> String {
        let mut out = String::from("Project metrics (all sessions)\n");
        out.push_str(&format!("  sessions:     {}\n", self.sessions));
        out.push_str(&format!("  turns:        {}\n", self.turns));
        out.push_str(&format!("  iterations:   {}\n", self.iterations));
        out.push_str(&format!(
            "  tool calls:   {} ({} errors)\n",
            self.tool_calls, self.tool_errors
        ));
        out.push_str(&format!("  tokens:       {}\n", self.total_tokens));
        out.push_str(&format!("  cost:         ${:.4}\n", self.cost_usd));
        out.push_str(&format!(
            "  wall time:    {:.1}s\n",
            self.duration_ms as f64 / 1000.0
        ));
        out.push_str(&format!("  compactions:  {}\n", self.compactions));
        out.push_str(&format!("  subagents:    {}\n", self.subagents));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_turn_accumulates() {
        let mut m = SessionMetrics::new();
        m.record_turn(
            3,
            TurnTokens {
                input: 100,
                output: 50,
                cache_read: 10,
                cache_write: 5,
            },
            0.01,
            1000,
        );
        m.record_turn(
            2,
            TurnTokens {
                input: 200,
                output: 60,
                cache_read: 0,
                cache_write: 0,
            },
            0.02,
            500,
        );
        assert_eq!(m.turns, 2);
        assert_eq!(m.iterations, 5);
        assert_eq!(m.input_tokens, 300);
        assert_eq!(m.output_tokens, 110);
        assert_eq!(m.cache_read_tokens, 10);
        assert_eq!(m.cache_write_tokens, 5);
        assert!((m.cost_usd - 0.03).abs() < 1e-9);
        assert_eq!(m.duration_ms, 1500);
    }

    #[test]
    fn test_record_tool_call_counts_and_errors() {
        let mut m = SessionMetrics::new();
        m.record_tool_call("bash", false);
        m.record_tool_call("bash", true);
        m.record_tool_call("read", false);
        assert_eq!(m.tool_calls["bash"], 2);
        assert_eq!(m.tool_calls["read"], 1);
        assert_eq!(m.tool_errors, 1);
        assert_eq!(m.total_tool_calls(), 3);
    }

    #[test]
    fn test_record_tool_call_ignores_empty_name() {
        let mut m = SessionMetrics::new();
        m.record_tool_call("", false);
        assert!(m.tool_calls.is_empty());
    }

    #[test]
    fn test_record_tool_call_caps_distinct_names() {
        let mut m = SessionMetrics::new();
        for i in 0..(MAX_TOOL_NAMES + 10) {
            m.record_tool_call(&format!("tool{i}"), false);
        }
        assert_eq!(m.tool_calls.len(), MAX_TOOL_NAMES);
    }

    #[test]
    fn test_total_tokens_includes_cache() {
        let mut m = SessionMetrics::new();
        m.record_turn(
            1,
            TurnTokens {
                input: 10,
                output: 20,
                cache_read: 30,
                cache_write: 40,
            },
            0.0,
            0,
        );
        assert_eq!(m.total_tokens(), 100);
    }

    #[test]
    fn test_is_empty() {
        let mut m = SessionMetrics::new();
        assert!(m.is_empty());
        m.record_tool_call("bash", false);
        assert!(!m.is_empty());
    }

    #[test]
    fn test_render_includes_key_fields() {
        let mut m = SessionMetrics::new();
        m.record_turn(
            4,
            TurnTokens {
                input: 100,
                output: 50,
                cache_read: 0,
                cache_write: 0,
            },
            0.0123,
            2500,
        );
        m.record_tool_call("bash", false);
        m.record_tool_call("bash", true);
        m.record_compaction();
        m.record_subagent();
        let r = m.render();
        assert!(r.contains("turns:        1"));
        assert!(r.contains("iterations:   4"));
        assert!(r.contains("bash×2"));
        assert!(r.contains("1 errors"));
        assert!(r.contains("$0.0123"));
        assert!(r.contains("2.5s"));
        assert!(r.contains("compactions:  1"));
        assert!(r.contains("subagents:    1"));
    }

    #[test]
    fn test_project_metrics_aggregates() {
        let mut a = SessionMetrics::new();
        a.record_turn(
            2,
            TurnTokens {
                input: 10,
                output: 20,
                cache_read: 0,
                cache_write: 0,
            },
            0.01,
            100,
        );
        a.record_tool_call("bash", false);
        let mut b = SessionMetrics::new();
        b.record_turn(
            3,
            TurnTokens {
                input: 30,
                output: 40,
                cache_read: 0,
                cache_write: 0,
            },
            0.02,
            200,
        );
        b.record_tool_call("read", true);

        let mut p = ProjectMetrics::default();
        p.add(&a);
        p.add(&b);
        assert_eq!(p.sessions, 2);
        assert_eq!(p.turns, 2);
        assert_eq!(p.iterations, 5);
        assert_eq!(p.tool_calls, 2);
        assert_eq!(p.tool_errors, 1);
        assert_eq!(p.total_tokens, 100);
        assert!((p.cost_usd - 0.03).abs() < 1e-9);
        assert_eq!(p.duration_ms, 300);
    }

    #[test]
    fn test_serde_roundtrip() {
        let mut m = SessionMetrics::new();
        m.record_turn(
            1,
            TurnTokens {
                input: 1,
                output: 2,
                cache_read: 3,
                cache_write: 4,
            },
            0.5,
            10,
        );
        m.record_tool_call("edit", false);
        let json = serde_json::to_string(&m).unwrap();
        let back: SessionMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(back.turns, 1);
        assert_eq!(back.tool_calls["edit"], 1);
        assert!((back.cost_usd - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_deserialize_legacy_empty_object() {
        // A legacy row with `{}` must deserialize to all-default metrics.
        let m: SessionMetrics = serde_json::from_str("{}").unwrap();
        assert!(m.is_empty());
    }
}
