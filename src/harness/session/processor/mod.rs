//! Session processor: the core agent loop.
//!
//! Streams LLM responses with native tool calls, executes tools (in parallel
//! when a single assistant message contains multiple calls), persists parts,
//! detects doom loops, and compacts context on overflow.

use std::sync::Arc;
use std::time::Duration;

use crate::harness::agent::AgentSpec;
use crate::harness::event::{EventSender, HarnessEvent};
use crate::harness::provider::ProviderStream;
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
    /// Hard cap on total iterations across all continuations of a single turn
    /// (R4). `None` = conservative default of `max_iterations * 3`. This is the
    /// outer safety net: even if every continuation resets the per-segment
    /// `max_iterations`, the turn cannot exceed this many iterations.
    pub max_total_iterations: Option<usize>,
    // Temperature is agent-calibrated: see AgentSpec::turn_temperature.
}

impl Default for ProcessorConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            max_iterations: 100,
            max_context_tokens: 100_000,
            turn_timeout_secs: DEFAULT_TURN_TIMEOUT_SECS,
            max_total_iterations: None,
        }
    }
}

/// Conservative default for the per-turn iteration budget (R4): three segments
/// worth of `max_iterations`, instead of the old implicit
/// `max_iterations * (DEFAULT_MAX_CONTINUATIONS + 1)` (= 11×).
pub fn default_total_iterations(max_iterations: usize) -> usize {
    max_iterations.saturating_mul(3).max(1)
}

/// Returns a copy of `req.messages` with all `Part::Image` parts removed from
/// user messages (degrading a vision request to text-only).
fn strip_images(req: &LlmRequest) -> Vec<Message> {
    req.messages
        .iter()
        .map(|m| {
            if m.has_image() {
                m.without_images()
            } else {
                m.clone()
            }
        })
        .collect()
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

/// The single decision point for how a turn ends. Produced by
/// [`decide_turn_end`] and applied by `SessionProcessor::apply_turn_decision`.
#[derive(Debug, PartialEq, Eq)]
enum TurnDecision {
    /// Stop the turn, surfacing this text as the final answer.
    Stop(String),
    /// Continue the turn: push `note` to the session, restart the watchdog and
    /// loop again. `reason`/`label` are for logging and the `AutoContinue` event.
    Continue {
        note: String,
        reason: String,
        label: &'static str,
    },
}

/// Decides how a turn ends, given the current state. This is the *only* place
/// the three former code paths (watchdog restart, stop_reason restart,
/// iteration-limit auto-continue) are expressed, so they cannot drift apart.
///
/// - `stop_reason`: set when the attempt ended early (watchdog, stalled stream,
///   transient stream error). `None` on a clean finish.
/// - `continuations`: how many times the turn already auto-continued.
/// - `total_iterations` / `iteration_budget`: the hard global cap that cannot
///   be restarted out of.
fn decide_turn_end(
    stop_reason: Option<String>,
    continuations: usize,
    total_iterations: usize,
    iteration_budget: usize,
) -> TurnDecision {
    if let Some(reason) = stop_reason {
        // The budget-exhausted reason is a hard stop, not a restartable one.
        if reason == "iteration budget exhausted for this turn" {
            return TurnDecision::Stop(format!("Stopped: {}.", reason));
        }
        if continuations >= DEFAULT_MAX_CONTINUATIONS {
            return TurnDecision::Stop(format!(
                "Stopped: {} after {} continuation(s).",
                reason, continuations
            ));
        }
        let next = continuations + 1;
        return TurnDecision::Continue {
            note: format!(
                "[turn restart {}/{}] The turn was interrupted: {}. Review the state \
                 so far, identify what happened, and decide whether to continue the \
                 task from where it stopped or report the blocker.",
                next, DEFAULT_MAX_CONTINUATIONS, reason
            ),
            reason,
            label: "turn restart",
        };
    }

    // No stop_reason: the iteration limit was reached with work still pending.
    // Continuations are checked before the global budget so the exhaustion
    // message reflects the continuation cap (the budget is the outer safety
    // net, hit only if continuations somehow outlive it).
    if continuations >= DEFAULT_MAX_CONTINUATIONS {
        return TurnDecision::Stop(format!(
            "Stopped: reached the iteration limit after {} continuation(s).",
            continuations
        ));
    }
    if total_iterations >= iteration_budget {
        return TurnDecision::Stop(
            "Stopped: iteration budget exhausted for this turn.".to_string(),
        );
    }
    let next = continuations + 1;
    TurnDecision::Continue {
        note: format!(
            "[auto-continue {}/{}] iteration limit reached — resuming the task \
             exactly where it stopped.",
            next, DEFAULT_MAX_CONTINUATIONS
        ),
        reason: "iteration limit reached".to_string(),
        label: "auto-continue",
    }
}

/// Decides how to handle a response the provider truncated at the output-token
/// limit (R3). A truncated response with no tool calls is *not* a final answer:
/// the model was cut off mid-thought, so we continue it. Respects the same
/// continuation/budget caps as [`decide_turn_end`].
fn truncation_decision(
    continuations: usize,
    total_iterations: usize,
    iteration_budget: usize,
) -> TurnDecision {
    if continuations >= DEFAULT_MAX_CONTINUATIONS {
        return TurnDecision::Stop(format!(
            "Stopped: response kept hitting the output-token limit after {} continuation(s).",
            continuations
        ));
    }
    if total_iterations >= iteration_budget {
        return TurnDecision::Stop(
            "Stopped: iteration budget exhausted for this turn.".to_string(),
        );
    }
    let next = continuations + 1;
    TurnDecision::Continue {
        note: format!(
            "[truncated {}/{}] Your previous response was cut off by the output-token \
             limit. Continue exactly where you stopped — do not repeat what you already \
             wrote.",
            next, DEFAULT_MAX_CONTINUATIONS
        ),
        reason: "response truncated by the output-token limit".to_string(),
        label: "truncation continue",
    }
}

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

    /// Applies a [`TurnDecision`] at the single point where the turn either
    /// stops or continues. On `Continue` it pushes/persists the note, restarts
    /// the turn (usage accumulation + watchdog reset) and returns `true` so the
    /// caller can `continue 'turn`. On `Stop` it sets `final_text` and returns
    /// `false`.
    ///
    /// This is the *only* place that mutates `final_text`/`continuations` for
    /// turn-end decisions, so the three former code paths (watchdog restart,
    /// stop_reason restart, iteration-limit auto-continue) can no longer drift.
    #[allow(clippy::too_many_arguments)]
    async fn apply_turn_decision(
        &self,
        decision: TurnDecision,
        session: &mut Session,
        final_text: &mut String,
        continuations: &mut usize,
        usage: &Usage,
        total_usage: &mut Usage,
        iterations: &mut usize,
        turn_deadline: &mut tokio::time::Instant,
        turn_secs: u64,
    ) -> bool {
        match decision {
            TurnDecision::Stop(text) => {
                *final_text = text;
                false
            }
            TurnDecision::Continue {
                note,
                reason,
                label,
            } => {
                *continuations += 1;
                let note = Message::user(note);
                session.push_message(note.clone());
                self.persist(&session.id, &session.cwd, &note).await;
                self.restart_turn(
                    session,
                    &reason,
                    label,
                    *continuations,
                    usage,
                    total_usage,
                    iterations,
                    turn_deadline,
                    turn_secs,
                );
                true
            }
        }
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
        let mut semantic = crate::harness::session::doom_loop::SemanticLoopDetector::new();
        let mut compact_tracker = crate::harness::session::compaction::CompactionTracker::new();
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
        // R4: explicit, conservative global cap across all continuations so
        // restarts cannot pile up unbounded work. Defaults to 3× the per-segment
        // limit (was implicitly 11× via DEFAULT_MAX_CONTINUATIONS + 1).
        let iteration_budget = self
            .config
            .max_total_iterations
            .unwrap_or_else(|| default_total_iterations(self.config.max_iterations));
        tracing::info!(
            "turn start: session={} max_iterations={} iteration_budget={} turn_timeout_secs={}",
            session.id,
            self.config.max_iterations,
            iteration_budget,
            turn_secs
        );

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

                // Compaction on overflow (R1: only when the context grew).
                self.maybe_compact(session, &mut compact_tracker).await?;
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

                let stream = match self.stream_with_image_fallback(&req, &ctx.abort).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::info!(
                            "turn stream aborted after image fallback (session={}): {}",
                            session.id,
                            e
                        );
                        return Err(e);
                    }
                };
                let stream_loop::StreamOutcome {
                    assistant_id,
                    text,
                    reasoning,
                    tool_calls,
                    usage,
                    stop_reason: stream_stop,
                    provider_stop,
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
                    let decision = decide_turn_end(
                        Some(reason),
                        continuations,
                        total_iterations,
                        iteration_budget,
                    );
                    if !self
                        .apply_turn_decision(
                            decision,
                            session,
                            &mut final_text,
                            &mut continuations,
                            &usage,
                            &mut total_usage,
                            &mut iterations,
                            &mut turn_deadline,
                            turn_secs,
                        )
                        .await
                    {
                        break;
                    }
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
                    // R3: a response truncated at the output-token limit is not
                    // a final answer — continue it instead of ending the turn.
                    if crate::harness::provider::is_truncated(provider_stop.as_deref()) {
                        tracing::info!(
                            "response truncated (stop_reason={:?}), continuing (session={})",
                            provider_stop,
                            session.id
                        );
                        let decision =
                            truncation_decision(continuations, total_iterations, iteration_budget);
                        if !self
                            .apply_turn_decision(
                                decision,
                                session,
                                &mut final_text,
                                &mut continuations,
                                &usage,
                                &mut total_usage,
                                &mut iterations,
                                &mut turn_deadline,
                                turn_secs,
                            )
                            .await
                        {
                            break;
                        }
                        continue 'turn;
                    }
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

                // Semantic loop check (R5): the same tool aimed at the same
                // target with no progress (empty/error/identical output). This
                // catches inputs that vary harmlessly (`ls`, `ls .`, `ls ./`)
                // and repeated failures, which the byte-identical detector
                // above misses. Reads the *updated* tool parts (with outputs)
                // from the session, not the pre-execution clone.
                let outcomes = session
                    .messages
                    .iter()
                    .find(|m| m.id == assistant_id)
                    .map(|m| {
                        m.tool_parts()
                            .iter()
                            .map(|t| crate::harness::session::doom_loop::ToolOutcome {
                                name: t.name.clone(),
                                target: crate::harness::session::doom_loop::tool_target(
                                    &t.name, &t.input,
                                ),
                                output: t.output.clone(),
                                is_error: t.status == crate::harness::event::ToolStatus::Error,
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                match semantic.record(outcomes) {
                    crate::harness::session::doom_loop::DoomAction::Stop => {
                        tracing::warn!(
                            "semantic loop detected: same tool/target repeated {} times \
                             without progress (session={})",
                            crate::harness::session::doom_loop::DOOM_LOOP_STOP,
                            session.id
                        );
                        final_text = "Stopped: the same tool was called repeatedly on the same \
                                      target without progress."
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
                            "System note: you are calling the same tool on the same target \
                             repeatedly without progress. Change the target or approach.",
                        );
                        session.push_message(warn.clone());
                        self.persist(&session.id, &session.cwd, &warn).await;
                    }
                    crate::harness::session::doom_loop::DoomAction::Continue => {}
                }
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
            let decision = decide_turn_end(
                stop_reason.take(),
                continuations,
                total_iterations,
                iteration_budget,
            );
            if !self
                .apply_turn_decision(
                    decision,
                    session,
                    &mut final_text,
                    &mut continuations,
                    &Usage::default(),
                    &mut total_usage,
                    &mut iterations,
                    &mut turn_deadline,
                    turn_secs,
                )
                .await
            {
                break 'turn;
            }
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

    /// Streams a request, degrading images to text when the provider rejects
    /// them (e.g. xAI grok returns a permanent 400 "Invalid PNG image" that
    /// would otherwise brick the session). The retry-with-policy handles
    /// transient errors; this handles the permanent image-rejection case by
    /// retrying once with the image parts stripped from the user message.
    async fn stream_with_image_fallback(
        &self,
        req: &LlmRequest,
        abort: &crate::harness::tool::context::AbortSignal,
    ) -> anyhow::Result<ProviderStream> {
        let retry_policy = crate::harness::provider::retry::RetryPolicy::default();
        let provider = self.provider.clone();
        let abort_flag = abort.clone();

        // Proactive degradation: if the catalog knows this provider/model is
        // text-only and the request carries images, strip them up front instead
        // of burning a request that is guaranteed to be rejected. The reactive
        // fallback below still covers providers that reject images at runtime.
        let has_image = req.messages.iter().any(|m| m.has_image());
        let req = if has_image
            && !crate::harness::provider::catalog::supports_image(provider.name(), &req.model)
        {
            tracing::warn!(
                provider = provider.name(),
                model = %req.model,
                "model is text-only; stripping image parts before request"
            );
            LlmRequest {
                messages: Arc::new(strip_images(req)),
                ..req.clone()
            }
        } else {
            req.clone()
        };
        let req_clone = req.clone();
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
            || abort.is_aborted(),
        )
        .await;

        match stream {
            Ok(s) => Ok(s),
            Err(e) => {
                // Only attempt image degradation when (a) the request still
                // carries image parts, and (b) the error mentions images/vision.
                let still_has_image = req.messages.iter().any(|m| m.has_image());
                let err_text = format!("{e:#}");
                let looks_like_image_rejection = err_text.to_lowercase().contains("image")
                    || err_text.to_lowercase().contains("picture")
                    || err_text.to_lowercase().contains("vision")
                    || err_text.to_lowercase().contains("multimodal");
                if !still_has_image || !looks_like_image_rejection {
                    return Err(e);
                }

                tracing::warn!(
                    "provider rejected image(s), degrading to text and retrying: {}",
                    err_text
                );
                let stripped = strip_images(&req);
                let req2 = LlmRequest {
                    messages: Arc::new(stripped),
                    ..req.clone()
                };
                let abort_flag2 = abort.clone();
                let provider2 = provider.clone();
                let req2c = req2.clone();
                let stream2 = crate::harness::provider::retry::retry_with_policy(
                    &retry_policy,
                    || {
                        let provider = provider2.clone();
                        let req = req2c.clone();
                        let abort = abort_flag2.clone();
                        async move {
                            if abort.is_aborted() {
                                return Err(anyhow::anyhow!("aborted by user"));
                            }
                            provider.stream(&req).await
                        }
                    },
                    || abort.is_aborted(),
                )
                .await;
                match stream2 {
                    Ok(s) => Ok(s),
                    Err(e2) => Err(e2.context(format!(
                        "image degraded to text, but provider still failed (original: {err_text})"
                    ))),
                }
            }
        }
    }

    async fn maybe_compact(
        &self,
        session: &mut Session,
        tracker: &mut crate::harness::session::compaction::CompactionTracker,
    ) -> anyhow::Result<()> {
        // R1: only re-evaluate when the context grew enough since the last
        // check — avoids an O(n) token scan on every loop iteration.
        if !tracker.should_check(&session.messages) {
            return Ok(());
        }
        crate::harness::session::compaction::compact_if_needed(
            session,
            self.provider.clone(),
            &self.store,
            self.config.max_context_tokens,
            false,
            Some(&self.events),
            &self.config.model,
        )
        .await?;
        tracker.record(&session.messages);
        Ok(())
    }
}

mod stream_loop;

#[cfg(test)]
mod tests;
