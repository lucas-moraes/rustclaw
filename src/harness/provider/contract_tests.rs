//! Cross-provider contract tests (ROADMAP item 7).
//!
//! Each test drives the **same scenario** through every real HTTP adapter
//! (OpenAI-compatible and Anthropic-compatible) against a local mock server
//! (`wiremock`) and asserts that the resulting [`ProviderEvent`] streams are
//! equivalent up to provider-specific stop-reason vocabulary. This is the
//! parity guarantee the processor relies on: it must not need per-provider
//! special cases beyond the adapters themselves.
//!
//! Contract dimensions covered:
//! - **C1 event parity** — text-only reply produces the same event sequence
//!   (`TextDelta*` then `End`) on both adapters.
//! - **C2 tool calling** — a tool-call reply produces `ToolCallStart` +
//!   `ToolCallEnd` with identical id/name/arguments on both adapters.
//! - **C3 usage** — the `End` event carries token usage on both adapters.
//! - **C4 truncation vocabulary** — `is_truncated` normalizes `max_tokens`
//!   (Anthropic) and `length` (OpenAI) to the same answer.
//! - **C5 request shape** — both adapters send `tools` with JSON Schema
//!   parameters and the system prompt (the shared request contract).
//! - **C6 error mapping** — a 429 maps to a retryable provider error on both.

use std::sync::Arc;

use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::harness::provider::anthropic::AnthropicProvider;
use crate::harness::provider::openai::OpenAiProvider;
use crate::harness::provider::{
    is_truncated, AuthStyle, HttpConfig, LlmRequest, Provider, ProviderEvent,
};
use crate::harness::session::Message;

fn test_request() -> LlmRequest {
    LlmRequest {
        model: "test-model".into(),
        system: "you are a test".into(),
        messages: Arc::new(vec![Message::user("hello")]),
        tools: vec![crate::harness::tool::ToolSpec {
            name: "bash".into(),
            description: "run a shell command".into(),
            parameters: json!({
                "type": "object",
                "properties": {"command": {"type": "string"}},
                "required": ["command"]
            }),
        }],
        max_tokens: None,
        temperature: 0.5,
    }
}

fn http_config(base_url: &str) -> HttpConfig {
    HttpConfig {
        client: crate::harness::provider::build_http_client(),
        base_url: base_url.to_string(),
        api_key: "test-key".into(),
    }
}

fn openai_provider(base_url: &str) -> OpenAiProvider {
    OpenAiProvider {
        http: http_config(base_url),
        auth: AuthStyle::Bearer,
    }
}

fn anthropic_provider(base_url: &str) -> AnthropicProvider {
    AnthropicProvider {
        http: http_config(base_url),
        auth: AuthStyle::ApiKey,
        prompt_cache: false,
    }
}

async fn collect_stream(mut s: crate::harness::provider::ProviderStream) -> Vec<ProviderEvent> {
    use futures_util::StreamExt;
    let mut out = Vec::new();
    while let Some(ev) = s.next().await {
        out.push(ev.expect("stream event"));
    }
    out
}

/// Normalizes an event stream for comparison: collapses consecutive
/// `TextDelta`s into one string and pairs tool starts with their ends.
fn normalize(events: &[ProviderEvent]) -> (String, Vec<(String, String, String)>, Option<u64>) {
    let mut text = String::new();
    let mut tools = Vec::new();
    let mut output_tokens = None;
    for ev in events {
        match ev {
            ProviderEvent::TextDelta(t) => text.push_str(t),
            ProviderEvent::ToolCallStart { id, name } => {
                tools.push((id.clone(), name.clone(), String::new()))
            }
            ProviderEvent::ToolCallEnd { id, arguments } => {
                if let Some(entry) = tools.iter_mut().find(|(tid, _, _)| tid == id) {
                    entry.2 = arguments.clone();
                }
            }
            ProviderEvent::End { usage, .. } => {
                output_tokens = usage.map(|u| u.output_tokens);
            }
            _ => {}
        }
    }
    (text, tools, output_tokens)
}

// ─────────────────────────────────────────────────────────────────────────────
// C1 + C3: text-only reply parity
// ─────────────────────────────────────────────────────────────────────────────

/// OpenAI SSE for a plain text reply with usage.
fn openai_text_sse() -> Vec<String> {
    vec![
        json!({"choices":[{"delta":{"role":"assistant","content":"Hel"}}]}).to_string(),
        json!({"choices":[{"delta":{"content":"lo world"}}]}).to_string(),
        json!({"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5}}).to_string(),
        "[DONE]".to_string(),
    ]
}

/// Anthropic SSE for a plain text reply with usage.
fn anthropic_text_sse() -> Vec<String> {
    vec![
        json!({"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":1}}}).to_string(),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}).to_string(),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}).to_string(),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo world"}}).to_string(),
        json!({"type":"content_block_stop","index":0}).to_string(),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}).to_string(),
        json!({"type":"message_stop"}).to_string(),
    ]
}

fn sse_body(events: &[String]) -> String {
    let mut body = String::new();
    for e in events {
        body.push_str("data: ");
        body.push_str(e);
        body.push_str("\n\n");
    }
    body
}

#[tokio::test]
async fn contract_text_reply_parity() {
    // OpenAI
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse_body(&openai_text_sse()))
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;
    let p = openai_provider(&server.uri());
    let events = collect_stream(p.stream(&test_request()).await.unwrap()).await;
    let (text, tools, out_tokens) = normalize(&events);
    assert_eq!(text, "Hello world", "openai text");
    assert!(tools.is_empty(), "openai should emit no tool calls");
    assert_eq!(out_tokens, Some(5), "openai usage");

    // Anthropic
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse_body(&anthropic_text_sse()))
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;
    let p = anthropic_provider(&server.uri());
    let events = collect_stream(p.stream(&test_request()).await.unwrap()).await;
    let (text, tools, out_tokens) = normalize(&events);
    assert_eq!(text, "Hello world", "anthropic text");
    assert!(tools.is_empty(), "anthropic should emit no tool calls");
    assert_eq!(out_tokens, Some(5), "anthropic usage");
}

// ─────────────────────────────────────────────────────────────────────────────
// C2: tool calling parity
// ─────────────────────────────────────────────────────────────────────────────

fn openai_tool_sse() -> Vec<String> {
    vec![
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"bash","arguments":""}}]}}]}).to_string(),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"comm"}}]}}]}).to_string(),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"and\":\"ls\"}"}}]}}]}).to_string(),
        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}).to_string(),
        "[DONE]".to_string(),
    ]
}

fn anthropic_tool_sse() -> Vec<String> {
    vec![
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_1","name":"bash"}}).to_string(),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"comm"}}).to_string(),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"and\":\"ls\"}"}}).to_string(),
        json!({"type":"content_block_stop","index":0}).to_string(),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}).to_string(),
        json!({"type":"message_stop"}).to_string(),
    ]
}

#[tokio::test]
async fn contract_tool_call_parity() {
    let cases: Vec<(&str, String, Vec<String>)> = vec![
        ("openai", "/chat/completions".into(), openai_tool_sse()),
        ("anthropic", "/messages".into(), anthropic_tool_sse()),
    ];
    for (name, api_path, sse) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(api_path))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sse_body(&sse))
                    .append_header("content-type", "text/event-stream"),
            )
            .mount(&server)
            .await;
        let p: Box<dyn Provider> = match name {
            "openai" => Box::new(openai_provider(&server.uri())),
            _ => Box::new(anthropic_provider(&server.uri())),
        };
        let events = collect_stream(p.stream(&test_request()).await.unwrap()).await;
        let (text, tools, _) = normalize(&events);
        assert!(text.is_empty(), "{name}: tool-only reply has no text");
        assert_eq!(
            tools,
            vec![(
                "call_1".to_string(),
                "bash".to_string(),
                "{\"command\":\"ls\"}".to_string()
            )],
            "{name}: tool call id/name/args must match exactly"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C4: truncation vocabulary normalization
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn contract_truncation_vocabulary() {
    // Anthropic-style and OpenAI-style truncation reasons agree.
    assert!(is_truncated(Some("max_tokens")));
    assert!(is_truncated(Some("length")));
    assert!(is_truncated(Some("MAX_TOKENS")), "case-insensitive");
    // Non-truncation reasons agree too.
    assert!(!is_truncated(Some("end_turn")));
    assert!(!is_truncated(Some("stop")));
    assert!(!is_truncated(Some("tool_use")));
    assert!(!is_truncated(Some("tool_calls")));
    assert!(!is_truncated(None));
}

// ─────────────────────────────────────────────────────────────────────────────
// C5: request shape (tools + system present on the wire)
// ─────────────────────────────────────────────────────────────────────────────

async fn received_body(server: &MockServer) -> Value {
    let req = &server.received_requests().await.unwrap()[0];
    let text: String = req.body.iter().map(|&b| b as char).collect();
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

#[tokio::test]
async fn contract_request_shape_tools_and_system() {
    // OpenAI: system message + tools array with function/parameters.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({
            "model": "test-model",
            "stream": true,
            "tools": [{"type": "function", "function": {"name": "bash"}}]
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse_body(&openai_text_sse()))
                .append_header("content-type", "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let p = openai_provider(&server.uri());
    let _ = collect_stream(p.stream(&test_request()).await.unwrap()).await;
    let received = received_body(&server).await;
    assert_eq!(
        received["messages"][0]["role"], "system",
        "openai: system prompt is the first message"
    );
    assert_eq!(
        received["messages"][0]["content"], "you are a test",
        "openai: system prompt content"
    );
    assert_eq!(
        received["tools"][0]["function"]["parameters"]["properties"]["command"],
        json!({"type": "string"}),
        "openai: JSON Schema parameters forwarded"
    );

    // Anthropic: top-level `system` + tools with input_schema.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse_body(&anthropic_text_sse()))
                .append_header("content-type", "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let p = anthropic_provider(&server.uri());
    let _ = collect_stream(p.stream(&test_request()).await.unwrap()).await;
    let received = received_body(&server).await;
    assert_eq!(
        received["system"], "you are a test",
        "anthropic: system prompt is top-level"
    );
    assert_eq!(
        received["tools"][0]["name"], "bash",
        "anthropic: tool name forwarded"
    );
    assert_eq!(
        received["tools"][0]["input_schema"]["properties"]["command"],
        json!({"type": "string"}),
        "anthropic: JSON Schema forwarded as input_schema"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// C6: error mapping parity (429 → retryable)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn contract_rate_limit_error_is_retryable() {
    for (api_path, is_openai) in [("/chat/completions", true), ("/messages", false)] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(api_path))
            .respond_with(
                ResponseTemplate::new(429)
                    .append_header("retry-after", "3")
                    .set_body_string(r#"{"error":{"message":"slow down"}}"#),
            )
            .mount(&server)
            .await;
        let p: Box<dyn Provider> = if is_openai {
            Box::new(openai_provider(&server.uri()))
        } else {
            Box::new(anthropic_provider(&server.uri()))
        };
        let err = match p.stream(&test_request()).await {
            Err(e) => e,
            Ok(_) => panic!("{api_path}: expected 429 to fail the request"),
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("429") || msg.contains("rate") || msg.contains("API error"),
            "{api_path}: error should surface the status, got: {msg}"
        );
        // The error must be classified as retryable (RetryPolicy consumes this).
        let kind = crate::harness::provider::retry::error_retry_kind(&err);
        assert_eq!(
            kind,
            crate::harness::provider::retry::RetryKind::Retryable,
            "{api_path}: 429 must be retryable"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C7: prompt caching (Anthropic) — breakpoints on the wire + cache usage parsed
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn contract_prompt_caching_breakpoints_and_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse_body(&[
                    json!({"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":8,"cache_creation_input_tokens":100}}}).to_string(),
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}).to_string(),
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}).to_string(),
                    json!({"type":"content_block_stop","index":0}).to_string(),
                    json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}).to_string(),
                    json!({"type":"message_stop"}).to_string(),
                ]))
                .append_header("content-type", "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let mut p = anthropic_provider(&server.uri());
    p.prompt_cache = true;
    let events = collect_stream(p.stream(&test_request()).await.unwrap()).await;

    // Wire: system + last tool carry cache_control breakpoints.
    let received = received_body(&server).await;
    assert!(
        received["system"].is_array()
            && received["system"][0]["cache_control"]["type"] == "ephemeral",
        "anthropic: system breakpoint when prompt_cache is on"
    );
    let tools = received["tools"].as_array().unwrap();
    assert_eq!(
        tools.last().unwrap()["cache_control"]["type"],
        "ephemeral",
        "anthropic: last tool breakpoint"
    );

    // Stream: cache usage parsed from message_start.
    let end = events
        .iter()
        .find_map(|e| match e {
            ProviderEvent::End { usage, .. } => *usage,
            _ => None,
        })
        .expect("End event carries usage");
    assert_eq!(end.cache_read_tokens, 8, "cache_read parsed");
    assert_eq!(end.cache_write_tokens, 100, "cache_write parsed");
    assert_eq!(end.input_tokens, 10, "input excludes cache tokens");
}

// ─────────────────────────────────────────────────────────────────────────────
// C8: opencode-go router — same adapter contract, model-based routing
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn contract_opencode_go_routes_by_model() {
    use crate::harness::provider::opencode_go::OpenCodeGoProvider;

    // minimax → /messages (Anthropic route)
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse_body(&anthropic_text_sse()))
                .append_header("content-type", "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let p = OpenCodeGoProvider {
        http: http_config(&server.uri()),
        prompt_cache: false,
    };
    let mut req = test_request();
    req.model = "minimax-m2.7".into();
    let events = collect_stream(p.stream(&req).await.unwrap()).await;
    let (text, _, _) = normalize(&events);
    assert_eq!(text, "Hello world", "minimax routes to /messages");

    // non-minimax → /chat/completions (OpenAI route)
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse_body(&openai_text_sse()))
                .append_header("content-type", "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let p = OpenCodeGoProvider {
        http: http_config(&server.uri()),
        prompt_cache: false,
    };
    let mut req = test_request();
    req.model = "qwen3-coder".into();
    let events = collect_stream(p.stream(&req).await.unwrap()).await;
    let (text, _, _) = normalize(&events);
    assert_eq!(text, "Hello world", "qwen routes to /chat/completions");
}
