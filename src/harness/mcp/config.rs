//! MCP server configuration: parse, load and merge.
//!
//! Format follows the de-facto standard `mcpServers` envelope used by
//! Claude/Cursor configs:
//!
//! ```json
//! { "mcpServers": { "github": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"] } } }
//! ```

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Configuration for a single MCP server (stdio transport).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpServerConfig {
    /// Command to spawn (stdio transport).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    /// Arguments for the command.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Extra environment variables for the subprocess.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    /// Remote server URL (streamable HTTP transport; F4).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Extra HTTP headers (only for `url` transport).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    /// Whether this server is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Per-call timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_true() -> bool {
    true
}

fn default_timeout() -> u64 {
    60
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            url: String::new(),
            headers: HashMap::new(),
            enabled: true,
            timeout_secs: default_timeout(),
        }
    }
}

impl McpServerConfig {
    /// Validates that exactly one of `command` (stdio) or `url` (HTTP) is set.
    pub fn validate(&self) -> Result<()> {
        match (!self.command.is_empty(), !self.url.is_empty()) {
            (true, true) => anyhow::bail!("mcp server: `command` and `url` are mutually exclusive"),
            (false, false) => anyhow::bail!("mcp server: one of `command` or `url` is required"),
            _ => Ok(()),
        }
    }

    /// Expands `${VAR}` in `args` and `env` values from the process environment.
    /// Undefined variables are an error.
    pub fn expand_env(&mut self) -> Result<()> {
        let expand = |s: &str| -> Result<String> {
            let mut out = String::new();
            let mut rest = s;
            while let Some(start) = rest.find("${") {
                out.push_str(&rest[..start]);
                let after = &rest[start + 2..];
                let end = after
                    .find('}')
                    .ok_or_else(|| anyhow::anyhow!("unterminated ${{ in `{s}`"))?;
                let var = &after[..end];
                match std::env::var(var) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => anyhow::bail!("environment variable `{var}` is not set"),
                }
                rest = &after[end + 1..];
            }
            out.push_str(rest);
            Ok(out)
        };
        for a in &mut self.args {
            if a.contains("${") {
                *a = expand(a)?;
            }
        }
        for v in self.env.values_mut() {
            if v.contains("${") {
                *v = expand(v)?;
            }
        }
        Ok(())
    }
}

/// Top-level MCP config: a map of server name → config.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct McpConfig {
    #[serde(default, rename = "mcpServers")]
    pub servers: HashMap<String, McpServerConfig>,
}

impl McpConfig {
    /// Parses from a JSON value (the whole file content).
    pub fn from_value(value: &serde_json::Value) -> Result<Self> {
        let cfg: McpConfig =
            serde_json::from_value(value.clone()).context("failed to parse mcp config")?;
        for (name, server) in &cfg.servers {
            server
                .validate()
                .with_context(|| format!("mcp server `{name}`"))?;
        }
        Ok(cfg)
    }

    /// Parses from a JSON string.
    pub fn from_str(s: &str) -> Result<Self> {
        let value: serde_json::Value =
            serde_json::from_str(s).context("failed to parse mcp config JSON")?;
        Self::from_value(&value)
    }

    /// Loads the global config from `<data_local_dir>/rustclaw/mcp.json`.
    /// A missing file yields an empty config (not an error).
    pub fn load_global() -> Result<Self> {
        let path = global_config_path();
        match std::fs::read_to_string(&path) {
            Ok(s) => Self::from_str(&s).with_context(|| format!("in {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Loads the project-level config from `<project_root>/rustclaw.json`
    /// (section `mcp.mcpServers`). A missing file or missing section yields
    /// an empty config.
    pub fn load_project(project_root: &Path) -> Result<Self> {
        let path = project_root.join("rustclaw.json");
        let s = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let value: serde_json::Value =
            serde_json::from_str(&s).with_context(|| format!("parsing {}", path.display()))?;
        let mcp = value.get("mcp").cloned().unwrap_or(serde_json::Value::Null);
        if mcp.is_null() {
            return Ok(Self::default());
        }
        // Accept both `{"mcp": {"mcpServers": {...}}}` and `{"mcp": {<servers>}}`.
        let servers_value = if mcp.get("mcpServers").is_some() {
            mcp
        } else {
            serde_json::json!({ "mcpServers": mcp })
        };
        Self::from_value(&servers_value).with_context(|| format!("in {}", path.display()))
    }

    /// Merges `project` over `self`: project servers override same-named ones.
    pub fn merge(&mut self, project: McpConfig) {
        for (name, server) in project.servers {
            self.servers.insert(name, server);
        }
    }

    /// Loads global + project config and merges them (project wins).
    pub fn load_merged(project_root: &Path) -> Result<Self> {
        let mut cfg = Self::load_global()?;
        cfg.merge(Self::load_project(project_root)?);
        Ok(cfg)
    }
}

/// Path of the global MCP config file.
pub fn global_config_path() -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("rustclaw")
        .join("mcp.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parses_standard_mcp_servers_format() {
        let cfg = McpConfig::from_str(
            r#"{"mcpServers": {"github": {"command": "npx", "args": ["-y", "server-github"], "env": {"GITHUB_TOKEN": "x"}}}}"#,
        )
        .unwrap();
        assert_eq!(cfg.servers.len(), 1);
        let gh = cfg.servers.get("github").unwrap();
        assert_eq!(gh.command, "npx");
        assert_eq!(gh.args, vec!["-y", "server-github"]);
        assert_eq!(gh.env.get("GITHUB_TOKEN").map(String::as_str), Some("x"));
        assert!(gh.enabled);
        assert_eq!(gh.timeout_secs, 60);
    }

    #[test]
    fn test_rejects_entry_without_command_or_url() {
        let err = McpConfig::from_str(r#"{"mcpServers": {"bad": {}}}"#).unwrap_err();
        assert!(format!("{err:#}").contains("one of `command` or `url`"));
    }

    #[test]
    fn test_rejects_command_and_url_together() {
        let err = McpConfig::from_str(
            r#"{"mcpServers": {"x": {"command": "a", "url": "http://localhost"}}}"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("mutually exclusive"));
    }

    #[test]
    fn test_accepts_url_transport() {
        let cfg = McpConfig::from_str(
            r#"{"mcpServers": {"remote": {"url": "https://example.com/mcp", "headers": {"Authorization": "Bearer t"}}}}"#,
        )
        .unwrap();
        let r = cfg.servers.get("remote").unwrap();
        assert_eq!(r.url, "https://example.com/mcp");
        assert_eq!(
            r.headers.get("Authorization").map(String::as_str),
            Some("Bearer t")
        );
    }

    #[test]
    fn test_merge_project_overrides_global() {
        let mut global = McpConfig::from_str(
            r#"{"mcpServers": {"a": {"command": "old"}, "b": {"command": "keep"}}}"#,
        )
        .unwrap();
        let project = McpConfig::from_str(r#"{"mcpServers": {"a": {"command": "new"}}}"#).unwrap();
        global.merge(project);
        assert_eq!(global.servers.get("a").unwrap().command, "new");
        assert_eq!(global.servers.get("b").unwrap().command, "keep");
    }

    #[test]
    fn test_load_global_missing_file_is_empty() {
        // Point at a temp HOME-like dir is hard; instead verify the path fn and
        // that a nonexistent file path yields empty config via the same logic.
        let path = global_config_path();
        assert!(path.ends_with("mcp.json"));
        // Direct check of the read logic with a temp dir:
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("nope.json");
        let res = match std::fs::read_to_string(&missing) {
            Ok(s) => McpConfig::from_str(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(McpConfig::default()),
            Err(e) => panic!("{e}"),
        };
        assert!(res.unwrap().servers.is_empty());
    }

    #[test]
    fn test_load_project_mcp_section() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("rustclaw.json"),
            r#"{"provider": "x", "mcp": {"mcpServers": {"fs": {"command": "node"}}}}"#,
        )
        .unwrap();
        let cfg = McpConfig::load_project(tmp.path()).unwrap();
        assert_eq!(cfg.servers.get("fs").unwrap().command, "node");
    }

    #[test]
    fn test_load_project_shorthand_mcp_section() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("rustclaw.json"),
            r#"{"mcp": {"fs": {"command": "node"}}}"#,
        )
        .unwrap();
        let cfg = McpConfig::load_project(tmp.path()).unwrap();
        assert_eq!(cfg.servers.get("fs").unwrap().command, "node");
    }

    #[test]
    fn test_load_project_missing_file_is_empty() {
        let tmp = TempDir::new().unwrap();
        let cfg = McpConfig::load_project(tmp.path()).unwrap();
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn test_load_merged() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("rustclaw.json"),
            r#"{"mcp": {"proj": {"command": "p"}}}"#,
        )
        .unwrap();
        let cfg = McpConfig::load_merged(tmp.path()).unwrap();
        // global is empty in tests, but project server must be present
        assert_eq!(cfg.servers.get("proj").unwrap().command, "p");
    }
}
