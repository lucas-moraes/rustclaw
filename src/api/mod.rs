//! REST API server for programmatic access.
//!
//! Provides HTTP endpoints for agent interaction.

use crate::agent::Agent;
use crate::config::Config;
use crate::tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub response: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

pub struct ApiServer {
    agent: Arc<RwLock<Option<Agent>>>,
    config: Config,
}

impl ApiServer {
    pub fn new(config: Config) -> Self {
        Self {
            agent: Arc::new(RwLock::new(None)),
            config,
        }
    }

    pub async fn init_agent(&self, tools: ToolRegistry) -> Result<(), String> {
        let memory_path = std::path::PathBuf::from("config/memory_api.db");
        let agent = Agent::new(self.config.clone(), tools, &memory_path)
            .map_err(|e| e.to_string())?;
        
        let mut agent_handle = self.agent.write().await;
        *agent_handle = Some(agent);
        Ok(())
    }

    pub async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, String> {
        let mut agent_guard = self.agent.write().await;
        
        if let Some(ref mut agent) = *agent_guard {
            let response = agent.prompt(&request.message)
                .await
                .map_err(|e| e.to_string())?;
            
            Ok(ChatResponse {
                response,
                session_id: request.session_id,
            })
        } else {
            Err("Agent not initialized".to_string())
        }
    }

    pub fn health(&self) -> HealthResponse {
        HealthResponse {
            status: "ok".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_response() {
        let health = HealthResponse {
            status: "ok".to_string(),
            version: "1.0.0".to_string(),
        };
        
        assert_eq!(health.status, "ok");
        assert_eq!(health.version, "1.0.0");
    }
}
