//! Tool execution context: session info, abort signal, permission/user askers,
//! and hooks for subagent spawning.

use crate::harness::permission::{PermissionDecision, PermissionEngine};
use crate::harness::project::ProjectMemoryStore;
use crate::harness::session::TodoItem;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Cooperative cancellation flag checked between tool/loop steps.
#[derive(Clone, Default)]
pub struct AbortSignal(Arc<AtomicBool>);

impl AbortSignal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn abort(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_aborted(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Debug)]
pub struct PermissionAskInput {
    pub tool: String,
    pub args_summary: String,
    pub path: Option<String>,
}

/// Decides whether a tool call may proceed.
#[async_trait::async_trait]
pub trait PermissionAsker: Send + Sync {
    /// Returns true if the user allowed the operation (for this run or "always").
    async fn ask(&self, request: PermissionAskInput) -> bool;
}

/// Free-form question to the end user (used by the `question` tool).
#[async_trait::async_trait]
pub trait UserAsker: Send + Sync {
    /// Presents the question + options; returns the chosen answer (free text allowed).
    async fn ask(&self, question: String, options: Vec<String>) -> Option<String>;
}

/// Runs a subagent task (implemented by the runtime; injected to avoid cycles).
#[async_trait::async_trait]
pub trait SubagentRunner: Send + Sync {
    async fn run_task(&self, agent: String, prompt: String) -> Result<String, String>;
}

/// Everything a tool needs to run, scoped to the current session.
#[derive(Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub agent: String,
    pub cwd: PathBufGuard,
    pub abort: AbortSignal,
    pub permission: Arc<PermissionEngine>,
    pub asker: Arc<dyn PermissionAsker>,
    pub user_asker: Arc<dyn UserAsker>,
    pub todos: Arc<tokio::sync::RwLock<Vec<TodoItem>>>,
    /// Extra shared state (e.g. task runner installed by the runtime).
    pub task_runner: Option<Arc<dyn SubagentRunner>>,
    /// Project memory store (SQLite) used by the `remember` tool.
    pub project_memory: Option<Arc<ProjectMemoryStore>>,
}

/// Working directory guard: all path resolution goes through this.
#[derive(Clone, Debug)]
pub struct PathBufGuard(pub std::path::PathBuf);

impl PathBufGuard {
    pub fn path(&self) -> &std::path::Path {
        &self.0
    }

    /// Resolves `p` against the session cwd if relative.
    pub fn resolve(&self, p: &str) -> std::path::PathBuf {
        let path = std::path::Path::new(p);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.0.join(path)
        }
    }
}

impl ToolContext {
    /// Checks permission for a tool call. Escalates to the asker when the
    /// engine says `Ask`. Returns an Err with a friendly model-facing message on deny.
    pub async fn check_permission(&self, tool: &str, args: &Value) -> Result<(), String> {
        let path = extract_path(args).map(|p| self.cwd.resolve(&p).to_string_lossy().to_string());
        let decision = self
            .permission
            .check(tool, path.as_deref(), self.cwd.path());
        match decision {
            PermissionDecision::Allow => Ok(()),
            PermissionDecision::Deny => Err(format!(
                "Permission denied: tool `{}` is not allowed by policy.",
                tool
            )),
            PermissionDecision::Ask => {
                let input = PermissionAskInput {
                    tool: tool.to_string(),
                    args_summary: crate::harness::session::preview(&args.to_string(), 200),
                    path,
                };
                let allowed = self.asker.ask(input).await;
                if allowed {
                    Ok(())
                } else {
                    Err(format!(
                        "The user denied permission for tool `{}`. Do not retry the same call; \
                         explain and ask how to proceed.",
                        tool
                    ))
                }
            }
        }
    }
}

fn extract_path(args: &Value) -> Option<String> {
    for key in ["path", "file_path", "working_dir", "pattern_path"] {
        if let Some(p) = args.get(key).and_then(|v| v.as_str()) {
            return Some(p.to_string());
        }
    }
    None
}
