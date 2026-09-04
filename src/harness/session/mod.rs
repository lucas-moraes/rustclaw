//! Canonical harness types: Session, Message, Part.
//!
//! These are the source of truth for a conversation. Provider adapters convert
//! them to/from OpenAI or Anthropic wire formats.

pub mod compaction;
pub mod processor;
pub mod store;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Pending,
    Running,
    Completed,
    Error,
}

impl std::fmt::Display for ToolStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolStatus::Pending => write!(f, "pending"),
            ToolStatus::Running => write!(f, "running"),
            ToolStatus::Completed => write!(f, "completed"),
            ToolStatus::Error => write!(f, "error"),
        }
    }
}

/// A single tool call/result inside an assistant message.
/// Input is stored when the model emits the call; output/error fill in after execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolPart {
    pub id: String,
    pub name: String,
    pub input: Value,
    pub status: ToolStatus,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub error: Option<String>,
}

impl ToolPart {
    pub fn pending(id: impl Into<String>, name: impl Into<String>, input: Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            input,
            status: ToolStatus::Pending,
            output: String::new(),
            title: String::new(),
            error: None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.status, ToolStatus::Completed | ToolStatus::Error)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text { text: String },
    Reasoning { text: String },
    Tool(ToolPart),
}

impl Part {
    pub fn text(s: impl Into<String>) -> Self {
        Part::Text { text: s.into() }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Part::Text { text } => Some(text),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub parts: Vec<Part>,
    pub created_at: DateTime<Utc>,
}

impl Message {
    pub fn new(role: Role, parts: Vec<Part>) -> Self {
        Self {
            id: new_id(),
            role,
            parts,
            created_at: Utc::now(),
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self::new(Role::User, vec![Part::text(text)])
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self::new(Role::System, vec![Part::text(text)])
    }

    pub fn with_id(id: impl Into<String>, role: Role, parts: Vec<Part>) -> Self {
        Self {
            id: id.into(),
            role,
            parts,
            created_at: Utc::now(),
        }
    }

    /// Concatenated text of all Text parts.
    pub fn text_content(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| p.as_text())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Tool call parts contained in this message.
    pub fn tool_parts(&self) -> Vec<&ToolPart> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Tool(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    pub fn has_tool_calls(&self) -> bool {
        !self.tool_parts().is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub agent: String,
    pub cwd: PathBuf,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub todos: Vec<TodoItem>,
    /// Skills (this session's "memory"). Chosen at session start, editable.
    #[serde(default)]
    pub skills: Vec<crate::harness::skill::SessionSkill>,
    /// Optional user-defined title (set via /sessions rename). Falls back to
    /// the first user message when empty/None.
    #[serde(default)]
    pub title: Option<String>,
}

impl Session {
    pub fn new(agent: impl Into<String>, cwd: PathBuf) -> Self {
        let now = Utc::now();
        Self {
            id: new_id(),
            agent: agent.into(),
            cwd,
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            todos: Vec::new(),
            skills: Vec::new(),
            title: None,
        }
    }

    /// Display title: custom title if set, else first user message preview.
    pub fn display_title(&self) -> String {
        if let Some(t) = &self.title {
            let t = t.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
        self.messages
            .iter()
            .find(|m| m.role.as_str() == "user")
            .and_then(|m| m.parts.iter().find_map(|p| p.as_text().map(str::to_string)))
            .map(|s| s.replace('\n', " ").trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "untitled".to_string())
    }

    pub fn push_message(&mut self, msg: Message) {
        self.messages.push(msg);
        self.updated_at = Utc::now();
    }

    /// Last message in the session, if any.
    pub fn last_message(&self) -> Option<&Message> {
        self.messages.last()
    }

    /// Estimate of context size (rough chars/4 token heuristic).
    pub fn approx_tokens(&self) -> usize {
        approx_tokens(&self.messages)
    }
}

/// Estimate of context size for a slice of messages (rough chars/4 heuristic).
/// Shared by `Session::approx_tokens` and the compaction module.
pub fn approx_tokens(messages: &[Message]) -> usize {
    let mut chars = 0usize;
    for m in messages {
        for p in &m.parts {
            match p {
                Part::Text { text } | Part::Reasoning { text } => chars += text.len(),
                Part::Tool(t) => {
                    chars += t.name.len() + t.output.len() + t.input.to_string().len();
                }
            }
        }
    }
    chars / 4
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// A todo item managed by the `todo` tools, persisted per session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

impl std::fmt::Display for TodoStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TodoStatus::Pending => write!(f, "pending"),
            TodoStatus::InProgress => write!(f, "in_progress"),
            TodoStatus::Completed => write!(f, "completed"),
            TodoStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// Short preview of a string for event/UI display.
pub fn preview(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{}…", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_serialization_roundtrip() {
        let msg = Message::new(
            Role::Assistant,
            vec![
                Part::text("hello"),
                Part::Reasoning {
                    text: "thinking".into(),
                },
                Part::Tool(ToolPart {
                    id: "tc1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "ls"}),
                    status: ToolStatus::Completed,
                    output: "file.txt".into(),
                    title: "ls".into(),
                    error: None,
                }),
            ],
        );
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role, Role::Assistant);
        assert_eq!(back.parts.len(), 3);
        assert_eq!(back.text_content(), "hello");
    }

    #[test]
    fn test_session_token_estimate() {
        let mut session = Session::new("build", PathBuf::from("/tmp"));
        session.push_message(Message::user("a".repeat(400)));
        assert_eq!(session.approx_tokens(), 100);
    }

    #[test]
    fn test_display_title_prefers_custom() {
        let mut session = Session::new("build", PathBuf::from("/tmp"));
        assert_eq!(session.display_title(), "untitled");
        session.push_message(Message::user("first prompt here"));
        assert_eq!(session.display_title(), "first prompt here");
        session.title = Some("  Meu título  ".into());
        assert_eq!(session.display_title(), "Meu título");
        session.title = Some("   ".into());
        assert_eq!(session.display_title(), "first prompt here");
    }

    #[test]
    fn test_preview_truncates() {
        assert_eq!(preview("hello", 10), "hello");
        let long = "x".repeat(50);
        let p = preview(&long, 10);
        assert!(p.ends_with('…'));
        assert_eq!(p.chars().count(), 11);
    }

    #[test]
    fn test_tool_part_pending_helper() {
        let tp = ToolPart::pending("id1", "bash", serde_json::json!({"command": "ls"}));
        assert_eq!(tp.status, ToolStatus::Pending);
        assert!(!tp.is_terminal());
        let mut done = tp.clone();
        done.status = ToolStatus::Completed;
        assert!(done.is_terminal());
    }
}
