//! Builtin agent definitions.

use super::AgentSpec;
use std::collections::HashMap;

pub const BUILD: &str = "build";
/// Unified read-only reasoning agent (merges the former `plan`, `explore`
/// and `general` modes).
pub const REASON: &str = "reason";
pub const CHAT_FREE: &str = "chat-free";
pub const CURSOR: &str = "cursor";
/// Agent name used when the Cursor CLI serves plan mode.
pub const CURSOR_PLAN: &str = "cursor_plan";

/// All build-mode tools except the file-writing ones (`write`/`edit`).
///
/// Used by reason/chat-free so they get the full build
/// capability set (bash, task, question, remember, todo_write, git, ...)
/// without being able to modify project files directly.
const BUILD_TOOLS_NO_WRITE: &[&str] = &[
    "bash",
    "read",
    "glob",
    "grep",
    "ast_search",
    "diagnostics",
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
        temperature: Some(0.0),
        permission_overrides: HashMap::new(),
    }
}

/// Cursor delegation agent: the sole agent of the `build` mode when the
/// `cursor_agent` toggle is on. Its only tool is `cursor`, which delegates the
/// whole task to the Cursor CLI. The system prompt below is a minimal proxy
/// instruction for the harness LLM — it is NEVER forwarded to Cursor (the
/// delegation prompt is built deterministically by the tool).
pub fn cursor() -> AgentSpec {
    AgentSpec {
        name: CURSOR.into(),
        description: "Delegates the whole build task to the Cursor CLI (agent -p --force).".into(),
        tools: vec!["cursor".to_string()],
        system_prompt: "You are a proxy for the Cursor CLI. The user's request is a build task. \
Call the `cursor` tool exactly once, passing the user's full request verbatim as the `task` \
argument. Do not attempt to solve the task yourself and do not call any other tool. After the \
tool returns, relay its summary to the user."
            .into(),
        model: None,
        temperature: Some(0.0),
        permission_overrides: HashMap::new(),
    }
}

/// Plan-mode proxy for the Cursor CLI: delegates the whole planning task to
/// `agent -p --mode plan` (read-only). Mirrors [`cursor`] for the plan mode.
pub fn cursor_plan() -> AgentSpec {
    AgentSpec {
        name: CURSOR_PLAN.into(),
        description:
            "Delegates the whole planning task to the Cursor CLI (agent -p --mode plan, read-only)."
                .into(),
        tools: vec!["cursor_plan".to_string()],
        system_prompt: "You are a proxy for the Cursor CLI in plan mode. The user's request is a \
planning task. Call the `cursor_plan` tool exactly once, passing the user's full request verbatim \
as the `task` argument. Do not attempt to solve the task yourself and do not call any other tool. \
After the tool returns, relay its plan to the user."
            .into(),
        model: None,
        temperature: Some(0.0),
        permission_overrides: HashMap::new(),
    }
}

/// Unified read-only reasoning agent: analysis, design, codebase research
/// and general assistance, all with the full build toolset except direct
/// file writing (write/edit are excluded).
///
/// Merges the former `plan` (0.2), `explore` (0.5) and `general` (0.7)
/// modes into one agent calibrated at 0.3 — low enough to keep file paths
/// and APIs faithful, high enough that prose doesn't come out stilted.
pub fn reason() -> AgentSpec {
    AgentSpec {
        name: REASON.into(),
        description: "Read-only reasoning agent: plans, researches and answers. Full build tool access except file writing (write/edit)."
            .into(),
        tools: BUILD_TOOLS_NO_WRITE.iter().map(|s| s.to_string()).collect(),
        system_prompt: "You are RustClaw in reasoning mode. You explore, analyse and plan — you \
do not modify project files (write/edit are disabled). \
Explore the codebase with read/glob/grep/ast_search and understand the requirements first. \
When asked to plan, your final answer is the plan itself: write it as prose with ordered steps, \
each naming the files involved and the reason for the change. The todo tools track progress — \
they are not the plan, so never reply with just a todo list. \
When asked to research something, answer with a concise, factual summary and cite file paths. \
For general questions, answer directly. \
You can run commands (bash), ask the user (question), delegate research (task) and persist \
learnings (remember). \
To inspect a symbol definition (struct, enum, trait, function, impl) in a .rs file, prefer the \
ast_search tool over reading the whole file — it extracts exactly the block you need without \
loading the full source into context."
            .into(),
        model: None,
        temperature: Some(0.3),
        permission_overrides: HashMap::new(),
    }
}

/// Free-form chat agent: a Gemini/ChatGPT-style conversational assistant,
/// not limited to software development. Full build toolset except file writing.
pub fn chat_free() -> AgentSpec {
    AgentSpec {
        name: CHAT_FREE.into(),
        description: "Free-form conversational assistant (Gemini/ChatGPT style). Full build tool access except file writing."
            .into(),
        tools: BUILD_TOOLS_NO_WRITE.iter().map(|s| s.to_string()).collect(),
        system_prompt: "You are a friendly, knowledgeable AI assistant. You can talk about \
anything — science, culture, philosophy, everyday life, creative writing, ideas, or the \
user's project. Be warm, clear and helpful, and match the user's language. \
You are not limited to software development. \
You can search the web (web_search) and read web pages (fetch_webpage) to give current, \
accurate answers, and you can inspect the project with read/glob/grep/ast_search when the \
user asks about it. You can also run commands (bash), delegate research (task) and ask the \
user (question), but you cannot write or edit files directly (write/edit are disabled). \
Keep answers natural and conversational, not overly technical."
            .into(),
        model: None,
        temperature: Some(0.8),
        permission_overrides: HashMap::new(),
    }
}
