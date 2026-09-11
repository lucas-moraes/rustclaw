//! Subagent task runner: spawns child sessions through the runtime.

use super::SessionRuntime;
use crate::harness::tool::context::{SubagentRunner, TaskOutcome};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

/// Runs a subagent in a fresh child session, returning its final summary.
/// Child events are forwarded to the caller's channel (tagged with the child's
/// session id and the parent session id) so UIs can render subagent activity.
pub struct TaskRunner {
    pub runtime: Arc<SessionRuntime>,
    /// Session that spawned the task (used to tag child events).
    pub parent_session_id: String,
    /// Whether the root agent may write files. When false, subagents are
    /// demoted to a non-writing agent so they can never edit files.
    pub allow_write: bool,
}

/// Resolves the effective subagent agent given whether the parent allows file
/// writes. When writes are not allowed, a requested `build` subagent is
/// demoted to `general` (same toolset minus write/edit) so subagents can
/// never edit files when the root agent isn't `build`.
pub(crate) fn resolve_subagent_agent(allow_write: bool, requested: &str) -> &str {
    if !allow_write && requested == crate::harness::agent::builtin::BUILD {
        crate::harness::agent::builtin::GENERAL
    } else {
        requested
    }
}

#[async_trait::async_trait]
impl SubagentRunner for TaskRunner {
    async fn run_task(
        &self,
        agent: String,
        prompt: String,
        events: crate::harness::event::EventSender,
    ) -> Result<TaskOutcome, String> {
        // Resolve agent to allow "explore" by default.
        let agent = if agent.is_empty() { "explore" } else { &agent };
        // Demote `build` subagents when the root agent can't write files.
        let agent = resolve_subagent_agent(self.allow_write, agent);
        let mut child = self
            .runtime
            .store
            .create_session(agent, &self.runtime_current_cwd())
            .map_err(|e| e.to_string())?;
        child.agent = agent.to_string();
        let child_cwd = child.cwd.clone();
        // Link the child to this session so UIs can group its events and the
        // store can garbage-collect orphans when the parent is deleted.
        self.runtime
            .store
            .set_session_parent(&child.id, &child_cwd, Some(&self.parent_session_id))
            .map_err(|e| e.to_string())?;

        let result = self
            .runtime
            .prompt(
                &mut child,
                &events,
                &prompt,
                crate::harness::tool::context::AbortSignal::new(),
                None,
            )
            .await
            .map_err(|e| e.to_string())?;

        Ok(TaskOutcome {
            final_text: result.final_text,
            session_id: child.id.clone(),
            iterations: result.iterations,
        })
    }
}

impl TaskRunner {
    fn runtime_current_cwd(&self) -> PathBuf {
        self.runtime.project_root.clone()
    }
}
