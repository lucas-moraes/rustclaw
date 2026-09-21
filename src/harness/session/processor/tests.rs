use super::{ProcessorConfig, SessionProcessor};
use crate::harness::agent::AgentSpec;
use crate::harness::provider::{
    LlmRequest, LlmResponse, Provider, ProviderEvent, ProviderStream, Usage,
};
use crate::harness::session::Message;
use crate::harness::tool::context::{AbortSignal, PathBufGuard, ToolContext};
use futures_util::StreamExt;
use std::sync::Arc as StdArc;
use std::time::Duration;

/// Provider whose stream never yields anything and never ends.
struct StalledProvider;

#[async_trait::async_trait]
impl Provider for StalledProvider {
    fn name(&self) -> &str {
        "stalled"
    }
    async fn stream(&self, _req: &LlmRequest) -> anyhow::Result<ProviderStream> {
        // A stream that stays pending forever.
        Ok(futures_util::stream::pending::<anyhow::Result<ProviderEvent>>().boxed())
    }
    async fn complete(&self, _req: &LlmRequest) -> anyhow::Result<LlmResponse> {
        unreachable!()
    }
}

fn test_ctx() -> ToolContext {
    use crate::harness::permission::PermissionEngine;
    struct AllowAsker;
    #[async_trait::async_trait]
    impl crate::harness::tool::context::PermissionAsker for AllowAsker {
        async fn ask(&self, _r: crate::harness::tool::context::PermissionAskInput) -> bool {
            true
        }
    }
    struct NoUserAsker;
    #[async_trait::async_trait]
    impl crate::harness::tool::context::UserAsker for NoUserAsker {
        async fn ask(&self, _q: String, _o: Vec<String>) -> Option<String> {
            None
        }
    }
    ToolContext {
        session_id: "s".into(),
        agent: "build".into(),
        agent_tools: vec![],
        cwd: PathBufGuard(std::path::PathBuf::from("/tmp")),
        abort: AbortSignal::new(),
        permission: StdArc::new(PermissionEngine::default()),
        asker: StdArc::new(AllowAsker),
        user_asker: StdArc::new(NoUserAsker),
        todos: StdArc::new(tokio::sync::RwLock::new(Vec::new())),
        task_runner: None,
        depth: 0,
        events: crate::harness::event::event_channel().0,
        project_memory: None,
        hooks: Default::default(),
        checkpoints: std::sync::Arc::new(crate::harness::tool::checkpoint::FileCheckpoints::new()),
        jobs: std::sync::Arc::new(crate::harness::tool::jobs::JobRegistry::new()),
        semantic_index: None,
        embedder: None,
        sandbox_policy: None,
    }
}

#[tokio::test]
async fn test_turn_timeout_stops_run() {
    let dir = tempfile::tempdir().unwrap();
    let store = StdArc::new(
        crate::harness::session::store::SessionStore::open(&dir.path().join("test.db")).unwrap(),
    );
    let (tx, _rx) = crate::harness::event::event_channel();
    let processor = SessionProcessor {
        provider: StdArc::new(StalledProvider),
        registry: crate::harness::tool::registry::ToolRegistry::builder().build(),
        events: tx,
        store: store.clone(),
        config: ProcessorConfig {
            model: "m".into(),
            max_iterations: 50,
            max_context_tokens: 100_000,
            turn_timeout_secs: 1, // 1s so the test is fast
            max_total_iterations: None,
            compact_trigger_ratio: 0.0,
            summary_model: String::new(),
        },
    };
    let mut session = store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("hello"));
    let agent = crate::harness::agent::AgentSpec {
        name: "build".into(),
        description: String::new(),
        tools: vec![],
        system_prompt: String::new(),
        model: None,
        temperature: None,
        permission_overrides: Default::default(),
    };
    let ctx = test_ctx();
    let outcome = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();
    // The watchdog now restarts the turn (up to 10 continuations); each
    // segment lasts the configured 1s before the next restart/stop.
    assert_eq!(outcome.continuations, 10);
    assert!(!outcome.aborted);
    assert!(
        outcome.final_text.contains("time limit"),
        "unexpected final_text: {}",
        outcome.final_text
    );
}

/// Provider that always ends the assistant message with exactly one tool
/// call (never a final answer). Simulates a long TODO-list run.
struct ToolCallProvider {
    n: AtomicUsize,
}

#[async_trait::async_trait]
impl Provider for ToolCallProvider {
    fn name(&self) -> &str {
        "toolcall"
    }
    async fn stream(&self, _req: &LlmRequest) -> anyhow::Result<ProviderStream> {
        // Distinct tool name per call so the doom-loop detector never
        // fires and the continuations cap is what ends the turn.
        let i = self.n.fetch_add(1, Ordering::SeqCst);
        let evs: Vec<anyhow::Result<ProviderEvent>> = vec![
            Ok(ProviderEvent::ToolCallStart {
                id: format!("t{i}"),
                name: format!("tool_{}", i),
            }),
            Ok(ProviderEvent::ToolCallEnd {
                id: format!("t{i}"),
                arguments: format!(r#"{{"n":{}}}"#, i),
            }),
            Ok(ProviderEvent::End {
                stop_reason: None,
                usage: None,
            }),
        ];
        Ok(futures_util::stream::iter(evs).boxed())
    }
    async fn complete(&self, _req: &LlmRequest) -> anyhow::Result<LlmResponse> {
        unreachable!()
    }
}

#[tokio::test]
async fn test_auto_continuation_resumes_after_max_iterations() {
    let dir = tempfile::tempdir().unwrap();
    let store = StdArc::new(
        crate::harness::session::store::SessionStore::open(&dir.path().join("test.db")).unwrap(),
    );
    let (tx, _rx) = crate::harness::event::event_channel();
    let processor = SessionProcessor {
        provider: StdArc::new(ToolCallProvider {
            n: AtomicUsize::new(0),
        }),
        registry: crate::harness::tool::registry::ToolRegistry::builder().build(),
        events: tx,
        store: store.clone(),
        config: ProcessorConfig {
            model: "m".into(),
            max_iterations: 1,
            max_context_tokens: 100_000,
            turn_timeout_secs: 5, // generous: no watchdog interference
            // Explicit budget so the continuation cap (10), not the budget,
            // ends the turn — this test is about auto-continuation.
            max_total_iterations: Some(11),
            compact_trigger_ratio: 0.0,
            summary_model: String::new(),
        },
    };
    let mut session = store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("do the long todo run"));
    let agent = crate::harness::agent::AgentSpec {
        name: "build".into(),
        description: String::new(),
        tools: vec![],
        system_prompt: String::new(),
        model: None,
        temperature: None,
        permission_overrides: Default::default(),
    };
    let ctx = test_ctx();
    let outcome = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();
    // One tool call per iteration + 10 automatic continuations.
    assert_eq!(outcome.continuations, 10);
    assert_eq!(outcome.iterations, 11);
    assert!(
        outcome.final_text.contains("after 10 continuation(s)"),
        "unexpected final_text: {}",
        outcome.final_text
    );
    // The auto-continue notes were persisted with the session history.
    let notes = session
        .messages
        .iter()
        .filter(|m| m.role.as_str() == "user" && m.text_content().contains("[auto-continue"))
        .count();
    assert_eq!(notes, 10);
}

// ─── Integration tests: full loop with a scripted MockProvider ──────────

use crate::harness::tool::Tool;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Scripted provider: `stream()` returns the event list for call N
/// (0-based, atomic counter). `complete` panics — the processor must only
/// use `stream`.
struct MockProvider {
    /// One Vec<ProviderEvent> per expected `stream()` call.
    script: Vec<Vec<ProviderEvent>>,
    calls: AtomicUsize,
}

impl MockProvider {
    fn new(script: Vec<Vec<ProviderEvent>>) -> Self {
        Self {
            script,
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }
    async fn stream(&self, _req: &LlmRequest) -> anyhow::Result<ProviderStream> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let evs = self
            .script
            .get(n)
            .cloned()
            .unwrap_or_else(|| panic!("unexpected stream() call #{}", n + 1));
        Ok(futures_util::stream::iter(evs.into_iter().map(Ok).collect::<Vec<_>>()).boxed())
    }
    async fn complete(&self, _req: &LlmRequest) -> anyhow::Result<LlmResponse> {
        panic!("processor must not call complete()")
    }
}

/// Fake tool returning a fixed output; optionally sleeps and/or aborts.
struct FakeTool {
    name: &'static str,
    output: &'static str,
    sleep_ms: u64,
    /// When set, the tool aborts this signal before returning.
    abort: Option<AbortSignal>,
}

#[async_trait::async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "fake test tool"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<crate::harness::tool::ToolResult, String> {
        if let Some(a) = &self.abort {
            a.abort();
        }
        if self.sleep_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.sleep_ms)).await;
        }
        Ok(crate::harness::tool::ToolResult::simple(
            self.name,
            self.output,
        ))
    }
}

fn agent_with_tools(tools: Vec<String>) -> AgentSpec {
    AgentSpec {
        name: "build".into(),
        description: String::new(),
        tools,
        system_prompt: String::new(),
        model: None,
        temperature: None,
        permission_overrides: Default::default(),
    }
}

fn tool_call_events(id: &str, name: &str, args: &str) -> Vec<ProviderEvent> {
    vec![
        ProviderEvent::ToolCallStart {
            id: id.to_string(),
            name: name.to_string(),
        },
        ProviderEvent::ToolCallEnd {
            id: id.to_string(),
            arguments: args.to_string(),
        },
        ProviderEvent::End {
            stop_reason: None,
            usage: Some(Usage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            }),
        },
    ]
}

fn final_text_events(text: &str) -> Vec<ProviderEvent> {
    vec![
        ProviderEvent::TextDelta(text.to_string()),
        ProviderEvent::End {
            stop_reason: None,
            usage: Some(Usage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            }),
        },
    ]
}

fn test_processor(
    provider: MockProvider,
    registry: crate::harness::tool::registry::ToolRegistry,
    max_iterations: usize,
) -> (SessionProcessor, tempfile::TempDir) {
    test_processor_with_budget(provider, registry, max_iterations, None)
}

/// Like [`test_processor`] but with an explicit per-turn iteration budget
/// (`None` = the conservative default of `max_iterations * 3`).
fn test_processor_with_budget(
    provider: MockProvider,
    registry: crate::harness::tool::registry::ToolRegistry,
    max_iterations: usize,
    max_total_iterations: Option<usize>,
) -> (SessionProcessor, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = StdArc::new(
        crate::harness::session::store::SessionStore::open(&dir.path().join("test.db")).unwrap(),
    );
    let (tx, _rx) = crate::harness::event::event_channel();
    let processor = SessionProcessor {
        provider: StdArc::new(provider),
        registry,
        events: tx,
        store: store.clone(),
        config: ProcessorConfig {
            model: "m".into(),
            max_iterations,
            max_context_tokens: 100_000,
            turn_timeout_secs: 30, // generous: no watchdog interference
            max_total_iterations,
            compact_trigger_ratio: 0.0,
            summary_model: String::new(),
        },
    };
    (processor, dir)
}

#[tokio::test]
async fn test_single_tool_call_result_flows_back_to_model() {
    let registry = crate::harness::tool::registry::ToolRegistry::builder()
        .register(StdArc::new(FakeTool {
            name: "fake_ok",
            output: "ok",
            sleep_ms: 0,
            abort: None,
        }))
        .build();
    let provider = MockProvider::new(vec![
        tool_call_events("t1", "fake_ok", "{}"),
        final_text_events("done"),
    ]);
    let (processor, dir) = test_processor(provider, registry, 10);
    let mut session = processor.store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("use the tool"));
    let agent = agent_with_tools(vec!["fake_ok".to_string()]);
    let ctx = test_ctx();

    let outcome = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();

    assert_eq!(outcome.final_text, "done");
    assert_eq!(outcome.iterations, 2);
    assert!(!outcome.aborted);
    assert_eq!(outcome.usage.input_tokens, 20);
    assert_eq!(outcome.usage.output_tokens, 10);

    // Session gained: user, assistant (with ToolPart), tool result is
    // stored on the assistant's ToolPart, then the final assistant text.
    let assistant: Vec<&Message> = session
        .messages
        .iter()
        .filter(|m| m.role.as_str() == "assistant")
        .collect();
    assert_eq!(assistant.len(), 2, "expected 2 assistant messages");
    let tool_msg = assistant[0];
    let tools = tool_msg.tool_parts();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "fake_ok");
    assert_eq!(tools[0].id, "t1");
    assert_eq!(
        tools[0].status,
        crate::harness::session::ToolStatus::Completed
    );
    assert_eq!(tools[0].output, "ok");
    assert_eq!(assistant[1].text_content(), "done");

    // The tool result was persisted to the store.
    let reloaded = processor
        .store
        .load_session(&session.id, dir.path())
        .unwrap()
        .unwrap();
    let persisted_tool = reloaded
        .messages
        .iter()
        .find_map(|m| m.tool_parts().into_iter().next());
    let pt = persisted_tool.expect("tool part persisted");
    assert_eq!(pt.output, "ok");
}

#[tokio::test]
async fn test_parallel_tool_calls_both_execute() {
    let registry = crate::harness::tool::registry::ToolRegistry::builder()
        .register(StdArc::new(FakeTool {
            name: "fake_a",
            output: "result-a",
            sleep_ms: 50,
            abort: None,
        }))
        .register(StdArc::new(FakeTool {
            name: "fake_b",
            output: "bee",
            sleep_ms: 50,
            abort: None,
        }))
        .build();
    let provider = MockProvider::new(vec![
        vec![
            ProviderEvent::ToolCallStart {
                id: "a1".into(),
                name: "fake_a".into(),
            },
            ProviderEvent::ToolCallEnd {
                id: "a1".into(),
                arguments: "{}".into(),
            },
            ProviderEvent::ToolCallStart {
                id: "a2".into(),
                name: "fake_b".into(),
            },
            ProviderEvent::ToolCallEnd {
                id: "a2".into(),
                arguments: "{}".into(),
            },
            ProviderEvent::End {
                stop_reason: None,
                usage: None,
            },
        ],
        final_text_events("both done"),
    ]);
    let (processor, dir) = test_processor(provider, registry, 10);
    let mut session = processor.store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("run both"));
    let agent = agent_with_tools(vec!["fake_a".to_string(), "fake_b".to_string()]);
    let ctx = test_ctx();

    let outcome = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();

    assert_eq!(outcome.final_text, "both done");
    assert_eq!(outcome.iterations, 2);

    // Both tool results present (order not guaranteed — set comparison).
    let tool_outputs: std::collections::HashSet<String> = session
        .messages
        .iter()
        .flat_map(|m| m.tool_parts())
        .map(|t| t.output.clone())
        .collect();
    assert!(tool_outputs.contains("result-a"), "missing fake_a result");
    assert!(tool_outputs.contains("bee"), "missing fake_b result");
    // Both completed.
    for t in session.messages.iter().flat_map(|m| m.tool_parts()) {
        assert_eq!(
            t.status,
            crate::harness::session::ToolStatus::Completed,
            "tool {} not completed",
            t.id
        );
    }
}

/// Tool that panics during execution.
struct PanickyTool {
    name: &'static str,
}

#[async_trait::async_trait]
impl Tool for PanickyTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "always panics"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<crate::harness::tool::ToolResult, String> {
        panic!("boom: tool exploded");
    }
}

#[tokio::test]
async fn test_panicking_tool_does_not_contaminate_others() {
    let registry = crate::harness::tool::registry::ToolRegistry::builder()
        .register(StdArc::new(FakeTool {
            name: "fake_ok",
            output: "result-ok",
            sleep_ms: 50,
            abort: None,
        }))
        .register(StdArc::new(PanickyTool { name: "fake_panic" }))
        .build();
    let provider = MockProvider::new(vec![
        vec![
            ProviderEvent::ToolCallStart {
                id: "p1".into(),
                name: "fake_panic".into(),
            },
            ProviderEvent::ToolCallEnd {
                id: "p1".into(),
                arguments: "{}".into(),
            },
            ProviderEvent::ToolCallStart {
                id: "o1".into(),
                name: "fake_ok".into(),
            },
            ProviderEvent::ToolCallEnd {
                id: "o1".into(),
                arguments: "{}".into(),
            },
            ProviderEvent::End {
                stop_reason: None,
                usage: None,
            },
        ],
        final_text_events("recovered"),
    ]);
    let (processor, dir) = test_processor(provider, registry, 10);
    let mut session = processor.store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("run tools"));
    let agent = agent_with_tools(vec!["fake_panic".to_string(), "fake_ok".to_string()]);
    let ctx = test_ctx();

    let outcome = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();

    assert_eq!(outcome.final_text, "recovered");
    let tools: Vec<crate::harness::session::ToolPart> = session
        .messages
        .iter()
        .flat_map(|m| m.tool_parts())
        .cloned()
        .collect();
    assert_eq!(tools.len(), 2);
    for t in &tools {
        if t.name == "fake_ok" {
            assert_eq!(
                t.status,
                crate::harness::session::ToolStatus::Completed,
                "healthy tool must complete despite sibling panic"
            );
            assert_eq!(t.output, "result-ok");
        } else {
            assert_eq!(t.name, "fake_panic");
            assert_eq!(t.status, crate::harness::session::ToolStatus::Error);
            assert!(t.error.as_deref().unwrap_or_default().contains("panicked"));
        }
    }
}

#[tokio::test]
async fn test_iteration_budget_exhausts_turn() {
    // The model never produces a final answer; every "turn" is a tool
    // call. The global budget caps the turn even across auto-continues.
    let registry = crate::harness::tool::registry::ToolRegistry::builder()
        .register(StdArc::new(FakeTool {
            name: "fake_ok",
            output: "ok",
            sleep_ms: 0,
            abort: None,
        }))
        .build();
    let script: Vec<Vec<ProviderEvent>> = (0..8)
        .map(|i| tool_call_events(&format!("i{}", i), "fake_ok", "{}"))
        .collect();
    let provider = MockProvider::new(script);
    // Explicit budget high enough that the doom-loop detector (not the budget)
    // ends the turn — this test is about the doom-loop, not the budget.
    let (processor, dir) = test_processor_with_budget(provider, registry, 1, Some(11));
    let mut session = processor.store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("loop forever"));
    let agent = agent_with_tools(vec!["fake_ok".to_string()]);
    let ctx = test_ctx();

    let outcome = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();

    assert!(
        outcome.final_text.contains("Stopped:"),
        "unexpected final: {}",
        outcome.final_text
    );
    // With the raised continuations cap (10), the (10+1)×max budget no
    // longer fires for a short script: the identical-call doom-loop
    // detector ends the turn at the 5th repeat instead.
    assert!(outcome.final_text.contains("same tool call"));
    assert!(!outcome.aborted);
    assert_eq!(outcome.iterations, 5);
    assert_eq!(outcome.continuations, 4);
}

#[tokio::test]
async fn test_abort_during_tool_execution_stops_turn() {
    // The tool aborts the shared signal as soon as it starts running.
    let ctx = test_ctx();
    let abort_clone = ctx.abort.clone();
    let registry = crate::harness::tool::registry::ToolRegistry::builder()
        .register(StdArc::new(FakeTool {
            name: "fake_abort",
            output: "x",
            sleep_ms: 0,
            abort: Some(abort_clone),
        }))
        .build();
    let provider = MockProvider::new(vec![tool_call_events("t1", "fake_abort", "{}")]);
    let (processor, dir) = test_processor(provider, registry, 10);
    let mut session = processor.store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("abort me"));
    let agent = agent_with_tools(vec!["fake_abort".to_string()]);

    let outcome = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();

    assert!(outcome.aborted, "expected aborted outcome");
    assert!(
        outcome.final_text.contains("aborted"),
        "unexpected final_text: {}",
        outcome.final_text
    );
}

#[tokio::test]
async fn test_text_loop_hard_stops_repeated_assistant_text() {
    // Each iteration: the SAME text plus a tool call that differs only in
    // args (so the tool doom-loop doesn't fire). The text-loop detector
    // must warn at 3 and hard-stop at 5 repetitions.
    let registry = crate::harness::tool::registry::ToolRegistry::builder()
        .register(StdArc::new(FakeTool {
            name: "fake_grep",
            output: "no results",
            sleep_ms: 0,
            abort: None,
        }))
        .build();
    const TEXT: &str = "Let me check the session/mod.rs:";
    let script: Vec<Vec<ProviderEvent>> = (0..6)
        .map(|i| {
            vec![
                ProviderEvent::TextDelta(TEXT.to_string()),
                ProviderEvent::ToolCallStart {
                    id: format!("t{i}"),
                    name: "fake_grep".into(),
                },
                ProviderEvent::ToolCallEnd {
                    id: format!("t{i}"),
                    arguments: format!(r#"{{"i":{i}}}"#),
                },
                ProviderEvent::End {
                    stop_reason: None,
                    usage: Some(Usage::default()),
                },
            ]
        })
        .collect();
    let provider = MockProvider::new(script);
    let (processor, dir) = test_processor(provider, registry, 50);
    let mut session = processor.store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("find preview"));
    let agent = agent_with_tools(vec!["fake_grep".to_string()]);
    let ctx = test_ctx();

    let outcome = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();

    assert!(
        outcome.final_text.contains("repeating the same response"),
        "unexpected final_text: {}",
        outcome.final_text
    );
    assert_eq!(outcome.iterations, 5);
    assert!(!outcome.aborted);
    assert_eq!(outcome.continuations, 0);
    // The warn note was persisted into the session history.
    let warns = session
        .messages
        .iter()
        .filter(|m| {
            m.role.as_str() == "user" && m.text_content().contains("repeating the same response")
        })
        .count();
    assert_eq!(warns, 1);
}

#[tokio::test]
async fn test_doom_loop_stops_after_repeated_identical_call() {
    // Always the same tool call (same id/name/args): the loop must stop at
    // DOOM_LOOP_STOP repetitions, not run to max_iterations.
    let registry = crate::harness::tool::registry::ToolRegistry::builder()
        .register(StdArc::new(FakeTool {
            name: "fake_loop",
            output: "same",
            sleep_ms: 0,
            abort: None,
        }))
        .build();
    let script = vec![tool_call_events("t1", "fake_loop", "{}"); 50];
    let provider = MockProvider::new(script);
    let (processor, dir) = test_processor(provider, registry, 50);
    let mut session = processor.store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("loop forever"));
    let agent = agent_with_tools(vec!["fake_loop".to_string()]);
    let ctx = test_ctx();

    let outcome = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();

    // DOOM_LOOP_STOP identical calls in a row -> stop. Iterations stay
    // well below max_iterations (50) and no auto-continuation happens.
    assert!(
        outcome.iterations <= crate::harness::session::doom_loop::DOOM_LOOP_STOP + 1,
        "doom loop not detected: {} iterations",
        outcome.iterations
    );
    assert!(!outcome.aborted);
    assert!(
        outcome.final_text.contains("repeated many times"),
        "unexpected final_text: {}",
        outcome.final_text
    );
    // The warn note (DOOM_LOOP_WARN) was injected into the history.
    let warns = session
        .messages
        .iter()
        .filter(|m| m.text_content().contains("System note: you just repeated"))
        .count();
    assert_eq!(warns, 1);
}

#[tokio::test]
async fn test_doom_loop_detects_multi_call_cycle() {
    // A,B,A,B cycle: each iteration emits TWO tool calls (fake_loop +
    // fake_loop2) with the same args. The old single-call detector missed
    // this; the set-based detector must stop it.
    let registry = crate::harness::tool::registry::ToolRegistry::builder()
        .register(StdArc::new(FakeTool {
            name: "fake_loop",
            output: "same",
            sleep_ms: 0,
            abort: None,
        }))
        .register(StdArc::new(FakeTool {
            name: "fake_loop2",
            output: "same2",
            sleep_ms: 0,
            abort: None,
        }))
        .build();
    // Each iteration: ToolCallStart/End for A, then for B, then End.
    let mut one_iter = tool_call_events("a1", "fake_loop", "{}");
    one_iter.pop(); // drop the trailing End
    one_iter.extend(tool_call_events("b1", "fake_loop2", "{}"));
    let script = vec![one_iter; 50];
    let provider = MockProvider::new(script);
    let (processor, dir) = test_processor(provider, registry, 50);
    let mut session = processor.store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("cycle forever"));
    let agent = agent_with_tools(vec!["fake_loop".to_string(), "fake_loop2".to_string()]);
    let ctx = test_ctx();

    let outcome = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();

    assert!(
        outcome.iterations <= crate::harness::session::doom_loop::DOOM_LOOP_STOP + 1,
        "multi-call cycle not detected: {} iterations",
        outcome.iterations
    );
    assert!(
        outcome.final_text.contains("repeated many times"),
        "unexpected final_text: {}",
        outcome.final_text
    );
}

// --- decide_turn_end: pure decision table (R2) ---

#[test]
fn test_decide_turn_end_clean_finish_continues() {
    // No stop_reason, budget left, continuations left -> auto-continue.
    let d = super::decide_turn_end(None, 0, 1, 11);
    match d {
        super::TurnDecision::Continue { label, reason, .. } => {
            assert_eq!(label, "auto-continue");
            assert_eq!(reason, "iteration limit reached");
        }
        other => panic!("expected Continue, got {other:?}"),
    }
}

#[test]
fn test_decide_turn_end_stop_reason_restarts() {
    let d = super::decide_turn_end(Some("turn exceeded the 5s time limit".into()), 0, 1, 11);
    match d {
        super::TurnDecision::Continue {
            label,
            reason,
            note,
        } => {
            assert_eq!(label, "turn restart");
            assert!(reason.contains("time limit"));
            assert!(note.contains("turn restart 1/10"));
        }
        other => panic!("expected Continue, got {other:?}"),
    }
}

#[test]
fn test_decide_turn_end_continuations_exhausted_stops() {
    let d = super::decide_turn_end(None, 10, 11, 11);
    match d {
        super::TurnDecision::Stop(t) => assert!(t.contains("after 10 continuation(s)")),
        other => panic!("expected Stop, got {other:?}"),
    }
}

#[test]
fn test_decide_turn_end_budget_reason_is_hard_stop() {
    let d = super::decide_turn_end(
        Some("iteration budget exhausted for this turn".into()),
        0,
        11,
        11,
    );
    match d {
        super::TurnDecision::Stop(t) => assert!(t.contains("iteration budget exhausted")),
        other => panic!("expected Stop, got {other:?}"),
    }
}

#[test]
fn test_decide_turn_end_global_budget_stops() {
    // No stop_reason, continuations left, but the global budget is spent.
    let d = super::decide_turn_end(None, 3, 11, 11);
    match d {
        super::TurnDecision::Stop(t) => assert!(t.contains("iteration budget exhausted")),
        other => panic!("expected Stop, got {other:?}"),
    }
}

// --- R3: honor provider stop_reason (truncation) ---

/// Provider that emits a text response truncated at the output-token limit
/// (`stop_reason = max_tokens`) for the first `truncations` calls, then a
/// clean `end_turn` final answer.
struct TruncatingProvider {
    truncations: usize,
    n: AtomicUsize,
}

#[async_trait::async_trait]
impl Provider for TruncatingProvider {
    fn name(&self) -> &str {
        "truncating"
    }
    async fn stream(&self, _req: &LlmRequest) -> anyhow::Result<ProviderStream> {
        let i = self.n.fetch_add(1, Ordering::SeqCst);
        let evs: Vec<anyhow::Result<ProviderEvent>> = if i < self.truncations {
            vec![
                Ok(ProviderEvent::TextDelta(format!("partial {i}"))),
                Ok(ProviderEvent::End {
                    stop_reason: Some("max_tokens".into()),
                    usage: None,
                }),
            ]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("final answer".into())),
                Ok(ProviderEvent::End {
                    stop_reason: Some("end_turn".into()),
                    usage: None,
                }),
            ]
        };
        Ok(futures_util::stream::iter(evs).boxed())
    }
    async fn complete(&self, _req: &LlmRequest) -> anyhow::Result<LlmResponse> {
        unreachable!()
    }
}

fn processor_with(provider: StdArc<dyn Provider>, max_iterations: usize) -> SessionProcessor {
    let dir = tempfile::tempdir().unwrap();
    let store = StdArc::new(
        crate::harness::session::store::SessionStore::open(&dir.path().join("test.db")).unwrap(),
    );
    let (tx, _rx) = crate::harness::event::event_channel();
    SessionProcessor {
        provider,
        registry: crate::harness::tool::registry::ToolRegistry::builder().build(),
        events: tx,
        store,
        config: ProcessorConfig {
            model: "m".into(),
            max_iterations,
            max_context_tokens: 100_000,
            turn_timeout_secs: 60,
            max_total_iterations: None,
            compact_trigger_ratio: 0.0,
            summary_model: String::new(),
        },
    }
}

fn agent_no_tools() -> AgentSpec {
    AgentSpec {
        name: "build".into(),
        description: String::new(),
        tools: vec![],
        system_prompt: String::new(),
        model: None,
        temperature: None,
        permission_overrides: Default::default(),
    }
}

#[tokio::test]
async fn test_truncated_response_continues_instead_of_ending() {
    let dir = tempfile::tempdir().unwrap();
    let store = StdArc::new(
        crate::harness::session::store::SessionStore::open(&dir.path().join("test.db")).unwrap(),
    );
    let processor = processor_with(
        StdArc::new(TruncatingProvider {
            truncations: 2,
            n: AtomicUsize::new(0),
        }),
        10,
    );
    let mut session = store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("write a long essay"));
    let ctx = test_ctx();
    let outcome = processor
        .run_turn(&mut session, &agent_no_tools(), "sys", &ctx)
        .await
        .unwrap();
    // Two truncations -> two continuations, then the clean final answer.
    assert_eq!(
        outcome.continuations, 2,
        "expected 2 truncation continuations"
    );
    assert_eq!(outcome.final_text, "final answer");
    // The truncation notes were persisted.
    let notes = session
        .messages
        .iter()
        .filter(|m| m.role.as_str() == "user" && m.text_content().contains("[truncated"))
        .count();
    assert_eq!(notes, 2, "expected 2 truncation notes");
}

#[tokio::test]
async fn test_end_turn_finishes_without_continuation() {
    let dir = tempfile::tempdir().unwrap();
    let store = StdArc::new(
        crate::harness::session::store::SessionStore::open(&dir.path().join("test.db")).unwrap(),
    );
    let processor = processor_with(
        StdArc::new(TruncatingProvider {
            truncations: 0,
            n: AtomicUsize::new(0),
        }),
        10,
    );
    let mut session = store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("hi"));
    let ctx = test_ctx();
    let outcome = processor
        .run_turn(&mut session, &agent_no_tools(), "sys", &ctx)
        .await
        .unwrap();
    assert_eq!(outcome.continuations, 0);
    assert_eq!(outcome.final_text, "final answer");
}

#[test]
fn test_is_truncated_normalizes_providers() {
    use crate::harness::provider::is_truncated;
    // Truncation reasons across providers.
    assert!(is_truncated(Some("max_tokens"))); // Anthropic
    assert!(is_truncated(Some("length"))); // OpenAI
    assert!(is_truncated(Some("MAX_TOKENS"))); // case-insensitive
    assert!(is_truncated(Some(" max_tokens "))); // trimmed
                                                 // Clean finishes are not truncation.
    assert!(!is_truncated(Some("end_turn")));
    assert!(!is_truncated(Some("stop")));
    assert!(!is_truncated(Some("tool_use")));
    assert!(!is_truncated(Some("tool_calls")));
    assert!(!is_truncated(None));
}

#[test]
fn test_truncation_decision_respects_caps() {
    // Budget left -> continue.
    match super::truncation_decision(0, 1, 11) {
        super::TurnDecision::Continue { label, .. } => assert_eq!(label, "truncation continue"),
        other => panic!("expected Continue, got {other:?}"),
    }
    // Continuations exhausted -> stop.
    match super::truncation_decision(10, 11, 11) {
        super::TurnDecision::Stop(t) => assert!(t.contains("output-token limit")),
        other => panic!("expected Stop, got {other:?}"),
    }
    // Global budget exhausted -> stop.
    match super::truncation_decision(3, 11, 11) {
        super::TurnDecision::Stop(t) => assert!(t.contains("iteration budget exhausted")),
        other => panic!("expected Stop, got {other:?}"),
    }
}

// --- R4: explicit, conservative iteration budget ---

#[test]
fn test_default_total_iterations_is_conservative() {
    // Default is 3× the per-segment limit, not the old 11×.
    assert_eq!(super::default_total_iterations(50), 150);
    assert_eq!(super::default_total_iterations(1), 3);
    // Never zero (a zero budget would stop every turn immediately).
    assert_eq!(super::default_total_iterations(0), 1);
}

#[tokio::test]
async fn test_explicit_budget_caps_turn() {
    // A provider that never finishes (distinct tool names so the doom-loop
    // detector never fires). With an explicit budget of 4, the turn must stop
    // after 4 iterations regardless of continuations.
    let registry = crate::harness::tool::registry::ToolRegistry::builder()
        .register(StdArc::new(FakeTool {
            name: "fake_ok",
            output: "ok",
            sleep_ms: 0,
            abort: None,
        }))
        .build();
    let script: Vec<Vec<ProviderEvent>> = (0..20)
        .map(|i| tool_call_events(&format!("i{}", i), "fake_ok", &format!(r#"{{"n":{}}}"#, i)))
        .collect();
    let provider = MockProvider::new(script);
    let (processor, dir) = test_processor_with_budget(provider, registry, 1, Some(4));
    let mut session = processor.store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("loop forever"));
    let agent = agent_with_tools(vec!["fake_ok".to_string()]);
    let ctx = test_ctx();

    let outcome = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();

    assert!(
        outcome.final_text.contains("iteration budget exhausted"),
        "unexpected final: {}",
        outcome.final_text
    );
    assert_eq!(outcome.iterations, 4, "budget must cap total iterations");
}

#[tokio::test]
async fn test_default_budget_caps_turn() {
    // No explicit budget: the default (max_iterations * 3) caps the turn.
    let registry = crate::harness::tool::registry::ToolRegistry::builder()
        .register(StdArc::new(FakeTool {
            name: "fake_ok",
            output: "ok",
            sleep_ms: 0,
            abort: None,
        }))
        .build();
    let script: Vec<Vec<ProviderEvent>> = (0..20)
        .map(|i| tool_call_events(&format!("i{}", i), "fake_ok", &format!(r#"{{"n":{}}}"#, i)))
        .collect();
    let provider = MockProvider::new(script);
    // max_iterations = 2 -> default budget = 6.
    let (processor, dir) = test_processor(provider, registry, 2);
    let mut session = processor.store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("loop forever"));
    let agent = agent_with_tools(vec!["fake_ok".to_string()]);
    let ctx = test_ctx();

    let outcome = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();

    assert!(
        outcome.final_text.contains("iteration budget exhausted"),
        "unexpected final: {}",
        outcome.final_text
    );
    assert_eq!(outcome.iterations, 6, "default budget = max_iterations * 3");
}

// --- R6: iteration rollback (end-to-end) ---

/// A mutable tool (`write`) that always fails, after touching the file.
struct FailingWriteTool;
#[async_trait::async_trait]
impl Tool for FailingWriteTool {
    fn name(&self) -> &str {
        "write"
    }
    fn description(&self) -> &str {
        "failing write"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<crate::harness::tool::ToolResult, String> {
        // Simulate a partial write that then fails.
        if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
            let _ = std::fs::write(p, "corrupted");
        }
        Err("write failed".to_string())
    }
}

/// A mutable tool (`edit`) that succeeds.
struct OkEditTool;
#[async_trait::async_trait]
impl Tool for OkEditTool {
    fn name(&self) -> &str {
        "edit"
    }
    fn description(&self) -> &str {
        "ok edit"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<crate::harness::tool::ToolResult, String> {
        if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
            let _ = std::fs::write(p, "edited");
        }
        Ok(crate::harness::tool::ToolResult::simple("edit", "ok"))
    }
}

#[tokio::test]
async fn test_all_failed_edits_are_rolled_back() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "original").unwrap();

    let registry = crate::harness::tool::registry::ToolRegistry::builder()
        .register(StdArc::new(FailingWriteTool))
        .build();
    let script = vec![
        tool_call_events(
            "t1",
            "write",
            &format!(r#"{{"path":"{}"}}"#, file.display()),
        ),
        final_text_events("done"),
    ];
    let provider = MockProvider::new(script);
    let (processor, _dir) = test_processor(provider, registry, 10);
    let mut session = processor.store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("edit the file"));
    let agent = agent_with_tools(vec!["write".to_string()]);
    let ctx = test_ctx();

    let _ = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();

    // The failed write was rolled back to the pre-iteration content.
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "original",
        "failed edit must be rolled back"
    );
    // A rollback note was injected for the model.
    let notes = session
        .messages
        .iter()
        .filter(|m| m.role.as_str() == "user" && m.text_content().contains("rolled back"))
        .count();
    assert_eq!(notes, 1, "expected a rollback note");
}

#[tokio::test]
async fn test_successful_edit_prevents_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "original").unwrap();

    let registry = crate::harness::tool::registry::ToolRegistry::builder()
        .register(StdArc::new(FailingWriteTool))
        .register(StdArc::new(OkEditTool))
        .build();
    // One failing `write` and one succeeding `edit` in the *same* iteration
    // (a single assistant message with two tool calls).
    let script = vec![
        vec![
            ProviderEvent::ToolCallStart {
                id: "t1".into(),
                name: "write".into(),
            },
            ProviderEvent::ToolCallEnd {
                id: "t1".into(),
                arguments: format!(r#"{{"path":"{}"}}"#, file.display()),
            },
            ProviderEvent::ToolCallStart {
                id: "t2".into(),
                name: "edit".into(),
            },
            ProviderEvent::ToolCallEnd {
                id: "t2".into(),
                arguments: format!(r#"{{"path":"{}"}}"#, file.display()),
            },
            ProviderEvent::End {
                stop_reason: None,
                usage: Some(Usage::default()),
            },
        ],
        final_text_events("done"),
    ];
    let provider = MockProvider::new(script);
    let (processor, _dir) = test_processor(provider, registry, 10);
    let mut session = processor.store.create_session("build", dir.path()).unwrap();
    session.messages.push(Message::user("edit the file"));
    let agent = agent_with_tools(vec!["write".to_string(), "edit".to_string()]);
    let ctx = test_ctx();

    let _ = processor
        .run_turn(&mut session, &agent, "sys", &ctx)
        .await
        .unwrap();

    // A successful mutation disables the rollback: the good edit is kept.
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "edited",
        "successful edit must not be reverted"
    );
    let notes = session
        .messages
        .iter()
        .filter(|m| m.role.as_str() == "user" && m.text_content().contains("rolled back"))
        .count();
    assert_eq!(notes, 0, "no rollback when something succeeded");
}

// --- elide_old_tool_outputs -----------------------------------------------

use crate::harness::session::{Part, Role, ToolPart, ToolStatus};

fn tool_msg(id: &str, name: &str, output: String) -> Message {
    let mut t = ToolPart::pending(id, name, serde_json::json!({}));
    t.status = ToolStatus::Completed;
    t.output = output;
    Message::new(Role::Assistant, vec![Part::Tool(t)])
}

fn big_output(n: usize) -> String {
    "x".repeat(n)
}

#[test]
fn elide_keeps_recent_outputs_intact() {
    // 8 tool results, KEEP_RECENT_TOOL_OUTPUTS = 6 -> the 2 oldest are elided.
    let mut msgs = vec![Message::user("go")];
    for i in 0..8 {
        msgs.push(tool_msg(&format!("t{i}"), "read", big_output(2000)));
    }
    let out = super::elide_old_tool_outputs(&msgs);

    let outputs: Vec<&str> = out
        .iter()
        .flat_map(|m| m.tool_parts())
        .map(|t| t.output.as_str())
        .collect();
    assert_eq!(outputs.len(), 8);
    // 2 oldest elided.
    assert!(outputs[0].contains("output omitted"), "oldest elided");
    assert!(outputs[1].contains("output omitted"));
    // 6 most recent intact.
    for o in &outputs[2..] {
        assert_eq!(o.len(), 2000, "recent output must stay intact");
    }
}

#[test]
fn elide_noop_when_few_tools() {
    let mut msgs = vec![Message::user("go")];
    for i in 0..4 {
        msgs.push(tool_msg(&format!("t{i}"), "read", big_output(2000)));
    }
    let out = super::elide_old_tool_outputs(&msgs);
    let all_intact = out
        .iter()
        .flat_map(|m| m.tool_parts())
        .all(|t| t.output.len() == 2000);
    assert!(all_intact, "fewer than KEEP recent -> nothing elided");
}

#[test]
fn elide_skips_small_outputs() {
    // Many small outputs: even the old ones stay (below MIN_ELIDE_BYTES).
    let mut msgs = vec![Message::user("go")];
    for i in 0..10 {
        msgs.push(tool_msg(&format!("t{i}"), "git_status", "ok".to_string()));
    }
    let out = super::elide_old_tool_outputs(&msgs);
    let all_intact = out
        .iter()
        .flat_map(|m| m.tool_parts())
        .all(|t| t.output == "ok");
    assert!(all_intact, "small outputs are never elided");
}

#[test]
fn elide_preserves_tool_name_and_size_in_placeholder() {
    let mut msgs = vec![Message::user("go")];
    for i in 0..8 {
        msgs.push(tool_msg(&format!("t{i}"), "bash", big_output(5000)));
    }
    let out = super::elide_old_tool_outputs(&msgs);
    let first = &out
        .iter()
        .flat_map(|m| m.tool_parts())
        .next()
        .unwrap()
        .output;
    assert!(first.contains("bash"), "placeholder names the tool");
    assert!(first.contains("5000"), "placeholder reports original size");
}
