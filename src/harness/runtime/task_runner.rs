//! Subagent task runner: spawns child sessions through the runtime.

use super::scheduler::SubagentScheduler;
use super::SessionRuntime;
use crate::harness::event::{EventReceiver, EventSender, HarnessEvent, SubagentInfo};
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
    /// Process-wide concurrency limiter shared by every subagent, so parallel
    /// batches and nested spawns queue behind the same cap.
    pub scheduler: SubagentScheduler,
}

/// Makes a terminal event available even when the run future is dropped.
/// A successful runtime may already have sent one; the relay stops at its first
/// terminal for this session, so this fallback is never duplicated downstream.
struct ChildRunGuard {
    events: EventSender,
    session_id: String,
    fallback: String,
}

impl Drop for ChildRunGuard {
    fn drop(&mut self) {
        let _ = self.events.send(HarnessEvent::RunFinished {
            session_id: self.session_id.clone(),
            parent_session_id: None,
        });
    }
}

async fn relay_child_events(
    mut child_rx: EventReceiver,
    events: EventSender,
    parent_id: String,
    child_id: String,
    depth: usize,
) {
    while let Some(event) = child_rx.recv().await {
        // Nested runs share this stream. Their terminals must not cut off the
        // immediate child's remaining output or its own terminal event.
        let finished = matches!(
            &event,
            HarnessEvent::RunFinished { session_id, .. } if session_id == &child_id
        );
        if events.send(event.tag_child(&parent_id, depth)).is_err() || finished {
            break;
        }
    }
}

/// Resolves the effective subagent agent given whether the parent allows file
/// writes. When writes are not allowed, a requested `build` subagent is
/// demoted to `reason` (same toolset minus write/edit) so subagents can
/// never edit files when the root agent isn't `build`.
pub(crate) fn resolve_subagent_agent(allow_write: bool, requested: &str) -> &str {
    use crate::harness::agent::builtin::{BUILD, REASON};

    // `build` is demoted to `reason` when writing is disabled.
    if !allow_write && requested == BUILD {
        return REASON;
    }

    // Normalise the merged-away mode names so subagent panels and events show
    // `reason` rather than a name that no longer exists in MODES.
    match requested {
        "plan" | "explore" | "general" => REASON,
        _ => requested,
    }
}

#[async_trait::async_trait]
impl SubagentRunner for TaskRunner {
    async fn run_task(
        &self,
        agent: String,
        prompt: String,
        events: crate::harness::event::EventSender,
        depth: usize,
    ) -> Result<TaskOutcome, String> {
        // Wait for a global slot before touching the provider. The permit is
        // held for the whole run and released on drop (including on error).
        let _slot = self.scheduler.acquire().await;

        // Resolve agent to allow "reason" by default.
        let agent = if agent.is_empty() {
            crate::harness::agent::builtin::REASON
        } else {
            &agent
        };
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

        // Relay until this child's terminal, not a nested descendant's. A
        // detached relay also drains the RAII terminal if this future is dropped.
        let (child_tx, child_rx) = crate::harness::event::event_channel();
        let relay = tokio::spawn(relay_child_events(
            child_rx,
            events,
            self.parent_session_id.clone(),
            child.id.clone(),
            depth,
        ));
        let mut run_guard = ChildRunGuard {
            events: child_tx.clone(),
            session_id: child.id.clone(),
            fallback: "Subagent cancelled before completion".into(),
        };
        let _ = child_tx.send(HarnessEvent::RunStarted {
            session_id: child.id.clone(),
            parent_session_id: None,
            subagent: Some(SubagentInfo {
                agent: agent.to_string(),
                depth,
            }),
        });

        // Tell the child who it is and who its parent is, so it can use
        // `message_send` / `shared_memory` without guessing session ids.
        let prompt = format!(
            "{prompt}\n\n---\n[swarm] You are session `{}` (agent `{agent}`, depth {depth}). \
Your parent session is `{}`.\n\
- `message_send` → the parent: fire-and-forget progress notes or findings. Use \
it when you discover something the parent should know *before* you finish.\n\
- `message_ask` → the parent: block and wait for a decision when you are stuck \
on a requirement that is not yours to invent (ambiguous spec, approach choice). \
Do not guess when the answer changes the design.\n\
- `message_inbox`: check for messages the parent sent you mid-run. Call it \
before assuming you are alone.\n\
- `shared_memory`: publish data other agents can read.",
            child.id, self.parent_session_id
        );

        let result = self
            .runtime
            .prompt_at_depth(
                &mut child,
                &child_tx,
                &prompt,
                crate::harness::tool::context::AbortSignal::new(),
                None,
                depth,
            )
            .await
            .map_err(|e| e.to_string());

        run_guard.fallback = match &result {
            Ok(result) => result.final_text.clone(),
            Err(error) => format!("Subagent failed: {error}"),
        };
        // Queue the fallback before awaiting the relay, including on errors.
        // Runtime/session tasks may retain sender clones, so channel closure is
        // not a reliable completion signal. The first own terminal is.
        drop(run_guard);
        drop(child_tx);
        let _ = relay.await;

        let result = result?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::event::{event_channel, HarnessEvent};
    use crate::harness::permission::PermissionEngine;
    use crate::harness::provider::scripted::{ScriptedProvider, ScriptedTurn};
    use crate::harness::runtime::registry::build_default_registry;
    use crate::harness::tool::context::{
        AbortSignal, PermissionAskInput, PermissionAsker, UserAsker,
    };

    struct AllowAsker;
    #[async_trait::async_trait]
    impl PermissionAsker for AllowAsker {
        async fn ask(&self, _req: PermissionAskInput) -> bool {
            true
        }
    }

    struct NoUserAsker;
    #[async_trait::async_trait]
    impl UserAsker for NoUserAsker {
        async fn ask(&self, _q: String, _o: Vec<String>) -> Option<String> {
            None
        }
    }

    /// Regression: the `TaskRunner` must tag every child event with the parent
    /// session id (and the child's depth). Before the fix, child events arrived
    /// untagged, so a subagent's `RunFinished` was mistaken for the parent turn
    /// ending and the UI flipped to "idle" mid-turn.
    #[tokio::test]
    async fn test_child_events_are_tagged_with_parent() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        // Turn 1: parent calls `task`. Turn 2: child answers. Turn 3: parent
        // answers. The provider is shared, so turns are consumed in that order.
        let provider = Arc::new(ScriptedProvider::new(vec![
            ScriptedTurn::tool(
                "task",
                serde_json::json!({"description": "child", "prompt": "do it"}),
            ),
            ScriptedTurn::text("child done"),
            ScriptedTurn::text("parent done"),
        ]));
        let runtime = Arc::new(
            SessionRuntime::new_in(
                dir.path(),
                provider,
                build_default_registry(),
                crate::config::RuntimeConfig {
                    model: "scripted".into(),
                    provider: "scripted".into(),
                    base_url: String::new(),
                    api_key: "test".into(),
                    ..Default::default()
                },
                &db,
                Arc::new(PermissionEngine::default()),
                Arc::new(AllowAsker),
                Arc::new(NoUserAsker),
            )
            .unwrap(),
        );
        let mut parent = runtime.create_session("build").await.unwrap();
        let parent_id = parent.id.clone();

        let (tx, mut rx) = event_channel();
        let collector = tokio::spawn(async move {
            let mut events = Vec::new();
            while let Some(ev) = rx.recv().await {
                events.push(ev);
            }
            events
        });

        runtime
            .prompt(
                &mut parent,
                &tx,
                "spawn a subagent",
                AbortSignal::new(),
                None,
            )
            .await
            .unwrap();
        drop(tx);
        let events = collector.await.unwrap();

        // Every child event that carries a `parent_session_id` field must be
        // tagged with the parent id. (`UserMessage` has no such field, so it is
        // skipped — it is an internal event the UI does not route by parent.)
        let child_events: Vec<_> = events
            .iter()
            .filter(|e| e.session_id() != Some(parent_id.as_str()))
            .filter(|e| {
                matches!(
                    e,
                    HarnessEvent::RunStarted { .. }
                        | HarnessEvent::RunFinished { .. }
                        | HarnessEvent::TextDelta { .. }
                        | HarnessEvent::ReasoningDelta { .. }
                        | HarnessEvent::MessageUpdated { .. }
                        | HarnessEvent::ToolStart { .. }
                        | HarnessEvent::ToolEnd { .. }
                        | HarnessEvent::CompactionStarted { .. }
                        | HarnessEvent::CompactionFinished { .. }
                        | HarnessEvent::AutoContinue { .. }
                        | HarnessEvent::Error { .. }
                        | HarnessEvent::Rollback { .. }
                )
            })
            .collect();
        assert!(
            !child_events.is_empty(),
            "expected child events, got none: {events:?}"
        );
        for ev in &child_events {
            assert_eq!(
                ev.parent_session_id(),
                Some(parent_id.as_str()),
                "child event not tagged with parent: {ev:?}"
            );
        }

        // The child's `RunFinished` must be tagged, so the UI does not treat it
        // as the parent turn ending.
        let child_finished = events.iter().find(|e| {
            matches!(e, HarnessEvent::RunFinished { parent_session_id, .. } if parent_session_id.is_some())
        });
        assert!(
            child_finished.is_some(),
            "child RunFinished was not tagged: {events:?}"
        );

        // The parent's own `RunFinished` stays untagged.
        let parent_finished = events.iter().find(|e| {
            matches!(e, HarnessEvent::RunFinished { session_id, parent_session_id }
                if session_id == &parent_id && parent_session_id.is_none())
        });
        assert!(
            parent_finished.is_some(),
            "parent RunFinished missing/incorrectly tagged: {events:?}"
        );
    }
}
