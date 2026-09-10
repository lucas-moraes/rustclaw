//! Event recording + replay (`/record`, `/replay`).
//!
//! Design (lowest-friction tee): the event bus is a per-turn unicast mpsc
//! channel consumed by the UI. Instead of restructuring the bus, the UI
//! surfaces forward every event they consume to a shared [`EventRecorder`]
//! (an `Arc<Mutex<…>>` on the runtime-adjacent state) when recording is on.
//! The recorder appends one JSON line per `HarnessEvent` to
//! `rustclaw-events-<session_id8>.jsonl` in the cwd (BufWriter, append).
//!
//! `/replay <file.jsonl>` reads the JSONL back and re-renders the events in
//! the current transcript (TUI: `apply_event`; CLI: the same lines
//! `print_events` would print).

use crate::harness::event::HarnessEvent;
use anyhow::{Context, Result};
use std::io::{BufRead, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Default filename for a session's event recording.
pub fn recording_path(session_id: &str) -> PathBuf {
    PathBuf::from(format!(
        "rustclaw-events-{}.jsonl",
        session_id.get(..8).unwrap_or(session_id)
    ))
}

/// Appends `HarnessEvent`s to a JSONL file. `None` writer = not recording.
pub struct EventRecorder {
    inner: Mutex<Option<RecorderInner>>,
}

struct RecorderInner {
    path: PathBuf,
    writer: BufWriter<std::fs::File>,
    count: usize,
}

impl EventRecorder {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Starts recording (append mode) to `path`. Overwrites nothing; a
    /// previous recording of the same session keeps growing.
    pub fn start(&self, path: PathBuf) -> Result<usize> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        let existing = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let count = if existing == 0 {
            0
        } else {
            // Count pre-existing lines so /record status reports the total.
            std::fs::File::open(&path)
                .and_then(|f| {
                    Ok(std::io::BufReader::new(f).lines().count()) as std::io::Result<usize>
                })
                .unwrap_or(0)
        };
        let mut guard = self.inner.lock().unwrap();
        *guard = Some(RecorderInner {
            path,
            writer: BufWriter::new(file),
            count,
        });
        Ok(count)
    }

    /// Stops recording and flushes; returns (path, events written this session).
    pub fn stop(&self) -> Option<(PathBuf, usize)> {
        let mut guard = self.inner.lock().unwrap();
        let inner = guard.take()?;
        let mut w = inner.writer;
        let _ = w.flush();
        Some((inner.path, inner.count))
    }

    pub fn status(&self) -> Option<(String, usize)> {
        let guard = self.inner.lock().unwrap();
        guard
            .as_ref()
            .map(|i| (i.path.display().to_string(), i.count))
    }

    /// Records one event (no-op when not recording). Errors are swallowed:
    /// recording must never break the UI/event flow.
    pub fn record(&self, event: &HarnessEvent) {
        let mut guard = self.inner.lock().unwrap();
        if let Some(inner) = guard.as_mut() {
            let line = match serde_json::to_string(event) {
                Ok(l) => l,
                Err(_) => return,
            };
            if writeln!(inner.writer, "{}", line).is_ok() {
                inner.count += 1;
            }
        }
    }
}

impl Default for EventRecorder {
    fn default() -> Self {
        Self::new()
    }
}

/// Reads a JSONL file back into events. Malformed lines are skipped (with a
/// count) so a partially-written recording still replays.
pub fn read_events(path: &Path) -> Result<(Vec<HarnessEvent>, usize)> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("recording not found: {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut events = Vec::new();
    let mut skipped = 0usize;
    for line in reader.lines() {
        let line = line.context("failed to read recording line")?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<HarnessEvent>(&line) {
            Ok(ev) => events.push(ev),
            Err(_) => skipped += 1,
        }
    }
    Ok((events, skipped))
}

/// Renders one event as the transcript line(s) the CLI would print — used by
/// `/replay` on the CLI surface and testable in isolation.
pub fn render_event_lines(event: &HarnessEvent) -> Vec<String> {
    match event {
        HarnessEvent::TextDelta { delta, .. } => vec![delta.clone()],
        HarnessEvent::ReasoningDelta { delta, .. } => {
            vec![format!("· thinking: {}", delta.trim())]
        }
        HarnessEvent::MessageUpdated { .. } => vec![],
        HarnessEvent::ToolStart { name, input, .. } => {
            let preview = crate::harness::session::preview(&input.to_string(), 100);
            vec![format!("● {} {}", name, preview)]
        }
        HarnessEvent::ToolEnd {
            name,
            status,
            title,
            ..
        } => {
            let mark = match status {
                crate::harness::session::ToolStatus::Completed => "✓",
                crate::harness::session::ToolStatus::Error => "✗",
                _ => "·",
            };
            let label = if title.is_empty() { name } else { title };
            vec![format!("{} {}", mark, label)]
        }
        HarnessEvent::CompactionStarted { .. } => vec!["[compacting context…]".to_string()],
        HarnessEvent::CompactionFinished {
            summarized_messages,
            ..
        } => vec![format!(
            "[compaction: summarized {} message(s)]",
            summarized_messages
        )],
        HarnessEvent::AutoContinue {
            round,
            total,
            reason,
            ..
        } => vec![format!(
            "[auto-continue {}/{}] {} — resuming…",
            round, total, reason
        )],
        HarnessEvent::Error { message, .. } => vec![format!("[error] {}", message)],
        HarnessEvent::RunStarted { .. } => vec!["working…".to_string()],
        HarnessEvent::RunFinished { .. } => vec![],
        HarnessEvent::UserMessage { .. } => vec![],
        HarnessEvent::JobFinished {
            job_id, exit_code, ..
        } => vec![format!(
            "[background job {} finished · exit {}]",
            job_id,
            exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into())
        )],
        HarnessEvent::PermissionAsk { .. } | HarnessEvent::PermissionResolved { .. } => vec![],
        HarnessEvent::BudgetWarn { message, .. } => vec![format!("[budget] {}", message)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::session::ToolStatus;

    fn sample_events() -> Vec<HarnessEvent> {
        vec![
            HarnessEvent::RunStarted {
                session_id: "sess-12345678".to_string(),
            },
            HarnessEvent::ToolEnd {
                session_id: "sess-12345678".to_string(),
                message_id: "m1".to_string(),
                tool_id: "t1".to_string(),
                name: "bash".to_string(),
                status: ToolStatus::Completed,
                title: "ls src".to_string(),
                output_preview: "main.rs".to_string(),
                diff: None,
                parent_session_id: None,
            },
            HarnessEvent::Error {
                session_id: "sess-12345678".to_string(),
                message: "boom".to_string(),
                parent_session_id: Some("parent-1".to_string()),
            },
        ]
    }

    #[test]
    fn test_record_replay_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let events = sample_events();

        let recorder = EventRecorder::new();
        recorder.start(path.clone()).unwrap();
        for ev in &events {
            recorder.record(ev);
        }
        let (stopped, count) = recorder.stop().unwrap();
        assert_eq!(stopped, path);
        assert_eq!(count, 3);

        let (back, skipped) = read_events(&path).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(back.len(), 3);
        assert_eq!(back, events);
    }

    #[test]
    fn test_read_missing_file_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_events(&dir.path().join("nope.jsonl")).unwrap_err();
        assert!(err.to_string().contains("recording not found"), "{err}");
    }

    #[test]
    fn test_recording_path_uses_session_prefix() {
        let p = recording_path("abcdefgh1234");
        assert_eq!(p.to_string_lossy(), "rustclaw-events-abcdefgh.jsonl");
    }

    #[test]
    fn test_render_event_lines() {
        let ev = HarnessEvent::ToolEnd {
            session_id: "s".into(),
            message_id: "m".into(),
            tool_id: "t".into(),
            name: "bash".into(),
            status: ToolStatus::Error,
            title: String::new(),
            output_preview: String::new(),
            diff: None,
            parent_session_id: None,
        };
        assert_eq!(render_event_lines(&ev), vec!["✗ bash"]);
        assert!(render_event_lines(&HarnessEvent::RunFinished {
            session_id: "s".into()
        })
        .is_empty());
    }

    #[test]
    fn test_record_without_start_is_noop() {
        let recorder = EventRecorder::new();
        recorder.record(&sample_events()[0]);
        assert!(recorder.stop().is_none());
    }
}
