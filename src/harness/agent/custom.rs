//! Custom agent discovery: `.agents/agents/*.md` (project) and
//! `~/.config/rustclaw/agents/*.md` (user-global), following the same
//! frontmatter + body pattern as the skill loader.
//!
//! Format:
//! ```text
//! ---
//! description: Short description
//! tools: [read, grep, bash]   # optional; absent = all tools
//! model: provider/model       # optional model override
//! ---
//! Body = full system prompt for the agent.
//! ```

use super::AgentSpec;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Roots scanned for custom agent definitions, in priority order
/// (dedup by name, first wins). Project-local beats user-global.
pub fn agent_roots(cwd: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    roots.push(cwd.join(".agents").join("agents"));
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".config").join("rustclaw").join("agents"));
    }
    roots
}

/// Discovers and parses all custom agents for the given working directory.
/// The returned map is keyed by agent name (file stem, sanitized).
pub fn load_custom_agents(cwd: &Path) -> HashMap<String, AgentSpec> {
    load_from_roots(&agent_roots(cwd))
}

/// Loads custom agents from an explicit set of root directories
/// (dedup by name, first wins).
pub fn load_from_roots(roots: &[PathBuf]) -> HashMap<String, AgentSpec> {
    let mut agents = HashMap::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") || !path.is_file() {
                continue;
            }
            let Some(spec) = parse_agent_file(&path) else {
                continue;
            };
            agents.entry(spec.name.clone()).or_insert(spec);
        }
    }
    agents
}

/// Parses a single agent `.md` file: frontmatter (`description`, `tools`,
/// `model`) + markdown body as the system prompt. Name = file stem.
pub fn parse_agent_file(path: &Path) -> Option<AgentSpec> {
    let raw = std::fs::read_to_string(path).ok()?;
    let stem = path.file_stem()?.to_string_lossy().to_string();
    Some(parse_agent(&stem, &raw))
}

/// Parses agent content (frontmatter + body) into an [`AgentSpec`].
/// Malformed files still yield a usable spec (empty description, all tools).
pub fn parse_agent(stem: &str, raw: &str) -> AgentSpec {
    let (meta, body) = split_frontmatter(raw).unwrap_or_else(|| (String::new(), raw.to_string()));
    let name = sanitize_name(stem);
    let description = get_field(&meta, "description").unwrap_or_default();
    let tools = get_field(&meta, "tools")
        .map(|v| parse_tool_list(&v))
        .unwrap_or_default();
    let model = get_field(&meta, "model").filter(|m| !m.is_empty());
    AgentSpec {
        name,
        description,
        tools,
        system_prompt: body.trim().to_string(),
        model,
        temperature: None,
        permission_overrides: Default::default(),
    }
}

/// Sanitizes a file stem into an agent name (lowercase, `[a-z0-9_-]`).
fn sanitize_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Splits YAML-ish frontmatter (`---\n...\n---`) from body. Returns (meta, body).
fn split_frontmatter(raw: &str) -> Option<(String, String)> {
    let rest = raw.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let meta = rest[..end].to_string();
    let body = rest[end + 4..].to_string();
    Some((meta, body))
}

/// Extracts a scalar field from frontmatter text like `model: gpt-5`.
fn get_field(meta: &str, key: &str) -> Option<String> {
    for line in meta.lines() {
        if let Some(v) = line
            .strip_prefix(&format!("{}:", key))
            .or_else(|| line.strip_prefix(&format!("{} :", key)))
        {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Parses a bracketed or bare comma-separated list: `[read, grep]` / `read, grep`.
fn parse_tool_list(value: &str) -> Vec<String> {
    let v = value.trim();
    let v = v
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(v);
    v.split(',')
        .map(|t| t.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_and_body() {
        let raw = "---\ndescription: Docs writer\ntools: [read, grep, bash]\nmodel: gpt-5\n---\n\n# Docs\nWrite good docs.";
        let spec = parse_agent("docs", raw);
        assert_eq!(spec.name, "docs");
        assert_eq!(spec.description, "Docs writer");
        assert_eq!(spec.tools, vec!["read", "grep", "bash"]);
        assert_eq!(spec.model.as_deref(), Some("gpt-5"));
        assert!(spec.system_prompt.contains("Write good docs"));
    }

    #[test]
    fn test_no_tools_means_all() {
        let spec = parse_agent("helper", "---\ndescription: d\n---\nbody");
        assert!(spec.tools.is_empty(), "empty allowlist = all tools");
        assert!(spec.model.is_none());
    }

    #[test]
    fn test_no_frontmatter_body_is_prompt() {
        let spec = parse_agent("plain", "Just a prompt body");
        assert_eq!(spec.system_prompt, "Just a prompt body");
        assert!(spec.description.is_empty());
    }

    #[test]
    fn test_discovery_in_tempdir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".agents").join("agents");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("foo.md"),
            "---\ndescription: Foo agent\ntools: [read, grep]\n---\nFoo prompt body.",
        )
        .unwrap();
        let agents = load_custom_agents(tmp.path());
        assert_eq!(agents.len(), 1);
        let spec = &agents["foo"];
        assert_eq!(spec.system_prompt, "Foo prompt body.");
        assert_eq!(spec.tools, vec!["read", "grep"]);
    }

    #[test]
    fn test_project_wins_over_home() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join(".agents").join("agents");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("same.md"),
            "---\ndescription: p\n---\nproject body",
        )
        .unwrap();
        let home = tmp.path().join("cfg").join("rustclaw").join("agents");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("same.md"), "---\ndescription: h\n---\nhome body").unwrap();
        let agents = load_from_roots(&[project, home]);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents["same"].system_prompt, "project body");
    }

    #[test]
    fn test_custom_shadows_builtin_in_resolve() {
        // Custom agent with a builtin name wins over the builtin.
        let custom = parse_agent(
            "build",
            "---\ndescription: custom build\n---\ncustom prompt",
        );
        assert_eq!(custom.name, "build");
        let mut map = HashMap::new();
        map.insert(custom.name.clone(), custom.clone());
        let resolved = map
            .get("build")
            .cloned()
            .or_else(|| super::super::find_builtin("build"))
            .unwrap();
        assert_eq!(resolved.system_prompt, "custom prompt");
    }
}
