//! OpenAI-compatible provider: `/chat/completions` with native `tools`.

use super::{
    AuthStyle, HttpConfig, LlmRequest, LlmResponse, Provider, ProviderEvent, ProviderStream,
    SseParser, Usage,
};
use crate::harness::session::{Message, Part, Role, ToolPart};
use anyhow::{anyhow, Context as AnyhowContext};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::pin::Pin;

pub struct OpenAiProvider {
    pub http: HttpConfig,
    pub auth: AuthStyle,
}

/// Converts harness messages to OpenAI chat format.
/// Tool results are synthesized as `role:"tool"` messages after each assistant
/// message that contains tool calls.
pub fn to_openai_messages(system: &str, messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    if !system.trim().is_empty() {
        out.push(json!({"role": "system", "content": system}));
    }

    for msg in messages {
        match msg.role {
            Role::System => {
                // System is passed as the top-level `system`; skip in-session ones.
            }
            Role::User => {
                let text = msg.text_content();
                if !text.is_empty() {
                    out.push(json!({"role": "user", "content": text}));
                }
            }
            Role::Assistant => {
                let text = msg.text_content();
                // Only include tool calls that actually have a terminal result
                // (completed or error). Pending/running tool calls from an
                // interrupted session have no result and would otherwise be
                // replayed as "did not complete" errors, confusing the model.
                let terminal: Vec<&ToolPart> = msg
                    .tool_parts()
                    .into_iter()
                    .filter(|t| t.is_terminal())
                    .collect();
                let tool_calls: Vec<Value> = terminal
                    .iter()
                    .map(|t| {
                        json!({
                            "id": t.id,
                            "type": "function",
                            "function": {
                                "name": t.name,
                                "arguments": t.input.to_string(),
                            }
                        })
                    })
                    .collect();

                let mut m = json!({"role": "assistant"});
                if text.is_empty() && !tool_calls.is_empty() {
                    m["content"] = Value::Null;
                } else {
                    m["content"] = json!(text);
                }
                if !tool_calls.is_empty() {
                    m["tool_calls"] = json!(tool_calls);
                }
                out.push(m);

                for t in terminal {
                    out.push(tool_result_message(t));
                }
            }
        }
    }
    out
}

fn tool_result_message(t: &crate::harness::session::ToolPart) -> Value {
    let content = match t.status {
        crate::harness::session::ToolStatus::Completed => t.output.clone(),
        crate::harness::session::ToolStatus::Error => {
            format!("Error: {}", t.error.clone().unwrap_or_default())
        }
        _ => format!("Error: tool call {} did not complete", t.id),
    };
    json!({"role": "tool", "tool_call_id": t.id, "content": content})
}

fn tools_body(tools: &[super::ToolSpec]) -> Value {
    json!(tools
        .iter()
        .map(|t| json!({
            "type": "function",
            "function": {
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            }
        }))
        .collect::<Vec<_>>())
}

fn build_request_body(req: &LlmRequest, stream: bool) -> Value {
    let mut body = json!({
        "model": req.model,
        "messages": to_openai_messages(&req.system, &req.messages),
        "temperature": req.temperature,
        "stream": stream,
    });
    // Omit max_tokens so the provider/model applies its own default.
    if let Some(v) = req.max_tokens {
        body["max_tokens"] = json!(v);
    }
    if !req.tools.is_empty() {
        body["tools"] = tools_body(&req.tools);
    }
    body
}

/// Parses a non-streaming OpenAI response into parts.
pub fn parse_response(json: &Value) -> anyhow::Result<(Vec<Part>, Option<Usage>, Option<String>)> {
    let choice = json["choices"]
        .as_array()
        .and_then(|c| c.first())
        .ok_or_else(|| anyhow!("no choices in response"))?;

    let message = choice
        .get("message")
        .ok_or_else(|| anyhow!("no message in choice"))?;

    let mut parts = Vec::new();
    if let Some(reasoning) = message.get("reasoning_content").and_then(|v| v.as_str()) {
        if !reasoning.is_empty() {
            parts.push(Part::Reasoning {
                text: reasoning.to_string(),
            });
        }
    }
    if let Some(text) = message.get("content").and_then(|v| v.as_str()) {
        if !text.is_empty() {
            parts.push(Part::text(text));
        }
    }
    if let Some(calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
        for call in calls {
            let id = call["id"].as_str().unwrap_or_default().to_string();
            let name = call["function"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let arguments = call["function"]["arguments"]
                .as_str()
                .unwrap_or("{}")
                .to_string();
            let input: Value =
                serde_json::from_str(&arguments).unwrap_or(Value::Object(Default::default()));
            parts.push(Part::Tool(ToolPart::pending(id, name, input)));
        }
    }

    let usage = json.get("usage").map(|u| Usage {
        input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0),
        output_tokens: u["completion_tokens"].as_u64().unwrap_or(0),
    });
    let stop_reason = choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok((parts, usage, stop_reason))
}

/// Accumulates partial OpenAI tool_call deltas by index.
#[derive(Default)]
struct ToolCallAccumulator {
    // index -> (id, name, args buffer, started)
    calls: BTreeMap<String, (String, String, String, bool)>,
    pending: VecDeque<ProviderEvent>,
}

use std::collections::BTreeMap;

impl ToolCallAccumulator {
    fn feed(&mut self, delta_tool_calls: &Value) {
        let Some(arr) = delta_tool_calls.as_array() else {
            return;
        };
        for call in arr {
            let index = call["index"]
                .as_u64()
                .map(|i| i.to_string())
                .unwrap_or_else(|| "0".to_string());
            let entry = self
                .calls
                .entry(index)
                .or_insert_with(|| (String::new(), String::new(), String::new(), false));
            if let Some(id) = call["id"].as_str() {
                entry.0 = id.to_string();
            }
            if let Some(name) = call["function"]["name"].as_str() {
                entry.1 = name.to_string();
            }
            if let Some(args) = call["function"]["arguments"].as_str() {
                entry.2.push_str(args);
            }
            if !entry.3 && !entry.0.is_empty() && !entry.1.is_empty() {
                entry.3 = true;
                self.pending.push_back(ProviderEvent::ToolCallStart {
                    id: entry.0.clone(),
                    name: entry.1.clone(),
                });
            }
        }
    }

    fn finish_all(&mut self) {
        let mut ids = Vec::new();
        for (id, name, args, _) in self.calls.values() {
            ids.push((id.clone(), name.clone(), args.clone()));
        }
        for (id, _name, args) in ids {
            self.pending.push_back(ProviderEvent::ToolCallEnd {
                id,
                arguments: args,
            });
        }
    }
}

fn parse_stream_json(json: &Value, acc: &mut ToolCallAccumulator) {
    let Some(choices) = json["choices"].as_array() else {
        return;
    };
    let Some(choice) = choices.first() else {
        return;
    };
    if let Some(delta) = choice.get("delta") {
        if let Some(text) = delta["content"].as_str() {
            if !text.is_empty() {
                acc.pending
                    .push_back(ProviderEvent::TextDelta(text.to_string()));
            }
        }
        if let Some(reasoning) = delta["reasoning_content"].as_str() {
            if !reasoning.is_empty() {
                acc.pending
                    .push_back(ProviderEvent::ReasoningDelta(reasoning.to_string()));
            }
        }
        if let Some(tool_calls) = delta.get("tool_calls") {
            acc.feed(tool_calls);
        }
    }
}

fn response_to_events(response: reqwest::Response) -> ProviderStream {
    let state = OpenAiStreamState {
        bytes: Box::pin(response.bytes_stream()),
        parser: SseParser::new(),
        acc: ToolCallAccumulator::default(),
        usage: None,
        stop_reason: None,
        pending: VecDeque::new(),
        bytes_done: false,
        end_emitted: false,
    };

    Box::pin(futures_util::stream::unfold(state, |mut st| async move {
        loop {
            // Drain queued events first.
            if let Some(ev) = st.pending.pop_front() {
                return Some((Ok(ev), st));
            }
            if st.end_emitted {
                return None;
            }

            // Feed more bytes while available.
            if !st.bytes_done {
                match st.bytes.next().await {
                    Some(Ok(chunk)) => {
                        for data in st.parser.push(&chunk) {
                            if data.trim() == "[DONE]" {
                                st.bytes_done = true;
                                continue;
                            }
                            if let Ok(json) = serde_json::from_str::<Value>(&data) {
                                if let Some(usage) = json.get("usage") {
                                    st.usage = Some(Usage {
                                        input_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0),
                                        output_tokens: usage["completion_tokens"]
                                            .as_u64()
                                            .unwrap_or(0),
                                    });
                                }
                                if let Some(fr) = json["choices"][0]["finish_reason"]
                                    .as_str()
                                    .map(|s| s.to_string())
                                {
                                    st.stop_reason = Some(fr);
                                }
                                parse_stream_json(&json, &mut st.acc);
                            }
                        }
                        continue;
                    }
                    Some(Err(e)) => {
                        return Some((Err(anyhow!("stream error: {}", e)), st));
                    }
                    None => {
                        for data in st.parser.finish() {
                            if data.trim() == "[DONE]" {
                                continue;
                            }
                            if let Ok(json) = serde_json::from_str::<Value>(&data) {
                                parse_stream_json(&json, &mut st.acc);
                            }
                        }
                        st.bytes_done = true;
                        continue;
                    }
                }
            }

            // Bytes done: finalize accumulated tool calls and emit End once.
            st.end_emitted = true;
            let mut acc = std::mem::take(&mut st.acc);
            acc.finish_all();
            while let Some(ev) = acc.pending.pop_front() {
                st.pending.push_back(ev);
            }
            let usage = st.usage;
            let stop_reason = st.stop_reason.clone();
            st.pending
                .push_back(ProviderEvent::End { stop_reason, usage });
        }
    }))
}

struct OpenAiStreamState {
    bytes: Pin<Box<dyn futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    parser: SseParser,
    acc: ToolCallAccumulator,
    usage: Option<Usage>,
    stop_reason: Option<String>,
    pending: VecDeque<ProviderEvent>,
    /// Byte stream fully consumed.
    bytes_done: bool,
    /// Terminal End event emitted (stream terminates after this).
    end_emitted: bool,
}

#[async_trait::async_trait]
impl Provider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai-compatible"
    }

    async fn stream(&self, req: &LlmRequest) -> anyhow::Result<ProviderStream> {
        let url = format!("{}/chat/completions", self.http.base_url);
        let body = build_request_body(req, true);

        let response = self
            .http
            .post(&url, self.auth)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("HTTP request to {} failed: {}", url, e))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow!("API error ({}): {}", status, text));
        }
        Ok(response_to_events(response))
    }

    async fn complete(&self, req: &LlmRequest) -> anyhow::Result<LlmResponse> {
        let url = format!("{}/chat/completions", self.http.base_url);
        let body = build_request_body(req, false);

        let response = self
            .http
            .post(&url, self.auth)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("HTTP request to {} failed: {}", url, e))?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow!("API error ({}): {}", status, text));
        }
        let json: Value = response.json().await.context("failed to parse response")?;
        let (parts, usage, stop_reason) = parse_response(&json)?;
        Ok(LlmResponse {
            parts,
            usage,
            stop_reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_openai_messages_system_and_user() {
        let msgs = vec![
            Message::system("be nice"),
            Message::user("hello"),
            Message::new(Role::Assistant, vec![Part::text("hi there")]),
        ];
        let out = to_openai_messages("sys", &msgs);
        // system param + user + assistant; the in-session System message is folded into `sys`
        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["role"], "system");
        assert_eq!(out[0]["content"], "sys");
        assert_eq!(out[1]["role"], "user");
        assert_eq!(out[2]["role"], "assistant");
        assert_eq!(out[2]["content"], "hi there");
    }

    #[test]
    fn test_to_openai_messages_with_tool_call_and_result() {
        let mut tool = crate::harness::session::ToolPart::pending(
            "tc1",
            "bash",
            serde_json::json!({"command": "ls"}),
        );
        tool.status = crate::harness::session::ToolStatus::Completed;
        tool.output = "files".into();
        let msgs = vec![
            Message::user("run ls"),
            Message::new(Role::Assistant, vec![Part::Tool(tool)]),
        ];
        let out = to_openai_messages("", &msgs);
        // user + assistant(tool_calls) + tool result
        assert_eq!(out.len(), 3);
        assert_eq!(out[1]["tool_calls"][0]["id"], "tc1");
        assert_eq!(out[1]["content"], Value::Null);
        assert_eq!(out[2]["role"], "tool");
        assert_eq!(out[2]["tool_call_id"], "tc1");
        assert_eq!(out[2]["content"], "files");
    }

    #[test]
    fn test_to_openai_messages_skips_pending_tool_calls() {
        // A pending/running tool call (e.g. from an interrupted session) has no
        // result and must not be replayed as a "did not complete" error.
        let mut pending = crate::harness::session::ToolPart::pending(
            "tc-pending",
            "bash",
            serde_json::json!({"command": "ls"}),
        );
        pending.status = crate::harness::session::ToolStatus::Running;
        let mut done = crate::harness::session::ToolPart::pending(
            "tc-done",
            "read",
            serde_json::json!({"path": "a.rs"}),
        );
        done.status = crate::harness::session::ToolStatus::Completed;
        done.output = "content".into();

        let msgs = vec![
            Message::user("do work"),
            Message::new(Role::Assistant, vec![Part::Tool(pending), Part::Tool(done)]),
        ];
        let out = to_openai_messages("", &msgs);
        // user + assistant(only the completed tool call) + tool result
        assert_eq!(out.len(), 3);
        let calls = out[1]["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 1, "pending tool call must be dropped");
        assert_eq!(calls[0]["id"], "tc-done");
        assert_eq!(out[2]["tool_call_id"], "tc-done");
    }

    #[test]
    fn test_parse_response_with_tool_calls() {
        let json = json!({
            "choices": [{
                "message": {
                    "content": "let me check",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "read", "arguments": "{\"path\":\"a.rs\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 20}
        });
        let (parts, usage, stop) = parse_response(&json).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].as_text(), Some("let me check"));
        match &parts[1] {
            Part::Tool(t) => {
                assert_eq!(t.name, "read");
                assert_eq!(t.input["path"], "a.rs");
            }
            _ => panic!("expected tool part"),
        }
        assert_eq!(usage.unwrap().input_tokens, 10);
        assert_eq!(stop.as_deref(), Some("tool_calls"));
    }

    #[test]
    fn test_parse_response_empty() {
        let json = json!({"choices": []});
        assert!(parse_response(&json).is_err());
    }
}
