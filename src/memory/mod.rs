pub mod checkpoint;
pub mod embeddings;
pub mod reminder;
pub mod search;
pub mod skill_context;
pub mod store;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub session_id: Option<String>,
    pub content: String,
    pub embedding: Vec<f32>,
    pub timestamp: DateTime<Utc>,
    pub importance: f32,
    pub memory_type: MemoryType,
    pub metadata: serde_json::Value,
    pub search_count: i32,
    pub scope: MemoryScope,
    pub access_count: i32,
    pub last_accessed: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum MemoryScope {
    #[default]
    Session,
    Project,
    Global,
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryScope::Session => write!(f, "session"),
            MemoryScope::Project => write!(f, "project"),
            MemoryScope::Global => write!(f, "global"),
        }
    }
}

impl From<&str> for MemoryScope {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "session" => MemoryScope::Session,
            "project" => MemoryScope::Project,
            "global" => MemoryScope::Global,
            _ => MemoryScope::Session,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MemoryType {
    Fact,
    Episode,
    ToolResult,
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryType::Fact => write!(f, "fact"),
            MemoryType::Episode => write!(f, "episode"),
            MemoryType::ToolResult => write!(f, "tool_result"),
        }
    }
}

impl From<&str> for MemoryType {
    fn from(s: &str) -> Self {
        match s {
            "fact" => MemoryType::Fact,
            "episode" => MemoryType::Episode,
            "tool_result" => MemoryType::ToolResult,
            _ => MemoryType::Episode,
        }
    }
}

impl MemoryEntry {
    pub fn new(
        content: String,
        embedding: Vec<f32>,
        memory_type: MemoryType,
        importance: f32,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            session_id: None,
            content,
            embedding,
            timestamp: Utc::now(),
            importance: importance.clamp(0.0, 1.0),
            memory_type,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            search_count: 0,
            scope: MemoryScope::Session,
            access_count: 0,
            last_accessed: Utc::now(),
        }
    }

    #[allow(dead_code)]
    pub fn with_session(mut self, session_id: String) -> Self {
        self.session_id = Some(session_id);
        self
    }
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_scope(mut self, scope: MemoryScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn with_project_context(mut self, project_path: String) -> Self {
        self.scope = MemoryScope::Project;
        if let serde_json::Value::Object(ref mut map) = self.metadata {
            map.insert("project_path".to_string(), serde_json::json!(project_path));
        }
        self
    }

    pub fn to_global(mut self) -> Self {
        self.scope = MemoryScope::Global;
        self
    }

    pub fn calculate_importance(&self) -> f32 {
        let base = self.importance;
        let access_factor = (self.access_count as f32 * 0.05).min(0.3);
        let recency = {
            let hours_old = (Utc::now() - self.timestamp).num_hours() as f32;
            let decay = (-hours_old / 720.0).exp();
            decay * 0.2
        };
        (base + access_factor + recency).clamp(0.0, 1.0)
    }

    pub fn touch(&mut self) {
        self.access_count += 1;
        self.last_accessed = Utc::now();
        self.importance = self.calculate_importance();
    }
}
