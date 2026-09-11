//! Stream consumption for one assistant attempt inside a processor turn.
//!
//! Races provider events against the abort signal and the turn watchdog,
//! accumulating text/reasoning/tool calls and emitting UI events. Extracted
//! from `SessionProcessor::run_turn` — behavior is identical.

use std::time::Duration;

use futures_util::StreamExt;

use super::{SessionProcessor, STREAM_TIMEOUT_SECS};
use crate::harness::event::HarnessEvent;
use crate::harness::provider::{ProviderEvent, ProviderStream, Usage};
use crate::harness::session::{Session, ToolPart};

/// Accumulated result of consuming one provider stream to completion (or to
/// an abort/timeout/stream error — in which case `stop_reason` is set).
pub(crate) struct StreamOutcome {
    pub assistant_id: String,
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolPart>,
    pub usage: Usage,
    /// Set when the attempt ended early due to the turn watchdog, a stalled
    /// stream or a transient stream error. `None` on a clean finish.
    pub stop_reason: Option<String>,
    /// The user abort signal fired mid-stream.
    pub aborted: bool,
}

/// Consumes one provider stream attempt, accumulating the assistant message
/// pieces and emitting delta events. Mirrors the inline block that used to
/// live inside `SessionProcessor::run_turn`.
///
/// `stop_reason`/`aborted` are the turn-level flags from `run_turn`: this
/// function only *sets* them (never clears) and the caller decides what to
/// do (restart vs. stop) afterwards.
pub(crate) async fn consume_stream(
    processor: &SessionProcessor,
    session: &Session,
    mut stream: ProviderStream,
    turn_deadline: tokio::time::Instant,
    turn_secs: u64,
    abort: &crate::harness::tool::context::AbortSignal,
) -> StreamOutcome {
    let assistant_id = crate::harness::session::new_id();
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<ToolPart> = Vec::new();
    let mut usage = Usage::default();
    let mut stop_reason: Option<String> = None;
    let mut aborted = false;

    loop {
        // Abort responsively mid-stream (Esc / Ctrl+C cancel).
        if abort.is_aborted() {
            aborted = true;
            break;
        }
        // Race the next stream event against a short abort poll so a
        // stuck/slow provider doesn't ignore Esc until the next token.
        let next = tokio::select! {
            biased;
            _ = async {
                loop {
                    if abort.is_aborted() {
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
                stop_reason = Some(format!("stream timed out after {}s", STREAM_TIMEOUT_SECS));
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
                processor.emit(HarnessEvent::TextDelta {
                    session_id: session.id.clone(),
                    message_id: assistant_id.to_string(),
                    delta: d,
                    parent_session_id: None,
                });
            }
            ProviderEvent::ReasoningDelta(d) => {
                reasoning.push_str(&d);
                processor.emit(HarnessEvent::ReasoningDelta {
                    session_id: session.id.clone(),
                    message_id: assistant_id.to_string(),
                    delta: d,
                    parent_session_id: None,
                });
            }
            ProviderEvent::ToolCallStart { id, name } => {
                tool_calls.push(ToolPart::pending(id, name, serde_json::Value::Null));
                processor.emit(HarnessEvent::MessageUpdated {
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

    StreamOutcome {
        assistant_id,
        text,
        reasoning,
        tool_calls,
        usage,
        stop_reason,
        aborted,
    }
}
