//! Builtin agent definitions.

use super::AgentSpec;
use std::collections::HashMap;

pub const BUILD: &str = "build";
pub const PLAN: &str = "plan";
pub const EXPLORE: &str = "explore";
pub const GENERAL: &str = "general";

const READONLY_TOOLS: &[&str] = &[
    "read",
    "glob",
    "grep",
    "todo_read",
    "web_search",
    "fetch_webpage",
    "git_status",
    "git_diff",
    "git_log",
];

/// Default implementation agent: all tools + task subagents.
pub fn build() -> AgentSpec {
    AgentSpec {
        name: BUILD.into(),
        description: "Implements features, fixes bugs, runs builds/tests. Full tool access.".into(),
        tools: vec![],
        system_prompt: "You are RustClaw, an expert software engineering agent operating as a \
coding harness inside the user's project. You implement features, fix bugs, run builds and \
tests using your tools. Prefer precise, minimal edits. Verify your work (build/tests) before \
claiming success. When done, summarize what changed and how you verified it."
            .into(),
        model: None,
        temperature: None,
        permission_overrides: HashMap::new(),
    }
}

/// Planning agent: analysis and design, no mutating tools.
pub fn plan() -> AgentSpec {
    AgentSpec {
        name: PLAN.into(),
        description: "Plans and designs solutions. Read-only plus todos - cannot write files or run commands."
            .into(),
        tools: READONLY_TOOLS.iter().map(|s| s.to_string()).collect(),
        system_prompt: "You are RustClaw in planning mode. Explore the codebase (read/glob/grep), \
understand requirements, and produce a concrete step-by-step plan using the todo tools. \
Do not attempt to write files or execute commands - you have read-only tools. \
Return the plan as your final answer with clear, ordered steps."
            .into(),
        model: None,
        temperature: None,
        permission_overrides: HashMap::new(),
    }
}

/// Exploration subagent: fast read-only research, returns summaries.
pub fn explore() -> AgentSpec {
    AgentSpec {
        name: EXPLORE.into(),
        description: "Read-only research agent for codebase exploration via the task tool.".into(),
        tools: READONLY_TOOLS.iter().map(|s| s.to_string()).collect(),
        system_prompt: "You are an exploration agent. Research the codebase quickly using \
read/glob/grep and answer the given question with a concise, factual summary. \
Cite file paths. Do not attempt to modify anything."
            .into(),
        model: None,
        temperature: None,
        permission_overrides: HashMap::new(),
    }
}

/// General chat agent: light tools, conversational.
pub fn general() -> AgentSpec {
    AgentSpec {
        name: GENERAL.into(),
        description: "General-purpose assistant with light tool access.".into(),
        tools: vec![
            "read".into(),
            "glob".into(),
            "grep".into(),
            "web_search".into(),
            "fetch_webpage".into(),
            "git_status".into(),
            "git_diff".into(),
            "git_log".into(),
        ],
        system_prompt: "You are RustClaw, a helpful assistant. You can inspect the project with \
read/glob/grep when needed. Keep answers direct and useful."
            .into(),
        model: None,
        temperature: None,
        permission_overrides: HashMap::new(),
    }
}
