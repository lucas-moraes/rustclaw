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
                    depth: ctx.depth,
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

    // R6: capture the pre-iteration state of every file a *mutable* tool call
    // is about to touch, so the whole batch can be rolled back if it fails
    // entirely. Conservative: only files targeted by write/edit participate.
    let iteration_snapshot = IterationSnapshot::capture(&pending, ctx.cwd.path());

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
                                depth: ctx.depth,
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
                    depth: ctx.depth,
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
                    depth: ctx.depth,
                });
            }
        }
    }

    // R6: if the batch touched files but *every* mutable tool call failed,
    // roll the files back to their pre-iteration state. A single successful
    // mutation disables the rollback (never revert good work).
    if let Some(msg) = session.messages.iter().find(|m| m.id == *assistant_id) {
        let mutable: Vec<&ToolPart> = msg
            .tool_parts()
            .into_iter()
            .filter(|t| is_mutable_tool(&t.name))
            .collect();
        let any_succeeded = mutable.iter().any(|t| t.status == ToolStatus::Completed);
        if !mutable.is_empty() && !any_succeeded {
            let restored = iteration_snapshot.rollback();
            if !restored.is_empty() {
                tracing::warn!(
                    "iteration rollback: all {} mutable tool call(s) failed; restored {} file(s) \
                     (session={})",
                    mutable.len(),
                    restored.len(),
                    session.id
                );
                processor.emit(HarnessEvent::Rollback {
                    session_id: session.id.clone(),
                    paths: restored.clone(),
                    parent_session_id: None,
                });
                let note = Message::user(format!(
                    "System note: every file edit in the last step failed, so the touched \
                     file(s) were rolled back to their previous state: {}.",
                    restored.join(", ")
                ));
                session.push_message(note.clone());
                processor.persist(&session.id, &session.cwd, &note).await;
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

/// Tools that mutate files on disk (participate in iteration rollback).
fn is_mutable_tool(name: &str) -> bool {
    matches!(name, "write" | "edit")
}

/// The path a mutable tool call targets, resolved against `cwd`.
fn mutable_target_path(
    name: &str,
    input: &serde_json::Value,
    cwd: &std::path::Path,
) -> Option<std::path::PathBuf> {
    if !is_mutable_tool(name) {
        return None;
    }
    let raw = input.get("path").and_then(|v| v.as_str())?;
    let p = std::path::Path::new(raw);
    Some(if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    })
}

/// Pre-iteration snapshot of the files a batch of mutable tool calls will
/// touch. Restoring writes back the captured content (or deletes a file that
/// did not exist), independent of the session-wide `FileCheckpoints` (which
/// keeps the *pre-agent* state for `/restore`).
struct IterationSnapshot {
    /// path -> prior content (`None` = file did not exist).
    files: Vec<(std::path::PathBuf, Option<String>)>,
}

impl IterationSnapshot {
    fn capture(pending: &[(String, String, serde_json::Value)], cwd: &std::path::Path) -> Self {
        let mut files = Vec::new();
        for (_id, name, input) in pending {
            let Some(path) = mutable_target_path(name, input, cwd) else {
                continue;
            };
            if files.iter().any(|(p, _)| p == &path) {
                continue;
            }
            let prior = std::fs::read_to_string(&path).ok();
            files.push((path, prior));
        }
        Self { files }
    }

    /// Restores every captured file. Returns the paths actually restored.
    fn rollback(&self) -> Vec<String> {
        let mut restored = Vec::new();
        for (path, prior) in &self.files {
            let ok = match prior {
                Some(content) => std::fs::write(path, content).is_ok(),
                None => match std::fs::remove_file(path) {
                    Ok(()) => true,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
                    Err(_) => false,
                },
            };
            if ok {
                restored.push(path.display().to_string());
            }
        }
        restored
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pending(name: &str, input: serde_json::Value) -> Vec<(String, String, serde_json::Value)> {
        vec![("t1".to_string(), name.to_string(), input)]
    }

    #[test]
    fn test_mutable_target_path_resolves_relative() {
        let cwd = std::path::Path::new("/work");
        let p = mutable_target_path("write", &json!({"path": "a.rs"}), cwd).unwrap();
        assert_eq!(p, std::path::PathBuf::from("/work/a.rs"));
        let abs = mutable_target_path("edit", &json!({"path": "/tmp/b.rs"}), cwd).unwrap();
        assert_eq!(abs, std::path::PathBuf::from("/tmp/b.rs"));
        // Non-mutable tools and missing path → None.
        assert!(mutable_target_path("read", &json!({"path": "a.rs"}), cwd).is_none());
        assert!(mutable_target_path("write", &json!({}), cwd).is_none());
    }

    #[test]
    fn test_rollback_restores_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "original").unwrap();

        let snap = IterationSnapshot::capture(
            &pending("write", json!({"path": file.to_str().unwrap()})),
            dir.path(),
        );
        // Simulate a failed edit that still changed the file.
        std::fs::write(&file, "broken").unwrap();

        let restored = snap.rollback();
        assert_eq!(restored.len(), 1);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");
    }

    #[test]
    fn test_rollback_deletes_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new.rs");
        // File does not exist at capture time.
        let snap = IterationSnapshot::capture(
            &pending("write", json!({"path": file.to_str().unwrap()})),
            dir.path(),
        );
        // A failed write created it anyway.
        std::fs::write(&file, "created").unwrap();

        let restored = snap.rollback();
        assert_eq!(restored.len(), 1);
        assert!(!file.exists(), "new file must be deleted on rollback");
    }

    #[test]
    fn test_capture_ignores_non_mutable_tools() {
        let dir = tempfile::tempdir().unwrap();
        let snap =
            IterationSnapshot::capture(&pending("read", json!({"path": "a.rs"})), dir.path());
        assert!(snap.files.is_empty());
        assert!(snap.rollback().is_empty());
    }

    #[test]
    fn test_capture_dedups_same_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "v1").unwrap();
        let pending = vec![
            (
                "t1".to_string(),
                "write".to_string(),
                json!({"path": file.to_str().unwrap()}),
            ),
            (
                "t2".to_string(),
                "edit".to_string(),
                json!({"path": file.to_str().unwrap()}),
            ),
        ];
        let snap = IterationSnapshot::capture(&pending, dir.path());
        assert_eq!(snap.files.len(), 1, "same path captured once");
    }
}
