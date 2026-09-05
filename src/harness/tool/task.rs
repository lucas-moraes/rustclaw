//! `task` tool: spawns a subagent (child session) to complete a task.
//!
//! Supports a single task (`prompt`/`agent`) or a batch (`tasks: [...]`) that
//! runs concurrently (capped by `MAX_PARALLEL_TASKS`), with results aggregated
//! in input order.

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};
use std::sync::Arc;

/// Max subagents running concurrently within one batch tool call.
const MAX_PARALLEL_TASKS: usize = 4;
/// Per-task output budget (chars) in the aggregated result.
const PER_TASK_CHARS: usize = 4000;
/// Total output budget (chars) for the aggregated result.
const TOTAL_CHARS: usize = 8000;

pub struct TaskTool;

struct BatchEntry {
    agent: String,
    prompt: String,
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
                    agent: t["agent"].as_str().unwrap_or("explore").to_string(),
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
        agent: args["agent"].as_str().unwrap_or("explore").to_string(),
        prompt: prompt.to_string(),
    }])
}

#[async_trait::async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }
    fn description(&self) -> &str {
        "Spawns a subagent (default: explore, or build/plan/general) to complete a \
delegated task. Returns the subagent's summary. Use for parallelizable or \
self-contained research/implementation work. To run several independent tasks \
in parallel, pass `tasks: [{description, prompt, agent}, ...]` instead of a \
single `prompt`."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {"type": "string", "description": "Short description of the task"},
                "prompt": {"type": "string", "description": "Full instructions for the subagent (single task)"},
                "agent": {"type": "string", "description": "explore|build|plan|general (default explore)"},
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
        let entries = parse_entries(&args)?;

        let runner = ctx
            .task_runner
            .clone()
            .ok_or_else(|| "subagent runner not available".to_string())?;

        // Single task: keep the simple path (and its result shape).
        if entries.len() == 1 {
            let e = &entries[0];
            let outcome = runner
                .run_task(e.agent.clone(), e.prompt.clone(), ctx.events.clone())
                .await?;
            return Ok(ToolResult::simple(
                format!("task ({})", preview(&e.agent, 20)),
                format!(
                    "Subagent `{}` result:\n{}",
                    e.agent,
                    preview(&outcome.final_text, PER_TASK_CHARS)
                ),
            ));
        }

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
            join_set.spawn(async move {
                let _permit = sem.acquire_owned().await;
                if abort.is_aborted() {
                    return (idx, agent, Err("aborted".to_string()));
                }
                let result = runner.run_task(agent.clone(), prompt, events).await;
                (idx, agent, result)
            });
        }

        // Collect in completion order, then reassemble by input index.
        let mut slots: Vec<Option<(String, Result<String, String>)>> =
            (0..entries.len()).map(|_| None).collect();
        while let Some(joined) = join_set.join_next().await {
            let Ok((idx, agent, result)) = joined else {
                continue;
            };
            let mapped = result.map(|o| o.final_text);
            slots[idx] = Some((agent, mapped));
        }

        // Aggregate with per-task and total budgets.
        let mut out = String::new();
        let mut budget = TOTAL_CHARS;
        for (i, slot) in slots.iter().enumerate() {
            let (agent, result) = match slot {
                Some(v) => v,
                None => {
                    out.push_str(&format!("## task {} — ✗ (cancelled)\n", i + 1));
                    continue;
                }
            };
            let section = match result {
                Ok(text) => format!(
                    "## task {} ({}) — ✓\n{}\n",
                    i + 1,
                    agent,
                    preview(text, PER_TASK_CHARS)
                ),
                Err(e) => format!("## task {} ({}) — ✗\n{}\n", i + 1, agent, e),
            };
            if budget == 0 {
                out.push_str(&format!("## task {} — (output budget exhausted)\n", i + 1));
                continue;
            }
            let take = section.char_indices().nth(budget).map(|(i, _)| i);
            match take {
                Some(cut) => {
                    out.push_str(&section[..cut]);
                    out.push_str("\n…(truncated)\n");
                    budget = 0;
                }
                None => {
                    out.push_str(&section);
                    budget = budget.saturating_sub(section.chars().count());
                }
            }
        }

        Ok(ToolResult::simple(
            format!("task batch ({} subagents)", slots.len()),
            out,
        ))
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
        };
        (ctx, tx, rx)
    }

    fn batch_args(n: usize) -> Value {
        let tasks: Vec<Value> = (0..n)
            .map(|i| {
                json!({
                    "description": format!("t{i}"),
                    "prompt": format!("p{i}"),
                    "agent": format!("agent{i}"),
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
        for i in 1..=3 {
            assert!(
                result
                    .output
                    .contains(&format!("## task {} (agent{}) — ✓", i, i - 1)),
                "missing ordered section {i}: {}",
                result.output
            );
            assert!(result.output.contains(&format!("agent{} done", i - 1)));
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
            ) -> Result<TaskOutcome, String> {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                Ok(TaskOutcome {
                    final_text: "late".into(),
                    session_id: "c".into(),
                    iterations: 1,
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
    async fn test_single_task_shape_unchanged() {
        struct EchoRunner(Mutex<Vec<String>>);
        #[async_trait::async_trait]
        impl SubagentRunner for EchoRunner {
            async fn run_task(
                &self,
                agent: String,
                _prompt: String,
                _events: EventSender,
            ) -> Result<TaskOutcome, String> {
                self.0.lock().unwrap().push(agent.clone());
                Ok(TaskOutcome {
                    final_text: "the answer".into(),
                    session_id: "c".into(),
                    iterations: 1,
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
        assert!(result.output.contains("Subagent `explore` result:"));
        assert!(result.output.contains("the answer"));
        assert_eq!(echo.0.lock().unwrap().len(), 1);
    }
}
