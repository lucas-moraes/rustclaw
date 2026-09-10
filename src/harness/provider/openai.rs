//! OpenAI-compatible provider: `/chat/completions` with native `tools`.

use super::{
    AuthStyle, HttpConfig, LlmRequest, LlmResponse, Provider, ProviderEvent, ProviderStream, Usage,
};
use crate::harness::session::{Message, Part, Role, ToolPart};
use anyhow::{anyhow, Context as AnyhowContext};
use serde_json::{json, Value};
use std::collections::VecDeque;

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
                let has_image = msg.parts.iter().any(|p| matches!(p, Part::Image { .. }));
                if has_image {
                    // Vision request: content must be an array of parts.
                    let mut content = Vec::new();
                    for part in &msg.parts {
                        match part {
                            Part::Text { text } if !text.is_empty() => {
                                content.push(json!({"type": "text", "text": text}));
                            }
                            Part::Image { path } => {
                                content.push(image_url_block_or_fallback(path));
                            }
                            _ => {}
                        }
                    }
                    if !content.is_empty() {
                        out.push(json!({"role": "user", "content": content}));
                    }
                } else {
                    let text = msg.text_content();
                    if !text.is_empty() {
                        out.push(json!({"role": "user", "content": text}));
                    }
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

/// Converts a `Part::Image` into an OpenAI `image_url` content block with a
/// `data:` URL. On errors, degrades to a text block so the request still goes
/// out.
fn image_url_block_or_fallback(path: &str) -> Value {
    match crate::harness::session::image::load_image(path) {
        Ok(img) => json!({
            "type": "image_url",
            "image_url": {"url": format!("data:{};base64,{}", img.media_type, img.data)}
        }),
        Err(reason) => json!({
            "type": "text",
            "text": crate::harness::session::image::image_fallback_text(path, &reason),
        }),
    }
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
        "messages": to_openai_messages(&req.system, &req.messages[..]),
        "temperature": req.temperature,
        "stream": stream,
    });
    // Ask the API to include a final usage chunk in streaming responses;
    // without this the stream never reports token usage (and we could not
    // account for cached tokens).
    if stream {
        body["stream_options"] = json!({ "include_usage": true });
    }
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
        // OpenAI includes cached tokens in `prompt_tokens`; the details
        // field is the cached subset (do not add it to input_tokens).
        cache_read_tokens: u["prompt_tokens_details"]["cached_tokens"]
            .as_u64()
            .unwrap_or(0),
        cache_write_tokens: 0,
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
        for (pos, call) in arr.iter().enumerate() {
            // `index` is the canonical key; when absent, fall back to the
            // position in the delta array. A non-numeric index is ignored
            // (with a warning) instead of collapsing everything into slot 0
            // and corrupting distinct tool-call arguments.
            let index = match call["index"].as_u64() {
                Some(i) => i.to_string(),
                None => {
                    if call["index"].is_null() {
                        pos.to_string()
                    } else {
                        tracing::warn!(
                            "tool_call delta with non-numeric index ignored: {}",
                            call["index"]
                        );
                        continue;
                    }
                }
            };
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

/// If the SSE payload carries a provider `{"error": {...}}` event, returns the
/// error message. Providers (OpenRouter, DeepInfra, …) send this after a 200 OK
/// to signal a mid-stream failure; without this check the turn would end with a
/// silent empty reply.
fn sse_error_message(json: &Value) -> Option<String> {
    let err = json.get("error")?;
    let msg = err["message"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| err.to_string());
    Some(msg)
}

fn response_to_events(response: reqwest::Response) -> ProviderStream {
    let state = OpenAiStreamState {
        acc: ToolCallAccumulator::default(),
        usage: None,
        stop_reason: None,
    };
    super::drive_sse_stream(Box::pin(response.bytes_stream()), state)
}

struct OpenAiStreamState {
    acc: ToolCallAccumulator,
    usage: Option<Usage>,
    stop_reason: Option<String>,
}

impl super::SseHandler for OpenAiStreamState {
    fn handle_event(&mut self, data: &str, out: &mut VecDeque<ProviderEvent>) -> super::SseAction {
        if data.trim() == "[DONE]" {
            return super::SseAction::Finish;
        }
        let Ok(json) = serde_json::from_str::<Value>(data) else {
            tracing::warn!("malformed SSE payload: {:.200}", data);
            return super::SseAction::Continue;
        };
        // Providers (OpenRouter, DeepInfra, …) may send an `{"error": {...}}`
        // event after a 200 OK. Surface it as a stream error instead of a
        // silent empty reply.
        if let Some(msg) = sse_error_message(&json) {
            return super::SseAction::Error(msg);
        }
        if let Some(usage) = json.get("usage") {
            self.usage = Some(Usage {
                input_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0),
                output_tokens: usage["completion_tokens"].as_u64().unwrap_or(0),
                cache_read_tokens: usage["prompt_tokens_details"]["cached_tokens"]
                    .as_u64()
                    .unwrap_or(0),
                cache_write_tokens: 0,
            });
        }
        if let Some(fr) = json["choices"][0]["finish_reason"]
            .as_str()
            .map(|s| s.to_string())
        {
            self.stop_reason = Some(fr);
        }
        parse_stream_json(&json, &mut self.acc);
        while let Some(ev) = self.acc.pending.pop_front() {
            out.push_back(ev);
        }
        super::SseAction::Continue
    }

    fn on_finish(&mut self, out: &mut VecDeque<ProviderEvent>) {
        self.acc.finish_all();
        while let Some(ev) = self.acc.pending.pop_front() {
            out.push_back(ev);
        }
    }

    fn end_event(&self) -> ProviderEvent {
        ProviderEvent::End {
            stop_reason: self.stop_reason.clone(),
            usage: self.usage,
        }
    }
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
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let text = response.text().await.unwrap_or_default();
            let kind = super::retry::classify_status(status.as_u16());
            return Err(super::retry::provider_error(
                kind,
                Some(status.as_u16()),
                retry_after,
                format!("API error ({}): {}", status, text),
            ));
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
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let text = response.text().await.unwrap_or_default();
            let kind = super::retry::classify_status(status.as_u16());
            return Err(super::retry::provider_error(
                kind,
                Some(status.as_u16()),
                retry_after,
                format!("API error ({}): {}", status, text),
            ));
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
    use base64::Engine as _;

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
    fn test_to_openai_messages_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pic.jpg");
        std::fs::write(&path, [0xFF, 0xD8, 0xFF]).unwrap();
        let msgs = vec![Message::new(
            Role::User,
            vec![
                Part::image(path.to_str().unwrap()),
                Part::text("what is this?"),
            ],
        )];
        let out = to_openai_messages("", &msgs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "user");
        let content = out[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "image_url");
        let url = content[0]["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,"), "url: {}", url);
        assert_eq!(
            url,
            format!(
                "data:image/jpeg;base64,{}",
                base64::engine::general_purpose::STANDARD.encode([0xFF, 0xD8, 0xFF])
            )
        );
        assert_eq!(content[1], json!({"type": "text", "text": "what is this?"}));
    }

    #[test]
    fn test_to_openai_messages_image_missing_file_degrades_to_text() {
        let msgs = vec![Message::new(
            Role::User,
            vec![Part::image("/nonexistent/nope.png")],
        )];
        let out = to_openai_messages("", &msgs);
        let content = out[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert!(content[0]["text"].as_str().unwrap().contains("[image:"));
    }

    #[test]
    fn test_to_openai_messages_plain_text_stays_string() {
        let msgs = vec![Message::user("hello")];
        let out = to_openai_messages("", &msgs);
        assert_eq!(out[0]["content"], "hello");
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

    #[test]
    fn test_sse_error_message_detects_provider_error() {
        let json = json!({"error": {"message": "rate limit exceeded", "type": "rate_limit"}});
        assert_eq!(
            sse_error_message(&json).as_deref(),
            Some("rate limit exceeded")
        );
    }

    #[test]
    fn test_sse_error_message_none_for_normal_chunk() {
        let json = json!({"choices": [{"delta": {"content": "hi"}}]});
        assert_eq!(sse_error_message(&json), None);
    }

    #[test]
    fn test_sse_error_message_falls_back_to_raw() {
        let json = json!({"error": {"code": 500}});
        assert!(sse_error_message(&json).is_some());
    }

    #[test]
    fn test_parse_response_cached_tokens() {
        // OpenAI includes cached tokens in prompt_tokens; the details field
        // is the cached subset.
        let json = json!({
            "choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}],
            "usage": {
                "prompt_tokens": 5000,
                "completion_tokens": 100,
                "prompt_tokens_details": {"cached_tokens": 4200}
            }
        });
        let (_, usage, _) = parse_response(&json).unwrap();
        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 5000);
        assert_eq!(u.cache_read_tokens, 4200);
        assert_eq!(u.cache_write_tokens, 0);
    }

    #[test]
    fn test_build_request_body_stream_options() {
        let req = LlmRequest {
            model: "gpt-test".into(),
            system: "sys".into(),
            messages: std::sync::Arc::new(vec![Message::user("hi")]),
            tools: vec![],
            max_tokens: None,
            temperature: 0.0,
        };
        let streaming = build_request_body(&req, true);
        assert_eq!(streaming["stream_options"]["include_usage"], true);
        let non_streaming = build_request_body(&req, false);
        assert!(non_streaming.get("stream_options").is_none());
    }
}
