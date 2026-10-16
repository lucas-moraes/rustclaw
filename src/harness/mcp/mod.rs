//! MCP (Model Context Protocol) client support.
//!
//! Connects to configured MCP servers (stdio or streamable HTTP), exposes
//! their tools as native `Tool`s in the registry with names
//! `mcp_<server>_<tool>`.

pub mod client;
pub mod config;
pub mod tool;
pub mod trust;

pub use trust::{fingerprint, short, McpTrustStore};

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
    /// Enabled but its config fingerprint is not in the trust store; the
    /// server was not started (fail-closed). Approve with `/mcp trust <name>`.
    PendingTrust,
}

/// Maximum tools exposed per server (protects the system prompt).
const MAX_TOOLS_PER_SERVER: usize = 50;

/// Manages the lifecycle of all configured MCP servers.
pub struct McpManager {
    /// name → client (None until connected / after failure).
    clients: RwLock<HashMap<String, Arc<McpClient>>>,
    status: RwLock<HashMap<String, McpServerStatus>>,
    config: RwLock<McpConfig>,
    /// Trust store gating which servers may be started (fail-closed).
    trust: McpTrustStore,
}

impl McpManager {
    /// Connects to all enabled servers in parallel. A failing server is
    /// logged and marked `Failed`, but never blocks the others or startup.
    ///
    /// Servers whose config fingerprint is not in `trust` are **not**
    /// started: they are marked `PendingTrust` (fail-closed, protects
    /// against a malicious project `rustclaw.json`). Approve with
    /// `/mcp trust <name>`.
    pub async fn connect_all(config: McpConfig, trust: McpTrustStore) -> Arc<Self> {
        let manager = Arc::new(Self {
            clients: RwLock::new(HashMap::new()),
            status: RwLock::new(HashMap::new()),
            config: RwLock::new(config),
            trust,
        });
        manager.reconnect_all().await;
        manager
    }

    /// (Re)connects every enabled server in parallel.
    ///
    /// Untrusted servers are skipped and marked `PendingTrust`.
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
            if !self.trust.is_config_trusted(name, server) {
                self.status
                    .write()
                    .await
                    .insert(name.clone(), McpServerStatus::PendingTrust);
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
                    // Safe on stderr: `reconnect_all` runs from `connect_all` during
                    // `init_mcp` before the TUI enters the alternate screen.
                    eprintln!("[warn] mcp server `{name}` failed: {e:#}");
                    self.status
                        .write()
                        .await
                        .insert(name, McpServerStatus::Failed(e.to_string()));
                }
            }
        }
    }

    /// Trusts `name`'s current config fingerprint and connects it now
    /// (late approval from `/mcp trust`). Returns `Ok(())` on success.
    pub async fn connect_one(&self, name: &str) -> anyhow::Result<()> {
        let server = {
            let cfg = self.config.read().await;
            cfg.servers
                .get(name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("unknown mcp server `{name}`"))?
        };
        if !server.enabled {
            anyhow::bail!("mcp server `{name}` is disabled");
        }
        self.trust
            .trust(&crate::harness::mcp::fingerprint(name, &server))?;
        let res = McpClient::connect(name, &server).await;
        match res {
            Ok(client) => {
                self.clients
                    .write()
                    .await
                    .insert(name.to_string(), Arc::new(client));
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

    /// Revokes trust in `name`'s config and disconnects it.
    pub async fn untrust_one(&self, name: &str) -> anyhow::Result<()> {
        let fp = {
            let cfg = self.config.read().await;
            let server = cfg
                .servers
                .get(name)
                .ok_or_else(|| anyhow::anyhow!("unknown mcp server `{name}`"))?;
            crate::harness::mcp::fingerprint(name, server)
        };
        self.trust.untrust(&fp)?;
        self.clients.write().await.remove(name);
        self.status
            .write()
            .await
            .insert(name.to_string(), McpServerStatus::PendingTrust);
        Ok(())
    }

    /// The trust store backing this manager (for `/mcp trust`).
    pub fn trust_store(&self) -> &McpTrustStore {
        &self.trust
    }

    /// The config of `name`, if configured.
    pub async fn server_config(
        &self,
        name: &str,
    ) -> Option<crate::harness::mcp::config::McpServerConfig> {
        self.config.read().await.servers.get(name).cloned()
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
                    tracing::warn!(
                        "mcp server `{name}` exposes more than {MAX_TOOLS_PER_SERVER} tools; truncating"
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
                tracing::warn!(
                    "mcp server `{server}` call failed ({first_err:#}); attempting reconnect"
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

    /// Temp-dir trust store with the given servers pre-trusted.
    fn trusted_store(servers: &[(&str, &McpServerConfig)]) -> (tempfile::TempDir, McpTrustStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = McpTrustStore::at(dir.path());
        for (name, cfg) in servers {
            store
                .trust(&crate::harness::mcp::fingerprint(name, cfg))
                .expect("trust");
        }
        (dir, store)
    }

    #[tokio::test]
    async fn test_connect_all_marks_failed_server_without_blocking() {
        let mut cfg = McpConfig::default();
        let bad = McpServerConfig {
            command: "definitely-not-a-real-command-xyz".to_string(),
            ..Default::default()
        };
        cfg.servers.insert("bad".to_string(), bad.clone());
        let (_dir, store) = trusted_store(&[("bad", &bad)]);
        let mgr = McpManager::connect_all(cfg, store).await;
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
        let (_dir, store) = trusted_store(&[]);
        let mgr = McpManager::connect_all(cfg, store).await;
        assert_eq!(mgr.status_snapshot().await[0].1, McpServerStatus::Disabled);
    }

    #[tokio::test]
    async fn test_restart_unknown_server_errors() {
        let (_dir, store) = trusted_store(&[]);
        let mgr = McpManager::connect_all(McpConfig::default(), store).await;
        assert!(mgr.restart("nope").await.is_err());
    }

    #[tokio::test]
    async fn test_configured_lists_servers() {
        let mut cfg = McpConfig::default();
        let fs = McpServerConfig {
            command: "node".to_string(),
            ..Default::default()
        };
        cfg.servers.insert("fs".to_string(), fs.clone());
        let (_dir, store) = trusted_store(&[("fs", &fs)]);
        let mgr = McpManager::connect_all(cfg, store).await;
        let configured = mgr.configured().await;
        assert_eq!(configured[0].0, "fs");
        assert_eq!(configured[0].1, "node");
        assert!(configured[0].2);
    }

    /// F2.1: an untrusted server must never spawn — it is marked
    /// `PendingTrust` and no client is created.
    #[tokio::test]
    async fn test_untrusted_server_is_pending_not_spawned() {
        let mut cfg = McpConfig::default();
        cfg.servers.insert(
            "evil".to_string(),
            McpServerConfig {
                command: "definitely-not-a-real-command-xyz".to_string(),
                ..Default::default()
            },
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let store = McpTrustStore::at(dir.path());
        let mgr = McpManager::connect_all(cfg, store).await;
        let status = mgr.status_snapshot().await;
        assert_eq!(status[0].1, McpServerStatus::PendingTrust);
        assert!(mgr.tools().await.is_empty());
        assert!(mgr.clients.read().await.is_empty());
    }

    /// F2.1: `connect_one` (late approval) trusts the fingerprint and
    /// spawns the server.
    #[tokio::test]
    async fn test_connect_one_after_trust() {
        let mut cfg = McpConfig::default();
        cfg.servers.insert(
            "late".to_string(),
            McpServerConfig {
                command: "definitely-not-a-real-command-xyz".to_string(),
                ..Default::default()
            },
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let store = McpTrustStore::at(dir.path());
        let mgr = McpManager::connect_all(cfg, store.clone()).await;
        assert_eq!(
            mgr.status_snapshot().await[0].1,
            McpServerStatus::PendingTrust
        );
        // Approval trusts the fingerprint; the spawn itself fails (bad
        // command) but the status must move off PendingTrust.
        let res = mgr.connect_one("late").await;
        assert!(res.is_err());
        assert!(store.is_config_trusted(
            "late",
            &McpServerConfig {
                command: "definitely-not-a-real-command-xyz".to_string(),
                ..Default::default()
            }
        ));
        assert!(!matches!(
            mgr.status_snapshot().await[0].1,
            McpServerStatus::PendingTrust
        ));
    }

    /// F2.1: changing the config invalidates the trust (new fingerprint).
    #[tokio::test]
    async fn test_config_change_invalidates_trust() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = McpTrustStore::at(dir.path());
        let v1 = McpServerConfig {
            command: "node".to_string(),
            ..Default::default()
        };
        store.trust(&fingerprint("srv", &v1)).expect("trust v1");
        assert!(store.is_config_trusted("srv", &v1));
        let v2 = McpServerConfig {
            command: "node".to_string(),
            args: vec!["-e".into(), "evil()".into()],
            ..Default::default()
        };
        assert!(!store.is_config_trusted("srv", &v2));
    }
}
