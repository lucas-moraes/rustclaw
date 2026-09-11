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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::event::event_channel;
    use crate::harness::provider::{LlmRequest, LlmResponse};
    use crate::harness::session::processor::ProcessorConfig;
    use crate::harness::session::store::SessionStore;
    use crate::harness::tool::context::AbortSignal;
    use crate::harness::tool::registry::ToolRegistry;
    use std::sync::Arc as StdArc;

    /// Minimal processor for driving `consume_stream` directly.
    fn test_processor() -> SessionProcessor {
        let dir = tempfile::tempdir().unwrap();
        let store = StdArc::new(SessionStore::open(&dir.path().join("test.db")).unwrap());
        let (tx, _rx) = event_channel();
        SessionProcessor {
            provider: StdArc::new(StalledProvider),
            registry: ToolRegistry::builder().build(),
            events: tx,
            store,
            config: ProcessorConfig {
                model: "m".into(),
                max_iterations: 10,
                max_context_tokens: 100_000,
                turn_timeout_secs: 60,
            },
        }
    }

    struct StalledProvider;
    #[async_trait::async_trait]
    impl crate::harness::provider::Provider for StalledProvider {
        fn name(&self) -> &str {
            "stalled"
        }
        async fn stream(&self, _req: &LlmRequest) -> anyhow::Result<ProviderStream> {
            Ok(futures_util::stream::pending::<anyhow::Result<ProviderEvent>>().boxed())
        }
        async fn complete(&self, _req: &LlmRequest) -> anyhow::Result<LlmResponse> {
            unreachable!()
        }
    }

    fn session() -> Session {
        Session::new("build", std::path::PathBuf::from("/tmp"))
    }

    fn stream_of(events: Vec<ProviderEvent>) -> ProviderStream {
        futures_util::stream::iter(events.into_iter().map(Ok)).boxed()
    }

    #[tokio::test]
    async fn test_clean_finish_accumulates_text_and_tools() {
        let p = test_processor();
        let s = session();
        let abort = AbortSignal::new();
        let stream = stream_of(vec![
            ProviderEvent::TextDelta("Hello ".into()),
            ProviderEvent::TextDelta("world".into()),
            ProviderEvent::ToolCallStart {
                id: "t1".into(),
                name: "bash".into(),
            },
            ProviderEvent::ToolCallEnd {
                id: "t1".into(),
                arguments: r#"{"command":"ls"}"#.into(),
            },
            ProviderEvent::End {
                stop_reason: None,
                usage: Some(Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                }),
            },
        ]);
        let out = consume_stream(
            &p,
            &s,
            stream,
            tokio::time::Instant::now() + Duration::from_secs(60),
            60,
            &abort,
        )
        .await;
        assert_eq!(out.text, "Hello world");
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].name, "bash");
        assert_eq!(
            out.tool_calls[0].input,
            serde_json::json!({"command": "ls"})
        );
        assert_eq!(out.usage.input_tokens, 10);
        assert_eq!(out.usage.output_tokens, 5);
        assert!(out.stop_reason.is_none());
        assert!(!out.aborted);
    }

    #[tokio::test]
    async fn test_abort_mid_stream() {
        let p = test_processor();
        let s = session();
        let abort = AbortSignal::new();
        // Stream that yields one delta then stays pending forever.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tx.send(Ok(ProviderEvent::TextDelta("partial".into())))
            .unwrap();
        let stream =
            futures_util::stream::unfold(
                rx,
                |mut rx| async move { rx.recv().await.map(|ev| (ev, rx)) },
            )
            .boxed();
        let abort_for_task = abort.clone();
        let handle = tokio::spawn(async move {
            consume_stream(
                &p,
                &s,
                stream,
                tokio::time::Instant::now() + Duration::from_secs(60),
                60,
                &abort_for_task,
            )
            .await
        });
        // Let the first delta be consumed, then abort.
        tokio::time::sleep(Duration::from_millis(50)).await;
        abort.abort();
        let out = handle.await.unwrap();
        assert!(out.aborted, "abort must be reflected in outcome");
        assert_eq!(out.text, "partial");
    }

    #[tokio::test]
    async fn test_stream_error_sets_stop_reason() {
        let p = test_processor();
        let s = session();
        let abort = AbortSignal::new();
        let stream = futures_util::stream::iter(vec![
            Ok(ProviderEvent::TextDelta("before".into())),
            Err(anyhow::anyhow!("connection reset")),
        ])
        .boxed();
        let out = consume_stream(
            &p,
            &s,
            stream,
            tokio::time::Instant::now() + Duration::from_secs(60),
            60,
            &abort,
        )
        .await;
        assert_eq!(out.text, "before");
        let reason = out.stop_reason.expect("stop_reason set on stream error");
        assert!(reason.contains("stream error"), "reason: {reason}");
        assert!(!out.aborted);
    }

    #[tokio::test]
    async fn test_tool_call_end_without_start_recovers() {
        let p = test_processor();
        let s = session();
        let abort = AbortSignal::new();
        let stream = stream_of(vec![
            ProviderEvent::ToolCallEnd {
                id: "orphan".into(),
                arguments: r#"{"x":1}"#.into(),
            },
            ProviderEvent::End {
                stop_reason: None,
                usage: None,
            },
        ]);
        let out = consume_stream(
            &p,
            &s,
            stream,
            tokio::time::Instant::now() + Duration::from_secs(60),
            60,
            &abort,
        )
        .await;
        assert_eq!(
            out.tool_calls.len(),
            1,
            "orphan ToolCallEnd must be recovered"
        );
        assert_eq!(out.tool_calls[0].id, "orphan");
        assert_eq!(out.tool_calls[0].input, serde_json::json!({"x": 1}));
    }

    #[tokio::test]
    async fn test_invalid_tool_arguments_marks_error() {
        let p = test_processor();
        let s = session();
        let abort = AbortSignal::new();
        let stream = stream_of(vec![
            ProviderEvent::ToolCallStart {
                id: "t1".into(),
                name: "bash".into(),
            },
            ProviderEvent::ToolCallEnd {
                id: "t1".into(),
                arguments: "not json".into(),
            },
            ProviderEvent::End {
                stop_reason: None,
                usage: None,
            },
        ]);
        let out = consume_stream(
            &p,
            &s,
            stream,
            tokio::time::Instant::now() + Duration::from_secs(60),
            60,
            &abort,
        )
        .await;
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(
            out.tool_calls[0].status,
            crate::harness::event::ToolStatus::Error
        );
        assert!(out.tool_calls[0].error.is_some());
    }
}
