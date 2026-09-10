//! Anthropic-compatible provider: `/messages` with content blocks (`tool_use`).
//! Used for opencode-go (MiniMax) and Anthropic-style endpoints.

use super::{
    AuthStyle, HttpConfig, LlmRequest, LlmResponse, Provider, ProviderEvent, ProviderStream, Usage,
};
use crate::harness::session::{Message, Part, Role, ToolPart};
use anyhow::{anyhow, Context as AnyhowContext};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};

pub struct AnthropicProvider {
    pub http: HttpConfig,
    pub auth: AuthStyle,
    /// When true, marks system/tools/last message with `cache_control`
    /// breakpoints (Anthropic prompt caching). MiniMax rejects them, so the
    /// opencode-go router keeps this off by default.
    pub prompt_cache: bool,
}

/// Converts harness messages to Anthropic `/messages` format.
/// - system goes to top-level `system`
/// - assistant tool_use blocks + following user tool_result blocks
pub fn to_anthropic_messages(messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {
                // System messages in-session become user text (rare).
                let text = msg.text_content();
                if !text.is_empty() {
                    out.push(json!({"role": "user", "content": [{"type": "text", "text": text}]}));
                }
            }
            Role::User => {
                let mut content = Vec::new();
                for part in &msg.parts {
                    match part {
                        Part::Text { text } if !text.is_empty() => {
                            content.push(json!({"type": "text", "text": text}));
                        }
                        Part::Image { path } => {
                            content.push(image_block_or_fallback(path));
                        }
                        _ => {}
                    }
                }
                if !content.is_empty() {
                    out.push(json!({"role": "user", "content": content}));
                }
            }
            Role::Assistant => {
                let mut content = Vec::new();
                for part in &msg.parts {
                    match part {
                        Part::Text { text } if !text.is_empty() => {
                            content.push(json!({"type": "text", "text": text}));
                        }
                        Part::Reasoning { .. } => {}
                        Part::Image { .. } => {}
                        Part::Tool(t) => {
                            // Skip pending/running tool calls (no result yet).
                            if t.is_terminal() {
                                content.push(json!({
                                    "type": "tool_use",
                                    "id": t.id,
                                    "name": t.name,
                                    "input": t.input,
                                }));
                            }
                        }
                        _ => {}
                    }
                }
                if !content.is_empty() {
                    out.push(json!({"role": "assistant", "content": content}));

                    // Synthesize tool_result blocks in a following user message.
                    let mut results = Vec::new();
                    for t in msg.tool_parts().iter().filter(|t| t.is_terminal()) {
                        let text = match t.status {
                            crate::harness::session::ToolStatus::Completed => t.output.clone(),
                            crate::harness::session::ToolStatus::Error => {
                                format!("Error: {}", t.error.clone().unwrap_or_default())
                            }
                            _ => format!("Error: tool call {} did not complete", t.id),
                        };
                        results.push(json!({
                            "type": "tool_result",
                            "tool_use_id": t.id,
                            "content": text,
                        }));
                    }
                    if !results.is_empty() {
                        out.push(json!({"role": "user", "content": results}));
                    }
                }
            }
        }
    }
    out
}

/// Converts a `Part::Image` into an Anthropic image content block. On read or
/// extension errors, degrades to a text block so the request still goes out.
fn image_block_or_fallback(path: &str) -> Value {
    match crate::harness::session::image::load_image(path) {
        Ok(img) => json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": img.media_type,
                "data": img.data,
            }
        }),
        Err(reason) => json!({
            "type": "text",
            "text": crate::harness::session::image::image_fallback_text(path, &reason),
        }),
    }
}

fn build_request_body(req: &LlmRequest, stream: bool, prompt_cache: bool) -> Value {
    // The Anthropic Messages API requires max_tokens; we cannot omit it, so
    // fall back to a conservative default when the request doesn't set one.
    const DEFAULT_MAX_TOKENS: usize = 4096;
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        "temperature": req.temperature,
        "stream": stream,
        "messages": to_anthropic_messages(&req.messages[..]),
    });
    if !req.system.trim().is_empty() {
        if prompt_cache {
            // Breakpoint 1: system prompt (stable prefix).
            body["system"] = json!([{
                "type": "text",
                "text": req.system,
                "cache_control": {"type": "ephemeral"},
            }]);
        } else {
            body["system"] = json!(req.system);
        }
    }
    if !req.tools.is_empty() {
        let mut tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect();
        if prompt_cache {
            // Breakpoint 2: last tool definition (tool list is stable).
            tools.last_mut().unwrap()["cache_control"] = json!({"type": "ephemeral"});
        }
        body["tools"] = json!(tools);
    }
    if prompt_cache {
        // Breakpoint 3: last content block of the last message (grows with
        // the conversation; the prefix up to it gets cached).
        if let Some(last) = body["messages"].as_array_mut().and_then(|m| m.last_mut()) {
            if last["content"].is_array() {
                if let Some(blocks) = last["content"].as_array_mut() {
                    if let Some(block) = blocks.last_mut() {
                        block["cache_control"] = json!({"type": "ephemeral"});
                    }
                }
            } else {
                // content is a string → convert to a 1-block text array.
                let text = last["content"].as_str().unwrap_or_default().to_string();
                last["content"] = json!([{
                    "type": "text",
                    "text": text,
                    "cache_control": {"type": "ephemeral"},
                }]);
            }
        }
    }
    body
}

/// Parses a non-streaming `/messages` response into parts.
pub fn parse_response(json: &Value) -> anyhow::Result<(Vec<Part>, Option<Usage>, Option<String>)> {
    let content = json["content"]
        .as_array()
        .ok_or_else(|| anyhow!("no content array in response"))?;

    let mut parts = Vec::new();
    for block in content {
        match block["type"].as_str() {
            Some("text") => {
                if let Some(text) = block["text"].as_str() {
                    if !text.is_empty() {
                        parts.push(Part::text(text));
                    }
                }
            }
            Some("thinking") => {
                if let Some(text) = block["thinking"].as_str() {
                    if !text.is_empty() {
                        parts.push(Part::Reasoning {
                            text: text.to_string(),
                        });
                    }
                }
            }
            Some("tool_use") => {
                let id = block["id"].as_str().unwrap_or_default().to_string();
                let name = block["name"].as_str().unwrap_or_default().to_string();
                let input = block["input"].clone();
                parts.push(Part::Tool(ToolPart::pending(id, name, input)));
            }
            _ => {}
        }
    }

    let usage = json.get("usage").map(|u| Usage {
        input_tokens: u["input_tokens"].as_u64().unwrap_or(0),
        output_tokens: u["output_tokens"].as_u64().unwrap_or(0),
        // Anthropic reports cache tokens as separate fields, EXCLUDED from
        // `input_tokens` — keep that semantics (do not add them up).
        cache_read_tokens: u["cache_read_input_tokens"].as_u64().unwrap_or(0),
        cache_write_tokens: u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
    });
    let stop_reason = json["stop_reason"].as_str().map(|s| s.to_string());
    Ok((parts, usage, stop_reason))
}

/// State machine for the Anthropic SSE stream.
struct AnthropicStreamState {
    /// index -> (block_type, tool_id, tool_name, json buffer)
    blocks: BTreeMap<u64, (String, String, String, String)>,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    stop_reason: Option<String>,
}

impl AnthropicStreamState {
    fn handle_event(&mut self, data: &str, out: &mut VecDeque<ProviderEvent>) -> super::SseAction {
        if data.trim() == "[DONE]" {
            return super::SseAction::Finish;
        }
        let Ok(json) = serde_json::from_str::<Value>(data) else {
            tracing::warn!("malformed SSE payload: {:.200}", data);
            return super::SseAction::Continue;
        };
        match json["type"].as_str() {
            Some("message_start") => {
                let usage = &json["message"]["usage"];
                self.input_tokens = usage["input_tokens"].as_u64().unwrap_or(0);
                self.cache_read_tokens = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
                self.cache_write_tokens =
                    usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
            }
            Some("content_block_start") => {
                let index = json["index"].as_u64().unwrap_or(0);
                let block = &json["content_block"];
                let btype = block["type"].as_str().unwrap_or("text").to_string();
                let id = block["id"].as_str().unwrap_or_default().to_string();
                let name = block["name"].as_str().unwrap_or_default().to_string();
                if btype == "tool_use" {
                    out.push_back(ProviderEvent::ToolCallStart {
                        id: id.clone(),
                        name: name.clone(),
                    });
                }
                self.blocks.insert(index, (btype, id, name, String::new()));
            }
            Some("content_block_delta") => {
                let delta = &json["delta"];
                match delta["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let text = delta["text"].as_str().unwrap_or("");
                        if !text.is_empty() {
                            out.push_back(ProviderEvent::TextDelta(text.to_string()));
                        }
                    }
                    "thinking_delta" => {
                        let text = delta["thinking"].as_str().unwrap_or("");
                        if !text.is_empty() {
                            out.push_back(ProviderEvent::ReasoningDelta(text.to_string()));
                        }
                    }
                    "input_json_delta" => {
                        let index = json["index"].as_u64().unwrap_or(0);
                        let partial = delta["partial_json"].as_str().unwrap_or("");
                        let entry = self.blocks.entry(index).or_default();
                        entry.3.push_str(partial);
                    }
                    _ => {}
                }
            }
            Some("content_block_stop") => {
                let index = json["index"].as_u64().unwrap_or(0);
                if let Some((btype, id, _name, buf)) = self.blocks.remove(&index) {
                    if btype == "tool_use" {
                        let arguments = if buf.trim().is_empty() {
                            "{}".to_string()
                        } else {
                            buf
                        };
                        out.push_back(ProviderEvent::ToolCallEnd { id, arguments });
                    }
                }
            }
            Some("message_delta") => {
                if let Some(reason) = json["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(reason.to_string());
                }
                self.output_tokens = json["usage"]["output_tokens"].as_u64().unwrap_or(0);
            }
            Some("message_stop") => {
                return super::SseAction::Finish;
            }
            _ => {}
        }
        super::SseAction::Continue
    }
}

impl super::SseHandler for AnthropicStreamState {
    fn handle_event(&mut self, data: &str, out: &mut VecDeque<ProviderEvent>) -> super::SseAction {
        AnthropicStreamState::handle_event(self, data, out)
    }

    fn end_event(&self) -> ProviderEvent {
        ProviderEvent::End {
            stop_reason: self.stop_reason.clone(),
            usage: Some(Usage {
                input_tokens: self.input_tokens,
                output_tokens: self.output_tokens,
                cache_read_tokens: self.cache_read_tokens,
                cache_write_tokens: self.cache_write_tokens,
            }),
        }
    }
}

fn response_to_events(response: reqwest::Response) -> ProviderStream {
    let state = AnthropicStreamState {
        blocks: BTreeMap::new(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        stop_reason: None,
    };
    super::drive_sse_stream(Box::pin(response.bytes_stream()), state)
}

#[async_trait::async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic-compatible"
    }

    async fn stream(&self, req: &LlmRequest) -> anyhow::Result<ProviderStream> {
        let url = format!("{}/messages", self.http.base_url);
        let body = build_request_body(req, true, self.prompt_cache);

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
        let url = format!("{}/messages", self.http.base_url);
        let body = build_request_body(req, false, self.prompt_cache);

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
    fn test_to_anthropic_messages_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pic.png");
        std::fs::write(&path, [0x89, b'P', b'N', b'G']).unwrap();
        let msgs = vec![Message::new(
            Role::User,
            vec![
                Part::image(path.to_str().unwrap()),
                Part::text("describe this image"),
            ],
        )];
        let out = to_anthropic_messages(&msgs);
        assert_eq!(out.len(), 1);
        let blocks = out[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "image");
        assert_eq!(blocks[0]["source"]["type"], "base64");
        assert_eq!(blocks[0]["source"]["media_type"], "image/png");
        assert_eq!(
            blocks[0]["source"]["data"],
            base64::engine::general_purpose::STANDARD.encode([0x89, b'P', b'N', b'G'])
        );
        assert_eq!(blocks[1]["type"], "text");
    }

    #[test]
    fn test_to_anthropic_messages_image_missing_file_degrades_to_text() {
        let msgs = vec![Message::new(
            Role::User,
            vec![Part::image("/nonexistent/nope.png")],
        )];
        let out = to_anthropic_messages(&msgs);
        let blocks = out[0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "text");
        assert!(
            blocks[0]["text"]
                .as_str()
                .unwrap()
                .contains("[image: /nonexistent/nope.png"),
            "must degrade to a text placeholder"
        );
    }

    #[test]
    fn test_to_anthropic_messages_image_unsupported_extension() {
        let msgs = vec![Message::new(Role::User, vec![Part::image("notes.txt")])];
        let out = to_anthropic_messages(&msgs);
        let blocks = out[0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "text");
    }

    #[test]
    fn test_to_anthropic_messages_basic() {
        let msgs = vec![
            Message::user("hello"),
            Message::new(Role::Assistant, vec![Part::text("hi")]),
        ];
        let out = to_anthropic_messages(&msgs);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["role"], "user");
        assert_eq!(out[0]["content"][0]["type"], "text");
        assert_eq!(out[1]["role"], "assistant");
    }

    #[test]
    fn test_to_anthropic_messages_tool_use_and_result() {
        use crate::harness::session::ToolStatus;
        let mut tool = ToolPart::pending("tu1", "bash", serde_json::json!({"command": "ls"}));
        tool.status = ToolStatus::Completed;
        tool.output = "out".into();
        let msgs = vec![
            Message::user("run"),
            Message::new(Role::Assistant, vec![Part::Tool(tool)]),
        ];
        let out = to_anthropic_messages(&msgs);
        // user, assistant(tool_use), user(tool_result)
        assert_eq!(out.len(), 3);
        assert_eq!(out[1]["content"][0]["type"], "tool_use");
        assert_eq!(out[1]["content"][0]["id"], "tu1");
        assert_eq!(out[2]["role"], "user");
        assert_eq!(out[2]["content"][0]["type"], "tool_result");
        assert_eq!(out[2]["content"][0]["tool_use_id"], "tu1");
        assert_eq!(out[2]["content"][0]["content"], "out");
    }

    #[test]
    fn test_to_anthropic_messages_skips_pending_tool_calls() {
        use crate::harness::session::ToolStatus;
        let mut pending = ToolPart::pending("tu-p", "bash", serde_json::json!({"command": "ls"}));
        pending.status = ToolStatus::Running;
        let mut done = ToolPart::pending("tu-d", "read", serde_json::json!({"path": "a.rs"}));
        done.status = ToolStatus::Completed;
        done.output = "content".into();
        let msgs = vec![
            Message::user("run"),
            Message::new(Role::Assistant, vec![Part::Tool(pending), Part::Tool(done)]),
        ];
        let out = to_anthropic_messages(&msgs);
        // user, assistant(only the completed tool_use), user(tool_result)
        assert_eq!(out.len(), 3);
        let blocks = out[1]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1, "pending tool_use must be dropped");
        assert_eq!(blocks[0]["id"], "tu-d");
        assert_eq!(out[2]["content"][0]["tool_use_id"], "tu-d");
    }

    #[test]
    fn test_parse_response_content_blocks() {
        let json = json!({
            "content": [
                {"type": "text", "text": "checking"},
                {"type": "tool_use", "id": "tu1", "name": "read", "input": {"path": "a.rs"}}
            ],
            "usage": {"input_tokens": 5, "output_tokens": 7},
            "stop_reason": "tool_use"
        });
        let (parts, usage, stop) = parse_response(&json).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(usage.unwrap().input_tokens, 5);
        assert_eq!(stop.as_deref(), Some("tool_use"));
    }

    #[test]
    fn test_parse_response_cache_usage() {
        // Anthropic reports cache tokens as separate fields, excluded from
        // input_tokens.
        let json = json!({
            "content": [{"type": "text", "text": "ok"}],
            "usage": {
                "input_tokens": 120,
                "output_tokens": 30,
                "cache_creation_input_tokens": 800,
                "cache_read_input_tokens": 4000
            }
        });
        let (_, usage, _) = parse_response(&json).unwrap();
        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 120);
        assert_eq!(u.output_tokens, 30);
        assert_eq!(u.cache_write_tokens, 800);
        assert_eq!(u.cache_read_tokens, 4000);
        assert_eq!(u.cache_total(), 4800);
    }

    #[test]
    fn test_stream_state_parses_cache_usage_from_message_start() {
        let mut st = AnthropicStreamState {
            blocks: BTreeMap::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            stop_reason: None,
        };
        let mut out = VecDeque::new();
        st.handle_event(
            r#"{"type":"message_start","message":{"usage":{
                "input_tokens":50,
                "cache_creation_input_tokens":100,
                "cache_read_input_tokens":200
            }}}"#,
            &mut out,
        );
        assert_eq!(st.input_tokens, 50);
        assert_eq!(st.cache_write_tokens, 100);
        assert_eq!(st.cache_read_tokens, 200);
    }

    #[test]
    fn test_build_request_body_includes_tools_and_system() {
        let req = LlmRequest {
            model: "minimax".into(),
            system: "sys".into(),
            messages: std::sync::Arc::new(vec![Message::user("hi")]),
            tools: vec![super::super::ToolSpec {
                name: "bash".into(),
                description: "shell".into(),
                parameters: serde_json::json!({"type": "object"}),
            }],
            max_tokens: Some(100),
            temperature: 0.5,
        };
        let body = build_request_body(&req, true, false);
        assert_eq!(body["system"], "sys");
        assert_eq!(body["tools"][0]["name"], "bash");
        assert_eq!(body["messages"][0]["role"], "user");
    }

    fn cache_control_count(v: &Value) -> usize {
        match v {
            Value::Object(map) => map
                .iter()
                .map(|(k, val)| {
                    if k == "cache_control" {
                        1
                    } else {
                        cache_control_count(val)
                    }
                })
                .sum(),
            Value::Array(items) => items.iter().map(cache_control_count).sum(),
            _ => 0,
        }
    }

    #[test]
    fn test_build_request_body_cache_off_unchanged() {
        let req = LlmRequest {
            model: "claude".into(),
            system: "sys".into(),
            messages: std::sync::Arc::new(vec![Message::user("hi")]),
            tools: vec![super::super::ToolSpec {
                name: "bash".into(),
                description: "shell".into(),
                parameters: serde_json::json!({"type": "object"}),
            }],
            max_tokens: Some(100),
            temperature: 0.5,
        };
        let body = build_request_body(&req, true, false);
        assert_eq!(body["system"], "sys");
        assert!(body["system"].is_string());
        assert!(cache_control_count(&body) == 0);
    }

    #[test]
    fn test_build_request_body_cache_breakpoints() {
        use crate::harness::session::ToolStatus;
        let mut tool = ToolPart::pending("tu1", "bash", serde_json::json!({"command": "ls"}));
        tool.status = ToolStatus::Completed;
        tool.output = "out".into();
        let msgs = vec![
            Message::user("run"),
            Message::new(Role::Assistant, vec![Part::Tool(tool)]),
        ];
        let req = LlmRequest {
            model: "claude".into(),
            system: "sys".into(),
            messages: std::sync::Arc::new(msgs),
            tools: vec![
                super::super::ToolSpec {
                    name: "bash".into(),
                    description: "shell".into(),
                    parameters: serde_json::json!({"type": "object"}),
                },
                super::super::ToolSpec {
                    name: "read".into(),
                    description: "read".into(),
                    parameters: serde_json::json!({"type": "object"}),
                },
            ],
            max_tokens: Some(100),
            temperature: 0.5,
        };
        let body = build_request_body(&req, true, true);
        // Breakpoint 1: system is an array with cache_control.
        assert!(body["system"].is_array());
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        // Breakpoint 2: last tool marked, first not.
        assert!(body["tools"][0]["cache_control"].is_null());
        assert_eq!(body["tools"][1]["cache_control"]["type"], "ephemeral");
        // Breakpoint 3: last block of the last message (tool_result) marked.
        let last_msg = body["messages"].as_array().unwrap().last().unwrap();
        let blocks = last_msg["content"].as_array().unwrap();
        assert_eq!(blocks.last().unwrap()["cache_control"]["type"], "ephemeral");
        // API limit: at most 4 breakpoints; we use 3.
        assert_eq!(cache_control_count(&body), 3);
    }

    #[test]
    fn test_build_request_body_cache_string_content_converted() {
        let req = LlmRequest {
            model: "claude".into(),
            system: "sys".into(),
            messages: std::sync::Arc::new(vec![Message::user("hi")]),
            tools: vec![],
            max_tokens: Some(100),
            temperature: 0.5,
        };
        let body = build_request_body(&req, false, true);
        let last_msg = body["messages"].as_array().unwrap().last().unwrap();
        assert!(last_msg["content"].is_array());
        assert_eq!(last_msg["content"][0]["type"], "text");
        assert_eq!(last_msg["content"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(cache_control_count(&body), 2); // system + message
    }
}
