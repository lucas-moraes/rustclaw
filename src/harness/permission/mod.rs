//! Permission engine: allow / ask / deny rules per tool, with path escalation.
//!
//! Defaults mirror OpenCode/Claude Code conventions:
//! - read-only tools (read/glob/grep/todo_read) => Allow
//! - mutating tools (write/edit/bash) => Ask
//! - paths outside the session cwd => escalated to Ask even for read tools.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

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

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
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

    /// True when no explicit rules or default are set.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty() && self.default.is_none()
    }
}

pub struct PermissionEngine {
    rules: std::sync::Mutex<HashMap<String, Rule>>,
    default: std::sync::Mutex<Option<Rule>>,
    /// "always allow" decisions cached per session run.
    always_allow: std::sync::Mutex<std::collections::HashSet<String>>,
    /// Optional callback invoked when a tool is marked "always allow", so the
    /// decision can be persisted (e.g. to the project's `rustclaw.json`).
    persist: std::sync::Mutex<Option<Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>>>,
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
        for t in [
            "read",
            "glob",
            "grep",
            "todo_read",
            "todo_write",
            "web_search",
            "fetch_webpage",
            "git_status",
            "git_diff",
            "git_log",
        ] {
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
            rules: std::sync::Mutex::new(config.tools.clone()),
            default: std::sync::Mutex::new(config.default),
            always_allow: std::sync::Mutex::new(std::collections::HashSet::new()),
            persist: std::sync::Mutex::new(None),
        }
    }

    /// Merges project-level rules into the engine. Project rules override the
    /// builtin defaults; the project `default` (if any) overrides ours.
    pub fn apply_project_config(&self, config: &PermissionConfig) {
        let mut rules = self.rules.lock().unwrap();
        for (tool, rule) in &config.tools {
            rules.insert(tool.clone(), *rule);
        }
        if let Some(d) = config.default {
            *self.default.lock().unwrap() = Some(d);
        }
    }

    /// Installs a callback invoked whenever a tool is marked "always allow",
    /// so the decision can be persisted across sessions.
    pub fn set_persist(&self, f: Option<Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>>) {
        *self.persist.lock().unwrap() = f;
    }

    pub fn set_always_allow(&self, tool: &str) {
        self.always_allow.lock().unwrap().insert(tool.to_string());
        if let Some(persist) = self.persist.lock().unwrap().as_ref() {
            if let Err(e) = persist(tool) {
                eprintln!(
                    "[warn] failed to persist always-allow for `{}`: {}",
                    tool, e
                );
            }
        }
    }

    /// Snapshot of the current per-tool rules (for `/permissions list`).
    pub fn rules_snapshot(&self) -> Vec<(String, Rule)> {
        let mut v: Vec<(String, Rule)> = self
            .rules
            .lock()
            .unwrap()
            .iter()
            .map(|(k, r)| (k.clone(), *r))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// Sets a per-tool rule in memory (used by `/permissions set`).
    pub fn set_rule(&self, tool: &str, rule: Rule) {
        self.rules.lock().unwrap().insert(tool.to_string(), rule);
    }

    /// Removes a per-tool rule, falling back to the default (used by `/permissions rm`).
    pub fn remove_rule(&self, tool: &str) -> bool {
        self.rules.lock().unwrap().remove(tool).is_some()
    }

    /// Resolves the decision for `tool` with optional `path` (absolute).
    /// Paths escaping the session cwd always escalate to Ask — even with an
    /// "always allow" cached for the tool — so "a" (always) only grants
    /// blanket permission for files inside the project.
    pub fn check(&self, tool: &str, path: Option<&str>, cwd: &Path) -> PermissionDecision {
        // Explicit "always allow" wins for this run, but only inside the cwd.
        let always = self.always_allow.lock().unwrap().contains(tool);

        let rule = self
            .rules
            .lock()
            .unwrap()
            .get(tool)
            .or(self.default.lock().unwrap().as_ref())
            .copied()
            .unwrap_or(Rule::Ask);

        // Paths outside the workspace always escalate to Ask (unless denied).
        if let Some(p) = path {
            if !Path::new(p).starts_with(cwd) {
                if rule == Rule::Deny {
                    return PermissionDecision::Deny;
                }
                return PermissionDecision::Ask;
            }
        }

        if always {
            return PermissionDecision::Allow;
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
    fn test_always_allow_grants_all_project_files() {
        let engine = PermissionEngine::default();
        let cwd = Path::new("/proj");
        engine.set_always_allow("edit");
        assert_eq!(
            engine.check("edit", Some("/proj/src/main.rs"), cwd),
            PermissionDecision::Allow
        );
        assert_eq!(
            engine.check("edit", Some("/proj/README.md"), cwd),
            PermissionDecision::Allow
        );
        // Other mutating tools still ask.
        assert_eq!(engine.check("write", None, cwd), PermissionDecision::Ask);
    }

    #[test]
    fn test_always_allow_keeps_asking_outside_cwd() {
        let engine = PermissionEngine::default();
        let cwd = Path::new("/proj");
        engine.set_always_allow("write");
        assert_eq!(
            engine.check("write", Some("/etc/passwd"), cwd),
            PermissionDecision::Ask
        );
        assert_eq!(
            engine.check("write", Some("/proj/src/main.rs"), cwd),
            PermissionDecision::Allow
        );
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

    #[test]
    fn test_apply_project_config_overrides_defaults() {
        let engine = PermissionEngine::default();
        let cwd = Path::new("/proj");
        assert_eq!(engine.check("bash", None, cwd), PermissionDecision::Ask);
        let mut tools = HashMap::new();
        tools.insert("bash".to_string(), Rule::Allow);
        engine.apply_project_config(&PermissionConfig {
            tools,
            default: None,
        });
        assert_eq!(engine.check("bash", None, cwd), PermissionDecision::Allow);
        // Unrelated defaults survive.
        assert_eq!(engine.check("read", None, cwd), PermissionDecision::Allow);
    }

    #[test]
    fn test_set_and_remove_rule() {
        let engine = PermissionEngine::default();
        let cwd = Path::new("/proj");
        engine.set_rule("bash", Rule::Allow);
        assert_eq!(engine.check("bash", None, cwd), PermissionDecision::Allow);
        assert!(engine.remove_rule("bash"));
        assert_eq!(engine.check("bash", None, cwd), PermissionDecision::Ask);
        assert!(!engine.remove_rule("bash"));
    }

    #[test]
    fn test_rules_snapshot_sorted() {
        let engine = PermissionEngine::default();
        engine.set_rule("zzz", Rule::Deny);
        engine.set_rule("aa", Rule::Allow);
        let snap = engine.rules_snapshot();
        let names: Vec<&str> = snap.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names.first().copied(), Some("aa"));
        assert!(names.contains(&"zzz"));
    }

    #[test]
    fn test_persist_callback_invoked_on_always_allow() {
        let engine = PermissionEngine::default();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        engine.set_persist(Some(Arc::new(move |tool: &str| {
            sink.lock().unwrap().push(tool.to_string());
            Ok(())
        })));
        engine.set_always_allow("bash");
        assert_eq!(*seen.lock().unwrap(), vec!["bash".to_string()]);
    }

    #[test]
    fn test_persist_error_does_not_panic() {
        let engine = PermissionEngine::default();
        engine.set_persist(Some(Arc::new(|_: &str| Err("boom".to_string()))));
        // Must not panic even when the callback fails.
        engine.set_always_allow("bash");
    }
}
