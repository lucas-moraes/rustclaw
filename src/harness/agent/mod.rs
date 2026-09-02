//! Agent specs: named tool/prompt/persona bundles (build, plan, explore, general).

pub mod builtin;

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
    pub fn allows_tool(&self, name: &str) -> bool {
        self.tools.is_empty() || self.tools.iter().any(|t| t == name)
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

    prompt.push_str(
        "\n# Operating rules\n\
1. Use the provided tools to accomplish the task. Tools are called natively - just decide which to call.\n\
2. Read before writing: inspect files with `read`/`grep`/`glob` before editing.\n\
3. Keep responses concise; summarize what you did rather than dumping full file contents.\n\
4. If a tool fails, adjust the input and retry differently - do not repeat the identical failing call.\n\
5. When finished, give a short final answer describing the outcome.\n\
6. When you discover a hidden convention, a tricky code pattern, or a specific \
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
pub fn unknown_tools(spec: &AgentSpec, available: &[String]) -> Vec<String> {
    spec.tools
        .iter()
        .filter(|t| !available.contains(t))
        .cloned()
        .collect()
}

/// Union of builtin agent names.
pub fn builtin_names() -> Vec<String> {
    vec![
        builtin::BUILD.to_string(),
        builtin::PLAN.to_string(),
        builtin::EXPLORE.to_string(),
        builtin::GENERAL.to_string(),
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
    fn test_explore_is_readonly() {
        let explore = builtin::explore();
        assert!(explore.allows_tool("read"));
        assert!(explore.allows_tool("grep"));
        assert!(!explore.allows_tool("write"));
        assert!(!explore.allows_tool("edit"));
        assert!(!explore.allows_tool("bash"));
    }

    #[test]
    fn test_build_has_all_tools() {
        let build = builtin::build();
        assert!(build.tools.is_empty()); // all tools
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
        );
        assert!(prompt.contains("/proj"));
        assert!(prompt.contains("injected ctx"));
        assert!(prompt.contains("Operating rules"));
        assert!(prompt.contains("Project context"));
        assert!(prompt.contains("Stack: rust"));
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
