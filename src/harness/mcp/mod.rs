//! MCP (Model Context Protocol) client support.
//!
//! Connects to configured MCP servers (stdio or streamable HTTP), exposes
//! their tools as native `Tool`s in the registry with names
//! `mcp_<server>_<tool>`.

pub mod client;
pub mod config;
pub mod tool;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::harness::mcp::client::McpClient;
use crate::harness::mcp::config::McpConfig;
use crate::harness::mcp::tool::McpTool;

/// Connection status of a single MCP server.
#[derive(Debug, Clone, PartialEq)]
pub enum McpServerStatus {
    Connected,
    Failed(String),
    Disabled,
}

/// Maximum tools exposed per server (protects the system prompt).
const MAX_TOOLS_PER_SERVER: usize = 50;

/// Manages the lifecycle of all configured MCP servers.
#[derive(Default)]
pub struct McpManager {
    /// name → client (None until connected / after failure).
    clients: RwLock<HashMap<String, Arc<McpClient>>>,
    status: RwLock<HashMap<String, McpServerStatus>>,
    config: RwLock<McpConfig>,
}

impl McpManager {
    /// Connects to all enabled servers in parallel. A failing server is
    /// logged and marked `Failed`, but never blocks the others or startup.
    pub async fn connect_all(config: McpConfig) -> Arc<Self> {
        let manager = Arc::new(Self {
            clients: RwLock::new(HashMap::new()),
            status: RwLock::new(HashMap::new()),
            config: RwLock::new(config),
        });
        manager.reconnect_all().await;
        manager
    }

    /// (Re)connects every enabled server in parallel.
    pub async fn reconnect_all(&self) {
        let cfg = self.config.read().await.clone();
        let mut set = tokio::task::JoinSet::new();
        for (name, server) in &cfg.servers {
            if !server.enabled {
                self.status
                    .write()
                    .await
                    .insert(name.clone(), McpServerStatus::Disabled);
                continue;
            }
            let name = name.clone();
            let server = server.clone();
            set.spawn(async move {
                let res = McpClient::connect(&name, &server).await;
                (name, res)
            });
        }
        while let Some(Ok((name, res))) = set.join_next().await {
            match res {
                Ok(client) => {
                    self.clients
                        .write()
                        .await
                        .insert(name.clone(), Arc::new(client));
                    self.status
                        .write()
                        .await
                        .insert(name, McpServerStatus::Connected);
                }
                Err(e) => {
                    eprintln!("[warn] mcp server `{name}` failed: {e:#}");
                    self.status
                        .write()
                        .await
                        .insert(name, McpServerStatus::Failed(e.to_string()));
                }
            }
        }
    }

    /// Starts a background health-check task: probes each connected server
    /// every 60s and marks failures (visible in `/mcp status`).
    pub fn start_health_checks(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let mgr = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            interval.tick().await; // skip immediate tick
            loop {
                interval.tick().await;
                let clients: Vec<(String, Arc<McpClient>)> = {
                    let c = mgr.clients.read().await;
                    c.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                };
                for (name, client) in clients {
                    if client.ping().await.is_err() {
                        mgr.status.write().await.insert(
                            name.clone(),
                            McpServerStatus::Failed("health check failed".to_string()),
                        );
                    }
                }
            }
        })
    }

    /// Restarts a single server by name.
    pub async fn restart(&self, name: &str) -> Result<()> {
        let server = self
            .config
            .read()
            .await
            .servers
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown mcp server `{name}`"))?;
        if !server.enabled {
            anyhow::bail!("mcp server `{name}` is disabled");
        }
        let res = McpClient::connect(name, &server).await;
        match res {
            Ok(client) => {
                // Terminate the previous client (closes transport + subprocess)
                // before swapping in the fresh one.
                let old = self
                    .clients
                    .write()
                    .await
                    .insert(name.to_string(), Arc::new(client));
                if let Some(old) = old {
                    old.shutdown().await;
                }
                self.status
                    .write()
                    .await
                    .insert(name.to_string(), McpServerStatus::Connected);
                Ok(())
            }
            Err(e) => {
                self.status
                    .write()
                    .await
                    .insert(name.to_string(), McpServerStatus::Failed(e.to_string()));
                Err(e)
            }
        }
    }

    /// All tools exposed by connected servers, as registry-ready `McpTool`s.
    pub async fn tools(self: &Arc<Self>) -> Vec<Arc<McpTool>> {
        let clients = self.clients.read().await;
        let cfg = self.config.read().await;
        let mut tools = Vec::new();
        for (name, client) in clients.iter() {
            let timeout = cfg.servers.get(name).map(|s| s.timeout_secs).unwrap_or(60);
            for spec in client.tools() {
                if tools
                    .iter()
                    .filter(|t: &&Arc<McpTool>| t.server == *name)
                    .count()
                    >= MAX_TOOLS_PER_SERVER
                {
                    eprintln!(
                        "[warn] mcp server `{name}` exposes more than {MAX_TOOLS_PER_SERVER} tools; truncating"
                    );
                    break;
                }
                let tool = Arc::new(
                    McpTool::new(name.clone(), spec, client.clone())
                        .with_timeout(timeout)
                        .with_manager(self.clone()),
                );
                tools.push(tool);
            }
        }
        tools
    }

    /// Calls a tool on a server, with one reconnect attempt on transport failure.
    pub async fn call_tool(&self, server: &str, tool: &str, args: Value) -> Result<String> {
        let (client, timeout) = {
            let clients = self.clients.read().await;
            let cfg = self.config.read().await;
            let client = clients
                .get(server)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("mcp server `{server}` is not connected"))?;
            let timeout = cfg
                .servers
                .get(server)
                .map(|s| s.timeout_secs)
                .unwrap_or(60);
            (client, timeout)
        };
        match client.call_tool(tool, args.clone(), timeout).await {
            Ok(out) => Ok(out),
            Err(first_err) => {
                // Transport may have died: try one reconnect + retry.
                eprintln!(
                    "[warn] mcp server `{server}` call failed ({first_err:#}); attempting reconnect"
                );
                self.restart(server).await.map_err(|e| {
                    anyhow::anyhow!(
                        "mcp tool `{tool}` failed: {first_err:#}; reconnect failed: {e:#}"
                    )
                })?;
                let clients = self.clients.read().await;
                let client = clients.get(server).cloned().ok_or_else(|| {
                    anyhow::anyhow!("mcp server `{server}` missing after reconnect")
                })?;
                client.call_tool(tool, args, timeout).await
            }
        }
    }

    /// Snapshot of per-server status (for `/mcp status`).
    pub async fn status_snapshot(&self) -> Vec<(String, McpServerStatus)> {
        let mut v: Vec<_> = self
            .status
            .read()
            .await
            .iter()
            .map(|(k, s)| (k.clone(), s.clone()))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// Configured servers (name, command/url, enabled) — for `/mcp list`.
    pub async fn configured(&self) -> Vec<(String, String, bool)> {
        let cfg = self.config.read().await;
        let mut v: Vec<_> = cfg
            .servers
            .iter()
            .map(|(k, s)| {
                let target = if !s.command.is_empty() {
                    s.command.clone()
                } else {
                    s.url.clone()
                };
                (k.clone(), target, s.enabled)
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::mcp::config::McpServerConfig;

    #[tokio::test]
    async fn test_connect_all_marks_failed_server_without_blocking() {
        let mut cfg = McpConfig::default();
        cfg.servers.insert(
            "bad".to_string(),
            McpServerConfig {
                command: "definitely-not-a-real-command-xyz".to_string(),
                ..Default::default()
            },
        );
        let mgr = McpManager::connect_all(cfg).await;
        let status = mgr.status_snapshot().await;
        assert!(matches!(status[0].1, McpServerStatus::Failed(_)));
        assert!(mgr.tools().await.is_empty());
    }

    #[tokio::test]
    async fn test_disabled_server_is_disabled() {
        let mut cfg = McpConfig::default();
        cfg.servers.insert(
            "off".to_string(),
            McpServerConfig {
                command: "true".to_string(),
                enabled: false,
                ..Default::default()
            },
        );
        let mgr = McpManager::connect_all(cfg).await;
        assert_eq!(mgr.status_snapshot().await[0].1, McpServerStatus::Disabled);
    }

    #[tokio::test]
    async fn test_restart_unknown_server_errors() {
        let mgr = McpManager::connect_all(McpConfig::default()).await;
        assert!(mgr.restart("nope").await.is_err());
    }

    #[tokio::test]
    async fn test_configured_lists_servers() {
        let mut cfg = McpConfig::default();
        cfg.servers.insert(
            "fs".to_string(),
            McpServerConfig {
                command: "node".to_string(),
                ..Default::default()
            },
        );
        let mgr = McpManager::connect_all(cfg).await;
        let configured = mgr.configured().await;
        assert_eq!(configured[0].0, "fs");
        assert_eq!(configured[0].1, "node");
        assert!(configured[0].2);
    }
}
