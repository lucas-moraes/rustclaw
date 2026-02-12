use crate::tools::Tool;
use serde_json::Value;
use std::path::Path;

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
        "Limpa a memória de conversas. Input: { \"confirm\": true }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let confirm = args["confirm"].as_bool().unwrap_or(false);

        if !confirm {
            return Ok("Para limpar a memória, use: clear_memory com {\"confirm\": true}".to_string());
        }

        let path = std::path::Path::new(&self.memory_path);
        
        if !path.exists() {
            return Ok("Não há memória para limpar".to_string());
        }

        match tokio::fs::remove_file(&self.memory_path).await {
            Ok(_) => {
                Ok("Memória limpa com sucesso".to_string())
            }
            Err(e) => {
                Err(format!("Erro ao limpar memória: {}", e))
            }
        }
    }
}
