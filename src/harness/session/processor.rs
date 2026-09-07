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

const DOOM_LOOP_WARN: usize = 3;
const DOOM_LOOP_STOP: usize = 5;
/// How many times a turn may auto-continue after hitting max_iterations
/// (effective iteration budget = (N + 1) × max_iterations).
const DEFAULT_MAX_CONTINUATIONS: usize = 3;
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
    fn persist(&self, session_id: &str, cwd: &std::path::Path, msg: &Message) {
        if let Err(e) = self.store.save_message(session_id, cwd, msg) {
            tracing::warn!("failed to persist message (session={}): {}", session_id, e);
        }
    }

    /// Emits an event to the bus, logging a warning on failure instead of
    /// silently dropping it.
    fn emit(&self, ev: HarnessEvent) {
        if let Err(e) = self.events.send(ev) {
            tracing::warn!("failed to emit event: {}", e);
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
        let mut recent_sigs: Vec<String> = Vec::new();
        let mut warned = false;
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

        'turn: loop {
            while iterations < self.config.max_iterations {
                if ctx.abort.is_aborted() {
                    aborted = true;
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
                    messages: session.messages.clone(),
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
                    match ev? {
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
                            }
                        }
                        ProviderEvent::End {
                            stop_reason: _,
                            usage: u,
                        } => {
                            if let Some(u) = u {
                                usage.input_tokens += u.input_tokens;
                                usage.output_tokens += u.output_tokens;
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
                    self.persist(&session.id, &session.cwd, &note);
                    tracing::info!(
                        "turn restart {}/{}: {} (session={})",
                        continuations,
                        DEFAULT_MAX_CONTINUATIONS,
                        reason,
                        session.id
                    );
                    self.emit(HarnessEvent::AutoContinue {
                        session_id: session.id.clone(),
                        round: continuations,
                        total: DEFAULT_MAX_CONTINUATIONS,
                        reason: format!("{} — turn restarted", reason),
                        parent_session_id: None,
                    });
                    total_usage.input_tokens += usage.input_tokens;
                    total_usage.output_tokens += usage.output_tokens;
                    iterations = 0;
                    turn_deadline = tokio::time::Instant::now() + Duration::from_secs(turn_secs);
                    continue 'turn;
                }

                total_usage.input_tokens += usage.input_tokens;
                total_usage.output_tokens += usage.output_tokens;

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
                let _ = self
                    .store
                    .save_message(&session.id, &session.cwd, &assistant);

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

                // Doom loop check (single repeated call across iterations).
                let sigs = assistant
                    .tool_parts()
                    .iter()
                    .map(|t| format!("{}:{}", t.name, t.input))
                    .collect::<Vec<_>>();
                if sigs.len() == 1 {
                    let sig = sigs[0].clone();
                    if recent_sigs.last().map(|s| s == &sig).unwrap_or(false) {
                        recent_sigs.push(sig.clone());
                    } else {
                        recent_sigs.clear();
                        recent_sigs.push(sig);
                    }
                } else {
                    recent_sigs.clear();
                }

                if recent_sigs.len() >= DOOM_LOOP_STOP {
                    tracing::warn!(
                        "doom loop detected: same tool call repeated {} times (session={})",
                        recent_sigs.len(),
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
                if recent_sigs.len() == DOOM_LOOP_WARN && !warned {
                    warned = true;
                    let warn = Message::user(
                        "System note: you just repeated the same tool call. Change the input \
                     or try a different approach.",
                    );
                    session.push_message(warn.clone());
                    self.persist(&session.id, &session.cwd, &warn);
                }
            }

            // Automatic continuation: the iteration limit was reached but the
            // turn is neither aborted nor has a final answer, so resume from
            // where it left off (up to DEFAULT_MAX_CONTINUATIONS times).
            if aborted || !final_text.is_empty() {
                break 'turn;
            }
            if let Some(reason) = stop_reason.take() {
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
                self.persist(&session.id, &session.cwd, &note);
                tracing::info!(
                    "turn restart {}/{}: {} (session={})",
                    continuations,
                    DEFAULT_MAX_CONTINUATIONS,
                    reason,
                    session.id
                );
                self.emit(HarnessEvent::AutoContinue {
                    session_id: session.id.clone(),
                    round: continuations,
                    total: DEFAULT_MAX_CONTINUATIONS,
                    reason: format!("{} — turn restarted", reason),
                    parent_session_id: None,
                });
                iterations = 0;
                turn_deadline = tokio::time::Instant::now() + Duration::from_secs(turn_secs);
                continue 'turn;
            }
            if continuations >= DEFAULT_MAX_CONTINUATIONS {
                final_text = format!(
                    "Stopped: reached the iteration limit after {} continuation(s).",
                    continuations
                );
                break 'turn;
            }
            continuations += 1;
            let note = Message::user(format!(
                "[auto-continue {}/{}] iteration limit reached — resuming the task \
                 exactly where it stopped.",
                continuations, DEFAULT_MAX_CONTINUATIONS
            ));
            session.push_message(note.clone());
            self.persist(&session.id, &session.cwd, &note);
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

        tracing::info!(
            "turn end: session={} iterations={} continuations={} aborted={} input_tokens={} output_tokens={}",
            session.id,
            total_iterations,
            continuations,
            aborted,
            total_usage.input_tokens,
            total_usage.output_tokens
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
        use crate::harness::event::ToolStatus;

        // Mark running + emit start events.
        let pending: Vec<(String, String, serde_json::Value)> = {
            let Some(msg) = session.messages.iter_mut().find(|m| m.id == *assistant_id) else {
                return;
            };
            msg.parts
                .iter_mut()
                .filter_map(|p| match p {
                    Part::Tool(t) => Some(t),
                    _ => None,
                })
                .filter(|t| t.status == ToolStatus::Pending)
                .map(|t| {
                    t.status = ToolStatus::Running;
                    self.emit(HarnessEvent::ToolStart {
                        session_id: session.id.clone(),
                        message_id: assistant_id.to_string(),
                        tool_id: t.id.clone(),
                        name: t.name.clone(),
                        input: t.input.clone(),
                        parent_session_id: None,
                    });
                    (t.id.clone(), t.name.clone(), t.input.clone())
                })
                .collect()
        };
        if let Some(msg) = session.messages.iter().find(|m| m.id == *assistant_id) {
            let snapshot = msg.clone();
            self.persist(&session.id, &session.cwd, &snapshot);
        }

        // Spawn executions (permission-checked, then run concurrently).
        let mut join_set = tokio::task::JoinSet::new();
        for (tool_id, name, input) in pending {
            if ctx.abort.is_aborted() {
                if let Some(msg) = session.messages.iter_mut().find(|m| m.id == *assistant_id) {
                    if let Some(t) = found_tool(msg, &tool_id) {
                        if t.status == ToolStatus::Pending || t.status == ToolStatus::Running {
                            t.status = ToolStatus::Error;
                            t.error = Some("aborted".to_string());
                        }
                    }
                }
                continue;
            }
            let registry = self.registry.clone();
            let mut ctx2 = ctx.clone();
            ctx2.session_id = session.id.clone();
            join_set.spawn(async move {
                if ctx2.abort.is_aborted() {
                    return (tool_id, name, Err("aborted".to_string()));
                }
                let result = match ctx2.check_permission(&name, &input).await {
                    Ok(()) => {
                        if ctx2.abort.is_aborted() {
                            Err("aborted".to_string())
                        } else {
                            registry.execute(&name, input, &ctx2).await
                        }
                    }
                    Err(e) => Err(e),
                };
                (tool_id, name, result)
            });
        }

        // Apply results in completion order. Esc mid-batch aborts the rest.
        while !join_set.is_empty() {
            if ctx.abort.is_aborted() {
                join_set.abort_all();
                if let Some(msg) = session.messages.iter_mut().find(|m| m.id == *assistant_id) {
                    for p in &mut msg.parts {
                        if let Part::Tool(t) = p {
                            if t.status == ToolStatus::Running || t.status == ToolStatus::Pending {
                                t.status = ToolStatus::Error;
                                t.error = Some("aborted".to_string());
                                self.emit(HarnessEvent::ToolEnd {
                                    session_id: session.id.clone(),
                                    message_id: assistant_id.to_string(),
                                    tool_id: t.id.clone(),
                                    name: t.name.clone(),
                                    status: ToolStatus::Error,
                                    title: String::new(),
                                    output_preview: "aborted".to_string(),
                                    diff: None,
                                    parent_session_id: None,
                                });
                            }
                        }
                    }
                }
                break;
            }
            let joined = tokio::select! {
                biased;
                _ = async {
                    loop {
                        if ctx.abort.is_aborted() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                } => {
                    continue;
                }
                j = join_set.join_next() => j,
            };
            let Some(joined) = joined else { break };
            let (tool_id, name, result) = match joined {
                Ok(tuple) => tuple,
                Err(e) => {
                    if e.is_cancelled() {
                        continue;
                    }
                    if let Some(msg) = session.messages.iter_mut().find(|m| m.id == *assistant_id) {
                        for p in &mut msg.parts {
                            if let Part::Tool(t) = p {
                                if t.status == ToolStatus::Running {
                                    t.status = ToolStatus::Error;
                                    t.error = Some(format!("tool task failed: {}", e));
                                }
                            }
                        }
                    }
                    continue;
                }
            };

            match result {
                Ok(r) => {
                    tracing::debug!("tool call completed: {} (session={})", name, session.id);
                    if let Some(msg) = session.messages.iter_mut().find(|m| m.id == *assistant_id) {
                        if let Some(t) = found_tool(msg, &tool_id) {
                            t.status = ToolStatus::Completed;
                            t.output = r.output;
                            t.title = if r.title.is_empty() {
                                name.clone()
                            } else {
                                r.title
                            };
                            t.error = None;
                        }
                    }
                    self.emit(HarnessEvent::ToolEnd {
                        session_id: session.id.clone(),
                        message_id: assistant_id.to_string(),
                        tool_id: tool_id.clone(),
                        name: name.clone(),
                        status: ToolStatus::Completed,
                        title: name.clone(),
                        output_preview: String::new(),
                        diff: r
                            .metadata
                            .get("diff")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        parent_session_id: None,
                    });
                }
                Err(e) => {
                    tracing::warn!("tool call failed: {} (session={}): {}", name, session.id, e);
                    if let Some(msg) = session.messages.iter_mut().find(|m| m.id == *assistant_id) {
                        if let Some(t) = found_tool(msg, &tool_id) {
                            t.status = ToolStatus::Error;
                            t.output = String::new();
                            t.error = Some(e.clone());
                            t.title = name.clone();
                        }
                    }
                    self.emit(HarnessEvent::ToolEnd {
                        session_id: session.id.clone(),
                        message_id: assistant_id.to_string(),
                        tool_id: tool_id.clone(),
                        name,
                        status: ToolStatus::Error,
                        title: String::new(),
                        output_preview: crate::harness::session::preview(&e, 160),
                        diff: None,
                        parent_session_id: None,
                    });
                }
            }
        }

        // Persist tool results.
        if let Some(msg) = session.messages.iter().find(|m| m.id == *assistant_id) {
            let snapshot = msg.clone();
            self.persist(&session.id, &session.cwd, &snapshot);
        }
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

fn found_tool<'a>(msg: &'a mut Message, tool_id: &str) -> Option<&'a mut ToolPart> {
    msg.parts.iter_mut().find_map(|p| match p {
        Part::Tool(t) if t.id == tool_id => Some(t),
        _ => None,
    })
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
        let mut session = store
            .create_session("build", &dir.path().to_path_buf())
            .unwrap();
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
        // The watchdog now restarts the turn (up to 3 continuations); each
        // segment lasts the configured 1s before the next restart/stop.
        assert_eq!(outcome.continuations, 3);
        assert!(!outcome.aborted);
        assert!(
            outcome.final_text.contains("time limit"),
            "unexpected final_text: {}",
            outcome.final_text
        );
    }

    /// Provider that always ends the assistant message with exactly one tool
    /// call (never a final answer). Simulates a long TODO-list run.
    struct ToolCallProvider;

    #[async_trait::async_trait]
    impl Provider for ToolCallProvider {
        fn name(&self) -> &str {
            "toolcall"
        }
        async fn stream(&self, _req: &LlmRequest) -> anyhow::Result<ProviderStream> {
            let evs: Vec<anyhow::Result<ProviderEvent>> = vec![
                Ok(ProviderEvent::ToolCallStart {
                    id: "t".to_string(),
                    name: "read".into(),
                }),
                Ok(ProviderEvent::ToolCallEnd {
                    id: "t".to_string(),
                    arguments: r#"{"path":"x"}"#.to_string(),
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
            provider: StdArc::new(ToolCallProvider),
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
        let mut session = store
            .create_session("build", &dir.path().to_path_buf())
            .unwrap();
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
        // One tool call per iteration + 3 automatic continuations.
        assert_eq!(outcome.continuations, 3);
        assert_eq!(outcome.iterations, 4);
        assert!(
            outcome.final_text.contains("after 3 continuation(s)"),
            "unexpected final_text: {}",
            outcome.final_text
        );
        // The auto-continue notes were persisted with the session history.
        let notes = session
            .messages
            .iter()
            .filter(|m| m.role.as_str() == "user" && m.text_content().contains("[auto-continue"))
            .count();
        assert_eq!(notes, 3);
    }
}
