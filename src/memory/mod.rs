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
    pub content: String,
    pub embedding: Vec<f32>,
    pub timestamp: DateTime<Utc>,
    pub importance: f32,
    pub memory_type: MemoryType,
    pub metadata: serde_json::Value,
    pub search_count: i32,
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

impl MemoryEntry {
    pub fn new(
        content: String,
        embedding: Vec<f32>,
        memory_type: MemoryType,
        importance: f32,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            content,
            embedding,
            timestamp: Utc::now(),
            importance: importance.clamp(0.0, 1.0),
            memory_type,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            search_count: 0,
        }
    }

    #[allow(dead_code)]
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}
