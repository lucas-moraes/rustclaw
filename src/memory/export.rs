//! Memory export and import functionality.
//!
//! Provides export of memories and conversations to various formats.

use crate::error::{AgentError, MemoryError};
use crate::memory::store::MemoryStore;
use crate::memory::MemoryEntry;
use chrono::{DateTime, Utc};
use std::path::Path;
use std::result::Result;

pub struct MemoryExporter<'a> {
    memory_store: &'a MemoryStore,
}

impl<'a> MemoryExporter<'a> {
    pub fn new(memory_store: &'a MemoryStore) -> Self {
        Self { memory_store }
    }

    pub fn export_to_markdown(&self, path: &Path) -> Result<usize, AgentError> {
        let memories = self
            .memory_store
            .get_all()
            .map_err(|e| MemoryError::StorageFailed(e.to_string()))?;

        let mut content = String::new();
        content.push_str("# RustClaw Memory Export\n\n");
        content.push_str(&format!(
            "Exported: {}\n\n",
            Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        ));
        content.push_str(&format!("Total entries: {}\n\n", memories.len()));
        content.push_str("---\n\n");

        for memory in &memories {
            content.push_str(&format!("## {}\n\n", memory.id));
            content.push_str(&format!("**Type:** {:?}\n\n", memory.memory_type));
            content.push_str(&format!("**Importance:** {:.2}\n\n", memory.importance));
            content.push_str(&format!(
                "**Created:** {}\n\n",
                memory.timestamp.format("%Y-%m-%d %H:%M:%S")
            ));

            if let Some(ref session_id) = memory.session_id {
                content.push_str(&format!("**Session:** {}\n\n", session_id));
            }

            content.push_str("### Content\n\n");
            content.push_str(memory.content.trim());
            content.push_str("\n\n---\n\n");
        }

        std::fs::write(path, &content)
            .map_err(|e| MemoryError::StorageFailed(format!("Failed to write file: {}", e)))?;

        Ok(memories.len())
    }

    pub fn export_to_json(&self, path: &Path) -> Result<usize, AgentError> {
        let memories = self
            .memory_store
            .get_all()
            .map_err(|e| MemoryError::StorageFailed(e.to_string()))?;

        let export_data = ExportData {
            exported_at: Utc::now(),
            total_entries: memories.len(),
            memories: memories
                .iter()
                .map(|m| MemoryExportItem {
                    id: m.id.clone(),
                    content: m.content.clone(),
                    memory_type: format!("{:?}", m.memory_type),
                    importance: m.importance,
                    timestamp: m.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
                    session_id: m.session_id.clone(),
                    metadata: m.metadata.clone(),
                })
                .collect(),
        };

        let json = serde_json::to_string_pretty(&export_data)
            .map_err(|e| MemoryError::StorageFailed(format!("JSON error: {}", e)))?;

        std::fs::write(path, json)
            .map_err(|e| MemoryError::StorageFailed(format!("Failed to write file: {}", e)))?;

        Ok(memories.len())
    }

    pub fn export_session_to_markdown(
        &self,
        session_id: &str,
        path: &Path,
    ) -> Result<usize, AgentError> {
        let all_memories = self
            .memory_store
            .get_all()
            .map_err(|e| MemoryError::StorageFailed(e.to_string()))?;

        let session_memories: Vec<&MemoryEntry> = all_memories
            .iter()
            .filter(|m| m.session_id.as_deref() == Some(session_id))
            .collect();

        if session_memories.is_empty() {
            return Err(MemoryError::NotFound(format!(
                "No memories found for session: {}",
                session_id
            ))
            .into());
        }

        let mut content = String::new();
        content.push_str(&format!("# Session: {}\n\n", session_id));
        content.push_str(&format!(
            "Exported: {}\n\n",
            Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        ));
        content.push_str(&format!("Total entries: {}\n\n", session_memories.len()));
        content.push_str("---\n\n");

        for memory in &session_memories {
            content.push_str(&format!("## {} ({:?})\n\n", memory.id, memory.memory_type));
            content.push_str(&format!(
                "**Created:** {}\n\n",
                memory.timestamp.format("%Y-%m-%d %H:%M:%S")
            ));
            content.push_str(memory.content.trim());
            content.push_str("\n\n---\n\n");
        }

        std::fs::write(path, &content)
            .map_err(|e| MemoryError::StorageFailed(format!("Failed to write file: {}", e)))?;

        Ok(session_memories.len())
    }
}

#[derive(serde::Serialize)]
struct ExportData {
    exported_at: DateTime<Utc>,
    total_entries: usize,
    memories: Vec<MemoryExportItem>,
}

#[derive(serde::Serialize)]
struct MemoryExportItem {
    id: String,
    content: String,
    memory_type: String,
    importance: f32,
    timestamp: String,
    session_id: Option<String>,
    metadata: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_export_to_json() {
        let dir = tempdir().unwrap();
        let store = MemoryStore::new(&dir.path().join("test.db")).unwrap();
        let exporter = MemoryExporter::new(&store);

        let export_path = dir.path().join("export.json");
        let result = exporter.export_to_json(&export_path);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
        assert!(export_path.exists());
    }

    #[test]
    fn test_export_to_markdown() {
        let dir = tempdir().unwrap();
        let store = MemoryStore::new(&dir.path().join("test.db")).unwrap();
        let exporter = MemoryExporter::new(&store);

        let export_path = dir.path().join("export.md");
        let result = exporter.export_to_markdown(&export_path);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
        assert!(export_path.exists());

        let content = std::fs::read_to_string(&export_path).unwrap();
        assert!(content.contains("# RustClaw Memory Export"));
    }

    #[test]
    fn test_export_session_not_found() {
        let dir = tempdir().unwrap();
        let store = MemoryStore::new(&dir.path().join("test.db")).unwrap();
        let exporter = MemoryExporter::new(&store);

        let export_path = dir.path().join("session.md");
        let result = exporter.export_session_to_markdown("nonexistent", &export_path);

        assert!(result.is_err());
    }
}
