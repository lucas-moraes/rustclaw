//! Anthropic-compatible provider: `/messages` with content blocks (`tool_use`).
//! Used for opencode-go (MiniMax) and Anthropic-style endpoints.

use super::{
    AuthStyle, HttpConfig, LlmRequest, LlmResponse, Provider, ProviderEvent, ProviderStream,
    SseParser, Usage,
};
use crate::harness::session::{Message, Part, Role, ToolPart};
use anyhow::{anyhow, Context as AnyhowContext};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::pin::Pin;

pub struct AnthropicProvider {
    pub http: HttpConfig,
    pub auth: AuthStyle,
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
                let text = msg.text_content();
                if !text.is_empty() {
                    out.push(json!({"role": "user", "content": [{"type": "text", "text": text}]}));
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

fn build_request_body(req: &LlmRequest, stream: bool) -> Value {
    // The Anthropic Messages API requires max_tokens; we cannot omit it, so
    // fall back to a conservative default when the request doesn't set one.
    const DEFAULT_MAX_TOKENS: usize = 4096;
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        "temperature": req.temperature,
        "stream": stream,
        "messages": to_anthropic_messages(&req.messages),
    });
    if !req.system.trim().is_empty() {
        body["system"] = json!(req.system);
    }
    if !req.tools.is_empty() {
        body["tools"] = json!(req
            .tools
            .iter()
            .map(|t| json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            }))
            .collect::<Vec<_>>());
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
    });
    let stop_reason = json["stop_reason"].as_str().map(|s| s.to_string());
    Ok((parts, usage, stop_reason))
}

/// State machine for the Anthropic SSE stream.
struct AnthropicStreamState {
    bytes: Pin<Box<dyn futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    parser: SseParser,
    /// pending events queue
    pending: VecDeque<ProviderEvent>,
    /// index -> (block_type, tool_id, tool_name, json buffer)
    blocks: BTreeMap<u64, (String, String, String, String)>,
    input_tokens: u64,
    output_tokens: u64,
    stop_reason: Option<String>,
    /// message_stop/[DONE] received (data complete).
    finished: bool,
    /// Terminal End event emitted (stream terminates after this).
    end_emitted: bool,
}

impl AnthropicStreamState {
    fn handle_event(&mut self, data: &str) {
        if data.trim() == "[DONE]" {
            self.finished = true;
            return;
        }
        let Ok(json) = serde_json::from_str::<Value>(data) else {
            return;
        };
        match json["type"].as_str() {
            Some("message_start") => {
                self.input_tokens = json["message"]["usage"]["input_tokens"]
                    .as_u64()
                    .unwrap_or(0);
            }
            Some("content_block_start") => {
                let index = json["index"].as_u64().unwrap_or(0);
                let block = &json["content_block"];
                let btype = block["type"].as_str().unwrap_or("text").to_string();
                let id = block["id"].as_str().unwrap_or_default().to_string();
                let name = block["name"].as_str().unwrap_or_default().to_string();
                if btype == "tool_use" {
                    self.pending.push_back(ProviderEvent::ToolCallStart {
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
                            self.pending
                                .push_back(ProviderEvent::TextDelta(text.to_string()));
                        }
                    }
                    "thinking_delta" => {
                        let text = delta["thinking"].as_str().unwrap_or("");
                        if !text.is_empty() {
                            self.pending
                                .push_back(ProviderEvent::ReasoningDelta(text.to_string()));
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
                        self.pending
                            .push_back(ProviderEvent::ToolCallEnd { id, arguments });
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
                self.finished = true;
            }
            _ => {}
        }
    }
}

fn response_to_events(response: reqwest::Response) -> ProviderStream {
    let state = AnthropicStreamState {
        bytes: Box::pin(response.bytes_stream()),
        parser: SseParser::new(),
        pending: VecDeque::new(),
        blocks: BTreeMap::new(),
        input_tokens: 0,
        output_tokens: 0,
        stop_reason: None,
        finished: false,
        end_emitted: false,
    };

    Box::pin(futures_util::stream::unfold(state, |mut st| async move {
        loop {
            if let Some(ev) = st.pending.pop_front() {
                return Some((Ok(ev), st));
            }
            if st.end_emitted {
                return None;
            }
            if !st.finished {
                match st.bytes.next().await {
                    Some(Ok(chunk)) => {
                        for data in st.parser.push(&chunk) {
                            st.handle_event(&data);
                        }
                        continue;
                    }
                    Some(Err(e)) => {
                        return Some((Err(anyhow!("stream error: {}", e)), st));
                    }
                    None => {
                        for data in st.parser.finish() {
                            st.handle_event(&data);
                        }
                        st.finished = true;
                        continue;
                    }
                }
            }

            st.end_emitted = true;
            st.pending.push_back(ProviderEvent::End {
                stop_reason: st.stop_reason.clone(),
                usage: Some(Usage {
                    input_tokens: st.input_tokens,
                    output_tokens: st.output_tokens,
                }),
            });
        }
    }))
}

#[async_trait::async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic-compatible"
    }

    async fn stream(&self, req: &LlmRequest) -> anyhow::Result<ProviderStream> {
        let url = format!("{}/messages", self.http.base_url);
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
        let url = format!("{}/messages", self.http.base_url);
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
    fn test_build_request_body_includes_tools_and_system() {
        let req = LlmRequest {
            model: "minimax".into(),
            system: "sys".into(),
            messages: vec![Message::user("hi")],
            tools: vec![super::super::ToolSpec {
                name: "bash".into(),
                description: "shell".into(),
                parameters: serde_json::json!({"type": "object"}),
            }],
            max_tokens: Some(100),
            temperature: 0.5,
        };
        let body = build_request_body(&req, true);
        assert_eq!(body["system"], "sys");
        assert_eq!(body["tools"][0]["name"], "bash");
        assert_eq!(body["messages"][0]["role"], "user");
    }
}
