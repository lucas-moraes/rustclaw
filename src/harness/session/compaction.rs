//! Context window compaction: keeps the session under a token budget by
//! summarizing older messages with the LLM.
//!
//! [`should_compact_and_execute`] decides whether a message list exceeds the
//! configured budget (and minimum size) and, if so, produces a new message list
//! with the older messages collapsed into a single summary message. The result
//! is returned as `Option<Vec<Message>>`; `None` means no compaction was needed.

use crate::harness::event::{EventSender, HarnessEvent};
use crate::harness::provider::{LlmRequest, Provider};
use crate::harness::session::store::SessionStore;
use crate::harness::session::{Message, Part, Role, Session};
use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;

/// How many recent messages to keep after compaction.
const KEEP_RECENT: usize = 6;
/// Minimum message count before compaction is worth attempting.
const MIN_MESSAGES: usize = 10;
/// Max time to wait for the LLM summary before falling back to a placeholder.
const SUMMARY_TIMEOUT: Duration = Duration::from_secs(120);
/// Max chained summaries retained (oldest folded away first). Bounds the
/// "summary of summaries" chain so it cannot grow without limit over a very
/// long session.
pub const MAX_SUMMARY_CHAIN: usize = 8;

/// Tunables for context-window compaction.
#[derive(Clone, Debug)]
pub struct CompactionConfig {
    pub max_context_tokens: usize,
    pub keep_recent_messages: usize,
    pub min_messages_to_compact: usize,
    /// Max time to wait for the LLM summary before falling back to a placeholder.
    pub summary_timeout: Duration,
    /// Fraction of `max_context_tokens` at which compaction triggers (proactive
    /// compaction). `0.7` means compact at 70% of the budget, so the context
    /// never hits the hard limit mid-turn. `1.0` restores the old reactive
    /// behavior (compact only on overflow).
    pub trigger_ratio: f64,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: 80_000,
            keep_recent_messages: 6,
            min_messages_to_compact: 10,
            summary_timeout: Duration::from_secs(120),
            trigger_ratio: DEFAULT_TRIGGER_RATIO,
        }
    }
}

/// Default proactive trigger: compact at 70% of the budget.
pub const DEFAULT_TRIGGER_RATIO: f64 = 0.7;

/// How many new messages must accumulate before the tracker re-checks the
/// context size. Avoids an O(n) `approx_tokens` scan on every single loop
/// iteration (the processor used to call `compact_if_needed` per iteration).
const COMPACT_CHECK_EVERY: usize = 4;
/// Token growth (since the last check) that forces a re-check even when fewer
/// than `COMPACT_CHECK_EVERY` messages were added — e.g. one huge tool output.
const COMPACT_TOKEN_DELTA: usize = 8_000;

/// Decides *when* to re-evaluate compaction, so the caller can skip the
/// (O(n)) token scan on most iterations. Owned by the processor and reset per
/// turn. This is purely an optimization: the actual decision still lives in
/// `should_compact_and_execute` / `compact_if_needed`.
#[derive(Debug, Default)]
pub struct CompactionTracker {
    /// Message count at the last evaluation.
    last_len: usize,
    /// Whether we have ever evaluated (the first call must always run).
    initialized: bool,
}

impl CompactionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when the context has grown enough since the last check to
    /// justify another (O(n)) evaluation. The caller then runs the real
    /// compaction and calls [`Self::record`] with the resulting state.
    ///
    /// The token-delta path only scans the *new* messages (O(new)), never the
    /// whole list, so a turn that grows by a few messages per iteration does
    /// not pay an O(n) scan every tick.
    pub fn should_check(&self, messages: &[Message]) -> bool {
        if !self.initialized {
            return true;
        }
        if messages.len() >= self.last_len + COMPACT_CHECK_EVERY {
            return true;
        }
        // Even without many new messages, a large token jump (big tool output)
        // warrants a check. Only pay the (small) scan when the list grew.
        if messages.len() > self.last_len {
            let new_tokens = crate::harness::session::approx_tokens(&messages[self.last_len..]);
            return new_tokens >= COMPACT_TOKEN_DELTA;
        }
        false
    }

    /// Records the post-evaluation state. Call after running compaction
    /// (whether or not it actually compacted) so the next `should_check` is
    /// relative to the current context.
    pub fn record(&mut self, messages: &[Message]) {
        self.last_len = messages.len();
        self.initialized = true;
    }
}

/// Compacts `messages` when the approximate token count exceeds the configured
/// budget (scaled by `trigger_ratio`) and there are enough messages to bother
/// summarizing.
///
/// Returns `Ok(None)` when no compaction is needed, or `Ok(Some(new_messages))`
/// where `new_messages` is the replacement list: a single summary message at the
/// front followed by the `keep_recent_messages` most recent messages.
///
/// `ledger` (when provided and non-empty) is rendered as a structured block and
/// prepended to the summary, so durable facts (files, commands, decisions)
/// survive the lossy LLM summary.
///
/// `prior_summaries` is the session's existing summary chain (oldest first).
/// When non-empty, the new summary is produced as a **summary of summaries**:
/// the previous summaries are folded into the new one instead of being dropped,
/// so decisions from the start of a long session survive repeated compactions.
/// The returned chain (via [`CompactionOutcome`]) is bounded by
/// [`MAX_SUMMARY_CHAIN`].
pub async fn should_compact_and_execute(
    messages: &[Message],
    provider: Arc<dyn Provider>,
    config: &CompactionConfig,
    model: &str,
    ledger: Option<&crate::harness::session::ledger::ContextLedger>,
    prior_summaries: &[String],
) -> Result<Option<CompactionOutcome>> {
    if messages.len() < config.min_messages_to_compact {
        return Ok(None);
    }
    // Proactive trigger: compact at `trigger_ratio` of the budget (default 70%)
    // so the context never reaches the hard limit in the middle of a turn.
    let budget = ((config.max_context_tokens as f64) * config.trigger_ratio) as usize;
    if crate::harness::session::approx_tokens(messages) <= budget {
        return Ok(None);
    }

    let keep = config.keep_recent_messages.min(messages.len());
    let cut = messages.len() - keep;
    let dropped: &[Message] = &messages[..cut];
    let recent: &[Message] = &messages[cut..];

    // Chained summary: fold the previous summaries into the new one so the
    // chain is a "summary of summaries" rather than a single lossy snapshot.
    let summary = summarize(
        dropped,
        prior_summaries,
        provider,
        config.summary_timeout,
        model,
    )
    .await?;
    let ledger_block = ledger.and_then(|l| l.render());
    let body = match ledger_block {
        Some(block) => format!("{block}\n{summary}"),
        None => summary.clone(),
    };
    let summary_message = Message::new(
        Role::User,
        vec![Part::text(format!(
            "[Context compacted] Summary of {} earlier messages:\n{}",
            dropped.len(),
            body
        ))],
    );

    let mut new_messages = Vec::with_capacity(recent.len() + 1);
    new_messages.push(summary_message);
    new_messages.extend_from_slice(recent);

    // Extend the chain with the freshly produced summary, then bound it.
    let mut chain: Vec<String> = prior_summaries.to_vec();
    chain.push(summary);
    if chain.len() > MAX_SUMMARY_CHAIN {
        let excess = chain.len() - MAX_SUMMARY_CHAIN;
        chain.drain(0..excess);
    }

    Ok(Some(CompactionOutcome {
        messages: new_messages,
        summary_chain: chain,
    }))
}

/// Result of a successful compaction: the replacement message list plus the
/// updated (bounded) summary chain to store on the session.
#[derive(Clone, Debug)]
pub struct CompactionOutcome {
    pub messages: Vec<Message>,
    pub summary_chain: Vec<String>,
}

/// Runs compaction on a session in place: decides whether the message list
/// exceeds the budget, summarizes older messages, persists the result, and
/// emits `CompactionStarted`/`CompactionFinished` events.
///
/// This is the single entry point used by both the runtime (session open,
/// `/compact` force) and the processor (per-iteration overflow check), so the
/// algorithm and its constants live in exactly one place.
///
/// - `force = true` treats the token budget as zero (any session with enough
///   messages is summarized) and lowers the minimum to 2 (summary + keep).
/// - `events` is optional: when `None`, no events are emitted.
///
/// Returns the number of messages summarized away (`0` when nothing changed).
pub async fn compact_if_needed(
    session: &mut Session,
    provider: Arc<dyn Provider>,
    store: &Arc<SessionStore>,
    max_context_tokens: usize,
    force: bool,
    events: Option<&EventSender>,
    model: &str,
) -> Result<usize> {
    let config = CompactionConfig {
        max_context_tokens: if force { 0 } else { max_context_tokens },
        keep_recent_messages: KEEP_RECENT,
        // Force still needs at least 2 messages (summary target + keep).
        min_messages_to_compact: if force { 2 } else { MIN_MESSAGES },
        summary_timeout: SUMMARY_TIMEOUT,
        // Force ignores the ratio (budget is already 0); otherwise compact
        // proactively at 70% of the budget.
        trigger_ratio: if force { 1.0 } else { DEFAULT_TRIGGER_RATIO },
    };

    // Decide first: events must only fire when a compaction actually runs,
    // otherwise the TUI would show "[compacting context…]" on every turn tick.
    let Some(outcome) = should_compact_and_execute(
        &session.messages,
        provider,
        &config,
        model,
        Some(&session.ledger),
        &session.summary_chain,
    )
    .await?
    else {
        return Ok(0);
    };

    if let Some(tx) = events {
        let _ = tx.send(HarnessEvent::CompactionStarted {
            session_id: session.id.clone(),
            parent_session_id: None,
        });
    }

    let before = session.messages.len();
    // The summary message adds one to the new list, so the number of
    // messages summarized away is before - new.len() + 1.
    let n = before.saturating_sub(outcome.messages.len()) + 1;
    session.messages = outcome.messages;
    session.summary_chain = outcome.summary_chain;
    session.invalidate_messages_cache();
    session.updated_at = chrono::Utc::now();
    // Persist immediately so orphaned pre-summary messages are dropped
    // from SQLite even if the turn aborts later.
    let snapshot = session.clone();
    let store_owned = store.clone();
    tokio::task::spawn_blocking(move || store_owned.save_session(&snapshot))
        .await
        .map_err(|e| anyhow::anyhow!("join error: {e}"))?
        .context("failed to persist compacted session")?;

    if let Some(tx) = events {
        let _ = tx.send(HarnessEvent::CompactionFinished {
            session_id: session.id.clone(),
            summarized_messages: n,
            parent_session_id: None,
        });
    }
    Ok(n)
}

/// Requests an LLM summary of the dropped messages, falling back to a plain
/// placeholder when the provider fails, times out, or returns no text.
///
/// When `prior_summaries` is non-empty, the request is a **summary of
/// summaries**: the previous summaries are included as context and the model is
/// asked to fold them into the new summary, so the chain preserves decisions
/// from earlier in the session instead of dropping them.
async fn summarize(
    dropped: &[Message],
    prior_summaries: &[String],
    provider: Arc<dyn Provider>,
    timeout: Duration,
    model: &str,
) -> Result<String> {
    let transcript = build_summary_request(dropped)
        .into_iter()
        .map(|(role, text)| format!("{}: {}", role, text))
        .collect::<Vec<_>>()
        .join("\n");

    let (system, user) = if prior_summaries.is_empty() {
        (
            "Summarize the following agent conversation in under 500 words, \
             preserving key decisions, file paths, and outcomes."
                .to_string(),
            transcript,
        )
    } else {
        // Fold the previous summaries into the new one (summary of summaries).
        let prior = prior_summaries.join("\n---\n");
        (
            "You are maintaining a rolling summary of a long agent session. \
             You are given the PREVIOUS SUMMARY (older context) and a NEW \
             TRANSCRIPT (recent messages). Produce a single updated summary in \
             under 500 words that folds the previous summary into the new one, \
             preserving key decisions, file paths, and outcomes from BOTH. Do \
             not lose facts from the previous summary."
                .to_string(),
            format!("PREVIOUS SUMMARY:\n{prior}\n\nNEW TRANSCRIPT:\n{transcript}"),
        )
    };

    let summary_req = LlmRequest {
        model: model.to_string(),
        system,
        messages: std::sync::Arc::new(vec![Message::user(user)]),
        tools: vec![],
        max_tokens: None,
        temperature: 0.2,
    };
    // Timeout so a slow/hung provider never blocks the turn during compaction.
    // Retry transient errors (429/5xx) with backoff before falling back.
    let retry_policy = crate::harness::provider::retry::RetryPolicy::default();
    let provider2 = provider.clone();
    let summary_req2 = summary_req.clone();
    let complete_fut = crate::harness::provider::retry::retry_with_policy(
        &retry_policy,
        || {
            let provider = provider2.clone();
            let req = summary_req2.clone();
            async move { provider.complete(&req).await }
        },
        || false,
    );
    match tokio::time::timeout(timeout, complete_fut).await {
        Ok(Ok(resp)) => {
            let text = resp
                .parts
                .iter()
                .filter_map(|p| p.as_text())
                .collect::<Vec<_>>()
                .join("\n");
            if text.trim().is_empty() {
                tracing::warn!("compaction summary returned empty text");
                Ok("(summary unavailable)".to_string())
            } else {
                Ok(text)
            }
        }
        Ok(Err(e)) => {
            tracing::warn!("compaction summary failed, falling back to trim: {}", e);
            Ok("(summary unavailable)".to_string())
        }
        Err(_) => {
            tracing::warn!("compaction summary timed out after {:?}", timeout);
            Ok("(summary unavailable)".to_string())
        }
    }
}

/// Builds the text sent to the LLM to produce a summary of the dropped messages.
pub fn build_summary_request(dropped: &[Message]) -> Vec<(String, String)> {
    dropped
        .iter()
        .map(|m| (m.role.as_str().to_string(), render_message(m)))
        .collect()
}

pub fn render_message(m: &Message) -> String {
    let mut out = format!("[{}]", m.role.as_str());
    for part in &m.parts {
        match part {
            Part::Text { text } => {
                out.push(' ');
                out.push_str(text);
            }
            Part::Reasoning { .. } => {}
            Part::Image { .. } => {}
            Part::Tool(t) => {
                out.push_str(&format!(
                    " (tool {} status={} output={})",
                    t.name,
                    t.status,
                    crate::harness::session::preview(&t.output, 300)
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::provider::{ProviderStream, Usage};
    use futures_util::StreamExt;

    /// Test double returning a fixed summary (or error) for `complete`.
    struct MockProvider {
        summary: Result<String, String>,
    }

    impl MockProvider {
        fn ok(text: &str) -> Self {
            Self {
                summary: Ok(text.to_string()),
            }
        }
        fn failing() -> Self {
            Self {
                summary: Err("boom".to_string()),
            }
        }
    }

    #[async_trait::async_trait]
    impl Provider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }
        async fn stream(&self, _req: &LlmRequest) -> anyhow::Result<ProviderStream> {
            Ok(futures_util::stream::empty().boxed())
        }
        async fn complete(
            &self,
            _req: &LlmRequest,
        ) -> anyhow::Result<crate::harness::provider::LlmResponse> {
            match &self.summary {
                Ok(text) => Ok(crate::harness::provider::LlmResponse {
                    parts: vec![Part::text(text)],
                    usage: Some(Usage::default()),
                    stop_reason: None,
                }),
                Err(e) => Err(anyhow::anyhow!(e.clone())),
            }
        }
    }

    /// Test double whose `complete` never returns (simulates a hung provider).
    struct HangingProvider;

    #[async_trait::async_trait]
    impl Provider for HangingProvider {
        fn name(&self) -> &str {
            "hanging"
        }
        async fn stream(&self, _req: &LlmRequest) -> anyhow::Result<ProviderStream> {
            Ok(futures_util::stream::empty().boxed())
        }
        async fn complete(
            &self,
            _req: &LlmRequest,
        ) -> anyhow::Result<crate::harness::provider::LlmResponse> {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            unreachable!()
        }
    }

    fn msgs(n: usize) -> Vec<Message> {
        // 100-char messages => ~25 tokens each, enough to exceed small budgets.
        (0..n)
            .map(|i| Message::user(format!("message {} {}", i, "x".repeat(90))))
            .collect()
    }

    fn cfg(max: usize, keep: usize, min: usize) -> CompactionConfig {
        CompactionConfig {
            max_context_tokens: max,
            keep_recent_messages: keep,
            min_messages_to_compact: min,
            summary_timeout: Duration::from_secs(120),
            // Tests use a 1.0 ratio so the budget is the literal `max` value.
            trigger_ratio: 1.0,
        }
    }

    fn cfg_with_timeout(
        max: usize,
        keep: usize,
        min: usize,
        timeout: Duration,
    ) -> CompactionConfig {
        CompactionConfig {
            max_context_tokens: max,
            keep_recent_messages: keep,
            min_messages_to_compact: min,
            summary_timeout: timeout,
            trigger_ratio: 1.0,
        }
    }

    #[test]
    fn test_default_config_values() {
        let c = CompactionConfig::default();
        assert_eq!(c.max_context_tokens, 80_000);
        assert_eq!(c.keep_recent_messages, 6);
        assert_eq!(c.min_messages_to_compact, 10);
        assert_eq!(c.trigger_ratio, DEFAULT_TRIGGER_RATIO);
        assert_eq!(DEFAULT_TRIGGER_RATIO, 0.7);
    }

    #[tokio::test]
    async fn test_proactive_trigger_at_ratio() {
        // 12 messages ≈ 300 tokens. With a 1000-token budget and a 0.7 ratio,
        // the effective trigger is 700 tokens → no compaction yet.
        let provider = Arc::new(MockProvider::ok("summary"));
        let mut c = cfg(1_000, 6, 10);
        c.trigger_ratio = 0.7;
        let out =
            should_compact_and_execute(&msgs(12), provider.clone(), &c, "grok-4.5", None, &[])
                .await
                .unwrap();
        assert!(out.is_none(), "below 70% of budget must not compact");

        // A 300-token budget → trigger at 210 tokens → compaction fires.
        let mut c2 = cfg(300, 6, 10);
        c2.trigger_ratio = 0.7;
        let out = should_compact_and_execute(&msgs(12), provider, &c2, "grok-4.5", None, &[])
            .await
            .unwrap();
        assert!(out.is_some(), "above 70% of budget must compact");
    }

    #[tokio::test]
    async fn test_ledger_injected_into_summary() {
        use crate::harness::session::ledger::{ContextLedger, FileOp};
        let provider = Arc::new(MockProvider::ok("the summary"));
        let mut ledger = ContextLedger::new();
        ledger.touch_file("src/main.rs", FileOp::Write);
        ledger.record_command("cargo test");
        ledger.record_decision("chose BTreeMap");

        let out = should_compact_and_execute(
            &msgs(12),
            provider,
            &cfg(1, 6, 10),
            "grok-4.5",
            Some(&ledger),
            &[],
        )
        .await
        .unwrap()
        .expect("expected compaction");
        let head = out.messages[0].text_content();
        assert!(head.contains("[Session ledger"), "ledger block missing");
        assert!(head.contains("src/main.rs (w)"));
        assert!(head.contains("`cargo test`"));
        assert!(head.contains("chose BTreeMap"));
        // The LLM summary is still present after the ledger block.
        assert!(head.contains("the summary"));
    }

    #[tokio::test]
    async fn test_empty_ledger_not_injected() {
        use crate::harness::session::ledger::ContextLedger;
        let provider = Arc::new(MockProvider::ok("the summary"));
        let ledger = ContextLedger::new();
        let out = should_compact_and_execute(
            &msgs(12),
            provider,
            &cfg(1, 6, 10),
            "grok-4.5",
            Some(&ledger),
            &[],
        )
        .await
        .unwrap()
        .expect("expected compaction");
        assert!(!out.messages[0].text_content().contains("[Session ledger"));
    }

    #[tokio::test]
    async fn test_first_compaction_seeds_summary_chain() {
        let provider = Arc::new(MockProvider::ok("first summary"));
        let out =
            should_compact_and_execute(&msgs(12), provider, &cfg(1, 6, 10), "grok-4.5", None, &[])
                .await
                .unwrap()
                .expect("expected compaction");
        // The chain starts with the freshly produced summary.
        assert_eq!(out.summary_chain, vec!["first summary".to_string()]);
    }

    #[tokio::test]
    async fn test_chain_folds_prior_summaries_into_request() {
        // A provider that echoes the user prompt lets us assert the previous
        // summary was folded into the request (summary of summaries).
        struct EchoProvider;
        #[async_trait::async_trait]
        impl Provider for EchoProvider {
            fn name(&self) -> &str {
                "echo"
            }
            async fn stream(&self, _req: &LlmRequest) -> anyhow::Result<ProviderStream> {
                Ok(futures_util::stream::empty().boxed())
            }
            async fn complete(
                &self,
                req: &LlmRequest,
            ) -> anyhow::Result<crate::harness::provider::LlmResponse> {
                let user = req.messages[0].text_content();
                Ok(crate::harness::provider::LlmResponse {
                    parts: vec![Part::text(user)],
                    usage: Some(Usage::default()),
                    stop_reason: None,
                })
            }
        }

        let prior = vec!["OLD DECISION: use BTreeMap".to_string()];
        let out = should_compact_and_execute(
            &msgs(12),
            Arc::new(EchoProvider),
            &cfg(1, 6, 10),
            "grok-4.5",
            None,
            &prior,
        )
        .await
        .unwrap()
        .expect("expected compaction");

        // The new summary (echoed request) contains the previous summary.
        let new_summary = out.summary_chain.last().unwrap();
        assert!(
            new_summary.contains("OLD DECISION: use BTreeMap"),
            "prior summary must be folded into the new one: {new_summary}"
        );
        assert!(new_summary.contains("PREVIOUS SUMMARY"));
        assert!(new_summary.contains("NEW TRANSCRIPT"));
        // Chain = prior + new.
        assert_eq!(out.summary_chain.len(), 2);
        assert_eq!(out.summary_chain[0], "OLD DECISION: use BTreeMap");
    }

    #[tokio::test]
    async fn test_chain_is_bounded_by_max() {
        let provider = Arc::new(MockProvider::ok("newest"));
        // Seed a chain already at the cap.
        let prior: Vec<String> = (0..MAX_SUMMARY_CHAIN)
            .map(|i| format!("summary {i}"))
            .collect();
        let out = should_compact_and_execute(
            &msgs(12),
            provider,
            &cfg(1, 6, 10),
            "grok-4.5",
            None,
            &prior,
        )
        .await
        .unwrap()
        .expect("expected compaction");
        assert_eq!(out.summary_chain.len(), MAX_SUMMARY_CHAIN);
        // Oldest dropped, newest appended.
        assert_eq!(out.summary_chain[0], "summary 1");
        assert_eq!(out.summary_chain.last().unwrap(), "newest");
    }

    #[tokio::test]
    async fn test_chain_survives_repeated_compactions() {
        // Regression: after N compactions the chain must still carry the very
        // first summary (until the cap), not just the most recent one.
        let provider = Arc::new(MockProvider::ok("s"));
        let mut chain: Vec<String> = Vec::new();
        for _ in 0..3 {
            let out = should_compact_and_execute(
                &msgs(12),
                provider.clone(),
                &cfg(1, 6, 10),
                "grok-4.5",
                None,
                &chain,
            )
            .await
            .unwrap()
            .expect("expected compaction");
            chain = out.summary_chain;
        }
        assert_eq!(chain.len(), 3, "one summary per compaction");
    }

    #[test]
    fn test_tracker_first_check_always_runs() {
        let t = CompactionTracker::new();
        assert!(t.should_check(&msgs(3)), "first check must always run");
    }

    #[test]
    fn test_tracker_skips_until_enough_new_messages() {
        let mut t = CompactionTracker::new();
        let mut messages = msgs(10);
        assert!(t.should_check(&messages));
        t.record(&messages);

        // Fewer than COMPACT_CHECK_EVERY new messages → skip.
        messages.push(Message::user("one"));
        assert!(!t.should_check(&messages));
        messages.push(Message::user("two"));
        assert!(!t.should_check(&messages));
        messages.push(Message::user("three"));
        assert!(!t.should_check(&messages));

        // Reaching the threshold → check again.
        messages.push(Message::user("four"));
        assert!(t.should_check(&messages));
    }

    #[test]
    fn test_tracker_checks_on_large_token_jump() {
        let mut t = CompactionTracker::new();
        let mut messages = msgs(10);
        t.record(&messages);

        // A single huge message (well over COMPACT_TOKEN_DELTA tokens) forces
        // a re-check even though only one message was added.
        messages.push(Message::user("x".repeat(COMPACT_TOKEN_DELTA * 4 + 100)));
        assert!(t.should_check(&messages));
    }

    #[test]
    fn test_tracker_token_delta_is_incremental() {
        // The delta is measured over the *new* messages only, so a small
        // addition to a large context does not trigger a re-check (and does
        // not pay an O(n) scan of the whole list).
        let mut t = CompactionTracker::new();
        let mut messages = msgs(50); // large baseline
        t.record(&messages);

        // One small message: below both the count threshold and the delta.
        messages.push(Message::user("tiny"));
        assert!(
            !t.should_check(&messages),
            "a small addition must not trigger a re-check"
        );
    }

    #[test]
    fn test_tracker_no_check_when_nothing_changed() {
        let mut t = CompactionTracker::new();
        let messages = msgs(10);
        t.record(&messages);
        // Same list, no growth → no re-check.
        assert!(!t.should_check(&messages));
    }

    #[test]
    fn test_tracker_record_after_compaction_resets_baseline() {
        let mut t = CompactionTracker::new();
        let mut messages = msgs(20);
        t.record(&messages);
        // Simulate compaction shrinking the list.
        messages.truncate(7);
        t.record(&messages);
        // After recording the smaller list, a few new messages still skip.
        messages.push(Message::user("a"));
        assert!(!t.should_check(&messages));
    }

    #[tokio::test]
    async fn test_noop_below_min_messages() {
        let provider = Arc::new(MockProvider::ok("summary"));
        let out =
            should_compact_and_execute(&msgs(3), provider, &cfg(0, 6, 10), "grok-4.5", None, &[])
                .await
                .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn test_noop_under_token_limit() {
        let provider = Arc::new(MockProvider::ok("summary"));
        let out = should_compact_and_execute(
            &msgs(12),
            provider,
            &cfg(10_000, 6, 10),
            "grok-4.5",
            None,
            &[],
        )
        .await
        .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn test_summarizes_and_keeps_recent() {
        let provider = Arc::new(MockProvider::ok("this is the summary"));
        let messages = msgs(12);
        let out =
            should_compact_and_execute(&messages, provider, &cfg(1, 6, 10), "grok-4.5", None, &[])
                .await
                .unwrap()
                .expect("expected compaction");
        // Summary at front + 6 recent = 7 total.
        assert_eq!(out.messages.len(), 7);
        let first = &out.messages[0];
        assert_eq!(first.role, Role::User);
        assert!(first.text_content().contains("[Context compacted]"));
        assert!(first
            .text_content()
            .contains("Summary of 6 earlier messages"));
        assert!(first.text_content().contains("this is the summary"));
        // Recent messages preserved in order (original indices 6..12 -> 0..6).
        assert!(out.messages[6].text_content().contains("message 11"));
    }

    #[tokio::test]
    async fn test_summary_failure_falls_back() {
        let provider = Arc::new(MockProvider::failing());
        let messages = msgs(12);
        let out =
            should_compact_and_execute(&messages, provider, &cfg(1, 6, 10), "grok-4.5", None, &[])
                .await
                .unwrap()
                .expect("expected compaction even on summary failure");
        assert_eq!(out.messages.len(), 7);
        assert!(out.messages[0]
            .text_content()
            .contains("(summary unavailable)"));
    }

    #[test]
    fn test_render_message_with_tool() {
        let msg = Message::new(
            Role::Assistant,
            vec![
                Part::text("checking"),
                Part::Tool(crate::harness::session::ToolPart {
                    id: "1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({}),
                    status: crate::harness::session::ToolStatus::Completed,
                    output: "ok".into(),
                    title: String::new(),
                    error: None,
                }),
            ],
        );
        let rendered = render_message(&msg);
        assert!(rendered.contains("[assistant] checking"));
        assert!(rendered.contains("tool bash"));
    }

    #[tokio::test]
    async fn test_summary_timeout_falls_back() {
        // A hung provider must not block the turn: compaction falls back to a
        // placeholder after the configured timeout.
        let provider = Arc::new(HangingProvider);
        let messages = msgs(12);
        let out = should_compact_and_execute(
            &messages,
            provider,
            &cfg_with_timeout(1, 6, 10, Duration::from_millis(50)),
            "grok-4.5",
            None,
            &[],
        )
        .await
        .unwrap()
        .expect("expected compaction even on summary timeout");
        assert_eq!(out.messages.len(), 7);
        assert!(out.messages[0]
            .text_content()
            .contains("(summary unavailable)"));
    }

    #[tokio::test]
    async fn test_compact_if_needed_persists_and_emits_events() {
        use crate::harness::event::event_channel;
        use crate::harness::session::store::SessionStore;

        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(SessionStore::open(&dir.path().join("test.db")).unwrap());
        let mut session = store.create_session("build", dir.path()).unwrap();
        // 12 messages exceed the tiny budget (1 token) and the min (10).
        session.messages = msgs(12);

        let provider = Arc::new(MockProvider::ok("this is the summary"));
        let (tx, mut rx) = event_channel();

        let summarized = compact_if_needed(
            &mut session,
            provider,
            &store,
            1, // max_context_tokens: tiny so compaction triggers
            false,
            Some(&tx),
            "grok-4.5",
        )
        .await
        .unwrap();

        // 12 -> summary + 6 recent = 7; summarized = 12 - 7 + 1 = 6.
        assert_eq!(summarized, 6);
        assert_eq!(session.messages.len(), 7);
        assert!(session.messages[0]
            .text_content()
            .contains("[Context compacted]"));

        // Persisted: reloading from the store reflects the compacted list.
        let reloaded = store
            .load_session(&session.id, dir.path())
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.messages.len(), 7);

        // Events: CompactionStarted then CompactionFinished(6).
        let mut started = false;
        let mut finished = 0usize;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                HarnessEvent::CompactionStarted { .. } => started = true,
                HarnessEvent::CompactionFinished {
                    summarized_messages,
                    ..
                } => finished = summarized_messages,
                _ => {}
            }
        }
        assert!(started, "expected CompactionStarted");
        assert_eq!(finished, 6, "expected CompactionFinished with 6 summarized");
    }

    #[tokio::test]
    async fn test_compact_if_needed_noop_returns_zero() {
        use crate::harness::session::store::SessionStore;

        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(SessionStore::open(&dir.path().join("test.db")).unwrap());
        let mut session = store.create_session("build", dir.path()).unwrap();
        // 3 messages: below the min (10) -> no compaction.
        session.messages = msgs(3);

        let provider = Arc::new(MockProvider::ok("summary"));
        let summarized =
            compact_if_needed(&mut session, provider, &store, 1, false, None, "grok-4.5")
                .await
                .unwrap();
        assert_eq!(summarized, 0);
        assert_eq!(session.messages.len(), 3);
    }

    #[tokio::test]
    async fn test_compact_if_needed_emits_no_events_when_noop() {
        use crate::harness::event::event_channel;
        use crate::harness::session::store::SessionStore;

        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(SessionStore::open(&dir.path().join("test.db")).unwrap());
        let mut session = store.create_session("build", dir.path()).unwrap();
        session.messages = msgs(3);

        let provider = Arc::new(MockProvider::ok("summary"));
        let (tx, mut rx) = event_channel();

        let summarized = compact_if_needed(
            &mut session,
            provider,
            &store,
            1,
            false,
            Some(&tx),
            "grok-4.5",
        )
        .await
        .unwrap();

        assert_eq!(summarized, 0);
        // No compaction -> no CompactionStarted/Finished spam on the TUI.
        assert!(
            rx.try_recv().is_err(),
            "no events should be emitted when nothing was compacted"
        );
    }
}
