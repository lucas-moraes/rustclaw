//! A deterministic, scripted [`Provider`] for tests and evals.
//!
//! Real evals need to run the *whole* loop (streaming, tool execution,
//! compaction, metrics) without a network or a live token. `ScriptedProvider`
//! replays a fixed list of turns: each call to [`Provider::stream`] pops the
//! next scripted turn and emits it as a normal [`ProviderEvent`] stream, so the
//! processor cannot tell it apart from a real adapter.
//!
//! A turn is either plain text, a set of tool calls, or both. When the script
//! is exhausted the provider keeps returning a terminal text turn (so a buggy
//! loop that never stops still terminates instead of hanging the test).
//!
//! This lives in the library (not `#[cfg(test)]`) so integration-style evals
//! and the `tests/` directory can reuse it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use futures_util::stream;
use futures_util::StreamExt;

use crate::harness::provider::{
    LlmRequest, LlmResponse, Provider, ProviderEvent, ProviderStream, Usage,
};
use crate::harness::session::Part;

/// One scripted assistant turn.
#[derive(Clone, Debug, Default)]
pub struct ScriptedTurn {
    /// Assistant text emitted as `TextDelta` events.
    pub text: String,
    /// Tool calls emitted as `ToolCallStart` + `ToolCallEnd` pairs.
    pub tool_calls: Vec<ScriptedToolCall>,
    /// Stop reason reported in the terminal `End` event.
    pub stop_reason: Option<String>,
    /// Token usage reported in the terminal `End` event.
    pub usage: Usage,
}

impl ScriptedTurn {
    /// A plain text turn (no tool calls).
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            stop_reason: Some("end_turn".into()),
            ..Default::default()
        }
    }

    /// A turn that calls a single tool with the given JSON arguments.
    pub fn tool(name: impl Into<String>, args: serde_json::Value) -> Self {
        let name = name.into();
        Self {
            text: String::new(),
            tool_calls: vec![ScriptedToolCall {
                id: format!("call_{name}"),
                name,
                arguments: args,
            }],
            stop_reason: Some("tool_use".into()),
            ..Default::default()
        }
    }

    /// A turn that calls several tools at once (parallel tool calling).
    pub fn tools(calls: Vec<ScriptedToolCall>) -> Self {
        Self {
            text: String::new(),
            tool_calls: calls,
            stop_reason: Some("tool_use".into()),
            ..Default::default()
        }
    }

    /// Sets the token usage reported for this turn.
    pub fn with_usage(mut self, input: u64, output: u64) -> Self {
        self.usage = Usage {
            input_tokens: input,
            output_tokens: output,
            ..Default::default()
        };
        self
    }
}

/// A single scripted tool call.
#[derive(Clone, Debug)]
pub struct ScriptedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// A provider that replays a fixed script of turns.
pub struct ScriptedProvider {
    turns: Vec<ScriptedTurn>,
    cursor: AtomicUsize,
    /// Every request the provider received, for assertions (e.g. "the model
    /// saw the tool result"). Stored as the last user/tool message text.
    seen: Mutex<Vec<String>>,
}

impl ScriptedProvider {
    /// Builds a provider that replays `turns` in order.
    pub fn new(turns: Vec<ScriptedTurn>) -> Self {
        Self {
            turns,
            cursor: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// Number of requests served so far (i.e. loop iterations observed).
    pub fn calls(&self) -> usize {
        self.cursor.load(Ordering::SeqCst)
    }

    /// The text of every request the provider received, in order.
    #[allow(dead_code)] // part of the eval API; used by tests and future evals
    pub fn seen_requests(&self) -> Vec<String> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Whether any request contained `needle` (e.g. a tool result).
    #[allow(dead_code)] // part of the eval API; used by tests and future evals
    pub fn saw(&self, needle: &str) -> bool {
        self.seen_requests().iter().any(|r| r.contains(needle))
    }

    fn next_turn(&self) -> ScriptedTurn {
        let i = self.cursor.fetch_add(1, Ordering::SeqCst);
        self.turns.get(i).cloned().unwrap_or_else(|| {
            // Script exhausted: emit a terminal turn so a runaway loop stops.
            ScriptedTurn::text("[scripted provider: script exhausted]")
        })
    }
}

#[async_trait::async_trait]
impl Provider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn stream(&self, req: &LlmRequest) -> anyhow::Result<ProviderStream> {
        // Record the last message's text so tests can assert what the model saw.
        if let Some(last) = req.messages.last() {
            self.seen
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(last.text_content());
        }
        let turn = self.next_turn();
        let mut events: Vec<Result<ProviderEvent, anyhow::Error>> = Vec::new();
        if !turn.text.is_empty() {
            events.push(Ok(ProviderEvent::TextDelta(turn.text.clone())));
        }
        for call in &turn.tool_calls {
            events.push(Ok(ProviderEvent::ToolCallStart {
                id: call.id.clone(),
                name: call.name.clone(),
            }));
            events.push(Ok(ProviderEvent::ToolCallEnd {
                id: call.id.clone(),
                arguments: call.arguments.to_string(),
            }));
        }
        events.push(Ok(ProviderEvent::End {
            stop_reason: turn.stop_reason.clone(),
            usage: Some(turn.usage),
        }));
        Ok(stream::iter(events).boxed())
    }

    async fn complete(&self, _req: &LlmRequest) -> anyhow::Result<LlmResponse> {
        let turn = self.next_turn();
        let mut parts = Vec::new();
        if !turn.text.is_empty() {
            parts.push(Part::text(turn.text));
        }
        for call in &turn.tool_calls {
            parts.push(Part::Tool(crate::harness::session::ToolPart::pending(
                call.id.clone(),
                call.name.clone(),
                call.arguments.clone(),
            )));
        }
        Ok(LlmResponse {
            parts,
            usage: Some(turn.usage),
            stop_reason: turn.stop_reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::provider::ProviderEvent;
    use futures_util::StreamExt;

    fn req() -> LlmRequest {
        LlmRequest {
            model: "test".into(),
            system: String::new(),
            messages: std::sync::Arc::new(vec![crate::harness::session::Message::user("hi")]),
            tools: Vec::new(),
            max_tokens: None,
            temperature: 0.0,
        }
    }

    #[tokio::test]
    async fn test_replays_turns_in_order() {
        let p = ScriptedProvider::new(vec![
            ScriptedTurn::text("first"),
            ScriptedTurn::text("second"),
        ]);
        let mut s1 = p.stream(&req()).await.unwrap();
        let mut got = Vec::new();
        while let Some(ev) = s1.next().await {
            if let Ok(ProviderEvent::TextDelta(d)) = ev {
                got.push(d);
            }
        }
        assert_eq!(got, vec!["first"]);
        assert_eq!(p.calls(), 1);

        let mut s2 = p.stream(&req()).await.unwrap();
        let mut got2 = Vec::new();
        while let Some(ev) = s2.next().await {
            if let Ok(ProviderEvent::TextDelta(d)) = ev {
                got2.push(d);
            }
        }
        assert_eq!(got2, vec!["second"]);
        assert_eq!(p.calls(), 2);
    }

    #[tokio::test]
    async fn test_tool_turn_emits_start_and_end() {
        let p = ScriptedProvider::new(vec![ScriptedTurn::tool(
            "bash",
            serde_json::json!({"command": "ls"}),
        )]);
        let mut s = p.stream(&req()).await.unwrap();
        let mut starts = 0;
        let mut ends = 0;
        while let Some(ev) = s.next().await {
            match ev.unwrap() {
                ProviderEvent::ToolCallStart { .. } => starts += 1,
                ProviderEvent::ToolCallEnd { arguments, .. } => {
                    ends += 1;
                    assert!(arguments.contains("ls"));
                }
                _ => {}
            }
        }
        assert_eq!(starts, 1);
        assert_eq!(ends, 1);
    }

    #[tokio::test]
    async fn test_exhausted_script_terminates() {
        let p = ScriptedProvider::new(vec![ScriptedTurn::text("only")]);
        let _ = p.stream(&req()).await.unwrap();
        // Second call falls back to a terminal turn instead of panicking.
        let mut s = p.stream(&req()).await.unwrap();
        let mut text = String::new();
        while let Some(ev) = s.next().await {
            if let Ok(ProviderEvent::TextDelta(d)) = ev {
                text.push_str(&d);
            }
        }
        assert!(text.contains("script exhausted"));
    }

    #[tokio::test]
    async fn test_records_seen_requests() {
        let p = ScriptedProvider::new(vec![ScriptedTurn::text("ok")]);
        let _ = p.stream(&req()).await.unwrap();
        assert!(p.saw("hi"));
        assert_eq!(p.seen_requests().len(), 1);
    }
}
