//! Agent specs: named tool/prompt/persona bundles (build, plan, explore, general).

pub mod builtin;
pub mod custom;

use crate::harness::tool::ToolSpec;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSpec {
    pub name: String,
    pub description: String,
    /// Tool allowlist. Empty = all registered tools.
    pub tools: Vec<String>,
    /// Full system prompt identity/instructions for this agent.
    pub system_prompt: String,
    /// Optional model override (falls back to config default).
    pub model: Option<String>,
    /// Optional temperature override.
    pub temperature: Option<f32>,
    /// Agent-specific permission overrides (tool -> rule string).
    pub permission_overrides: std::collections::HashMap<String, String>,
}

impl AgentSpec {
    /// Whether the agent's allowlist admits the given tool (empty = all).
    /// Only used by tests.
    #[cfg(test)]
    pub fn allows_tool(&self, name: &str) -> bool {
        self.tools.is_empty() || self.tools.iter().any(|t| t == name)
    }

    /// Fallback sampling temperature for agents that carry no explicit
    /// `temperature` override (custom/unknown agents). Builtins set their own
    /// calibrated temperature in `builtin::*()`; this is only the safety net.
    pub fn default_temperature(&self) -> f32 {
        0.0
    }

    /// Effective turn temperature: an explicit spec override wins;
    /// otherwise the calibrated default for the mode applies.
    pub fn turn_temperature(&self) -> f32 {
        self.temperature
            .unwrap_or_else(|| self.default_temperature())
    }
}

/// Builds the final system prompt for a turn: identity + project context +
/// injected context (skills). `injected_context` is the session's enabled
/// skills block (empty when the user opted out for this turn).
///
/// `project_context` is the auto-discovered + curated project memory
/// (`# Project context`). Precedence: the manual `AGENTS.md` is the primary
/// source of project instructions; the auto summary complements it in a
/// separate section.
pub fn build_system_prompt(
    agent: &AgentSpec,
    cwd: &std::path::Path,
    injected_context: &str,
    extra_instructions: Option<&str>,
    project_context: &str,
    available_tools: &[ToolSpec],
) -> String {
    let mut prompt = String::new();
    prompt.push_str(&agent.system_prompt);
    prompt.push_str("\n\n# Environment\n");
    prompt.push_str(&format!("- Working directory: {}\n", cwd.display()));
    prompt.push_str(
        "- Today: you are operating inside this project directory. \
Paths in tool calls are resolved relative to it.\n",
    );

    if let Some(agents_md) = load_agents_md(cwd) {
        prompt.push_str("\n# Project instructions (AGENTS.md)\n");
        prompt.push_str(&agents_md);
    }

    if !project_context.trim().is_empty() {
        prompt.push('\n');
        prompt.push_str(project_context);
        prompt.push('\n');
    }

    if !injected_context.trim().is_empty() {
        prompt.push_str(injected_context);
        prompt.push('\n');
    }

    if let Some(extra) = extra_instructions {
        if !extra.trim().is_empty() {
            prompt.push_str("\n# Additional context\n");
            prompt.push_str(extra);
            prompt.push('\n');
        }
    }

    // List the tools the agent can actually invoke, so the model knows what
    // it may trigger. This is derived from the registry filtered by the
    // agent's allowlist (empty = all registered tools).
    if !available_tools.is_empty() {
        prompt.push_str("\n# Available tools\n");
        prompt.push_str(
            "You can invoke any of the following tools natively. Each is described \
with its JSON Schema in the request; use them to accomplish the task.\n",
        );
        for spec in available_tools {
            prompt.push_str(&format!("- `{}`: {}\n", spec.name, spec.description));
        }
    }

    prompt.push_str(
        "\n# Operating rules\n\
1. Reply in the user's language: detect the language of the user's message \
and write all your responses in it (keep code, paths and identifiers unchanged).\n\
2. Use the provided tools to accomplish the task. Tools are called natively - just decide which to call.\n\
3. Read before writing: inspect files with `read`/`grep`/`glob` before editing.\n\
4. Keep responses concise; summarize what you did rather than dumping full file contents.\n\
5. If a tool fails, adjust the input and retry differently - do not repeat the identical failing call.\n\
6. When finished, give a short final answer describing the outcome.\n\
7. When you discover a hidden convention, a tricky code pattern, or a specific \
build/test command that is not obvious from the repo, persist it with the \
`remember` tool so future sessions can reuse it.\n",
    );
    prompt
}

fn load_agents_md(cwd: &std::path::Path) -> Option<String> {
    let path = cwd.join("AGENTS.md");
    let content = std::fs::read_to_string(path).ok()?;
    const MAX: usize = 12_000;
    if content.len() > MAX {
        Some(format!(
            "{}\n[AGENTS.md truncated]",
            crate::harness::tool::truncate::truncate_output(&content, MAX)
        ))
    } else {
        Some(content)
    }
}

/// Validates tool names in a spec against the registry (helper for tests/CLI).
/// Only used by tests.
#[cfg(test)]
pub fn unknown_tools(spec: &AgentSpec, available: &[String]) -> Vec<String> {
    spec.tools
        .iter()
        .filter(|t| !available.contains(t))
        .cloned()
        .collect()
}

/// Union of builtin agent names. Only used by tests.
#[cfg(test)]
pub fn builtin_names() -> Vec<String> {
    vec![
        builtin::BUILD.to_string(),
        builtin::PLAN.to_string(),
        builtin::EXPLORE.to_string(),
        builtin::GENERAL.to_string(),
        builtin::CHAT_FREE.to_string(),
    ]
}

/// Looks up a builtin agent by name (case-insensitive).
pub fn find_builtin(name: &str) -> Option<AgentSpec> {
    let lower = name.to_lowercase();
    let agent = match lower.as_str() {
        "build" => builtin::build(),
        "plan" => builtin::plan(),
        "explore" => builtin::explore(),
        "general" => builtin::general(),
        "chat-free" | "chat_free" | "chatfree" => builtin::chat_free(),
        _ => return None,
    };
    Some(agent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_find_builtin_agents() {
        for name in builtin_names() {
            let spec = find_builtin(&name);
            assert!(spec.is_some(), "missing builtin: {}", name);
        }
        assert!(find_builtin("nope").is_none());
    }

    #[test]
    fn test_explore_has_build_tools_except_write() {
        let explore = builtin::explore();
        assert!(explore.allows_tool("read"));
        assert!(explore.allows_tool("grep"));
        assert!(explore.allows_tool("bash"));
        assert!(explore.allows_tool("task"));
        assert!(explore.allows_tool("remember"));
        assert!(!explore.allows_tool("write"));
        assert!(!explore.allows_tool("edit"));
    }

    /// Every non-build mode gets the full build toolset except file writing
    /// (write/edit). bash/remember/task are allowed.
    #[test]
    fn test_non_build_modes_allow_all_except_write_edit() {
        let file_writing = ["write", "edit"];
        let build_tools = [
            "bash",
            "read",
            "glob",
            "grep",
            "ast_search",
            "todo_read",
            "todo_write",
            "question",
            "task",
            "remember",
            "fetch_webpage",
            "web_search",
            "git_status",
            "git_diff",
            "git_log",
        ];
        for name in ["plan", "explore", "general", "chat-free"] {
            let spec = crate::harness::agent::find_builtin(name).unwrap();
            for m in file_writing {
                assert!(!spec.allows_tool(m), "{name} must not allow {m}");
            }
            for t in build_tools {
                assert!(spec.allows_tool(t), "{name} must allow {t}");
            }
        }
        let build = crate::harness::agent::find_builtin("build").unwrap();
        for m in file_writing {
            assert!(build.allows_tool(m));
        }
    }

    #[test]
    fn test_build_has_all_tools() {
        let build = builtin::build();
        assert!(build.tools.is_empty()); // all tools
    }

    #[test]
    fn test_chat_free_agent() {
        let chat = builtin::chat_free();
        assert_eq!(chat.name, "chat-free");
        // Conversational: higher temperature.
        assert_eq!(chat.turn_temperature(), 0.8);
        // Full build toolset except file writing.
        assert!(chat.allows_tool("web_search"));
        assert!(chat.allows_tool("fetch_webpage"));
        assert!(chat.allows_tool("read"));
        assert!(chat.allows_tool("bash"));
        assert!(chat.allows_tool("task"));
        assert!(!chat.allows_tool("write"));
        assert!(!chat.allows_tool("edit"));
        // Prompt is conversational, not coding-focused.
        assert!(chat.system_prompt.contains("friendly"));
        assert!(chat
            .system_prompt
            .contains("not limited to software development"));
    }

    #[test]
    fn test_system_prompt_contains_env_and_rules() {
        let build = builtin::build();
        let prompt = build_system_prompt(
            &build,
            &PathBuf::from("/proj"),
            "injected ctx",
            None,
            "# Project context\n- Stack: rust",
            &[],
        );
        assert!(prompt.contains("/proj"));
        assert!(prompt.contains("injected ctx"));
        assert!(prompt.contains("Operating rules"));
        assert!(prompt.contains("user's language"));
        assert!(prompt.contains("Project context"));
        assert!(prompt.contains("Stack: rust"));
    }

    #[test]
    fn test_system_prompt_lists_available_tools() {
        let build = builtin::build();
        let tools = vec![
            ToolSpec {
                name: "read".into(),
                description: "Reads a file".into(),
                parameters: serde_json::json!({"type": "object"}),
            },
            ToolSpec {
                name: "ast_search".into(),
                description: "Syntactic search over .rs files".into(),
                parameters: serde_json::json!({"type": "object"}),
            },
        ];
        let prompt = build_system_prompt(&build, &PathBuf::from("/proj"), "", None, "", &tools);
        assert!(prompt.contains("# Available tools"));
        assert!(prompt.contains("`read`: Reads a file"));
        assert!(prompt.contains("`ast_search`: Syntactic search over .rs files"));
    }

    #[test]
    fn test_system_prompt_omits_tools_section_when_empty() {
        let build = builtin::build();
        let prompt = build_system_prompt(&build, &PathBuf::from("/proj"), "", None, "", &[]);
        assert!(!prompt.contains("# Available tools"));
    }

    #[test]
    fn test_precedence_manual_and_auto_separate_sections() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("AGENTS.md"),
            "# Manual\n- always use tabs\n- stack must be rust",
        )
        .unwrap();
        let build = builtin::build();
        let prompt = build_system_prompt(
            &build,
            d.path(),
            "",
            None,
            "# Project context\n- Stack: rust\n- Build: cargo build",
            &[],
        );
        // Manual AGENTS.md is the primary source of project instructions...
        assert!(prompt.contains("# Project instructions (AGENTS.md)"));
        assert!(prompt.contains("always use tabs"));
        // ...and the auto summary is a separate, complementary section.
        assert!(prompt.contains("# Project context"));
        assert!(prompt.contains("Build: cargo build"));
        // Manual section appears before the auto summary.
        let manual_pos = prompt.find("Project instructions").unwrap();
        let auto_pos = prompt.find("Project context").unwrap();
        assert!(manual_pos < auto_pos);
    }

    #[test]
    fn test_default_temperature_per_mode() {
        // Builtins carry an explicit temperature override; `turn_temperature`
        // is the effective value used at runtime.
        let cases = [
            ("build", 0.0),
            ("plan", 0.2),
            ("explore", 0.5),
            ("general", 0.7),
            ("chat-free", 0.8),
        ];
        for (name, expected) in cases {
            let spec = find_builtin(name).unwrap();
            assert!(
                (spec.turn_temperature() - expected).abs() < 1e-6,
                "mode: {}",
                name
            );
        }
        // Custom/unknown agent falls back to 0.0 (no explicit override).
        let custom = AgentSpec {
            name: "my-custom".into(),
            description: String::new(),
            tools: vec![],
            system_prompt: String::new(),
            model: None,
            temperature: None,
            permission_overrides: Default::default(),
        };
        assert_eq!(custom.default_temperature(), 0.0);
        assert_eq!(custom.turn_temperature(), 0.0);
    }

    #[test]
    fn test_turn_temperature_override_wins() {
        let mut custom = find_builtin("explore").unwrap();
        custom.temperature = Some(1.0);
        assert!((custom.turn_temperature() - 1.0).abs() < 1e-6);
        // The explicit override wins over the builtin's own temperature.
        assert!((find_builtin("explore").unwrap().turn_temperature() - 0.5).abs() < 1e-6);
        // `default_temperature` is only the fallback for agents without an
        // override; it is not the per-mode calibration anymore.
        assert_eq!(custom.default_temperature(), 0.0);
    }

    #[test]
    fn test_unknown_tools() {
        let spec = AgentSpec {
            name: "t".into(),
            description: String::new(),
            tools: vec!["read".into(), "ghost".into()],
            system_prompt: String::new(),
            model: None,
            temperature: None,
            permission_overrides: Default::default(),
        };
        let unknown = unknown_tools(&spec, &["read".to_string()]);
        assert_eq!(unknown, vec!["ghost".to_string()]);
    }
}
