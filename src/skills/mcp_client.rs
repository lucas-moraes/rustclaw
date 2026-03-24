use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
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
}

impl McpClient {
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
            tools: HashMap::new(),
        }
    }

    pub fn add_server(&mut self, name: String, server: McpServer) {
        info!("Adding MCP server: {}", name);
        self.servers.insert(name, server);
    }

    pub fn list_servers(&self) -> Vec<String> {
        self.servers.keys().cloned().collect()
    }

    pub async fn list_tools(&mut self, server_name: &str) -> Result<Vec<McpTool>, String> {
        let server = self.servers.get(server_name)
            .ok_or_else(|| format!("Server '{}' not found", server_name))?;

        let mut cmd = Command::new(&server.command);
        cmd.args(&server.args);
        cmd.envs(&server.env);
        cmd.arg("tools/list");
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = cmd.output()
            .await
            .map_err(|e| format!("Failed to run MCP server: {}", e))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let tools: Vec<McpTool> = serde_json::from_str(&stdout)
                .map_err(|e| format!("Failed to parse tools: {}", e))?;
            self.tools.insert(server_name.to_string(), tools.clone());
            Ok(tools)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("MCP server error: {}", stderr))
        }
    }

    pub async fn call_tool(&self, server_name: &str, tool_name: &str, args: serde_json::Value) -> Result<String, String> {
        let server = self.servers.get(server_name)
            .ok_or_else(|| format!("Server '{}' not found", server_name))?;

        let request = serde_json::json!({
            "name": tool_name,
            "arguments": args
        });

        let request_str = serde_json::to_string(&request).unwrap();

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
            Ok(response["content"][0]["text"].as_str().unwrap_or("").to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("MCP server error: {}", stderr))
        }
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
