use super::Tool;
use serde_json::Value;
use std::process::Command;
use std::time::Duration;

const BLOCKED_COMMANDS: &[&str] = &[
    "rm", "del", "rd", "shutdown", "reboot", "halt", "poweroff",
    "mkfs", "dd", "fdisk", "format", ">", ">>", "|", ";", "&&", "||",
];

const MAX_OUTPUT_SIZE: usize = 10000; // 10KB max

pub struct ShellTool;

impl ShellTool {
    pub fn new() -> Self {
        Self
    }

    fn is_blocked(command: &str) -> bool {
        let cmd_lower = command.to_lowercase();
        BLOCKED_COMMANDS.iter().any(|&blocked| cmd_lower.contains(blocked))
    }

    fn sanitize_output(output: &[u8]) -> String {
        // Converte bytes para string, substituindo caracteres inválidos
        let text = String::from_utf8_lossy(output);
        
        // Remove caracteres de controle exceto newline e tab
        let sanitized: String = text
            .chars()
            .filter(|&c| c == '\n' || c == '\t' || c == '\r' || !c.is_control())
            .collect();
        
        // Limita tamanho
        if sanitized.len() > MAX_OUTPUT_SIZE {
            format!("{}\n... (output truncado)", &sanitized[..MAX_OUTPUT_SIZE])
        } else {
            sanitized
        }
    }
}

#[async_trait::async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Executa comandos shell. Input: { \"command\": \"ls -la\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'command' é obrigatório".to_string())?;

        if command.is_empty() {
            return Err("Comando vazio".to_string());
        }

        if Self::is_blocked(command) {
            return Err(format!(
                "Comando bloqueado por segurança: {:?}",
                BLOCKED_COMMANDS
            ));
        }

        let working_dir = args["working_dir"].as_str();

        // Parse command more carefully
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Err("Comando vazio".to_string());
        }

        let mut cmd = Command::new(parts[0]);
        if parts.len() > 1 {
            cmd.args(&parts[1..]);
        }

        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        }

        // Execute with timeout
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            tokio::task::spawn_blocking(move || cmd.output())
        )
        .await;

        let output = match result {
            Ok(Ok(Ok(output))) => output,
            Ok(Ok(Err(e))) => return Err(format!("Erro ao executar: {}", e)),
            Ok(Err(_)) => return Err("Erro interno na execução".to_string()),
            Err(_) => return Err("Timeout: comando excedeu 30 segundos".to_string()),
        };

        let stdout = Self::sanitize_output(&output.stdout);
        let stderr = Self::sanitize_output(&output.stderr);

        if output.status.success() {
            if stdout.is_empty() && !stderr.is_empty() {
                Ok(format!("⚠️  {}", stderr.trim()))
            } else {
                Ok(stdout.trim().to_string())
            }
        } else {
            Err(format!(
                "❌ Erro (código {}): {}",
                output.status.code().unwrap_or(-1),
                if stderr.is_empty() { stdout.trim() } else { stderr.trim() }
            ))
        }
    }
}

impl Default for ShellTool {
    fn default() -> Self {
        Self::new()
    }
}
