//! Permission engine: allow / ask / deny rules per tool, with path escalation.
//!
//! Defaults mirror OpenCode/Claude Code conventions:
//! - read-only tools (read/glob/grep/todo_read) => Allow
//! - mutating tools (write/edit/bash) => Ask
//! - paths outside the session cwd => escalated to Ask even for read tools.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Request shown to the user when a tool needs approval.
#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub id: String,
    pub session_id: String,
    pub tool: String,
    pub args_summary: String,
    pub path: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionDecision {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rule {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PermissionConfig {
    /// Per-tool rules, e.g. { "bash": "ask", "edit": "allow" }
    #[serde(default)]
    pub tools: HashMap<String, Rule>,
    /// Wildcard default, e.g. "*": "ask"
    #[serde(default)]
    pub default: Option<Rule>,
}

impl PermissionConfig {
    /// Parses a project config file (JSON subset, opencode-like `permission` object).
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(value.get("permission")?.clone()).ok()
    }
}

pub struct PermissionEngine {
    rules: HashMap<String, Rule>,
    default: Option<Rule>,
    /// "always allow" decisions cached per session run.
    always_allow: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl Default for PermissionEngine {
    fn default() -> Self {
        Self::from_config(&PermissionConfig::with_defaults())
    }
}

impl PermissionConfig {
    /// Sensible defaults matching the TODO F3 spec.
    pub fn with_defaults() -> Self {
        let mut tools = HashMap::new();
        for t in ["read", "glob", "grep", "todo_read", "todo_write"] {
            tools.insert(t.to_string(), Rule::Allow);
        }
        for t in ["write", "edit", "bash", "task", "question", "remember"] {
            tools.insert(t.to_string(), Rule::Ask);
        }
        Self {
            tools,
            default: Some(Rule::Ask),
        }
    }
}

impl PermissionEngine {
    pub fn from_config(config: &PermissionConfig) -> Self {
        Self {
            rules: config.tools.clone(),
            default: config.default,
            always_allow: std::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    pub fn set_always_allow(&self, tool: &str) {
        self.always_allow.lock().unwrap().insert(tool.to_string());
    }

    /// Resolves the decision for `tool` with optional `path` (absolute).
    /// Escalates Allow => Ask when the path escapes the session cwd.
    pub fn check(&self, tool: &str, path: Option<&str>, cwd: &Path) -> PermissionDecision {
        // Explicit "always allow" wins for this run.
        if self.always_allow.lock().unwrap().contains(tool) {
            return PermissionDecision::Allow;
        }

        let rule = self
            .rules
            .get(tool)
            .or(self.default.as_ref())
            .copied()
            .unwrap_or(Rule::Ask);

        // Paths outside the workspace always escalate to Ask.
        if let Some(p) = path {
            if !Path::new(p).starts_with(cwd) {
                if rule == Rule::Deny {
                    return PermissionDecision::Deny;
                }
                return PermissionDecision::Ask;
            }
        }

        match rule {
            Rule::Allow => PermissionDecision::Allow,
            Rule::Ask => PermissionDecision::Ask,
            Rule::Deny => PermissionDecision::Deny,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_readonly_allow_by_default() {
        let engine = PermissionEngine::default();
        let cwd = Path::new("/proj");
        assert_eq!(engine.check("read", None, cwd), PermissionDecision::Allow);
        assert_eq!(engine.check("glob", None, cwd), PermissionDecision::Allow);
        assert_eq!(engine.check("grep", None, cwd), PermissionDecision::Allow);
    }

    #[test]
    fn test_mutating_ask_by_default() {
        let engine = PermissionEngine::default();
        let cwd = Path::new("/proj");
        assert_eq!(engine.check("write", None, cwd), PermissionDecision::Ask);
        assert_eq!(engine.check("bash", None, cwd), PermissionDecision::Ask);
        assert_eq!(engine.check("edit", None, cwd), PermissionDecision::Ask);
    }

    #[test]
    fn test_unknown_tool_asks() {
        let engine = PermissionEngine::default();
        assert_eq!(
            engine.check("mystery", None, Path::new("/proj")),
            PermissionDecision::Ask
        );
    }

    #[test]
    fn test_path_outside_cwd_escalates() {
        let engine = PermissionEngine::default();
        let cwd = Path::new("/proj");
        assert_eq!(
            engine.check("read", Some("/etc/passwd"), cwd),
            PermissionDecision::Ask
        );
        assert_eq!(
            engine.check("read", Some("/proj/src/main.rs"), cwd),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn test_always_allow_cache() {
        let engine = PermissionEngine::default();
        let cwd = Path::new("/proj");
        engine.set_always_allow("bash");
        assert_eq!(engine.check("bash", None, cwd), PermissionDecision::Allow);
    }

    #[test]
    fn test_config_override() {
        let mut tools = HashMap::new();
        tools.insert("bash".to_string(), Rule::Allow);
        let engine = PermissionEngine::from_config(&PermissionConfig {
            tools,
            default: Some(Rule::Deny),
        });
        let cwd = Path::new("/proj");
        assert_eq!(engine.check("bash", None, cwd), PermissionDecision::Allow);
        assert_eq!(engine.check("edit", None, cwd), PermissionDecision::Deny);
    }
}
