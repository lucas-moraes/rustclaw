//! `McpTool`: adapts an MCP server tool to the harness `Tool` trait.

use std::sync::Arc;

use serde_json::Value;

use crate::harness::mcp::client::{McpClient, McpToolSpec};
use crate::harness::tool::context::ToolContext;
use crate::harness::tool::{Tool, ToolResult};

/// A harness tool backed by a remote MCP server tool.
pub struct McpTool {
    /// Server name (config key).
    pub server: String,
    spec: McpToolSpec,
    client: Arc<McpClient>,
    manager: Option<Arc<crate::harness::mcp::McpManager>>,
    /// Registry name: `mcp_<server>_<tool>`.
    registry_name: String,
    /// Per-call timeout (from server config).
    timeout_secs: u64,
}

/// Sanitizes a name component to `[a-z0-9_]`.
fn sanitize(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        out.push('x');
    }
    out
}

impl McpTool {
    pub fn new(server: String, spec: McpToolSpec, client: Arc<McpClient>) -> Self {
        let registry_name = format!("mcp_{}_{}", sanitize(&server), sanitize(&spec.name));
        Self {
            server,
            spec,
            client,
            manager: None,
            registry_name,
            timeout_secs: 60,
        }
    }

    /// Attaches the manager (enables reconnect-on-failure).
    pub fn with_manager(mut self, manager: Arc<crate::harness::mcp::McpManager>) -> Self {
        self.manager = Some(manager);
        self
    }

    /// Sets the per-call timeout (from the server config).
    pub fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = timeout_secs;
        self
    }
}

#[async_trait::async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.registry_name
    }

    fn description(&self) -> &str {
        &self.spec.description
    }

    fn parameters(&self) -> Value {
        self.spec.input_schema.clone()
    }

    fn read_only(&self) -> bool {
        self.spec.read_only
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        if ctx.abort.is_aborted() {
            return Err("aborted".to_string());
        }
        let result = match &self.manager {
            Some(mgr) => mgr.call_tool(&self.server, &self.spec.name, args).await,
            None => {
                self.client
                    .call_tool(&self.spec.name, args, self.timeout_secs)
                    .await
            }
        };
        let output = result.map_err(|e| e.to_string())?;
        Ok(ToolResult {
            title: format!("mcp {} {}", self.server, self.spec.name),
            output,
            metadata: serde_json::json!({ "mcp_server": self.server, "mcp_tool": self.spec.name }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize() {
        assert_eq!(sanitize("GitHub"), "github");
        assert_eq!(sanitize("create-issue"), "create_issue");
        assert_eq!(sanitize(""), "x");
    }

    #[test]
    fn test_registry_name_prefix() {
        // Build a minimal McpTool without a live client by constructing fields
        // through the public constructor with a dummy spec.
        let spec = McpToolSpec {
            name: "List Files".to_string(),
            description: "d".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            read_only: true,
        };
        // We can't easily build a McpClient in a unit test; test sanitize path
        // via the name formatting logic instead.
        let server = sanitize("filesystem");
        let tool = sanitize("List Files");
        assert_eq!(format!("mcp_{server}_{tool}"), "mcp_filesystem_list_files");
        let _ = spec; // silence unused in this test shape
    }
}
