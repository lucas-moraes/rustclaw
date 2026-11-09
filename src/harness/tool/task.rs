//! `task` tool: spawns a subagent (child session) to complete a task.
//!
//! Supports a single task (`prompt`/`agent`) or a batch (`tasks: [...]`) that
//! runs concurrently (capped by `MAX_PARALLEL_TASKS`), with results aggregated
//! in input order.

use super::{Tool, ToolResult};
use crate::harness::agent::find_builtin;
use crate::harness::session::preview;
use crate::harness::tool::context::{TaskOutcome, ToolContext};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Max subagents running concurrently within one batch tool call.
const MAX_PARALLEL_TASKS: usize = 4;
/// Agent used when the caller does not specify one.
const DEFAULT_AGENT: &str = "reason";
/// Maximum subagent nesting depth (root agent = 0). A subagent at this depth
/// may not spawn further subagents, preventing unbounded recursion.
pub const MAX_SUBAGENT_DEPTH: usize = 3;
/// Per-task output cap (chars) in the aggregated result.
const PER_TASK_CHARS: usize = 4000;
/// Total output budget (chars) for the aggregated batch result. Divided
/// evenly among the batch tasks, so 4 tasks get 4000 chars each.
const TOTAL_CHARS: usize = 16000;

pub struct TaskTool;

struct BatchEntry {
    description: String,
    /// Canonical agent name (as reported by the registry), never an alias.
    agent: String,
    prompt: String,
}

/// Validates an agent name against the builtin registry and returns its
/// canonical name. Aliases (plan/explore/general) resolve to `reason`.
fn canonical_agent(raw: &str) -> Result<String, String> {
    match find_builtin(raw) {
        Some(spec) => Ok(spec.name),
        None => Err(format!(
            "unknown agent `{raw}` (valid: build, reason, chat-free, cursor, cursor_plan)"
        )),
    }
}

/// Label for an agent in the output. Shows `requested → effective` when the
/// runner demoted the subagent (e.g. build requested but run as reason).
fn agent_label(requested: &str, effective: &str) -> String {
    if effective.is_empty() || effective.eq_ignore_ascii_case(requested) {
        requested.to_string()
    } else {
        format!("{requested} → {effective} (demoted)")
    }
}

fn delivery_status(delivery: &crate::harness::session::delivery::DeliveryReport) -> &'static str {
    if !delivery.has_changes() {
        ""
    } else if delivery.verified {
        " [delivery: verified]"
    } else {
        " [delivery: pending]"
    }
}

/// Parses either the single-task form or the batch form.
fn parse_entries(args: &Value) -> Result<Vec<BatchEntry>, String> {
    if let Some(list) = args["tasks"].as_array() {
        if list.is_empty() {
            return Err("tasks: empty batch".to_string());
        }
        return list
            .iter()
            .enumerate()
            .map(|(i, t)| {
                Ok(BatchEntry {
                    description: t["description"].as_str().unwrap_or("").to_string(),
                    agent: canonical_agent(t["agent"].as_str().unwrap_or(DEFAULT_AGENT))
                        .map_err(|e| format!("tasks[{i}]: {e}"))?,
                    prompt: t["prompt"]
                        .as_str()
                        .ok_or_else(|| format!("tasks[{i}]: missing required argument: prompt"))?
                        .to_string(),
                })
            })
            .collect();
    }
    let prompt = args["prompt"]
        .as_str()
        .ok_or_else(|| "missing required argument: prompt".to_string())?;
    Ok(vec![BatchEntry {
        description: args["description"].as_str().unwrap_or("").to_string(),
        agent: canonical_agent(args["agent"].as_str().unwrap_or(DEFAULT_AGENT))?,
        prompt: prompt.to_string(),
    }])
}

#[async_trait::async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }
    fn description(&self) -> &str {
        "Spawns a subagent (default: reason; also build, chat-free, cursor, \
         cursor_plan) to complete a delegated task. Returns the subagent's \
         summary. Use for parallelizable or self-contained research/implementation \
         work. To run several independent tasks in parallel, pass \
         `tasks: [{description, prompt, agent}, ...]` instead of a single `prompt`."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {"type": "string", "description": "Short description of the task"},
                "prompt": {"type": "string", "description": "Full instructions for the subagent (single task)"},
                "agent": {"type": "string", "description": "reason|build|chat-free|cursor|cursor_plan (default reason)"},
                "fail_fast": {"type": "boolean", "description": "Batch only: skip remaining tasks once one fails (default false)"},
                "tasks": {
                    "type": "array",
                    "description": "Batch of independent tasks to run in parallel (max 4 concurrent)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "description": {"type": "string"},
                            "prompt": {"type": "string"},
                            "agent": {"type": "string"}
                        },
                        "required": ["description", "prompt"]
                    }
                }
            },
            "required": ["description"]
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        // Enforce the nesting cap: a subagent at max depth must finish the work
        // itself rather than delegating further.
        if ctx.depth >= MAX_SUBAGENT_DEPTH {
            return Err(format!(
                "Maximum subagent nesting depth ({MAX_SUBAGENT_DEPTH}) reached. \
                 Complete this task directly instead of spawning another subagent."
            ));
        }

        let entries = parse_entries(&args)?;

        let runner = ctx
            .task_runner
            .clone()
            .ok_or_else(|| "subagent runner not available".to_string())?;

        // Single task: keep the simple path (and its result shape).
        if entries.len() == 1 {
            let e = &entries[0];
            let outcome = runner
                .run_task(
                    e.agent.clone(),
                    e.prompt.clone(),
                    ctx.events.clone(),
                    ctx.depth + 1,
                )
                .await?;
            let label = agent_label(&e.agent, &outcome.effective_agent);
            return Ok(ToolResult {
                title: format!("task ({})", preview(&describe(&e.description, &label), 40)),
                output: format!(
                    "Subagent `{}` (session `{}`, {} iterations, {}s){} result:\n{}",
                    label,
                    outcome.session_id,
                    outcome.iterations,
                    outcome.duration_secs,
                    delivery_status(&outcome.delivery),
                    preview(&outcome.final_text, PER_TASK_CHARS)
                ),
                metadata: json!({ "delivery": outcome.delivery }),
            });
        }

        let fail_fast = args["fail_fast"].as_bool().unwrap_or(false);
        let failed = Arc::new(AtomicBool::new(false));

        // Batch: run concurrently with a concurrency cap, preserving input order.
        let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_PARALLEL_TASKS));
        let mut join_set = tokio::task::JoinSet::new();
        for (idx, e) in entries.iter().enumerate() {
            let runner = runner.clone();
            let events = ctx.events.clone();
            let agent = e.agent.clone();
            let prompt = e.prompt.clone();
            let sem = semaphore.clone();
            let abort = ctx.abort.clone();
            let failed = failed.clone();
            let child_depth = ctx.depth + 1;
            join_set.spawn(async move {
                let _permit = sem.acquire_owned().await;
                if abort.is_aborted() {
                    return (idx, Err("aborted".to_string()));
                }
                if fail_fast && failed.load(Ordering::SeqCst) {
                    return (
                        idx,
                        Err("skipped (fail_fast: an earlier task failed)".to_string()),
                    );
                }
                let result = runner.run_task(agent, prompt, events, child_depth).await;
                if result.is_err() {
                    failed.store(true, Ordering::SeqCst);
                }
                (idx, result)
            });
        }

        // Collect in completion order, then reassemble by input index.
        let mut slots: Vec<Option<Result<TaskOutcome, String>>> =
            (0..entries.len()).map(|_| None).collect();
        while let Some(joined) = join_set.join_next().await {
            let Ok((idx, result)) = joined else {
                continue;
            };
            slots[idx] = Some(result);
        }

        // Aggregate: each task gets an equal share of TOTAL_CHARS (and at most
        // PER_TASK_CHARS), so late tasks are not starved by early ones.
        let per_task_budget = (TOTAL_CHARS / entries.len()).min(PER_TASK_CHARS);
        let mut out = String::new();
        let mut truncated_count = 0usize;
        for (i, (entry, slot)) in entries.iter().zip(slots.iter()).enumerate() {
            let section = match slot {
                Some(Ok(outcome)) => {
                    let label = agent_label(&entry.agent, &outcome.effective_agent);
                    format!(
                        "## task {} ({}) — ✓{}\n{}\n",
                        i + 1,
                        describe(&entry.description, &label),
                        delivery_status(&outcome.delivery),
                        preview(&outcome.final_text, per_task_budget)
                    )
                }
                Some(Err(e)) => format!(
                    "## task {} ({}) — ✗\n{}\n",
                    i + 1,
                    describe(&entry.description, &entry.agent),
                    e
                ),
                None => format!("## task {} — ✗ (cancelled)\n", i + 1),
            };
            // Sections are already capped by `preview`; this only flags when
            // the final task text itself was cut.
            if let Some(Ok(outcome)) = slot {
                if outcome.final_text.chars().count() > per_task_budget {
                    truncated_count += 1;
                }
            }
            out.push_str(&section);
        }
        if truncated_count > 0 {
            out.push_str(&format!(
                "\n[{} of {} tasks truncated — ask a specific follow-up via message_send if you need the full output]\n",
                truncated_count,
                entries.len()
            ));
        }

        let deliveries: Vec<_> = slots
            .iter()
            .filter_map(|slot| match slot {
                Some(Ok(outcome)) => Some(&outcome.delivery),
                _ => None,
            })
            .collect();
        Ok(ToolResult {
            title: format!("task batch ({} subagents)", entries.len()),
            output: out,
            metadata: json!({ "deliveries": deliveries }),
        })
    }
}

/// Builds the `description (agent)` label used in batch section headers.
fn describe(description: &str, agent_label: &str) -> String {
    if description.is_empty() {
        agent_label.to_string()
    } else {
        format!("{} — {}", agent_label, preview(description, 80))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::event::EventSender;
    use crate::harness::tool::context::{SubagentRunner, TaskOutcome};
    use std::sync::Mutex;

    struct FakeRunner {
        /// Delay (ms) per task index, to exercise concurrency.
        delay_ms: u64,
        concurrent: Arc<std::sync::atomic::AtomicUsize>,
        max_concurrent: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl SubagentRunner for FakeRunner {
        async fn run_task(
            &self,
            agent: String,
            _prompt: String,
            _events: EventSender,
            _depth: usize,
        ) -> Result<TaskOutcome, String> {
            let now = self
                .concurrent
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            self.max_concurrent
                .fetch_max(now, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            self.concurrent
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            Ok(TaskOutcome {
                final_text: format!("{} done", agent),
                session_id: format!("child-{}", agent),
                iterations: 1,
                effective_agent: String::new(),
                duration_secs: 0,
                delivery: Default::default(),
            })
        }
    }

    fn ctx_with(
        runner: Arc<dyn SubagentRunner>,
    ) -> (
        ToolContext,
        EventSender,
        tokio::sync::mpsc::UnboundedReceiver<crate::harness::event::HarnessEvent>,
    ) {
        use crate::harness::permission::PermissionEngine;
        struct AllowAsker;
        #[async_trait::async_trait]
        impl crate::harness::tool::context::PermissionAsker for AllowAsker {
            async fn ask(&self, _: crate::harness::tool::context::PermissionAskInput) -> bool {
                true
            }
        }
        struct NoUserAsker;
        #[async_trait::async_trait]
        impl crate::harness::tool::context::UserAsker for NoUserAsker {
            async fn ask(&self, _: String, _: Vec<String>) -> Option<String> {
                None
            }
        }
        let (tx, rx) = crate::harness::event::event_channel();
        let ctx = ToolContext {
            session_id: "s".into(),
            agent: "build".into(),
            agent_tools: vec![],
            cwd: crate::harness::tool::context::PathBufGuard(std::path::PathBuf::from("/tmp")),
            abort: crate::harness::tool::context::AbortSignal::new(),
            permission: Arc::new(PermissionEngine::default()),
            asker: Arc::new(AllowAsker),
            user_asker: Arc::new(NoUserAsker),
            todos: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            task_runner: Some(runner),
            events: tx.clone(),
            project_memory: None,
            hooks: Default::default(),
            checkpoints: std::sync::Arc::new(
                crate::harness::tool::checkpoint::FileCheckpoints::new(),
            ),
            jobs: std::sync::Arc::new(crate::harness::tool::jobs::JobRegistry::new()),
            semantic_index: None,
            embedder: None,
            sandbox_policy: None,
            depth: 0,
            message_bus: None,
            shared_memory: None,
            delivery: Default::default(),
        };
        (ctx, tx, rx)
    }

    fn batch_args(n: usize) -> Value {
        let agents = ["reason", "build", "reason", "build"];
        let tasks: Vec<Value> = (0..n)
            .map(|i| {
                json!({
                    "description": format!("t{i}"),
                    "prompt": format!("p{i}"),
                    "agent": agents[i % agents.len()],
                })
            })
            .collect();
        json!({"description": "batch", "tasks": tasks})
    }

    #[tokio::test]
    async fn test_batch_runs_all_and_preserves_order() {
        let runner = Arc::new(FakeRunner {
            delay_ms: 10,
            concurrent: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            max_concurrent: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        });
        let (ctx, _tx, _rx) = ctx_with(runner.clone() as Arc<dyn SubagentRunner>);
        let result = TaskTool.execute(batch_args(3), &ctx).await.unwrap();
        let agents = ["reason", "build", "reason"];
        for i in 1..=3 {
            assert!(
                result.output.contains(&format!(
                    "## task {} ({} — t{}) — ✓",
                    i,
                    agents[i - 1],
                    i - 1
                )),
                "missing ordered section {i}: {}",
                result.output
            );
            assert!(result.output.contains(&format!("{} done", agents[i - 1])));
        }
    }

    #[tokio::test]
    async fn test_batch_respects_concurrency_cap() {
        let runner = Arc::new(FakeRunner {
            delay_ms: 50,
            concurrent: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            max_concurrent: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        });
        let (ctx, _tx, _rx) = ctx_with(runner.clone() as Arc<dyn SubagentRunner>);
        let _ = TaskTool.execute(batch_args(6), &ctx).await.unwrap();
        let max = runner
            .max_concurrent
            .load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            max <= MAX_PARALLEL_TASKS,
            "concurrency {max} exceeded cap {MAX_PARALLEL_TASKS}"
        );
    }

    #[tokio::test]
    async fn test_batch_aborts_remaining_tasks() {
        struct SlowRunner;
        #[async_trait::async_trait]
        impl SubagentRunner for SlowRunner {
            async fn run_task(
                &self,
                _agent: String,
                _prompt: String,
                _events: EventSender,
                _depth: usize,
            ) -> Result<TaskOutcome, String> {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                Ok(TaskOutcome {
                    final_text: "late".into(),
                    session_id: "c".into(),
                    iterations: 1,
                    effective_agent: String::new(),
                    duration_secs: 0,
                    delivery: Default::default(),
                })
            }
        }
        let (ctx, _tx, _rx) = ctx_with(Arc::new(SlowRunner));
        let abort_ctx = ctx.clone();
        let handle = tokio::spawn(async move { TaskTool.execute(batch_args(2), &abort_ctx).await });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        ctx.abort.abort();
        let result = handle.await.unwrap().unwrap();
        // Aborted tasks either fail fast (✗) or complete late (✓) — the point
        // is that the tool call returns promptly instead of hanging.
        assert!(
            result.output.contains("task 1") && result.output.contains("task 2"),
            "expected both sections: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_batch_fail_fast_skips_remaining() {
        /// Task 1 fails immediately; the rest are slow, so the tasks queued
        /// behind the concurrency cap observe the failure and are skipped.
        struct FailFirst;
        #[async_trait::async_trait]
        impl SubagentRunner for FailFirst {
            async fn run_task(
                &self,
                agent: String,
                prompt: String,
                _events: EventSender,
                _depth: usize,
            ) -> Result<TaskOutcome, String> {
                if prompt == "p0" {
                    return Err("boom".into());
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                Ok(TaskOutcome {
                    final_text: format!("{agent} done"),
                    session_id: "c".into(),
                    iterations: 1,
                    effective_agent: String::new(),
                    duration_secs: 0,
                    delivery: Default::default(),
                })
            }
        }
        let (ctx, _tx, _rx) = ctx_with(Arc::new(FailFirst));
        let mut args = batch_args(6);
        args["fail_fast"] = json!(true);
        let result = TaskTool.execute(args, &ctx).await.unwrap();
        assert!(result.output.contains("## task 1 (reason — t0) — ✗"));
        assert!(result.output.contains("boom"));
        assert!(
            result.output.contains("skipped (fail_fast"),
            "expected skipped sections: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_single_task_shape_unchanged() {
        struct EchoRunner(Mutex<Vec<String>>);
        #[async_trait::async_trait]
        impl SubagentRunner for EchoRunner {
            async fn run_task(
                &self,
                agent: String,
                _prompt: String,
                _events: EventSender,
                _depth: usize,
            ) -> Result<TaskOutcome, String> {
                self.0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(agent.clone());
                Ok(TaskOutcome {
                    final_text: "the answer".into(),
                    session_id: "c".into(),
                    iterations: 1,
                    effective_agent: String::new(),
                    duration_secs: 0,
                    delivery: Default::default(),
                })
            }
        }
        let echo = Arc::new(EchoRunner(Mutex::new(Vec::new())));
        let (ctx, _tx, _rx) = ctx_with(echo.clone() as Arc<dyn SubagentRunner>);
        let result = TaskTool
            .execute(
                json!({"description": "d", "prompt": "p", "agent": "explore"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result
            .output
            .contains("Subagent `reason` (session `c`, 1 iterations, 0s) result:"));
        assert!(result.output.contains("the answer"));
        assert_eq!(echo.0.lock().unwrap_or_else(|e| e.into_inner()).len(), 1);
        // The legacy alias is canonicalised before reaching the runner.
        assert_eq!(
            echo.0.lock().unwrap_or_else(|e| e.into_inner())[0],
            "reason"
        );
    }

    struct DeliveryRunner;

    #[async_trait::async_trait]
    impl SubagentRunner for DeliveryRunner {
        async fn run_task(
            &self,
            agent: String,
            prompt: String,
            _events: EventSender,
            _depth: usize,
        ) -> Result<TaskOutcome, String> {
            if prompt == "error" {
                return Err("child failed".into());
            }
            let mut outcome = TaskOutcome {
                final_text: format!("{prompt} result"),
                session_id: "child-delivery".into(),
                iterations: 1,
                effective_agent: agent,
                duration_secs: 0,
                delivery: Default::default(),
            };
            if prompt != "unchanged" {
                outcome.delivery.changed_files.push(format!("{prompt}.rs"));
                outcome.delivery.verified = prompt == "verified";
            }
            Ok(outcome)
        }
    }

    #[tokio::test]
    async fn test_single_delivery_metadata_and_status_without_merging() {
        for (prompt, expected_status) in [
            ("unchanged", ""),
            ("pending", " [delivery: pending]"),
            ("verified", " [delivery: verified]"),
        ] {
            let (ctx, _tx, _rx) = ctx_with(Arc::new(DeliveryRunner));
            let result = TaskTool
                .execute(json!({"prompt": prompt, "agent": "build"}), &ctx)
                .await
                .unwrap();
            let delivery: crate::harness::session::delivery::DeliveryReport =
                serde_json::from_value(result.metadata["delivery"].clone()).unwrap();
            assert_eq!(delivery.has_changes(), prompt != "unchanged");
            assert_eq!(delivery.verified, prompt == "verified");
            assert_eq!(
                result.output,
                format!(
                    "Subagent `build` (session `child-delivery`, 1 iterations, 0s){expected_status} result:\n{prompt} result"
                )
            );
            assert!(!ctx.delivery.snapshot().has_changes());
        }
    }

    #[tokio::test]
    async fn test_batch_delivery_metadata_preserves_success_order_without_merging() {
        let (ctx, _tx, _rx) = ctx_with(Arc::new(DeliveryRunner));
        let result = TaskTool
            .execute(
                json!({"tasks": [
                    {"prompt": "verified", "agent": "build", "description": "verify"},
                    {"prompt": "error", "agent": "build"},
                    {"prompt": "unchanged", "agent": "build"},
                    {"prompt": "pending", "agent": "build"}
                ]}),
                &ctx,
            )
            .await
            .unwrap();
        let deliveries: Vec<crate::harness::session::delivery::DeliveryReport> =
            serde_json::from_value(result.metadata["deliveries"].clone()).unwrap();
        assert_eq!(deliveries.len(), 3);
        assert_eq!(deliveries[0].changed_files, ["verified.rs"]);
        assert!(deliveries[0].verified);
        assert!(!deliveries[1].has_changes());
        assert_eq!(deliveries[2].changed_files, ["pending.rs"]);
        assert!(!deliveries[2].verified);
        assert!(result
            .output
            .contains("## task 1 (build — verify) — ✓ [delivery: verified]\nverified result"));
        assert!(result
            .output
            .contains("## task 2 (build) — ✗\nchild failed"));
        assert!(result
            .output
            .contains("## task 3 (build) — ✓\nunchanged result"));
        assert!(result
            .output
            .contains("## task 4 (build) — ✓ [delivery: pending]\npending result"));
        assert!(!ctx.delivery.snapshot().has_changes());
    }

    #[tokio::test]
    async fn test_depth_cap_rejects_nested_spawn() {
        struct NeverRunner;
        #[async_trait::async_trait]
        impl SubagentRunner for NeverRunner {
            async fn run_task(
                &self,
                _agent: String,
                _prompt: String,
                _events: EventSender,
                _depth: usize,
            ) -> Result<TaskOutcome, String> {
                panic!("runner must not be called at max depth");
            }
        }
        let (mut ctx, _tx, _rx) = ctx_with(Arc::new(NeverRunner));
        ctx.depth = MAX_SUBAGENT_DEPTH;
        let err = TaskTool
            .execute(json!({"description": "d", "prompt": "p"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.contains("Maximum subagent nesting depth"), "got: {err}");
    }

    #[tokio::test]
    async fn test_depth_propagates_to_child() {
        struct DepthRunner(Arc<std::sync::Mutex<Vec<usize>>>);
        #[async_trait::async_trait]
        impl SubagentRunner for DepthRunner {
            async fn run_task(
                &self,
                _agent: String,
                _prompt: String,
                _events: EventSender,
                depth: usize,
            ) -> Result<TaskOutcome, String> {
                self.0.lock().unwrap_or_else(|e| e.into_inner()).push(depth);
                Ok(TaskOutcome {
                    final_text: "ok".into(),
                    session_id: "c".into(),
                    iterations: 1,
                    effective_agent: String::new(),
                    duration_secs: 0,
                    delivery: Default::default(),
                })
            }
        }
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (mut ctx, _tx, _rx) = ctx_with(Arc::new(DepthRunner(seen.clone())));
        ctx.depth = 1;
        TaskTool
            .execute(json!({"description": "d", "prompt": "p"}), &ctx)
            .await
            .unwrap();
        assert_eq!(*seen.lock().unwrap_or_else(|e| e.into_inner()), vec![2]);
    }
}
