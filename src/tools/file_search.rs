use super::Tool;
use glob::glob;
use serde_json::Value;
use std::fs;
use std::path::Path;

const DEFAULT_MAX_DEPTH: usize = 5;
const MAX_RESULTS: usize = 50;

pub struct FileSearchTool;

impl FileSearchTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for FileSearchTool {
    fn name(&self) -> &str {
        "file_search"
    }

    fn description(&self) -> &str {
        "Busca arquivos por nome ou conteúdo. Input: { \"path\": \".\", \"pattern\": \"*.rs\", \"content\": \"fn main\", \"max_depth\": 5 }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let path_str = args["path"].as_str().unwrap_or(".");
        let pattern = args["pattern"].as_str();
        let content = args["content"].as_str();
        let max_depth = args["max_depth"]
            .as_u64()
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_MAX_DEPTH);

        let path = Path::new(path_str);

        if !path.exists() {
            return Err(format!("Diretório não existe: {}", path_str));
        }

        if !path.is_dir() {
            return Err(format!("Caminho não é um diretório: {}", path_str));
        }

        let mut results = Vec::new();

        // Busca por nome usando glob
        if let Some(pattern) = pattern {
            let glob_pattern = format!("{}/{}", path_str, pattern);

            for entry in glob(&glob_pattern).map_err(|e| format!("Pattern inválido: {}", e))? {
                if let Ok(path) = entry {
                    // Verificar profundidade
                    let depth = path
                        .components()
                        .count()
                        .saturating_sub(path.components().count().min(3));

                    if depth <= max_depth {
                        // Se tem busca por conteúdo
                        if let Some(search_content) = content {
                            if let Ok(file_content) = fs::read_to_string(&path) {
                                if file_content.contains(search_content) {
                                    results.push(path.to_string_lossy().to_string());
                                }
                            }
                        } else {
                            results.push(path.to_string_lossy().to_string());
                        }

                        if results.len() >= MAX_RESULTS {
                            break;
                        }
                    }
                }
            }
        }
        // Busca apenas por conteúdo (recursiva)
        else if let Some(search_content) = content {
            for entry in walkdir::WalkDir::new(path)
                .max_depth(max_depth)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                if let Ok(file_content) = fs::read_to_string(entry.path()) {
                    if file_content.contains(search_content) {
                        results.push(entry.path().to_string_lossy().to_string());
                        if results.len() >= MAX_RESULTS {
                            break;
                        }
                    }
                }
            }
        }

        if results.is_empty() {
            return Ok("Nenhum arquivo encontrado".to_string());
        }

        let mut output = format!("Encontrados {} arquivos:\n\n", results.len());
        for result in &results {
            output.push_str(&format!("  {}\n", result));
        }

        if results.len() >= MAX_RESULTS {
            output.push_str("\n[Máximo de resultados atingido]");
        }

        Ok(output.trim().to_string())
    }
}

impl Default for FileSearchTool {
    fn default() -> Self {
        Self::new()
    }
}
