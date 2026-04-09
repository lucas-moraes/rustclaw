#![allow(dead_code)]

use crate::memory::checkpoint::{CheckpointStore, DevelopmentCheckpoint, SessionSummary};
use crate::memory::embeddings::EmbeddingService;
use crate::memory::store::MemoryStore;
use crate::memory::{MemoryEntry, MemoryType};

pub struct SessionManager<'a> {
    checkpoint_store: &'a CheckpointStore,
    memory_store: &'a MemoryStore,
    embedding_service: &'a EmbeddingService,
}

impl<'a> SessionManager<'a> {
    pub fn new(
        checkpoint_store: &'a CheckpointStore,
        memory_store: &'a MemoryStore,
        embedding_service: &'a EmbeddingService,
    ) -> Self {
        Self {
            checkpoint_store,
            memory_store,
            embedding_service,
        }
    }

    pub fn list_sessions(&self) -> anyhow::Result<Vec<DevelopmentCheckpoint>> {
        self.checkpoint_store
            .list_all(50)
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    pub fn list_session_summaries(&self) -> anyhow::Result<Vec<SessionSummary>> {
        self.checkpoint_store
            .list_session_summaries(50)
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    pub fn list_sessions_with_hierarchy(&self) -> anyhow::Result<Vec<(SessionSummary, usize)>> {
        let sessions = self
            .checkpoint_store
            .list_session_summaries(100)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let mut result = Vec::new();
        let mut session_map: std::collections::HashMap<String, &SessionSummary> =
            std::collections::HashMap::new();

        for session in &sessions {
            session_map.insert(session.session_id.clone(), session);
        }

        fn get_depth(
            session: &SessionSummary,
            session_map: &std::collections::HashMap<String, &SessionSummary>,
        ) -> usize {
            let mut depth = 0;
            let mut current = session;
            while let Some(ref parent_id) = current.parent_id {
                if let Some(parent) = session_map.get(parent_id) {
                    depth += 1;
                    current = parent;
                } else {
                    break;
                }
            }
            depth
        }

        for session in &sessions {
            let depth = get_depth(session, &session_map);
            result.push((session.clone(), depth));
        }

        result.sort_by(|a, b| {
            let depth_cmp = a.1.cmp(&b.1);
            if depth_cmp == std::cmp::Ordering::Equal {
                b.0.updated_at.cmp(&a.0.updated_at)
            } else {
                depth_cmp
            }
        });

        Ok(result)
    }

    pub fn get_session_details(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Option<crate::agent::SessionDetails>> {
        if let Ok(Some(cp)) = self.checkpoint_store.get(session_id) {
            return Ok(Some(crate::agent::SessionDetails {
                id: cp.id.clone(),
                user_input: cp.user_input.clone(),
                phase: format!("{:?}", cp.phase),
                state: format!("{:?}", cp.state),
                plan_text: cp.plan_text.clone(),
                project_dir: cp.project_dir.clone(),
                message_count: cp.completed_tools_json.len(),
                created_at: cp.created_at,
            }));
        }

        if let Ok(Some(cp)) = self.checkpoint_store.find_by_id_prefix(session_id) {
            return Ok(Some(crate::agent::SessionDetails {
                id: cp.id.clone(),
                user_input: cp.user_input.clone(),
                phase: format!("{:?}", cp.phase),
                state: format!("{:?}", cp.state),
                plan_text: cp.plan_text.clone(),
                project_dir: cp.project_dir.clone(),
                message_count: cp.completed_tools_json.len(),
                created_at: cp.created_at,
            }));
        }

        Ok(None)
    }

    pub async fn save_conversation_to_memory(
        &self,
        user_input: &str,
        assistant_response: &str,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        if user_input.len() < 10 {
            return Ok(());
        }

        let content = format!(
            "Usuário: {}\nAssistente: {}",
            user_input, assistant_response
        );

        let embedding = self.embedding_service.embed(&content).await?;

        let memory = if let Some(sid) = session_id {
            MemoryEntry::new(content, embedding, MemoryType::Episode, 0.6)
                .with_session(sid.to_string())
        } else {
            MemoryEntry::new(content, embedding, MemoryType::Episode, 0.6)
        };

        self.memory_store.save(&memory)?;
        tracing::info!("Saved conversation to long-term memory");

        Ok(())
    }

    pub async fn save_tool_result_to_memory(
        &self,
        tool_name: &str,
        input: &str,
        output: &str,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        if output.starts_with("Erro:") || output.len() > 1000 {
            return Ok(());
        }

        let content = format!("Tool: {}\nInput: {}\nOutput: {}", tool_name, input, output);

        let embedding = self.embedding_service.embed(&content).await?;

        let memory = if let Some(sid) = session_id {
            MemoryEntry::new(content, embedding, MemoryType::ToolResult, 0.3)
                .with_session(sid.to_string())
        } else {
            MemoryEntry::new(content, embedding, MemoryType::ToolResult, 0.3)
        };

        self.memory_store.save(&memory)?;

        Ok(())
    }
}

pub struct SessionCommands;

impl SessionCommands {
    pub async fn resume_session(
        checkpoint_store: &CheckpointStore,
        session_id: &str,
    ) -> anyhow::Result<(DevelopmentCheckpoint, String)> {
        let checkpoint = if let Ok(Some(cp)) = checkpoint_store.get(session_id) {
            cp
        } else if let Ok(Some(cp)) = checkpoint_store.find_by_id_prefix(session_id) {
            cp
        } else {
            return Err(anyhow::anyhow!("Session not found: {}", session_id));
        };

        let session_name = checkpoint
            .session_name
            .clone()
            .unwrap_or_else(|| session_id.to_string());

        Ok((checkpoint, session_name))
    }

    pub async fn delete_session(
        checkpoint_store: &CheckpointStore,
        session_id: &str,
    ) -> Result<String, String> {
        tracing::debug!("delete_session called with: {}", session_id);

        if let Err(e) = checkpoint_store.delete_session_summary(session_id) {
            tracing::warn!("Failed to delete session summary: {}", e);
        }

        if let Ok(Some(cp)) = checkpoint_store.get(session_id) {
            if let Err(e) = checkpoint_store.delete(&cp.id) {
                tracing::warn!("Failed to delete checkpoint: {}", e);
            }
            return Ok(format!("Session '{}' deleted successfully", session_id));
        }

        if let Ok(Some(cp)) = checkpoint_store.find_by_id_prefix(session_id) {
            if let Err(e) = checkpoint_store.delete(&cp.id) {
                tracing::warn!("Failed to delete checkpoint: {}", e);
            }
            return Ok(format!(
                "Session '{}' deleted successfully",
                &session_id[..8.min(session_id.len())]
            ));
        }

        Err("Session not found".to_string())
    }

    pub async fn rename_session(
        checkpoint_store: &CheckpointStore,
        session_id: &str,
        new_name: &str,
    ) -> Result<String, String> {
        if let Ok(Some(mut cp)) = checkpoint_store.get(session_id) {
            cp.session_name = Some(new_name.to_string());
            checkpoint_store
                .save(&cp)
                .map_err(|e| format!("Erro ao renomear: {}", e))?;
            return Ok(format!("Sessão renomeada para: {}", new_name));
        }

        if let Ok(Some(mut cp)) = checkpoint_store.find_by_id_prefix(session_id) {
            cp.session_name = Some(new_name.to_string());
            checkpoint_store
                .save(&cp)
                .map_err(|e| format!("Erro ao renomear: {}", e))?;
            return Ok(format!("Sessão renomeada para: {}", new_name));
        }

        Err("Sessão não encontrada".to_string())
    }
}
