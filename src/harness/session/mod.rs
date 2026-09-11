//! Canonical harness types: Session, Message, Part.
//!
//! These are the source of truth for a conversation. Provider adapters convert
//! them to/from OpenAI or Anthropic wire formats.

pub mod compaction;
pub mod doom_loop;
pub mod image;
pub mod processor;
pub mod store;
pub mod tool_exec;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

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
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    Tool(ToolPart),
    /// Image attached by the user (`/image <path>`); converted to a vision
    /// content block by providers that support it (Anthropic, OpenAI).
    Image {
        path: String,
    },
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

    pub fn image(path: impl Into<String>) -> Self {
        Part::Image { path: path.into() }
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

    /// Builds a System-role message. Only used by tests.
    #[cfg(test)]
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

    /// Whether this message carries any image attachment.
    pub fn has_image(&self) -> bool {
        self.parts.iter().any(|p| matches!(p, Part::Image { .. }))
    }

    /// Returns a copy of this message with all `Part::Image` parts removed
    /// (used to degrade a vision request to text when the provider rejects it).
    pub fn without_images(&self) -> Message {
        let mut m = self.clone();
        m.parts.retain(|p| !matches!(p, Part::Image { .. }));
        m
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
    /// Cached `Arc<Vec<Message>>` snapshot of `messages`, invalidated on any
    /// mutation. Lets the processor build `LlmRequest.messages` (an
    /// `Arc<Vec<Message>>`) with a cheap refcount clone instead of a deep
    /// `Vec<Message>` clone every iteration — O(n²) over a long turn.
    #[serde(skip)]
    #[allow(dead_code)] // read via `messages_arc()`; kept private
    pub(crate) messages_arc: Option<Arc<Vec<Message>>>,
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
            messages_arc: None,
            todos: Vec::new(),
            skills: Vec::new(),
            title: None,
        }
    }

    /// Display title: custom title if set, else first user-visible prompt
    /// (skips runtime-injected `<project-memory>` parts).
    pub fn display_title(&self) -> String {
        if let Some(t) = &self.title {
            let t = t.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
        self.messages
            .iter()
            .filter(|m| m.role.as_str() == "user")
            .find_map(|m| {
                m.parts.iter().find_map(|p| {
                    let raw = p.as_text()?;
                    let cleaned = crate::harness::project::memory::strip_memory_blocks(raw);
                    let cleaned = cleaned.replace('\n', " ").trim().to_string();
                    if cleaned.is_empty() {
                        None
                    } else {
                        Some(cleaned)
                    }
                })
            })
            .unwrap_or_else(|| "untitled".to_string())
    }

    pub fn push_message(&mut self, msg: Message) {
        self.messages.push(msg);
        self.messages_arc = None; // invalidate cache
        self.updated_at = Utc::now();
    }

    /// Cheap `Arc` snapshot of the message history. Reuses a cached snapshot
    /// when the history hasn't changed since the last call, so the processor
    /// can build `LlmRequest.messages` without deep-cloning the whole history
    /// on every iteration.
    pub fn messages_arc(&mut self) -> Arc<Vec<Message>> {
        if let Some(arc) = &self.messages_arc {
            return arc.clone();
        }
        let arc = Arc::new(self.messages.clone());
        self.messages_arc = Some(arc.clone());
        arc
    }

    /// Invalidates the cached snapshot after a direct mutation of `messages`
    /// (e.g. `truncate`/`pop` outside `push_message`).
    pub fn invalidate_messages_cache(&mut self) {
        self.messages_arc = None;
    }

    /// Last message in the session, if any. Public convenience; kept for API
    /// completeness (not currently consumed by the UI/processor).
    #[allow(dead_code)]
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
                Part::Image { .. } => {}
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
    fn test_without_images_strips_only_images() {
        let msg = Message::new(
            Role::User,
            vec![
                Part::text("describe"),
                Part::image("/tmp/a.png"),
                Part::text("please"),
            ],
        );
        assert!(msg.has_image());
        let stripped = msg.without_images();
        assert!(!stripped.has_image());
        assert_eq!(stripped.text_content(), "describe\nplease");
        // Original is untouched.
        assert!(msg.has_image());
    }

    #[test]
    fn test_has_image_false_when_no_image() {
        let msg = Message::user("hello");
        assert!(!msg.has_image());
    }

    #[test]
    fn test_part_image_serde_roundtrip() {
        let part = Part::image("/tmp/pic.png");
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("\"image\""), "json: {}", json);
        assert!(json.contains("/tmp/pic.png"));
        let back: Part = serde_json::from_str(&json).unwrap();
        match back {
            Part::Image { path } => assert_eq!(path, "/tmp/pic.png"),
            _ => panic!("expected image part"),
        }
    }

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
    fn test_display_title_skips_project_memory() {
        use crate::harness::project::memory::{MEMORY_BLOCK_END, MEMORY_BLOCK_START};
        let mut session = Session::new("build", PathBuf::from("/tmp"));
        let mut msg = Message::user("real user question");
        // Runtime injects memory as a leading text part.
        msg.parts.insert(
            0,
            Part::text(format!(
                "{MEMORY_BLOCK_START}\n- [x] fact\n{MEMORY_BLOCK_END}"
            )),
        );
        session.push_message(msg);
        assert_eq!(session.display_title(), "real user question");
        // Combined single part (memory prefix + prompt).
        let mut session2 = Session::new("build", PathBuf::from("/tmp"));
        session2.push_message(Message::user(format!(
            "{MEMORY_BLOCK_START}\n- fact\n{MEMORY_BLOCK_END}\n\nfix the sidebar"
        )));
        assert_eq!(session2.display_title(), "fix the sidebar");
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
    fn test_messages_arc_cache_invalidates_on_push() {
        let mut session = Session::new("build", PathBuf::from("/tmp"));
        session.push_message(Message::user("first"));
        let a1 = session.messages_arc();
        // Same snapshot reused (no deep clone) while history is unchanged.
        let a2 = session.messages_arc();
        assert!(Arc::ptr_eq(&a1, &a2), "cache should be reused");
        assert_eq!(a1.len(), 1);

        // A push invalidates the cache and produces a fresh snapshot.
        session.push_message(Message::user("second"));
        let b = session.messages_arc();
        assert!(!Arc::ptr_eq(&a1, &b), "cache must be invalidated on push");
        assert_eq!(b.len(), 2);

        // Direct mutation + explicit invalidation also refreshes.
        session.messages.truncate(1);
        session.invalidate_messages_cache();
        let c = session.messages_arc();
        assert_eq!(c.len(), 1);
        assert!(!Arc::ptr_eq(&b, &c));
    }

    #[test]
    fn test_messages_arc_not_serialized() {
        let mut session = Session::new("build", PathBuf::from("/tmp"));
        session.push_message(Message::user("hi"));
        let _ = session.messages_arc(); // populate cache
        let json = serde_json::to_string(&session).unwrap();
        assert!(
            !json.contains("messages_arc"),
            "cache must not be serialized"
        );
        let back: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(back.messages.len(), 1);
        assert!(back.messages_arc.is_none());
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
