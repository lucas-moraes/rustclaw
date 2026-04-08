#![allow(dead_code)]

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub transport: McpTransport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum McpTransport {
    #[default]
    Stdio,
    Http {
        url: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    pub mime_type: Option<String>,
}

pub struct McpClient {
    servers: HashMap<String, McpServer>,
    tools: HashMap<String, Vec<McpTool>>,
    http_client: Client,
}

impl McpClient {
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
            tools: HashMap::new(),
            http_client: Client::new(),
        }
    }

    pub fn add_server(&mut self, name: String, server: McpServer) {
        info!("Adding MCP server: {}", name);
        self.servers.insert(name, server);
    }

    pub fn add_http_server(&mut self, name: String, url: String) {
        let server = McpServer {
            name: name.clone(),
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            transport: McpTransport::Http { url: url.clone() },
        };
        info!("Adding MCP HTTP server: {} at {}", name, url);
        self.servers.insert(name, server);
    }

    pub fn list_servers(&self) -> Vec<String> {
        self.servers.keys().cloned().collect()
    }

    pub async fn list_tools(&mut self, server_name: &str) -> Result<Vec<McpTool>, String> {
        let server = match self.servers.get(server_name) {
            Some(s) => s,
            None => return Err(format!("Server '{}' not found", server_name)),
        };

        let server = std::sync::Arc::new(server.clone());
        let server_name_owned = server_name.to_string();

        let tools = match server.transport {
            McpTransport::Stdio => self.list_tools_stdio(&server).await?,
            McpTransport::Http { ref url } => self.list_tools_http(&server_name_owned, url).await?,
        };

        Ok(tools)
    }

    async fn list_tools_stdio(&mut self, server: &McpServer) -> Result<Vec<McpTool>, String> {
        let mut cmd = Command::new(&server.command);
        cmd.args(&server.args);
        cmd.envs(&server.env);
        cmd.arg("tools/list");
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = cmd
            .output()
            .await
            .map_err(|e| format!("Failed to run MCP server: {}", e))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let tools: Vec<McpTool> = serde_json::from_str(&stdout)
                .map_err(|e| format!("Failed to parse tools: {}", e))?;
            Ok(tools)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("MCP server error: {}", stderr))
        }
    }

    async fn list_tools_http(
        &mut self,
        server_name: &str,
        url: &str,
    ) -> Result<Vec<McpTool>, String> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        });

        let response = self
            .http_client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("HTTP error: {}", response.status()));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        let tools: Vec<McpTool> = serde_json::from_value(
            json.get("result")
                .and_then(|r| r.get("tools"))
                .cloned()
                .unwrap_or(serde_json::Value::Array(vec![])),
        )
        .map_err(|e| format!("Failed to parse tools: {}", e))?;

        self.tools.insert(server_name.to_string(), tools.clone());
        Ok(tools)
    }

    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<String, String> {
        let server = self
            .servers
            .get(server_name)
            .ok_or_else(|| format!("Server '{}' not found", server_name))?;

        let transport = server.transport.clone();

        match transport {
            McpTransport::Stdio => self.call_tool_stdio(server, tool_name, args).await,
            McpTransport::Http { url } => self.call_tool_http(&url, tool_name, args).await,
        }
    }

    async fn call_tool_stdio(
        &self,
        server: &McpServer,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<String, String> {
        let _request = serde_json::json!({
            "name": tool_name,
            "arguments": args
        });

        let output = Command::new(&server.command)
            .args(&server.args)
            .envs(&server.env)
            .arg("tools/call")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn MCP server: {}", e))?
            .wait_with_output()
            .await
            .map_err(|e| format!("Failed to run MCP server: {}", e))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let response: serde_json::Value = serde_json::from_str(&stdout)
                .map_err(|e| format!("Failed to parse response: {}", e))?;
            Ok(response["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("MCP server error: {}", stderr))
        }
    }

    async fn call_tool_http(
        &self,
        url: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<String, String> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": args
            }
        });

        let response = self
            .http_client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("HTTP error: {}", response.status()));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        json.get("result")
            .and_then(|r| r.get("content"))
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|o| o.get("text"))
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Invalid response format".to_string())
    }

    pub fn get_tools(&self, server_name: &str) -> Option<&Vec<McpTool>> {
        self.tools.get(server_name)
    }

    pub fn get_all_tools(&self) -> HashMap<String, Vec<McpTool>> {
        self.tools.clone()
    }
}

impl Default for McpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_client() {
        let client = McpClient::new();
        assert!(client.list_servers().is_empty());
    }
}
