//! Token/cost accounting helpers for the TUI session.

use crate::harness::provider::{format_tokens, Usage};

use super::state::App;

impl App {
    pub fn record_usage(&mut self, usage: Usage, iterations: usize) {
        self.last_iterations = iterations;
        self.last_usage = usage;
        self.session_usage.add_assign(usage);
    }

    pub fn reset_usage(&mut self) {
        self.last_usage = Usage::default();
        self.session_usage = Usage::default();
        self.last_iterations = 0;
    }

    pub fn context_tokens(&self) -> usize {
        self.session.approx_tokens()
    }

    pub fn max_context_tokens(&self) -> usize {
        self.runtime.config.max_context_tokens
    }

    /// Estimated USD cost of the whole session at the current provider/model.
    pub fn session_cost(&self) -> f64 {
        crate::harness::provider::catalog::estimate_cost_cached(
            &self.runtime.config.provider,
            &self.runtime.config.model,
            &self.session_usage,
        )
    }

    /// Estimated USD cost of the last turn at the current provider/model.
    pub fn last_cost(&self) -> f64 {
        crate::harness::provider::catalog::estimate_cost_cached(
            &self.runtime.config.provider,
            &self.runtime.config.model,
            &self.last_usage,
        )
    }

    /// Multi-line usage report for `/usage` and turn summaries.
    pub fn usage_report(&self) -> Vec<String> {
        let ctx = self.context_tokens();
        let max = self.max_context_tokens();
        let pct = if max == 0 { 0 } else { (ctx * 100) / max };
        let mut lines = vec![
            format!(
                "last turn · in {} · out {} · total {} · {} iter(s)",
                format_tokens(self.last_usage.input_tokens),
                format_tokens(self.last_usage.output_tokens),
                format_tokens(self.last_usage.total()),
                self.last_iterations
            ),
            format!(
                "session  · in {} · out {} · total {}",
                format_tokens(self.session_usage.input_tokens),
                format_tokens(self.session_usage.output_tokens),
                format_tokens(self.session_usage.total())
            ),
        ];
        // Prompt-cache line (only when the provider reports cache activity).
        let cached = self.session_usage.cache_total();
        if cached > 0 {
            let pct = if self.session_usage.input_tokens > 0 {
                (self.session_usage.cache_read_tokens * 100) / self.session_usage.input_tokens
            } else {
                0
            };
            lines.push(format!(
                "cache    · read {} ({}% of in) · write {}",
                format_tokens(self.session_usage.cache_read_tokens),
                pct,
                format_tokens(self.session_usage.cache_write_tokens)
            ));
        }
        lines.push(format!(
            "context  · ~{} / {} ({}%)",
            format_tokens(ctx as u64),
            format_tokens(max as u64),
            pct
        ));
        lines
    }
}
