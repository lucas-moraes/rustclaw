use super::Tool;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const DANGEROUS_COMMANDS: &[&str] = &[
    "rm", "del", "rd", "mkfs", "dd", "fdisk", "format",
];

const SYSTEM_COMMANDS: &[&str] = &[
    "shutdown", "reboot", "halt", "poweroff",
];

const SAFE_OUTPUT_COMMANDS: &[&str] = &[
    "echo", "printf", "tee", "cat",
];

const MAX_OUTPUT_SIZE: usize = 10000; // 10KB max

pub struct ShellTool;

impl ShellTool {
    pub fn new() -> Self {
        Self
    }

    fn is_blocked(&self, command: &str, force: bool, working_dir: Option<&str>) -> Result<bool, String> {
        let cmd_lower = command.to_lowercase();

        // Allow heredoc patterns: cat > file << 'EOF' or cat > file <<EOF
        if Self::is_heredoc_pattern(&cmd_lower) {
            return Ok(false);
        }

        // Allow safe redirect patterns: echo "text" > file, printf > file, etc.
        if Self::is_safe_redirect(&cmd_lower) {
            return Ok(false);
        }

        // System commands are always blocked
        if SYSTEM_COMMANDS.iter().any(|&c| cmd_lower.contains(c)) {
            return Err(format!(
                "Comando de sistema bloqueado: {:?}. Use force:true para executar.",
                SYSTEM_COMMANDS
            ));
        }

        // Dangerous commands need force flag AND must be in working directory
        if DANGEROUS_COMMANDS.iter().any(|&c| cmd_lower.contains(c)) {
            if force {
                // Check if the path is within working directory
                if let Some(dir) = working_dir {
                    if Self::is_path_restricted(&cmd_lower, dir) {
                        return Err(format!(
                            "⚠️ Acesso restrito!\n\
                             Comandos perigosos só podem operar dentro do diretório de trabalho: {}\n\
                             Tentar acessar fora deste diretório não é permitido.",
                            dir
                        ));
                    }
                }
                return Ok(false);
            } else {
                return Err(format!(
                    "Comando perigoso detectado: {:?}. Use force:true para confirmar.\n\
                     ⚠️ AVISO: Este comando pode causar perda de dados!",
                    DANGEROUS_COMMANDS
                ));
            }
        }

        Ok(false)
    }

    fn is_path_restricted(command: &str, working_dir: &str) -> bool {
        // Extract paths from the command
        let paths = Self::extract_paths_from_command(command);
        
        if paths.is_empty() {
            return false; // No specific path, allow with force
        }

        let work_dir = Path::new(working_dir).canonicalize().unwrap_or_else(|_| PathBuf::from(working_dir));

        // Get home directory
        let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let home_str = home_dir.to_string_lossy().to_lowercase();

        // System directories that should never be modified
        let system_dirs = [
            "/etc", "/usr", "/bin", "/sbin", "/var", "/boot", 
            "/root", "/sys", "/proc", "/opt", "/srv", "/lib"
        ];

        for path_str in paths {
            let target_path = Path::new(&path_str);
            let path_str_lower = path_str.to_lowercase();
            
            // Check for dangerous system directories
            for sys_dir in &system_dirs {
                let sys_dir_lower = sys_dir.to_lowercase();
                if path_str_lower.starts_with(&sys_dir_lower) && path_str_lower != sys_dir_lower {
                    // Block system directories like /etc, /usr, etc.
                    return true;
                }
            }

            // Allow if it's within the working directory OR within home directory
            if path_str_lower.starts_with('/') {
                // It's an absolute path - check if it's in home or allowed
                let is_in_home = path_str_lower.starts_with(&home_str);
                let is_in_work_dir = if let Ok(canonical_target) = target_path.canonicalize() {
                    if let Ok(canonical_work) = work_dir.canonicalize() {
                        canonical_target.starts_with(&canonical_work)
                    } else {
                        false
                    }
                } else {
                    false
                };

                if !is_in_home && !is_in_work_dir {
                    return true; // Path is outside allowed areas
                }
            } else if path_str_lower.contains("..") {
                // Has parent directory reference - check if it escapes
                if let Ok(canonical_target) = target_path.canonicalize() {
                    if let Ok(canonical_work) = work_dir.canonicalize() {
                        if !canonical_target.starts_with(&canonical_work) {
                            // Also check if it escapes home
                            if !canonical_target.to_string_lossy().to_lowercase().starts_with(&home_str) {
                                return true;
                            }
                        }
                    }
                }
            }
        }

        false
    }

    fn extract_paths_from_command(command: &str) -> Vec<String> {
        let mut paths = Vec::new();
        let mut current_path = String::new();
        let mut in_quotes = false;
        let mut in_single_quotes = false;
        
        for c in command.chars() {
            match c {
                '"' if !in_single_quotes => in_quotes = !in_quotes,
                '\'' if !in_quotes => in_single_quotes = !in_single_quotes,
                ' ' | '\t' | '>' | '<' | '|' | ';' | '&' | '\n' => {
                    if !current_path.is_empty() {
                        paths.push(current_path.clone());
                        current_path.clear();
                    }
                }
                _ => {
                    if !in_quotes && !in_single_quotes {
                        current_path.push(c);
                    }
                }
            }
        }
        
        if !current_path.is_empty() {
            paths.push(current_path);
        }
        
        paths
    }

    fn is_heredoc_pattern(command: &str) -> bool {
        // Pattern: cat > filename << or cat > filename <<'
        command.contains(">") 
            && command.contains("<<")
            && (command.starts_with("cat ") || command.starts_with("tee "))
    }

    fn is_safe_redirect(command: &str) -> bool {
        // Already checked heredoc above, now check other safe patterns
        
        // Check for dangerous redirects to devices
        let dangerous_patterns = ["/dev/", "/sys/", "/proc/"];
        for pattern in &dangerous_patterns {
            if command.contains(pattern) {
                return false;
            }
        }

        // Allow: echo "text" > file
        // Allow: printf "text" > file  
        // Allow: command > file.log
        // Allow: command 2> error.txt
        // Allow: command >> file (append)
        
        let has_redirect = command.contains("> ") || command.ends_with('>');
        let has_append = command.contains(">>");
        let has_stderr_redirect = command.contains("2>");
        
        if has_redirect || has_append || has_stderr_redirect {
            // Check if it's a safe output command
            let is_safe_cmd = SAFE_OUTPUT_COMMANDS.iter().any(|&c| command.starts_with(c));
            
            // Or has a space before > (indicating redirect to file)
            if is_safe_cmd || has_redirect || has_append || has_stderr_redirect {
                return true;
            }
        }

        false
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
        "Executa comandos shell. Input: { \"command\": \"ls -la\", \"force\": true }. \
         Comandos perigosos (rm, dd) precisam de force:true"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'command' é obrigatório".to_string())?;

        if command.is_empty() {
            return Err("Comando vazio".to_string());
        }

        // Get force flag for dangerous commands
        let force = args["force"].as_bool().unwrap_or(false);
        let working_dir = args["working_dir"].as_str();

        // Check if blocked
        match self.is_blocked(command, force, working_dir) {
            Ok(false) => {}
            Ok(true) => unreachable!(),
            Err(msg) => return Err(msg),
        }

        // Check if it's a heredoc pattern - use shell -c for proper parsing
        if Self::is_heredoc_pattern(&command.to_lowercase()) {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(&command);

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

            return Self::handle_output(output);
        }

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

        Self::handle_output(output)
    }
}

impl ShellTool {
    fn handle_output(output: std::process::Output) -> Result<String, String> {
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
