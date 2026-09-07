//! Builtin agent definitions.

use super::AgentSpec;
use std::collections::HashMap;

pub const BUILD: &str = "build";
pub const PLAN: &str = "plan";
pub const EXPLORE: &str = "explore";
pub const GENERAL: &str = "general";
pub const CHAT_FREE: &str = "chat-free";

const READONLY_TOOLS: &[&str] = &[
    "read",
    "glob",
    "grep",
    "ast_search",
    "todo_read",
    "web_search",
    "fetch_webpage",
    "git_status",
    "git_diff",
    "git_log",
    // Special marker: admits MCP tools annotated `readOnlyHint: true`.
    "mcp_readonly",
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
claiming success. When done, summarize what changed and how you verified it. \
For independent research or verification work, delegate to subagents with the task tool — \
pass `tasks: [...]` (a batch) so independent tasks run in parallel instead of one by one. \
To inspect a symbol definition (struct, enum, trait, function, impl) in a .rs file, prefer \
the ast_search tool over reading the whole file — it extracts exactly the block you need \
without loading the full source into context."
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
Cite file paths. Do not attempt to modify anything. \
To map or inspect symbol definitions (structs, enums, traits, functions, impls) in .rs \
files, prefer the ast_search tool — it extracts exactly the blocks you need without \
reading whole files."
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
            "ast_search".into(),
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

/// Free-form chat agent: a Gemini/ChatGPT-style conversational assistant,
/// not limited to software development. Light read-only tools for when the
/// user asks about the project or wants current web info.
pub fn chat_free() -> AgentSpec {
    AgentSpec {
        name: CHAT_FREE.into(),
        description: "Free-form conversational assistant (Gemini/ChatGPT style).".into(),
        tools: vec![
            "read".into(),
            "glob".into(),
            "grep".into(),
            "ast_search".into(),
            "web_search".into(),
            "fetch_webpage".into(),
        ],
        system_prompt: "You are a friendly, knowledgeable AI assistant. You can talk about \
anything — science, culture, philosophy, everyday life, creative writing, ideas, or the \
user's project. Be warm, clear and helpful, and match the user's language. \
You are not limited to software development. \
You can search the web (web_search) and read web pages (fetch_webpage) to give current, \
accurate answers, and you can inspect the project with read/glob/grep/ast_search when the \
user asks about it. Keep answers natural and conversational, not overly technical."
            .into(),
        model: None,
        temperature: Some(0.8),
        permission_overrides: HashMap::new(),
    }
}
