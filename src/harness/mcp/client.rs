//! MCP client wrapper over `rmcp` (stdio transport).

use anyhow::{Context, Result};
use rmcp::model::{CallToolRequestParams, ClientInfo, ContentBlock, Tool as RmcpTool};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use serde_json::Value;
use tokio::process::Command;

use crate::harness::mcp::config::McpServerConfig;

/// Timeout for the connection handshake (spawn + initialize + tools/list).
const CONNECT_TIMEOUT_SECS: u64 = 10;

/// A tool exposed by an MCP server.
#[derive(Debug, Clone)]
pub struct McpToolSpec {
    /// Original tool name on the server.
    pub name: String,
    /// Description (may be truncated).
    pub description: String,
    /// JSON Schema for the input arguments.
    pub input_schema: Value,
    /// `readOnlyHint` annotation (hint only).
    pub read_only: bool,
}

/// A connected MCP client.
pub struct McpClient {
    /// Server name (config key).
    pub server: String,
    service: tokio::sync::Mutex<RunningService<RoleClient, ClientInfo>>,
    tools: Vec<McpToolSpec>,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient")
            .field("server", &self.server)
            .field("tools", &self.tools.len())
            .finish()
    }
}

/// Serializes a `CallToolResult` content list into a single text blob.
fn content_to_text(content: &[ContentBlock]) -> String {
    let mut parts = Vec::new();
    for block in content {
        match block {
            ContentBlock::Text(t) => parts.push(t.text.clone()),
            ContentBlock::Image(img) => {
                parts.push(format!("[image: {}]", img.mime_type));
            }
            ContentBlock::Audio(a) => {
                parts.push(format!("[audio: {}]", a.mime_type));
            }
            ContentBlock::Resource(res) => {
                let uri = match &res.resource {
                    rmcp::model::ResourceContents::TextResourceContents { uri, .. } => uri.clone(),
                    rmcp::model::ResourceContents::BlobResourceContents { uri, .. } => uri.clone(),
                    _ => String::new(),
                };
                parts.push(format!("[resource: {uri}]"));
            }
            ContentBlock::ResourceLink(link) => {
                parts.push(format!("[link: {}]", link.uri));
            }
            _ => {}
        }
    }
    parts.join("\n")
}

impl McpClient {
    /// Spawns the server subprocess, performs the MCP handshake and lists tools.
    pub async fn connect(name: &str, cfg: &McpServerConfig) -> Result<Self> {
        cfg.validate()
            .with_context(|| format!("mcp server `{name}`"))?;
        let mut cfg = cfg.clone();
        cfg.expand_env()
            .with_context(|| format!("mcp server `{name}`"))?;

        let service = if !cfg.command.is_empty() {
            Self::connect_stdio(name, &cfg).await?
        } else {
            Self::connect_http(name, &cfg).await?
        };

        // List tools with a timeout.
        let list = tokio::time::timeout(
            std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS),
            service.list_all_tools(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("mcp server `{name}`: tools/list timed out"))?
        .map_err(|e| anyhow::anyhow!("mcp server `{name}`: tools/list failed: {e}"))?;

        let tools = list
            .into_iter()
            .map(|t: RmcpTool| McpToolSpec {
                name: t.name.to_string(),
                description: t
                    .description
                    .map(|d| truncate_chars(&d, 200))
                    .unwrap_or_default(),
                input_schema: serde_json::to_value(&*t.input_schema)
                    .unwrap_or_else(|_| Value::Object(Default::default())),
                read_only: t
                    .annotations
                    .as_ref()
                    .and_then(|a| a.read_only_hint)
                    .unwrap_or(false),
            })
            .collect();

        Ok(Self {
            server: name.to_string(),
            service: tokio::sync::Mutex::new(service),
            tools,
        })
    }

    async fn connect_http(
        name: &str,
        cfg: &McpServerConfig,
    ) -> Result<RunningService<RoleClient, ClientInfo>> {
        use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
        use rmcp::transport::StreamableHttpClientTransport;

        let mut config = StreamableHttpClientTransportConfig::with_uri(cfg.url.clone());
        if let Some(auth) = cfg.headers.get("Authorization") {
            config = config.auth_header(auth.clone());
        }
        let transport = StreamableHttpClientTransport::from_config(config);
        let client_info = ClientInfo::default();
        let service = tokio::time::timeout(
            std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS),
            client_info.serve(transport),
        )
        .await
        .map_err(|_| anyhow::anyhow!("mcp server `{name}`: initialize timed out"))?
        .map_err(|e| anyhow::anyhow!("mcp server `{name}`: initialize failed: {e}"))?;
        Ok(service)
    }

    async fn connect_stdio(
        name: &str,
        cfg: &McpServerConfig,
    ) -> Result<RunningService<RoleClient, ClientInfo>> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args);
        // Sanitize the inherited environment (strip secrets) and merge the
        // server's explicit `env` config on top (user-provided, always wins).
        let extra: std::collections::BTreeMap<String, String> = cfg
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        cmd.env_clear();
        cmd.envs(crate::harness::tool::env::sanitized_env(&extra));
        let transport = TokioChildProcess::new(cmd)
            .with_context(|| format!("mcp server `{name}`: failed to spawn `{}`", cfg.command))?;

        let client_info = ClientInfo::default();
        let service = tokio::time::timeout(
            std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS),
            client_info.serve(transport),
        )
        .await
        .map_err(|_| anyhow::anyhow!("mcp server `{name}`: initialize timed out"))?
        .map_err(|e| anyhow::anyhow!("mcp server `{name}`: initialize failed: {e}"))?;
        Ok(service)
    }

    /// Lightweight health probe (uses `tools/list`, cached by rmcp).
    pub async fn ping(&self) -> Result<()> {
        let service = self.service.lock().await;
        tokio::time::timeout(std::time::Duration::from_secs(10), service.list_all_tools())
            .await
            .map_err(|_| anyhow::anyhow!("health check timed out"))?
            .map_err(|e| anyhow::anyhow!("health check failed: {e}"))?;
        Ok(())
    }

    /// Tools exposed by this server.
    pub fn tools(&self) -> Vec<McpToolSpec> {
        self.tools.clone()
    }

    /// Calls a tool on the server with the configured timeout.
    pub async fn call_tool(&self, name: &str, args: Value, timeout_secs: u64) -> Result<String> {
        let arguments = match args {
            Value::Object(map) => Some(map),
            Value::Null => None,
            other => Some(
                other
                    .as_object()
                    .cloned()
                    .unwrap_or_else(|| [("value".to_string(), other)].into_iter().collect()),
            ),
        };
        let mut params = CallToolRequestParams::new(name.to_string());
        if let Some(args) = arguments {
            params = params.with_arguments(args);
        }
        let service = self.service.lock().await;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            service.call_tool(params),
        )
        .await
        .map_err(|_| anyhow::anyhow!("mcp tool `{name}` timed out after {timeout_secs}s"))?
        .map_err(|e| anyhow::anyhow!("mcp tool `{name}` failed: {e}"))?;

        if result.is_error.unwrap_or(false) {
            let text = content_to_text(&result.content);
            anyhow::bail!(
                "mcp tool `{name}` returned error: {}",
                truncate_chars(&text, 500)
            );
        }
        Ok(content_to_text(&result.content))
    }

    /// Gracefully shuts down the MCP session (closes the transport and
    /// terminates the server subprocess). Safe to call through an `Arc`;
    /// subsequent calls are no-ops (the service is already closed).
    pub async fn shutdown(&self) {
        let mut service = self.service.lock().await;
        let _ = service.close().await;
    }
}

/// Truncates a string to `max` chars (adding an ellipsis).
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_chars() {
        assert_eq!(truncate_chars("abc", 5), "abc");
        assert_eq!(truncate_chars("abcdef", 3), "abc…");
    }

    #[test]
    fn test_expand_env_ok() {
        std::env::set_var("RUSTCLAW_TEST_VAR_XYZ", "hello");
        let mut cfg = McpServerConfig {
            command: "echo".to_string(),
            args: vec!["${RUSTCLAW_TEST_VAR_XYZ}".to_string()],
            env: [("K".to_string(), "${RUSTCLAW_TEST_VAR_XYZ}-v".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        cfg.expand_env().unwrap();
        assert_eq!(cfg.args[0], "hello");
        assert_eq!(cfg.env["K"], "hello-v");
    }

    #[test]
    fn test_expand_env_missing_var() {
        let mut cfg = McpServerConfig {
            command: "echo".to_string(),
            args: vec!["${RUSTCLAW_TEST_MISSING_VAR_XYZ}".to_string()],
            ..Default::default()
        };
        let err = cfg.expand_env().unwrap_err();
        assert!(format!("{err:#}").contains("not set"));
    }

    #[test]
    fn test_content_to_text() {
        let blocks = vec![ContentBlock::text("hello"), ContentBlock::text("world")];
        assert_eq!(content_to_text(&blocks), "hello\nworld");
    }

    #[tokio::test]
    async fn test_connect_rejects_invalid_config() {
        let cfg = McpServerConfig::default();
        let err = McpClient::connect("x", &cfg).await.unwrap_err();
        assert!(format!("{err:#}").contains("one of `command` or `url`"));
    }

    #[tokio::test]
    async fn test_connect_fails_on_missing_command() {
        let cfg = McpServerConfig {
            command: "definitely-not-a-real-command-xyz".to_string(),
            ..Default::default()
        };
        let err = McpClient::connect("x", &cfg).await.unwrap_err();
        assert!(format!("{err:#}").contains("failed to spawn"));
    }
}
