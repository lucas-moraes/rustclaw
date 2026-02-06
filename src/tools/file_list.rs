use super::Tool;
use serde_json::Value;
use std::fs;
use std::path::Path;

pub struct FileListTool;

impl FileListTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for FileListTool {
    fn name(&self) -> &str {
        "file_list"
    }

    fn description(&self) -> &str {
        "Lista diretório. Input: { \"path\": \".\", \"show_hidden\": false } (show_hidden opcional)"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let path_str = args["path"].as_str().unwrap_or(".");
        let show_hidden = args["show_hidden"].as_bool().unwrap_or(false);

        let path = Path::new(path_str);

        if !path.exists() {
            return Err(format!("Diretório não existe: {}", path_str));
        }

        if !path.is_dir() {
            return Err(format!("Caminho não é um diretório: {}", path_str));
        }

        let entries: Vec<_> = fs::read_dir(path)
            .map_err(|e| format!("Erro ao ler diretório: {}", e))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                if show_hidden {
                    true
                } else {
                    entry
                        .file_name()
                        .to_str()
                        .map(|name| !name.starts_with('.'))
                        .unwrap_or(true)
                }
            })
            .collect();

        if entries.is_empty() {
            return Ok(format!("Diretório vazio: {}", path_str));
        }

        let mut dirs: Vec<_> = entries
            .iter()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();

        let mut files: Vec<_> = entries
            .iter()
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let size = e.metadata().ok()?.len();
                Some((name, size))
            })
            .collect();

        dirs.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        files.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

        let mut output = format!("📁 {} ({} itens)\n\n", path_str, entries.len());

        if !dirs.is_empty() {
            output.push_str("Diretórios:\n");
            for dir in dirs {
                output.push_str(&format!("  [DIR] {}\n", dir));
            }
            output.push('\n');
        }

        if !files.is_empty() {
            output.push_str("Arquivos:\n");
            for (name, size) in files {
                output.push_str(&format!("  {} ({} bytes)\n", name, size));
            }
        }

        Ok(output.trim().to_string())
    }
}

impl Default for FileListTool {
    fn default() -> Self {
        Self::new()
    }
}
