//! Session processor: the core agent loop.
//!
//! Streams LLM responses with native tool calls, executes tools (in parallel
//! when a single assistant message contains multiple calls), persists parts,
//! detects doom loops, and compacts context on overflow.

use crate::harness::agent::AgentSpec;
use crate::harness::event::{EventSender, HarnessEvent};
use crate::harness::provider::{LlmRequest, Provider, ProviderEvent, ToolSpec, Usage};
use crate::harness::session::compaction::{self, CompactionConfig};
use crate::harness::session::{Message, Part, Role, Session, ToolPart};
use crate::harness::tool::registry::ToolRegistry;
use futures_util::StreamExt;
use std::sync::Arc;

/// Tunables for one turn of the processor.
#[derive(Clone, Debug)]
pub struct ProcessorConfig {
    pub model: String,
    pub max_iterations: usize,
    pub max_context_tokens: usize,
    // Temperature is agent-calibrated: see AgentSpec::turn_temperature.
}

impl Default for ProcessorConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            max_iterations: 50,
            max_context_tokens: 100_000,
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
    pub usage: Usage,
    pub aborted: bool,
}

const DOOM_LOOP_WARN: usize = 3;
const DOOM_LOOP_STOP: usize = 5;
const COMPACTION_KEEP_RECENT: usize = 6;
const COMPACTION_MIN_MESSAGES: usize = 10;

impl SessionProcessor {
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

        while iterations < self.config.max_iterations {
            if ctx.abort.is_aborted() {
                aborted = true;
                break;
            }
            iterations += 1;

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

            let mut stream = self.provider.stream(&req).await?;
            let assistant_id = crate::harness::session::new_id();
            let mut text = String::new();
            let mut reasoning = String::new();
            let mut tool_calls: Vec<ToolPart> = Vec::new();
            let mut usage = Usage::default();

            while let Some(ev) = stream.next().await {
                // Abort responsively mid-stream.
                if ctx.abort.is_aborted() {
                    break;
                }
                match ev? {
                    ProviderEvent::TextDelta(d) => {
                        text.push_str(&d);
                        let _ = self.events.send(HarnessEvent::TextDelta {
                            session_id: session.id.clone(),
                            message_id: assistant_id.to_string(),
                            delta: d,
                        });
                    }
                    ProviderEvent::ReasoningDelta(d) => {
                        reasoning.push_str(&d);
                        let _ = self.events.send(HarnessEvent::ReasoningDelta {
                            session_id: session.id.clone(),
                            message_id: assistant_id.to_string(),
                            delta: d,
                        });
                    }
                    ProviderEvent::ToolCallStart { id, name } => {
                        tool_calls.push(ToolPart::pending(id, name, serde_json::Value::Null));
                        let _ = self.events.send(HarnessEvent::MessageUpdated {
                            session_id: session.id.clone(),
                            message_id: assistant_id.to_string(),
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
                final_text =
                    "Stopped: the same tool call was repeated many times without progress."
                        .to_string();
                let _ = self.events.send(HarnessEvent::Error {
                    session_id: session.id.clone(),
                    message: final_text.clone(),
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
                let _ = self.store.save_message(&session.id, &session.cwd, &warn);
            }
        }

        if iterations >= self.config.max_iterations && final_text.is_empty() {
            final_text = "Stopped: reached the maximum number of iterations.".to_string();
        }
        if ctx.abort.is_aborted() && final_text.is_empty() {
            final_text = "Run aborted by user.".to_string();
        }

        Ok(TurnOutcome {
            final_text,
            iterations,
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
                    let _ = self.events.send(HarnessEvent::ToolStart {
                        session_id: session.id.clone(),
                        message_id: assistant_id.to_string(),
                        tool_id: t.id.clone(),
                        name: t.name.clone(),
                        input: t.input.clone(),
                    });
                    (t.id.clone(), t.name.clone(), t.input.clone())
                })
                .collect()
        };
        if let Some(msg) = session.messages.iter().find(|m| m.id == *assistant_id) {
            let snapshot = msg.clone();
            let _ = self
                .store
                .save_message(&session.id, &session.cwd, &snapshot);
        }

        // Spawn executions (permission-checked, then run concurrently).
        let mut join_set = tokio::task::JoinSet::new();
        for (tool_id, name, input) in pending {
            let registry = self.registry.clone();
            let mut ctx2 = ctx.clone();
            ctx2.session_id = session.id.clone();
            join_set.spawn(async move {
                let result = match ctx2.check_permission(&name, &input).await {
                    Ok(()) => registry.execute(&name, input, &ctx2).await,
                    Err(e) => Err(e),
                };
                (tool_id, name, result)
            });
        }

        // Apply results in completion order.
        while let Some(joined) = join_set.join_next().await {
            let (tool_id, name, result) = match joined {
                Ok(tuple) => tuple,
                Err(e) => {
                    // JoinSet item panicked: mark any running call as error.
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
                    let _ = self.events.send(HarnessEvent::ToolEnd {
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
                    });
                }
                Err(e) => {
                    if let Some(msg) = session.messages.iter_mut().find(|m| m.id == *assistant_id) {
                        if let Some(t) = found_tool(msg, &tool_id) {
                            t.status = ToolStatus::Error;
                            t.output = String::new();
                            t.error = Some(e.clone());
                            t.title = name.clone();
                        }
                    }
                    let _ = self.events.send(HarnessEvent::ToolEnd {
                        session_id: session.id.clone(),
                        message_id: assistant_id.to_string(),
                        tool_id: tool_id.clone(),
                        name,
                        status: ToolStatus::Error,
                        title: String::new(),
                        output_preview: crate::harness::session::preview(&e, 160),
                        diff: None,
                    });
                }
            }
        }

        // Persist tool results.
        if let Some(msg) = session.messages.iter().find(|m| m.id == *assistant_id) {
            let snapshot = msg.clone();
            let _ = self
                .store
                .save_message(&session.id, &session.cwd, &snapshot);
        }
    }

    async fn maybe_compact(&self, session: &mut Session) -> anyhow::Result<()> {
        let _ = self.events.send(HarnessEvent::CompactionStarted {
            session_id: session.id.clone(),
        });

        let config = CompactionConfig {
            max_context_tokens: self.config.max_context_tokens,
            keep_recent_messages: COMPACTION_KEEP_RECENT,
            min_messages_to_compact: COMPACTION_MIN_MESSAGES,
        };
        let before = session.messages.len();
        if let Some(new_messages) = compaction::should_compact_and_execute(
            &session.messages,
            self.provider.clone(),
            &config,
        )
        .await?
        {
            // The summary message adds one to the new list, so the number of
            // messages summarized away is before - new.len() + 1.
            let summarized = before.saturating_sub(new_messages.len()) + 1;
            session.messages = new_messages;
            session.updated_at = chrono::Utc::now();
            let _ = self.events.send(HarnessEvent::CompactionFinished {
                session_id: session.id.clone(),
                summarized_messages: summarized,
            });
        }
        Ok(())
    }
}

fn found_tool<'a>(msg: &'a mut Message, tool_id: &str) -> Option<&'a mut ToolPart> {
    msg.parts.iter_mut().find_map(|p| match p {
        Part::Tool(t) if t.id == tool_id => Some(t),
        _ => None,
    })
}
