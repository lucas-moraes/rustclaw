//! Session processor: the core agent loop.
//!
//! Streams LLM responses with native tool calls, executes tools (in parallel
//! when a single assistant message contains multiple calls), persists parts,
//! detects doom loops, and compacts context on overflow.

use std::sync::Arc;
use std::time::Duration;

use crate::harness::agent::AgentSpec;
use crate::harness::event::{EventSender, HarnessEvent};
use crate::harness::provider::{LlmRequest, Provider, ToolSpec, Usage};
use crate::harness::session::{Message, Part, Role, Session};
use crate::harness::tool::registry::ToolRegistry;

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
                    messages: session.messages_arc(),
                    tools: tool_specs.clone(),
                    max_tokens: None,
                    temperature: agent.turn_temperature(),
                };

                let retry_policy = crate::harness::provider::retry::RetryPolicy::default();
                let provider = self.provider.clone();
                let req_clone = req.clone();
                let abort_flag = ctx.abort.clone();
                let stream = crate::harness::provider::retry::retry_with_policy(
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
                let stream_loop::StreamOutcome {
                    assistant_id,
                    text,
                    reasoning,
                    tool_calls,
                    usage,
                    stop_reason: stream_stop,
                    aborted: stream_aborted,
                } = stream_loop::consume_stream(
                    self,
                    session,
                    stream,
                    turn_deadline,
                    turn_secs,
                    &ctx.abort,
                )
                .await;
                if stream_aborted {
                    aborted = true;
                }
                if stop_reason.is_none() {
                    stop_reason = stream_stop;
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

mod stream_loop;

#[cfg(test)]
mod tests;
