//! Session processor: the core agent loop.
//!
//! Streams LLM responses with native tool calls, executes tools (in parallel
//! when a single assistant message contains multiple calls), persists parts,
//! detects doom loops, and compacts context on overflow.

use crate::harness::agent::AgentSpec;
use crate::harness::event::{EventSender, HarnessEvent};
use crate::harness::provider::{LlmRequest, Provider, ProviderEvent, ToolSpec, Usage};
use crate::harness::session::{Message, Part, Role, Session, ToolPart};
use crate::harness::tool::registry::ToolRegistry;
use futures_util::StreamExt;
use std::sync::Arc;
use std::time::Duration;

/// Tunables for one turn of the processor.
#[derive(Clone, Debug)]
pub struct ProcessorConfig {
    pub model: String,
    pub max_iterations: usize,
    pub max_context_tokens: usize,
    /// Wall-clock limit per turn, in seconds (0 = default).
    pub turn_timeout_secs: u64,
    // Temperature is agent-calibrated: see AgentSpec::turn_temperature.
}

impl Default for ProcessorConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            max_iterations: 100,
            max_context_tokens: 100_000,
            turn_timeout_secs: DEFAULT_TURN_TIMEOUT_SECS,
        }
    }
}

/// Deps the processor needs; all shared, cheap to clone.
#[derive(Clone)]
pub struct SessionProcessor {
    pub provider: ArcProvider,
    pub registry: ToolRegistry,
    pub events: EventSender,
    pub store: Arc<crate::harness::session::store::SessionStore>,
    pub config: ProcessorConfig,
}

pub type ArcProvider = Arc<dyn Provider>;

/// Result of a full turn.
pub struct TurnOutcome {
    pub final_text: String,
    pub iterations: usize,
    /// How many times the turn auto-continued after hitting the limit.
    /// Informational; not consumed by callers yet.
    #[allow(dead_code)]
    pub continuations: usize,
    pub usage: Usage,
    pub aborted: bool,
}

/// How many times a turn may auto-continue after hitting max_iterations
/// (effective iteration budget = (N + 1) × max_iterations).
const DEFAULT_MAX_CONTINUATIONS: usize = 10;
/// Safety net: if the provider stream stalls (no event for this long), abort
/// the turn instead of hanging forever. Generous so long reasoning streams
/// aren't interrupted; the client `read_timeout` normally fires first.
const STREAM_TIMEOUT_SECS: u64 = 300;
/// Wall-clock safety net for a whole turn: even if every individual stream
/// and tool call stays under its own timeout, a turn that keeps going for
/// this long is stopped instead of hanging forever. 0 = use the default.
const DEFAULT_TURN_TIMEOUT_SECS: u64 = 1800;

impl SessionProcessor {
    /// Persists a message to the store, logging a warning on failure instead
    /// of silently swallowing the error (so the history never silently diverges
    /// from what the agent sees).
    pub(crate) async fn persist(&self, session_id: &str, cwd: &std::path::Path, msg: &Message) {
        let store = self.store.clone();
        let sid = session_id.to_string();
        let cwd = cwd.to_path_buf();
        let msg = msg.clone();
        let res = tokio::task::spawn_blocking(move || store.save_message(&sid, &cwd, &msg)).await;
        if let Err(e) = res
            .map_err(|e| anyhow::anyhow!("join error: {e}"))
            .and_then(|r| r)
        {
            tracing::warn!("failed to persist message (session={}): {}", session_id, e);
        }
    }

    /// Emits an event to the bus, logging a warning on failure instead of
    /// silently dropping it.
    pub(crate) fn emit(&self, ev: HarnessEvent) {
        if let Err(e) = self.events.send(ev) {
            tracing::warn!("failed to emit event: {}", e);
        }
    }

    /// Shared turn-restart logic: budget check, restart note, event emission,
    /// usage accumulation and watchdog reset. Returns `false` when the
    /// continuation budget is exhausted (caller should stop with `reason`).
    #[allow(clippy::too_many_arguments)]
    fn restart_turn(
        &self,
        session: &Session,
        reason: &str,
        label: &str,
        continuations: usize,
        usage: &Usage,
        total_usage: &mut Usage,
        iterations: &mut usize,
        turn_deadline: &mut tokio::time::Instant,
        turn_secs: u64,
    ) -> bool {
        total_usage.input_tokens += usage.input_tokens;
        total_usage.output_tokens += usage.output_tokens;
        total_usage.cache_read_tokens += usage.cache_read_tokens;
        total_usage.cache_write_tokens += usage.cache_write_tokens;
        // Note: push/persist of the restart note happen in the caller
        // (needs `&mut Session`).
        tracing::info!(
            "{label} {continuations}/{}: {reason} (session={})",
            DEFAULT_MAX_CONTINUATIONS,
            session.id
        );
        self.emit(HarnessEvent::AutoContinue {
            session_id: session.id.clone(),
            round: continuations,
            total: DEFAULT_MAX_CONTINUATIONS,
            reason: format!("{reason} — turn restarted"),
            parent_session_id: None,
        });
        *iterations = 0;
        *turn_deadline = tokio::time::Instant::now() + Duration::from_secs(turn_secs);
        true
    }

    /// Runs one user turn: loops stream -> tool exec until the model answers
    /// without tool calls, hits max iterations, or is aborted.
    pub async fn run_turn(
        &self,
        session: &mut Session,
        agent: &AgentSpec,
        system_prompt: &str,
        ctx: &crate::harness::tool::context::ToolContext,
    ) -> anyhow::Result<TurnOutcome> {
        let tool_specs: Vec<ToolSpec> = self.registry.specs(&agent.tools);
        let mut total_usage = Usage::default();
        let mut iterations = 0usize;
        let mut final_text = String::new();
        let mut aborted = false;
        let mut doom = crate::harness::session::doom_loop::DoomLoopDetector::new();
        let mut text_loop = crate::harness::session::doom_loop::TextLoopDetector::new();
        let mut continuations = 0usize;
        let mut total_iterations = 0usize;
        let mut stop_reason: Option<String> = None;

        tracing::info!(
            "turn start: session={} agent={} model={} tools={}",
            session.id,
            agent.name,
            agent.model.as_deref().unwrap_or(&self.config.model),
            tool_specs.len()
        );

        let turn_secs = if self.config.turn_timeout_secs == 0 {
            DEFAULT_TURN_TIMEOUT_SECS
        } else {
            self.config.turn_timeout_secs
        };
        let mut turn_deadline = tokio::time::Instant::now() + Duration::from_secs(turn_secs);
        // Global cap across all continuations so restarts cannot pile up
        // unbounded work (e.g. 74 iterations / million-token turns).
        let iteration_budget = self.config.max_iterations * (DEFAULT_MAX_CONTINUATIONS + 1);

        'turn: loop {
            while iterations < self.config.max_iterations {
                if ctx.abort.is_aborted() {
                    aborted = true;
                    break;
                }
                if total_iterations >= iteration_budget {
                    stop_reason = Some("iteration budget exhausted for this turn".to_string());
                    break;
                }
                if tokio::time::Instant::now() >= turn_deadline {
                    stop_reason = Some(format!("turn exceeded the {}s time limit", turn_secs));
                    break;
                }
                iterations += 1;
                total_iterations += 1;

                // Compaction on overflow.
                self.maybe_compact(session).await?;
                if ctx.abort.is_aborted() {
                    aborted = true;
                    break;
                }

                let req = LlmRequest {
                    model: agent
                        .model
                        .clone()
                        .unwrap_or_else(|| self.config.model.clone()),
                    system: system_prompt.to_string(),
                    messages: std::sync::Arc::new(session.messages.clone()),
                    tools: tool_specs.clone(),
                    max_tokens: None,
                    temperature: agent.turn_temperature(),
                };

                let retry_policy = crate::harness::provider::retry::RetryPolicy::default();
                let provider = self.provider.clone();
                let req_clone = req.clone();
                let abort_flag = ctx.abort.clone();
                let mut stream = crate::harness::provider::retry::retry_with_policy(
                    &retry_policy,
                    || {
                        let provider = provider.clone();
                        let req = req_clone.clone();
                        let abort = abort_flag.clone();
                        async move {
                            if abort.is_aborted() {
                                return Err(anyhow::anyhow!("aborted by user"));
                            }
                            provider.stream(&req).await
                        }
                    },
                    || ctx.abort.is_aborted(),
                )
                .await?;
                let assistant_id = crate::harness::session::new_id();
                let mut text = String::new();
                let mut reasoning = String::new();
                let mut tool_calls: Vec<ToolPart> = Vec::new();
                let mut usage = Usage::default();

                loop {
                    // Abort responsively mid-stream (Esc / Ctrl+C cancel).
                    if ctx.abort.is_aborted() {
                        aborted = true;
                        break;
                    }
                    // Race the next stream event against a short abort poll so a
                    // stuck/slow provider doesn't ignore Esc until the next token.
                    let next = tokio::select! {
                        biased;
                        _ = async {
                            loop {
                                if ctx.abort.is_aborted() {
                                    break;
                                }
                                tokio::time::sleep(Duration::from_millis(50)).await;
                            }
                        } => {
                            aborted = true;
                            break;
                        }
                        _ = tokio::time::sleep(
                            turn_deadline.saturating_duration_since(tokio::time::Instant::now()),
                        ) => {
                            // Turn-level watchdog: stop this attempt but let the
                            // restart decision decide whether to resume.
                            stop_reason =
                                Some(format!("turn exceeded the {}s time limit", turn_secs));
                            break;
                        }
                        ev = tokio::time::timeout(
                            Duration::from_secs(STREAM_TIMEOUT_SECS),
                            stream.next(),
                        ) => ev,
                    };
                    let ev = match next {
                        Ok(Some(ev)) => ev,
                        Ok(None) => break, // stream terminou
                        Err(_) => {
                            // Safety net: provider stream stalled. Record and
                            // let the restart decision decide.
                            stop_reason =
                                Some(format!("stream timed out after {}s", STREAM_TIMEOUT_SECS));
                            break;
                        }
                    };
                    // Transient stream errors (malformed SSE, connection
                    // reset) become a restart reason instead of failing the
                    // whole turn — accumulated text/tools are kept.
                    let ev = match ev {
                        Ok(ev) => ev,
                        Err(e) => {
                            stop_reason = Some(format!("stream error: {e}"));
                            break;
                        }
                    };
                    match ev {
                        ProviderEvent::TextDelta(d) => {
                            text.push_str(&d);
                            self.emit(HarnessEvent::TextDelta {
                                session_id: session.id.clone(),
                                message_id: assistant_id.to_string(),
                                delta: d,
                                parent_session_id: None,
                            });
                        }
                        ProviderEvent::ReasoningDelta(d) => {
                            reasoning.push_str(&d);
                            self.emit(HarnessEvent::ReasoningDelta {
                                session_id: session.id.clone(),
                                message_id: assistant_id.to_string(),
                                delta: d,
                                parent_session_id: None,
                            });
                        }
                        ProviderEvent::ToolCallStart { id, name } => {
                            tool_calls.push(ToolPart::pending(id, name, serde_json::Value::Null));
                            self.emit(HarnessEvent::MessageUpdated {
                                session_id: session.id.clone(),
                                message_id: assistant_id.to_string(),
                                parent_session_id: None,
                            });
                        }
                        ProviderEvent::ToolCallDelta { id, args_delta } => {
                            // Consume but do not surface; args complete at ToolCallEnd.
                            let _ = (id, args_delta);
                        }
                        ProviderEvent::ToolCallEnd { id, arguments } => {
                            if let Some(part) = tool_calls.iter_mut().find(|t| t.id == id) {
                                match serde_json::from_str::<serde_json::Value>(&arguments) {
                                    Ok(v) => part.input = v,
                                    Err(e) => {
                                        part.status = crate::harness::event::ToolStatus::Error;
                                        part.error = Some(format!("invalid tool arguments: {}", e));
                                    }
                                }
                            } else {
                                // A ToolCallEnd without a matching ToolCallStart (provider
                                // bug or malformed stream). Recover by creating the part so
                                // the call isn't silently dropped and the model gets a
                                // consistent history.
                                tracing::warn!(
                                    "ToolCallEnd for unknown id `{}` (no ToolCallStart seen)",
                                    id
                                );
                                let input = serde_json::from_str::<serde_json::Value>(&arguments)
                                    .unwrap_or(serde_json::Value::Null);
                                tool_calls.push(ToolPart::pending(id, "unknown", input));
                            }
                        }
                        ProviderEvent::End {
                            stop_reason: _,
                            usage: u,
                        } => {
                            if let Some(u) = u {
                                usage.input_tokens += u.input_tokens;
                                usage.output_tokens += u.output_tokens;
                                usage.cache_read_tokens += u.cache_read_tokens;
                                usage.cache_write_tokens += u.cache_write_tokens;
                            }
                        }
                    }
                }

                // User cancel or watchdog/stream timeout: stop this attempt.
                // Aborts skip saving partial state; watchdogs go through the
                // restart decision below (after the inner while).
                if aborted || ctx.abort.is_aborted() {
                    aborted = true;
                    if final_text.is_empty() {
                        final_text = "Run aborted by user.".to_string();
                    }
                    break;
                }
                if let Some(reason) = stop_reason.take() {
                    if continuations >= DEFAULT_MAX_CONTINUATIONS {
                        final_text = format!(
                            "Stopped: {} after {} continuation(s).",
                            reason, continuations
                        );
                        break;
                    }
                    continuations += 1;
                    let note = Message::user(format!(
                        "[turn restart {}/{}] The turn was interrupted: {}. Review the state \
                         so far, identify what happened, and decide whether to continue the \
                         task from where it stopped or report the blocker.",
                        continuations, DEFAULT_MAX_CONTINUATIONS, reason
                    ));
                    session.push_message(note.clone());
                    self.persist(&session.id, &session.cwd, &note).await;
                    self.restart_turn(
                        session,
                        &reason,
                        "turn restart",
                        continuations,
                        &usage,
                        &mut total_usage,
                        &mut iterations,
                        &mut turn_deadline,
                        turn_secs,
                    );
                    continue 'turn;
                }

                total_usage.input_tokens += usage.input_tokens;
                total_usage.output_tokens += usage.output_tokens;
                total_usage.cache_read_tokens += usage.cache_read_tokens;
                total_usage.cache_write_tokens += usage.cache_write_tokens;

                // Build assistant message.
                let mut parts = Vec::new();
                if !reasoning.is_empty() {
                    parts.push(Part::Reasoning { text: reasoning });
                }
                if !text.is_empty() {
                    parts.push(Part::text(text.clone()));
                }
                for t in tool_calls {
                    parts.push(Part::Tool(t));
                }
                if parts.is_empty() {
                    parts.push(Part::text(""));
                }

                let assistant = Message::with_id(assistant_id.clone(), Role::Assistant, parts);
                session.push_message(assistant.clone());
                {
                    let sid = session.id.clone();
                    let cwd = session.cwd.clone();
                    self.persist(&sid, &cwd, &assistant).await;
                }

                if !assistant.has_tool_calls() {
                    final_text = text;
                    break;
                }

                // Execute tool calls (parallel where possible).
                self.execute_tool_calls(session, &assistant_id, ctx).await;

                // Esc during tool execution: stop the turn immediately.
                if ctx.abort.is_aborted() {
                    aborted = true;
                    if final_text.is_empty() {
                        final_text = "Run aborted by user.".to_string();
                    }
                    break;
                }

                // Doom loop check: detect repeated tool calls across iterations,
                // including multi-call cycles (A,B,A,B) — not just a single
                // repeated call. We compare the *set* of signatures of the
                // current turn against the previous turns' sets.
                let sigs = assistant
                    .tool_parts()
                    .iter()
                    .map(|t| format!("{}:{}", t.name, t.input))
                    .collect::<Vec<_>>();
                match doom.record(sigs) {
                    crate::harness::session::doom_loop::DoomAction::Stop => {
                        tracing::warn!(
                            "doom loop detected: same tool call(s) repeated {} times (session={})",
                            crate::harness::session::doom_loop::DOOM_LOOP_STOP,
                            session.id
                        );
                        final_text =
                            "Stopped: the same tool call was repeated many times without progress."
                                .to_string();
                        self.emit(HarnessEvent::Error {
                            session_id: session.id.clone(),
                            message: final_text.clone(),
                            parent_session_id: None,
                        });
                        break;
                    }
                    crate::harness::session::doom_loop::DoomAction::Warn => {
                        let warn = Message::user(
                            "System note: you just repeated the same tool call. Change the input \
                         or try a different approach.",
                        );
                        session.push_message(warn.clone());
                        self.persist(&session.id, &session.cwd, &warn).await;
                    }
                    crate::harness::session::doom_loop::DoomAction::Continue => {}
                }

                // Text-loop detection across iterations: the same (or
                // alternating) assistant text with no real progress. Warns
                // once, then hard-stops the turn.
                let text_sig = crate::harness::session::doom_loop::normalize_text(&text);
                match text_loop.record(text_sig) {
                    crate::harness::session::doom_loop::DoomAction::Stop => {
                        tracing::warn!(
                            "text loop detected: same assistant text repeated {} times \
                             (session={})",
                            crate::harness::session::doom_loop::TEXT_LOOP_STOP,
                            session.id
                        );
                        final_text =
                            "Stopped: the model was repeating the same response without progress."
                                .to_string();
                        self.emit(HarnessEvent::Error {
                            session_id: session.id.clone(),
                            message: final_text.clone(),
                            parent_session_id: None,
                        });
                        break;
                    }
                    crate::harness::session::doom_loop::DoomAction::Warn => {
                        let warn = Message::user(
                            "System note: you are repeating the same response. Change the \
                             approach or give your final answer.",
                        );
                        session.push_message(warn.clone());
                        self.persist(&session.id, &session.cwd, &warn).await;
                    }
                    crate::harness::session::doom_loop::DoomAction::Continue => {}
                }
            }

            // Automatic continuation: the iteration limit was reached but the
            // turn is neither aborted nor has a final answer, so resume from
            // where it left off (up to DEFAULT_MAX_CONTINUATIONS times).
            if aborted || !final_text.is_empty() {
                break 'turn;
            }
            if let Some(reason) = stop_reason.take() {
                // A hard global budget cannot be restarted out of.
                if reason == "iteration budget exhausted for this turn" {
                    final_text = format!("Stopped: {}.", reason);
                    break 'turn;
                }
                // Watchdog stop (e.g. time limit): let the model review what
                // happened and decide whether to continue, budget permitting.
                if continuations >= DEFAULT_MAX_CONTINUATIONS {
                    final_text = format!(
                        "Stopped: {} after {} continuation(s).",
                        reason, continuations
                    );
                    break 'turn;
                }
                continuations += 1;
                let note = Message::user(format!(
                    "[turn restart {}/{}] The turn was interrupted: {}. Review the state \
                     so far, identify what happened, and decide whether to continue the \
                     task from where it stopped or report the blocker.",
                    continuations, DEFAULT_MAX_CONTINUATIONS, reason
                ));
                session.push_message(note.clone());
                self.persist(&session.id, &session.cwd, &note).await;
                self.restart_turn(
                    session,
                    &reason,
                    "turn restart",
                    continuations,
                    &Usage::default(),
                    &mut total_usage,
                    &mut iterations,
                    &mut turn_deadline,
                    turn_secs,
                );
                continue 'turn;
            }
            if continuations >= DEFAULT_MAX_CONTINUATIONS {
                final_text = format!(
                    "Stopped: reached the iteration limit after {} continuation(s).",
                    continuations
                );
                break 'turn;
            }
            if total_iterations >= iteration_budget {
                final_text = "Stopped: iteration budget exhausted for this turn.".to_string();
                break 'turn;
            }
            continuations += 1;
            let note = Message::user(format!(
                "[auto-continue {}/{}] iteration limit reached — resuming the task \
                 exactly where it stopped.",
                continuations, DEFAULT_MAX_CONTINUATIONS
            ));
            session.push_message(note.clone());
            self.persist(&session.id, &session.cwd, &note).await;
            tracing::info!(
                "auto-continue {}/{}: iteration limit reached (session={})",
                continuations,
                DEFAULT_MAX_CONTINUATIONS,
                session.id
            );
            self.emit(HarnessEvent::AutoContinue {
                session_id: session.id.clone(),
                round: continuations,
                total: DEFAULT_MAX_CONTINUATIONS,
                reason: "iteration limit reached".to_string(),
                parent_session_id: None,
            });
            iterations = 0;
            turn_deadline = tokio::time::Instant::now() + Duration::from_secs(turn_secs);
        }

        if ctx.abort.is_aborted() && final_text.is_empty() {
            final_text = "Run aborted by user.".to_string();
        }

        // Fire-and-forget on_turn_end hooks (never break the turn).
        crate::harness::hooks::spawn_turn_end(&ctx.hooks, ctx.cwd.path());

        tracing::info!(
            "turn end: session={} iterations={} continuations={} aborted={} input_tokens={} output_tokens={} cache_read_tokens={} cache_write_tokens={}",
            session.id,
            total_iterations,
            continuations,
            aborted,
            total_usage.input_tokens,
            total_usage.output_tokens,
            total_usage.cache_read_tokens,
            total_usage.cache_write_tokens
        );

        Ok(TurnOutcome {
            final_text,
            iterations: total_iterations,
            continuations,
            usage: total_usage,
            aborted,
        })
    }

    /// Executes all pending tool calls in one assistant message,
    /// running them concurrently and applying results as they finish.
    async fn execute_tool_calls(
        &self,
        session: &mut Session,
        assistant_id: &str,
        ctx: &crate::harness::tool::context::ToolContext,
    ) {
        crate::harness::session::tool_exec::execute_tool_calls(self, session, assistant_id, ctx)
            .await;
    }

    async fn maybe_compact(&self, session: &mut Session) -> anyhow::Result<()> {
        crate::harness::session::compaction::compact_if_needed(
            session,
            self.provider.clone(),
            &self.store,
            self.config.max_context_tokens,
            false,
            Some(&self.events),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::provider::{LlmResponse, ProviderStream};
    use crate::harness::tool::context::{AbortSignal, PathBufGuard, ToolContext};
    use std::sync::Arc as StdArc;

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
            events: crate::harness::event::event_channel().0,
            project_memory: None,
            hooks: Default::default(),
            checkpoints: std::sync::Arc::new(
                crate::harness::tool::checkpoint::FileCheckpoints::new(),
            ),
            jobs: std::sync::Arc::new(crate::harness::tool::jobs::JobRegistry::new()),
        }
    }

    #[tokio::test]
    async fn test_turn_timeout_stops_run() {
        let dir = tempfile::tempdir().unwrap();
        let store = StdArc::new(
            crate::harness::session::store::SessionStore::open(&dir.path().join("test.db"))
                .unwrap(),
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
            crate::harness::session::store::SessionStore::open(&dir.path().join("test.db"))
                .unwrap(),
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
        let dir = tempfile::tempdir().unwrap();
        let store = StdArc::new(
            crate::harness::session::store::SessionStore::open(&dir.path().join("test.db"))
                .unwrap(),
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
        let (processor, dir) = test_processor(provider, registry, 1);
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
                m.role.as_str() == "user"
                    && m.text_content().contains("repeating the same response")
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
}
