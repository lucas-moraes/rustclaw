//! Reproducible evals: synthetic tasks with automatic success criteria.
//!
//! Without evals there is no way to tell whether a change to the prompt, the
//! loop or a tool helped or hurt. Each [`Eval`] drives the *real* runtime
//! (streaming loop, tool execution, compaction, metrics) against a
//! [`ScriptedProvider`], so the whole pipeline is exercised deterministically
//! and offline — no network, no token, no flakiness.
//!
//! An eval asserts on observable outcomes: which tools ran, what the model saw
//! after a tool result, how many iterations the loop took, and the recorded
//! [`SessionMetrics`]. The suite is meant to run in CI as a fast regression
//! gate (see `run_all`).
//!
//! [`ScriptedProvider`]: crate::harness::provider::scripted::ScriptedProvider
//! [`SessionMetrics`]: crate::harness::session::metrics::SessionMetrics

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::harness::permission::PermissionEngine;
use crate::harness::provider::scripted::{ScriptedProvider, ScriptedTurn};
use crate::harness::runtime::SessionRuntime;
use crate::harness::tool::context::{AbortSignal, PermissionAskInput, PermissionAsker, UserAsker};
use crate::harness::tool::registry::ToolRegistry;

/// An asker that always allows, so evals never block on a prompt.
struct AllowAsker;
#[async_trait::async_trait]
impl PermissionAsker for AllowAsker {
    async fn ask(&self, _req: PermissionAskInput) -> bool {
        true
    }
}

/// A user asker that never answers (evals must not depend on interaction).
struct NoUserAsker;
#[async_trait::async_trait]
impl UserAsker for NoUserAsker {
    async fn ask(&self, _q: String, _o: Vec<String>) -> Option<String> {
        None
    }
}

/// The outcome of running one eval, with everything a criterion may inspect.
pub struct EvalOutcome {
    /// Final assistant text of the turn.
    pub final_text: String,
    /// Loop iterations (LLM requests) the turn took.
    pub iterations: usize,
    /// Names of every tool the model called, in order.
    pub tools_called: Vec<String>,
    /// The provider, for asserting on what the model saw.
    pub provider: Arc<ScriptedProvider>,
    /// The session after the turn (metrics, messages, ledger).
    pub session: crate::harness::session::Session,
}

impl EvalOutcome {
    /// Whether a tool with this name was called at least once.
    pub fn used_tool(&self, name: &str) -> bool {
        self.tools_called.iter().any(|t| t == name)
    }

    /// Number of times a tool was called.
    pub fn tool_count(&self, name: &str) -> usize {
        self.tools_called.iter().filter(|t| *t == name).count()
    }
}

/// A synthetic task with an automatic success criterion.
pub struct Eval {
    /// Stable name (used in reports and CI output).
    pub name: &'static str,
    /// One-line description of what the eval checks.
    pub description: &'static str,
    /// The scripted turns the provider will replay.
    pub script: Vec<ScriptedTurn>,
    /// The user prompt that starts the turn.
    pub prompt: &'static str,
    /// The success criterion, evaluated against the outcome.
    pub check: fn(&EvalOutcome) -> Result<()>,
}

/// Builds a runtime backed by a scripted provider, rooted at `dir`.
fn eval_runtime(dir: &Path, provider: Arc<ScriptedProvider>) -> Result<SessionRuntime> {
    let db = dir.join("evals.db");
    SessionRuntime::new_in(
        dir,
        provider,
        ToolRegistry::builder().build(),
        crate::config::RuntimeConfig {
            model: "scripted".into(),
            provider: "scripted".into(),
            base_url: String::new(),
            api_key: "eval".into(),
            ..Default::default()
        },
        &db,
        Arc::new(PermissionEngine::default()),
        Arc::new(AllowAsker),
        Arc::new(NoUserAsker),
    )
}

/// Runs one eval in a fresh tempdir and returns its outcome.
pub async fn run(eval: &Eval) -> Result<EvalOutcome> {
    let dir = tempfile::tempdir()?;
    let provider = Arc::new(ScriptedProvider::new(eval.script.clone()));
    let runtime = eval_runtime(dir.path(), provider.clone())?;
    let mut session = runtime.create_session("build").await?;

    let (tx, mut rx) = crate::harness::event::event_channel();
    // Drain events so the channel never blocks the loop.
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });

    let result = runtime
        .prompt(&mut session, &tx, eval.prompt, AbortSignal::new(), None)
        .await?;
    let _ = tx.send(crate::harness::event::HarnessEvent::RunFinished {
        session_id: session.id.clone(),
        parent_session_id: None,
    });
    drain.abort();

    // Collect the tool names the model called, in order, from the transcript.
    let mut tools_called = Vec::new();
    for msg in &session.messages {
        for part in &msg.parts {
            if let crate::harness::session::Part::Tool(t) = part {
                tools_called.push(t.name.clone());
            }
        }
    }

    Ok(EvalOutcome {
        final_text: result.final_text,
        iterations: result.iterations,
        tools_called,
        provider,
        session,
    })
}

/// Runs every eval, returning `(name, result)` pairs. Never panics: a failing
/// eval is reported as `Err` so the caller can aggregate a report.
pub async fn run_all(evals: &[Eval]) -> Vec<(&'static str, Result<()>)> {
    let mut out = Vec::new();
    for eval in evals {
        let res = match run(eval).await {
            Ok(outcome) => (eval.check)(&outcome),
            Err(e) => Err(e),
        };
        out.push((eval.name, res));
    }
    out
}

/// Runs the suite and renders a human-readable report (name, description,
/// pass/fail). Returns the report and whether every eval passed.
pub async fn report(evals: &[Eval]) -> (String, bool) {
    let results = run_all(evals).await;
    let mut out = String::from("Eval report\n");
    let mut all_ok = true;
    for (eval, (_, res)) in evals.iter().zip(results.iter()) {
        match res {
            Ok(()) => out.push_str(&format!("  PASS  {} — {}\n", eval.name, eval.description)),
            Err(e) => {
                all_ok = false;
                out.push_str(&format!(
                    "  FAIL  {} — {}\n        {e:#}\n",
                    eval.name, eval.description
                ));
            }
        }
    }
    let passed = results.iter().filter(|(_, r)| r.is_ok()).count();
    out.push_str(&format!("{passed}/{} evals passed\n", results.len()));
    (out, all_ok)
}

/// The built-in eval suite. Kept small and fast so it can gate CI.
pub fn suite() -> Vec<Eval> {
    vec![
        Eval {
            name: "plain_answer_no_tools",
            description: "a text-only turn ends without calling any tool",
            script: vec![ScriptedTurn::text("The answer is 42.")],
            prompt: "What is the answer?",
            check: |o| {
                anyhow::ensure!(
                    o.tools_called.is_empty(),
                    "expected no tool calls, got {:?}",
                    o.tools_called
                );
                anyhow::ensure!(o.final_text.contains("42"), "final text: {}", o.final_text);
                anyhow::ensure!(
                    o.iterations == 1,
                    "expected 1 iteration, got {}",
                    o.iterations
                );
                Ok(())
            },
        },
        Eval {
            name: "single_tool_then_answer",
            description: "the loop runs a tool, feeds the result back, then answers",
            script: vec![
                ScriptedTurn::tool("glob", serde_json::json!({"pattern": "*.rs"})),
                ScriptedTurn::text("Found the files."),
            ],
            prompt: "List the Rust files.",
            check: |o| {
                anyhow::ensure!(o.used_tool("glob"), "glob was not called");
                anyhow::ensure!(
                    o.iterations == 2,
                    "expected 2 iterations (tool + answer), got {}",
                    o.iterations
                );
                // The model must have seen the tool result on the second call.
                anyhow::ensure!(
                    o.provider.calls() == 2,
                    "provider served {} requests, expected 2",
                    o.provider.calls()
                );
                Ok(())
            },
        },
        Eval {
            name: "parallel_tool_calls",
            description: "two tool calls in one turn are both executed",
            script: vec![
                ScriptedTurn::tools(vec![
                    crate::harness::provider::scripted::ScriptedToolCall {
                        id: "c1".into(),
                        name: "glob".into(),
                        arguments: serde_json::json!({"pattern": "*.rs"}),
                    },
                    crate::harness::provider::scripted::ScriptedToolCall {
                        id: "c2".into(),
                        name: "glob".into(),
                        arguments: serde_json::json!({"pattern": "*.toml"}),
                    },
                ]),
                ScriptedTurn::text("Done."),
            ],
            prompt: "List Rust and TOML files.",
            check: |o| {
                anyhow::ensure!(
                    o.tool_count("glob") == 2,
                    "expected 2 glob calls, got {}",
                    o.tool_count("glob")
                );
                Ok(())
            },
        },
        Eval {
            name: "metrics_recorded",
            description: "the turn records turns, iterations, tool calls and tokens",
            script: vec![
                ScriptedTurn::tool("glob", serde_json::json!({"pattern": "*.rs"}))
                    .with_usage(100, 20),
                ScriptedTurn::text("Done.").with_usage(150, 30),
            ],
            prompt: "List the Rust files.",
            check: |o| {
                let m = &o.session.metrics;
                anyhow::ensure!(m.turns == 1, "expected 1 turn, got {}", m.turns);
                anyhow::ensure!(
                    m.iterations == 2,
                    "expected 2 iterations, got {}",
                    m.iterations
                );
                anyhow::ensure!(
                    m.tool_calls.get("glob").copied() == Some(1),
                    "glob not counted: {:?}",
                    m.tool_calls
                );
                anyhow::ensure!(
                    m.input_tokens == 250 && m.output_tokens == 50,
                    "tokens: in={} out={}",
                    m.input_tokens,
                    m.output_tokens
                );
                Ok(())
            },
        },
        Eval {
            name: "tool_error_is_surfaced",
            description: "a failing tool is recorded as an error and the loop continues",
            script: vec![
                // `read` on a path that does not exist → tool error.
                ScriptedTurn::tool(
                    "read",
                    serde_json::json!({"path": "definitely/not/here.rs"}),
                ),
                ScriptedTurn::text("The file does not exist."),
            ],
            prompt: "Read a missing file.",
            check: |o| {
                anyhow::ensure!(o.used_tool("read"), "read was not called");
                anyhow::ensure!(
                    o.session.metrics.tool_errors >= 1,
                    "expected at least one tool error, got {}",
                    o.session.metrics.tool_errors
                );
                // The loop must still finish with an answer.
                anyhow::ensure!(
                    o.final_text.contains("does not exist"),
                    "final text: {}",
                    o.final_text
                );
                Ok(())
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_suite_passes() {
        let results = run_all(&suite()).await;
        let failures: Vec<_> = results
            .iter()
            .filter_map(|(name, r)| r.as_ref().err().map(|e| format!("{name}: {e:#}")))
            .collect();
        assert!(
            failures.is_empty(),
            "eval failures:\n{}",
            failures.join("\n")
        );
        assert_eq!(results.len(), suite().len());
    }

    #[tokio::test]
    async fn test_failing_criterion_is_reported() {
        // A deliberately wrong criterion must surface as an error, not a panic.
        let bad = Eval {
            name: "intentionally_bad",
            description: "should fail",
            script: vec![ScriptedTurn::text("hi")],
            prompt: "hi",
            check: |_o| anyhow::bail!("expected failure"),
        };
        let results = run_all(&[bad]).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_err());
    }

    #[tokio::test]
    async fn test_report_renders_pass_and_fail() {
        let evals = vec![
            Eval {
                name: "ok_eval",
                description: "passes",
                script: vec![ScriptedTurn::text("hi")],
                prompt: "hi",
                check: |_o| Ok(()),
            },
            Eval {
                name: "bad_eval",
                description: "fails",
                script: vec![ScriptedTurn::text("hi")],
                prompt: "hi",
                check: |_o| anyhow::bail!("boom"),
            },
        ];
        let (text, all_ok) = report(&evals).await;
        assert!(!all_ok);
        assert!(text.contains("PASS  ok_eval — passes"));
        assert!(text.contains("FAIL  bad_eval — fails"));
        assert!(text.contains("1/2 evals passed"));
    }
}
