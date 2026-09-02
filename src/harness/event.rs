//! Harness event bus: surfaces (CLI/TUI) consume these events while a prompt runs.
#![allow(dead_code)] // events are the API surface consumed by the UI

use crate::harness::permission::PermissionRequest;
use serde_json::Value;

/// Tool execution status, mirrors `ToolPart::status`.
pub use crate::harness::session::ToolStatus;

#[derive(Clone, Debug)]
pub enum HarnessEvent {
    RunStarted {
        session_id: String,
    },
    RunFinished {
        session_id: String,
    },
    /// User message accepted.
    UserMessage {
        session_id: String,
        message_id: String,
    },
    /// Streaming text delta for the in-flight assistant message.
    TextDelta {
        session_id: String,
        message_id: String,
        delta: String,
    },
    /// Streaming reasoning delta for the in-flight assistant message.
    ReasoningDelta {
        session_id: String,
        message_id: String,
        delta: String,
    },
    /// Assistant message finalized/persisted (text, reasoning or tool parts).
    MessageUpdated {
        session_id: String,
        message_id: String,
    },
    ToolStart {
        session_id: String,
        message_id: String,
        tool_id: String,
        name: String,
        input: Value,
    },
    ToolEnd {
        session_id: String,
        message_id: String,
        tool_id: String,
        name: String,
        status: ToolStatus,
        title: String,
        output_preview: String,
        /// Optional unified diff from tools that mutate files (edit/write).
        diff: Option<String>,
    },
    PermissionAsk {
        request: PermissionRequest,
    },
    PermissionResolved {
        id: String,
        allowed: bool,
    },
    CompactionStarted {
        session_id: String,
    },
    CompactionFinished {
        session_id: String,
        summarized_messages: usize,
    },
    Error {
        session_id: String,
        message: String,
    },
}

impl HarnessEvent {
    pub fn session_id(&self) -> Option<&str> {
        match self {
            HarnessEvent::RunStarted { session_id }
            | HarnessEvent::RunFinished { session_id }
            | HarnessEvent::UserMessage { session_id, .. }
            | HarnessEvent::TextDelta { session_id, .. }
            | HarnessEvent::ReasoningDelta { session_id, .. }
            | HarnessEvent::MessageUpdated { session_id, .. }
            | HarnessEvent::ToolStart { session_id, .. }
            | HarnessEvent::ToolEnd { session_id, .. }
            | HarnessEvent::CompactionStarted { session_id }
            | HarnessEvent::CompactionFinished { session_id, .. }
            | HarnessEvent::Error { session_id, .. } => Some(session_id),
            HarnessEvent::PermissionAsk { request } => Some(&request.session_id),
            HarnessEvent::PermissionResolved { .. } => None,
        }
    }
}

/// Unbounded sender half used by the runtime; consumers own the receiver.
pub type EventSender = tokio::sync::mpsc::UnboundedSender<HarnessEvent>;
pub type EventReceiver = tokio::sync::mpsc::UnboundedReceiver<HarnessEvent>;

/// Creates a connected (sender, receiver) pair for event fan-out.
pub fn event_channel() -> (EventSender, EventReceiver) {
    tokio::sync::mpsc::unbounded_channel()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_channel_roundtrip() {
        let (tx, mut rx) = event_channel();
        let event = HarnessEvent::RunStarted {
            session_id: "s1".to_string(),
        };
        tx.send(event).unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received.session_id(), Some("s1"));
    }

    #[tokio::test]
    async fn test_multiple_events_preserve_order() {
        let (tx, mut rx) = event_channel();
        tx.send(HarnessEvent::RunStarted {
            session_id: "a".into(),
        })
        .unwrap();
        tx.send(HarnessEvent::RunFinished {
            session_id: "a".into(),
        })
        .unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(HarnessEvent::RunStarted { .. })
        ));
        assert!(matches!(
            rx.recv().await,
            Some(HarnessEvent::RunFinished { .. })
        ));
    }
}
