use super::Tool;
use serde_json::Value;
use std::fs;
use std::path::Path;

const DEFAULT_MAX_BYTES: usize = 10_000;
const ABSOLUTE_MAX_BYTES: usize = 1_000_000;

pub struct FileReadTool;

impl FileReadTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Lê conteúdo de arquivo. Input: { \"path\": \"/etc/hosts\", \"max_bytes\": 10000 } (max_bytes opcional, max 1MB)"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let path_str = args["path"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'path' é obrigatório".to_string())?;

        let path = Path::new(path_str);

        if !path.exists() {
            return Err(format!("Arquivo não existe: {}", path_str));
        }

        if !path.is_file() {
            return Err(format!("Caminho não é um arquivo: {}", path_str));
        }

        let max_bytes = args["max_bytes"]
            .as_u64()
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_MAX_BYTES)
            .min(ABSOLUTE_MAX_BYTES);

        let metadata = fs::metadata(path)
            .map_err(|e| format!("Erro ao ler metadados: {}", e))?;

        if metadata.len() > max_bytes as u64 {
            let content = fs::read(path)
                .map_err(|e| format!("Erro ao ler arquivo: {}", e))?;
            let truncated = String::from_utf8_lossy(&content[..max_bytes.min(content.len())]);
            return Ok(format!(
                "{}\n\n[ARQUIVO TRUNCADO - {} bytes lidos de {} total]",
                truncated,
                max_bytes,
                metadata.len()
            ));
        }

        let content = fs::read_to_string(path)
            .map_err(|e| format!("Erro ao ler arquivo: {}", e))?;

        Ok(content)
    }
}

impl Default for FileReadTool {
    fn default() -> Self {
        Self::new()
    }
}
