use super::Tool;
use serde_json::Value;
use std::process::Command;
use std::time::Duration;

const BLOCKED_COMMANDS: &[&str] = &[
    "rm",
    "del",
    "rd",
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    "mkfs",
    "dd",
    "fdisk",
    "format",
];

pub struct ShellTool;

impl ShellTool {
    pub fn new() -> Self {
        Self
    }

    fn is_blocked(command: &str) -> bool {
        let cmd_lower = command.to_lowercase();
        BLOCKED_COMMANDS.iter().any(|&blocked| cmd_lower.contains(blocked))
    }
}

#[async_trait::async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Executa comandos shell. Input: { \"command\": \"ls -la\", \"working_dir\": \"/tmp\" } (working_dir opcional)"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'command' é obrigatório".to_string())?;

        if Self::is_blocked(command) {
            return Err(format!(
                "Comando bloqueado por segurança. Comandos proibidos: {:?}",
                BLOCKED_COMMANDS
            ));
        }

        let working_dir = args["working_dir"].as_str();

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

        let output = tokio::time::timeout(Duration::from_secs(30), async {
            cmd.output()
        })
        .await
        .map_err(|_| "Timeout: comando excedeu 30 segundos".to_string())?
        .map_err(|e| format!("Erro ao executar comando: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if output.status.success() {
            if stdout.is_empty() && !stderr.is_empty() {
                Ok(format!("Aviso: {}", stderr.trim()))
            } else {
                Ok(stdout.trim().to_string())
            }
        } else {
            Err(format!(
                "Comando falhou (código {}): {}",
                output.status.code().unwrap_or(-1),
                if stderr.is_empty() {
                    stdout.trim()
                } else {
                    stderr.trim()
                }
            ))
        }
    }
}

impl Default for ShellTool {
    fn default() -> Self {
        Self::new()
    }
}
