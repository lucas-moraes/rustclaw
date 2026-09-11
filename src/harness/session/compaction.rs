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

/// Tunables for context-window compaction.
#[derive(Clone, Debug)]
pub struct CompactionConfig {
    pub max_context_tokens: usize,
    pub keep_recent_messages: usize,
    pub min_messages_to_compact: usize,
    /// Max time to wait for the LLM summary before falling back to a placeholder.
    pub summary_timeout: Duration,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: 80_000,
            keep_recent_messages: 6,
            min_messages_to_compact: 10,
            summary_timeout: Duration::from_secs(120),
        }
    }
}

/// Compacts `messages` when the approximate token count exceeds the configured
/// budget and there are enough messages to bother summarizing.
///
/// Returns `Ok(None)` when no compaction is needed, or `Ok(Some(new_messages))`
/// where `new_messages` is the replacement list: a single summary message at the
/// front followed by the `keep_recent_messages` most recent messages.
pub async fn should_compact_and_execute(
    messages: &[Message],
    provider: Arc<dyn Provider>,
    config: &CompactionConfig,
    model: &str,
) -> Result<Option<Vec<Message>>> {
    if messages.len() < config.min_messages_to_compact {
        return Ok(None);
    }
    if crate::harness::session::approx_tokens(messages) <= config.max_context_tokens {
        return Ok(None);
    }

    let keep = config.keep_recent_messages.min(messages.len());
    let cut = messages.len() - keep;
    let dropped: &[Message] = &messages[..cut];
    let recent: &[Message] = &messages[cut..];

    let summary = summarize(dropped, provider, config.summary_timeout, model).await?;
    let summary_message = Message::new(
        Role::User,
        vec![Part::text(format!(
            "[Context compacted] Summary of {} earlier messages:\n{}",
            dropped.len(),
            summary
        ))],
    );

    let mut new_messages = Vec::with_capacity(recent.len() + 1);
    new_messages.push(summary_message);
    new_messages.extend_from_slice(recent);
    Ok(Some(new_messages))
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
    };

    // Decide first: events must only fire when a compaction actually runs,
    // otherwise the TUI would show "[compacting context…]" on every turn tick.
    let Some(new_messages) =
        should_compact_and_execute(&session.messages, provider, &config, model).await?
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
    let n = before.saturating_sub(new_messages.len()) + 1;
    session.messages = new_messages;
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
async fn summarize(
    dropped: &[Message],
    provider: Arc<dyn Provider>,
    timeout: Duration,
    model: &str,
) -> Result<String> {
    let transcript = build_summary_request(dropped)
        .into_iter()
        .map(|(role, text)| format!("{}: {}", role, text))
        .collect::<Vec<_>>()
        .join("\n");

    let summary_req = LlmRequest {
        model: model.to_string(),
        system: "Summarize the following agent conversation in under 500 words, \
preserving key decisions, file paths, and outcomes."
            .to_string(),
        messages: std::sync::Arc::new(vec![Message::user(transcript)]),
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
        }
    }

    #[test]
    fn test_default_config_values() {
        let c = CompactionConfig::default();
        assert_eq!(c.max_context_tokens, 80_000);
        assert_eq!(c.keep_recent_messages, 6);
        assert_eq!(c.min_messages_to_compact, 10);
    }

    #[tokio::test]
    async fn test_noop_below_min_messages() {
        let provider = Arc::new(MockProvider::ok("summary"));
        let out = should_compact_and_execute(&msgs(3), provider, &cfg(0, 6, 10), "grok-4.5")
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn test_noop_under_token_limit() {
        let provider = Arc::new(MockProvider::ok("summary"));
        let out = should_compact_and_execute(&msgs(12), provider, &cfg(10_000, 6, 10), "grok-4.5")
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn test_summarizes_and_keeps_recent() {
        let provider = Arc::new(MockProvider::ok("this is the summary"));
        let messages = msgs(12);
        let out = should_compact_and_execute(&messages, provider, &cfg(1, 6, 10), "grok-4.5")
            .await
            .unwrap()
            .expect("expected compaction");
        // Summary at front + 6 recent = 7 total.
        assert_eq!(out.len(), 7);
        let first = &out[0];
        assert_eq!(first.role, Role::User);
        assert!(first.text_content().contains("[Context compacted]"));
        assert!(first
            .text_content()
            .contains("Summary of 6 earlier messages"));
        assert!(first.text_content().contains("this is the summary"));
        // Recent messages preserved in order (original indices 6..12 -> 0..6).
        assert!(out[6].text_content().contains("message 11"));
    }

    #[tokio::test]
    async fn test_summary_failure_falls_back() {
        let provider = Arc::new(MockProvider::failing());
        let messages = msgs(12);
        let out = should_compact_and_execute(&messages, provider, &cfg(1, 6, 10), "grok-4.5")
            .await
            .unwrap()
            .expect("expected compaction even on summary failure");
        assert_eq!(out.len(), 7);
        assert!(out[0].text_content().contains("(summary unavailable)"));
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
        )
        .await
        .unwrap()
        .expect("expected compaction even on summary timeout");
        assert_eq!(out.len(), 7);
        assert!(out[0].text_content().contains("(summary unavailable)"));
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
