//! Tool execution for the session processor.
//!
//! Extracted from `processor.rs`: runs the pending tool calls of an assistant
//! message concurrently (JoinSet), permission-checked, with panic isolation
//! so one broken tool does not contaminate the rest of the batch.

use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures_util::FutureExt as _;

use crate::harness::event::{HarnessEvent, ToolStatus};
use crate::harness::session::processor::SessionProcessor;
use crate::harness::session::{Message, Part, Session, ToolPart};
use crate::harness::tool::context::ToolContext;

/// Executes all pending tool calls of the assistant message `assistant_id`,
/// applying results back onto the session in completion order. Aborts the
/// remaining batch if the shared abort signal fires mid-execution.
pub async fn execute_tool_calls(
    processor: &SessionProcessor,
    session: &mut Session,
    assistant_id: &str,
    ctx: &ToolContext,
) {
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
                processor.emit(HarnessEvent::ToolStart {
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
        processor
            .persist(&session.id, &session.cwd, &snapshot)
            .await;
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
        let registry = processor.registry.clone();
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
                        match crate::harness::hooks::run_pre_tool(
                            &ctx2.hooks,
                            &name,
                            &input,
                            ctx2.cwd.path(),
                        )
                        .await
                        {
                            Ok(()) => {
                                // Catch panics so one broken tool does
                                // not contaminate the rest of the batch
                                // (JoinSet would otherwise lose the
                                // tool_id and mark every Running tool
                                // as Error).
                                let fut_name = name.clone();
                                let fut = async move {
                                    let out =
                                        registry.execute(&fut_name, input.clone(), &ctx2).await;
                                    crate::harness::hooks::spawn_post_tool(
                                        &ctx2.hooks,
                                        &fut_name,
                                        &input,
                                        ctx2.cwd.path(),
                                    );
                                    out
                                };
                                match tokio::task::spawn(async move {
                                    #[cfg(panic = "unwind")]
                                    {
                                        AssertUnwindSafe(fut).catch_unwind().await.unwrap_or_else(
                                            |p| {
                                                Err(format!("tool panicked: {}", panic_message(&p)))
                                            },
                                        )
                                    }
                                    #[cfg(not(panic = "unwind"))]
                                    {
                                        fut.await
                                    }
                                })
                                .await
                                {
                                    Ok(r) => r,
                                    Err(e) if e.is_cancelled() => Err("aborted".to_string()),
                                    Err(e) => Err(format!("tool task failed: {}", e)),
                                }
                            }
                            Err(e) => Err(e),
                        }
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
                            processor.emit(HarnessEvent::ToolEnd {
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
                processor.emit(HarnessEvent::ToolEnd {
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
                processor.emit(HarnessEvent::ToolEnd {
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
        processor
            .persist(&session.id, &session.cwd, &snapshot)
            .await;
    }
}

fn found_tool<'a>(msg: &'a mut Message, tool_id: &str) -> Option<&'a mut ToolPart> {
    msg.parts.iter_mut().find_map(|p| match p {
        Part::Tool(t) if t.id == tool_id => Some(t),
        _ => None,
    })
}

fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}
