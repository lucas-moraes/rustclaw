use crate::tools::Tool;
use serde_json::Value;
use std::path::Path;
use tracing::{info, warn};

pub struct ClearMemoryTool {
    memory_path: String,
}

impl ClearMemoryTool {
    pub fn new<P: AsRef<Path>>(memory_path: P) -> Self {
        Self {
            memory_path: memory_path.as_ref().to_string_lossy().to_string(),
        }
    }
}

#[async_trait::async_trait]
impl Tool for ClearMemoryTool {
    fn name(&self) -> &str {
        "clear_memory"
    }

    fn description(&self) -> &str {
        "Limpa a memória de conversas. Input: { \"confirm\": true, \"include_tasks\": false }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let confirm = args["confirm"].as_bool().unwrap_or(false);
        let include_tasks = args["include_tasks"].as_bool().unwrap_or(false);

        if !confirm {
            return Ok("⚠️ Para limpar a memória, use: clear_memory com {\"confirm\": true}\n\
                     ❗ Isso apagará todas as memórias de conversa permanentemente.".to_string());
        }

        info!("Clearing memory at path: {}, include_tasks: {}", self.memory_path, include_tasks);

        let path = std::path::Path::new(&self.memory_path);
        
        if !path.exists() {
            return Ok("🧹 Não há memória para limpar (arquivo não existe)".to_string());
        }


        match tokio::fs::remove_file(&self.memory_path).await {
            Ok(_) => {
                let msg = if include_tasks {
                    "🧹✅ Memória e tarefas limpas com sucesso!\n\
                     📊 O agente será reiniciado sem memórias na próxima mensagem.".to_string()
                } else {
                    "🧹✅ Memória de conversas limpa com sucesso!\n\
                     📊 O agente será reiniciado sem memórias na próxima mensagem.".to_string()
                };
                info!("Memory cleared successfully");
                Ok(msg)
            }
            Err(e) => {
                warn!("Failed to clear memory: {}", e);
                Err(format!("❌ Erro ao limpar memória: {}", e))
            }
        }
    }
}
