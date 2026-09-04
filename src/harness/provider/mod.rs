//! LLM provider abstraction with native tool calling.
//!
//! Each adapter converts harness `Message`/`Part` into its wire format and
//! streams back unified `ProviderEvent`s.

pub mod anthropic;
pub mod catalog;
pub mod openai;
pub mod opencode_go;
pub mod user_store;

use crate::harness::session::{Message, Part};

/// Tool definition sent to the provider (single definition, from tool module).
pub use crate::harness::tool::ToolSpec;

#[derive(Clone, Copy, Debug, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Usage {
    pub fn total(self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    pub fn add_assign(&mut self, other: Usage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
    }
}

/// Compact token count for status bars (`1.2k`, `45k`, `1.1M`).
pub fn format_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 10_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else if n < 1_000_000 {
        format!("{}k", n / 1_000)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;

    #[test]
    fn test_usage_add_and_total() {
        let mut a = Usage {
            input_tokens: 10,
            output_tokens: 5,
        };
        a.add_assign(Usage {
            input_tokens: 3,
            output_tokens: 7,
        });
        assert_eq!(a.input_tokens, 13);
        assert_eq!(a.output_tokens, 12);
        assert_eq!(a.total(), 25);
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(42), "42");
        assert_eq!(format_tokens(1500), "1.5k");
        assert_eq!(format_tokens(45_000), "45k");
        assert_eq!(format_tokens(1_200_000), "1.2M");
    }
}

/// Unified provider stream events.
#[derive(Clone, Debug)]
pub enum ProviderEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallDelta {
        id: String,
        args_delta: String,
    },
    /// Emitted when the complete arguments for a tool call are known.
    ToolCallEnd {
        id: String,
        /// Complete JSON string of arguments.
        arguments: String,
    },
    End {
        stop_reason: Option<String>,
        usage: Option<Usage>,
    },
}

pub type ProviderStream =
    Pin<Box<dyn futures_util::Stream<Item = Result<ProviderEvent, anyhow::Error>> + Send>>;

use std::pin::Pin;

/// LLM request in harness form; adapters convert messages/tools.
pub struct LlmRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    /// Optional cap on output tokens. `None` = omit the field so the
    /// provider/model applies its own default (except Anthropic, which
    /// requires the field — see `anthropic::build_request_body`).
    pub max_tokens: Option<usize>,
    pub temperature: f32,
}

pub struct LlmResponse {
    pub parts: Vec<Part>,
    pub usage: Option<Usage>,
    pub stop_reason: Option<String>,
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    /// Streaming request. Primary path for the processor.
    async fn stream(&self, req: &LlmRequest) -> anyhow::Result<ProviderStream>;
    /// Non-streaming fallback.
    async fn complete(&self, req: &LlmRequest) -> anyhow::Result<LlmResponse>;
}

/// HTTP client + auth shared by adapters.
#[derive(Clone)]
pub struct HttpConfig {
    pub client: reqwest::Client,
    pub base_url: String,
    pub api_key: String,
}

#[derive(Clone, Copy, Debug)]
pub enum AuthStyle {
    Bearer,
    ApiKey,
}

impl HttpConfig {
    /// Builds a POST request with provider-appropriate auth headers.
    pub fn post(&self, url: &str, auth: AuthStyle) -> reqwest::RequestBuilder {
        let mut rb = self.client.post(url);
        rb = match auth {
            AuthStyle::Bearer => rb.header("Authorization", format!("Bearer {}", self.api_key)),
            // opencode-go accepts/needs both headers; sending both is safe.
            AuthStyle::ApiKey => rb
                .header("X-API-Key", &self.api_key)
                .header("Authorization", format!("Bearer {}", self.api_key)),
        };
        rb.header("Content-Type", "application/json")
    }
}

/// Extracts complete `data: ...` SSE payloads from a byte-chunk stream.
///
/// Buffers until `\n\n` (event boundary) and yields each `data:` line content.
pub struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Feeds a chunk and returns any complete data payloads found.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(pos) = find_subslice(&self.buffer, b"\n\n") {
            let raw: Vec<u8> = self.buffer.drain(..pos + 2).collect();
            for line in String::from_utf8_lossy(&raw).lines() {
                let line = line.trim_start();
                if let Some(data) = line.strip_prefix("data:") {
                    events.push(data.trim_start().to_string());
                }
            }
        }
        events
    }

    /// Flushes any trailing event (no final newline).
    pub fn finish(&mut self) -> Vec<String> {
        let raw = std::mem::take(&mut self.buffer);
        let mut events = Vec::new();
        for line in String::from_utf8_lossy(&raw).lines() {
            let line = line.trim_start();
            if let Some(data) = line.strip_prefix("data:") {
                events.push(data.trim_start().to_string());
            }
        }
        events
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_parser_single_event() {
        let mut parser = SseParser::new();
        let events = parser.push(b"data: {\"a\":1}\n\n");
        assert_eq!(events, vec!["{\"a\":1}"]);
    }

    #[test]
    fn test_sse_parser_split_chunks() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"data: {\"a\":").is_empty());
        let events = parser.push(b"1}\n\ndata: [DONE]\n\n");
        assert_eq!(events, vec!["{\"a\":1}", "[DONE]"]);
    }

    #[test]
    fn test_sse_parser_finish_flushes() {
        let mut parser = SseParser::new();
        parser.push(b"data: hello\n");
        let events = parser.finish();
        assert_eq!(events, vec!["hello"]);
    }

    #[test]
    fn test_tool_spec_shape() {
        let spec = ToolSpec {
            name: "bash".into(),
            description: "run shell".into(),
            parameters: serde_json::json!({"type": "object"}),
        };
        assert_eq!(spec.name, "bash");
    }
}
