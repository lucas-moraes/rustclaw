//! Context window compaction: keeps the session under a token budget by
//! summarizing older messages with the LLM.
//!
//! [`should_compact_and_execute`] decides whether a message list exceeds the
//! configured budget (and minimum size) and, if so, produces a new message list
//! with the older messages collapsed into a single summary message. The result
//! is returned as `Option<Vec<Message>>`; `None` means no compaction was needed.

use crate::harness::provider::{LlmRequest, Provider};
use crate::harness::session::{Message, Part, Role};
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;

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

    let summary = summarize(dropped, provider, config.summary_timeout).await?;
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

/// Requests an LLM summary of the dropped messages, falling back to a plain
/// placeholder when the provider fails, times out, or returns no text.
async fn summarize(
    dropped: &[Message],
    provider: Arc<dyn Provider>,
    timeout: Duration,
) -> Result<String> {
    let transcript = build_summary_request(dropped)
        .into_iter()
        .map(|(role, text)| format!("{}: {}", role, text))
        .collect::<Vec<_>>()
        .join("\n");

    let summary_req = LlmRequest {
        model: String::new(),
        system: "Summarize the following agent conversation in under 500 words, \
preserving key decisions, file paths, and outcomes."
            .to_string(),
        messages: vec![Message::user(transcript)],
        tools: vec![],
        max_tokens: None,
        temperature: 0.2,
    };
    // Timeout so a slow/hung provider never blocks the turn during compaction.
    match tokio::time::timeout(timeout, provider.complete(&summary_req)).await {
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
        let out = should_compact_and_execute(&msgs(3), provider, &cfg(0, 6, 10))
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn test_noop_under_token_limit() {
        let provider = Arc::new(MockProvider::ok("summary"));
        let out = should_compact_and_execute(&msgs(12), provider, &cfg(10_000, 6, 10))
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn test_summarizes_and_keeps_recent() {
        let provider = Arc::new(MockProvider::ok("this is the summary"));
        let messages = msgs(12);
        let out = should_compact_and_execute(&messages, provider, &cfg(1, 6, 10))
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
        let out = should_compact_and_execute(&messages, provider, &cfg(1, 6, 10))
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
        )
        .await
        .unwrap()
        .expect("expected compaction even on summary timeout");
        assert_eq!(out.len(), 7);
        assert!(out[0].text_content().contains("(summary unavailable)"));
    }
}
