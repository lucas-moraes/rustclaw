//! Tool abstraction with JSON Schema parameters (native tool calling).

pub mod bash;
pub mod context;
pub mod diff;
pub mod edit;
pub mod fetch_webpage;
pub mod glob;
pub mod grep;
pub mod question;
pub mod read;
pub mod registry;
pub mod remember;
pub mod task;
pub mod todo;
pub mod truncate;
pub mod web_search;
pub mod write;

use crate::harness::session::ToolPart;
use serde_json::Value;

/// Result of a tool execution.
#[derive(Clone, Debug)]
pub struct ToolResult {
    /// Short human/model-friendly title (e.g. "ls src/", "edit main.rs").
    pub title: String,
    /// Full output returned to the model.
    pub output: String,
    /// Optional structured metadata for UI/events.
    pub metadata: Value,
}

impl ToolResult {
    pub fn simple(title: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            output: output.into(),
            metadata: Value::Null,
        }
    }
}

/// A harness tool with JSON Schema parameters.
/// Unlike the legacy text-ReAct `Tool`, these are advertised to the model
/// via native `tools` in the LLM request.
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// JSON Schema (draft 2020-12) describing the `input` object.
    fn parameters(&self) -> serde_json::Value;
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &context::ToolContext,
    ) -> Result<ToolResult, String>;
}

/// Wire-format spec sent to LLM providers.
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    pub fn from_tool(tool: &dyn Tool) -> Self {
        Self {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            parameters: tool.parameters(),
        }
    }
}

/// Builds a `ToolPart` (pending) for a completed provider tool call.
pub fn tool_part_from_call(id: &str, name: &str, arguments: &str) -> Result<ToolPart, String> {
    let input: serde_json::Value = if arguments.trim().is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(arguments)
            .map_err(|e| format!("invalid JSON arguments for tool `{}`: {}", name, e))?
    };
    Ok(ToolPart::pending(id, name, input))
}
