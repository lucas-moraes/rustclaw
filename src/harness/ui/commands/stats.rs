//! `/stats` — per-session and per-project metrics.
//!
//! Reports the current session's cumulative metrics (turns, iterations, tool
//! calls, tokens, cost, wall time) plus the aggregate across every session of
//! the project. The aggregate is read from the persisted `metrics_json` column,
//! so it survives restarts and covers sessions that are no longer loaded.

use crate::harness::runtime::SessionRuntime;
use crate::harness::session::Session;
use anyhow::Result;

/// Renders the `/stats` output for the current session and the project.
pub fn handle_stats_command(runtime: &SessionRuntime, session: &Session) -> Result<Vec<String>> {
    let mut out = Vec::new();
    out.push(session.metrics.render());

    match runtime.store.aggregate_metrics(&session.cwd) {
        Ok(agg) => out.push(agg.render()),
        Err(e) => out.push(format!(
            "[error] failed to aggregate project metrics: {e:#}"
        )),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::harness::session::metrics::{SessionMetrics, TurnTokens};

    #[test]
    fn test_render_session_metrics_has_sections() {
        let mut m = SessionMetrics::new();
        m.record_turn(
            2,
            TurnTokens {
                input: 100,
                output: 50,
                ..Default::default()
            },
            0.01,
            1000,
        );
        m.record_tool_call("bash", false);
        let r = m.render();
        assert!(r.contains("Session metrics"));
        assert!(r.contains("turns:        1"));
        assert!(r.contains("bash×1"));
    }
}
