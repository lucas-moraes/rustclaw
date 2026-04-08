use async_trait::async_trait;
use serde_json::Value;
use std::fs;
use std::path::Path;

use super::Tool;

pub struct FileEditTool;

impl FileEditTool {
    pub fn new() -> Self {
        Self
    }

    pub fn validate_path(path: &Path) -> Result<(), String> {
        let path_str = path.to_string_lossy();

        if path_str.contains("/etc/")
            || path_str.contains("/usr/")
            || path_str.contains("/bin/")
            || path_str.contains("/sbin/")
            || path_str.contains("/boot/")
            || path_str.contains("/sys/")
            || path_str.contains("/proc/")
        {
            return Err(format!(
                "Acesso negado: caminho de sistema '{}' não permitido",
                path.display()
            ));
        }

        Ok(())
    }
}

impl Default for FileEditTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FileEditTool {
    fn name(&self) -> &str {
        "file_edit"
    }

    fn description(&self) -> &str {
        r#"Edita um arquivo substituindo texto exato. Útil para mudanças cirúrgicas sem reescrever o arquivo inteiro.
Exemplo: {"path": "src/main.rs", "old_str": "println!(\"Hello\");", "new_str": "println!(\"Hello, world!\");", "expected_replacements": 1}
Parâmetros:
- path (string, obrigatório): caminho do arquivo
- old_str (string, obrigatório): texto exato a ser substituído
- new_str (string, obrigatório): texto de substituição
- expected_replacements (número, opcional, padrão=1): número esperado de ocorrências; falha se diferente (proteção contra substituições indesejadas)"#
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'path' é obrigatório".to_string())?;

        let old_str = args["old_str"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'old_str' é obrigatório".to_string())?;

        let new_str = args["new_str"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'new_str' é obrigatório".to_string())?;

        let expected_replacements = args["expected_replacements"].as_u64().unwrap_or(1) as usize;

        let path_obj = Path::new(path);
        Self::validate_path(path_obj)?;

        if !path_obj.exists() {
            return Err(format!("Arquivo não existe: {}", path));
        }

        if !path_obj.is_file() {
            return Err(format!("Caminho não é um arquivo: {}", path));
        }

        let content = fs::read_to_string(path)
            .map_err(|e| format!("Erro ao ler arquivo '{}': {}", path, e))?;

        let occurrences = content.matches(old_str).count();

        if occurrences == 0 {
            return Err(format!(
                "old_str não encontrado no arquivo '{}'. Nenhuma ocorrência de:\n{}",
                path, old_str
            ));
        }

        if occurrences != expected_replacements {
            return Err(format!(
                "old_str encontrado {} vez(es) em '{}', mas esperado {} (use expected_replacements para confirmar múltiplas substituições)",
                occurrences, path, expected_replacements
            ));
        }

        let new_content = content.replace(old_str, new_str);

        fs::write(path, new_content.as_bytes())
            .map_err(|e| format!("Erro ao escrever arquivo '{}': {}", path, e))?;

        Ok(format!(
            "✅ Editado: {} ({} substituição(ões) de {} bytes → {} bytes)",
            path,
            occurrences,
            old_str.len(),
            new_str.len()
        ))
    }
}
