use super::Tool;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;

pub struct FileWriteTool;

impl FileWriteTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Escreve ou cria arquivo. Input: { \"path\": \"/tmp/test.txt\", \"content\": \"hello\", \"append\": false } (append opcional)"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let path_str = args["path"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'path' é obrigatório".to_string())?;

        let content = args["content"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'content' é obrigatório".to_string())?;

        let append = args["append"].as_bool().unwrap_or(false);

        let path = Path::new(path_str);

        
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Erro ao criar diretórios: {}", e))?;
            }
        }

        let mut file = if append {
            fs::OpenOptions::new()
                .write(true)
                .append(true)
                .create(true)
                .open(path)
        } else {
            fs::File::create(path)
        }
        .map_err(|e| format!("Erro ao abrir/criar arquivo: {}", e))?;

        file.write_all(content.as_bytes())
            .map_err(|e| format!("Erro ao escrever arquivo: {}", e))?;

        let action = if append { "Atualizado" } else { "Criado" };
        let bytes = content.len();

        Ok(format!("{}: {} ({} bytes)", action, path_str, bytes))
    }
}

impl Default for FileWriteTool {
    fn default() -> Self {
        Self::new()
    }
}
