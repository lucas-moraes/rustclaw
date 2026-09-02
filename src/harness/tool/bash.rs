//! `bash` tool: runs shell commands with denylist, timeout and output truncation.

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};
use std::time::Duration;

const DANGEROUS_COMMANDS: &[&str] = &[
    "rm -rf /",
    "mkfs",
    "dd if=",
    "fdisk",
    "wipe",
    "shred",
    ":(){:|:&};:",
    "mv / ",
    "mv /*",
];

const SYSTEM_COMMANDS: &[&str] = &[
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    "init 0",
    "init 6",
    "systemctl stop",
    "systemctl disable",
];

const MAX_OUTPUT_BYTES: usize = 20_000;
const DEFAULT_TIMEOUT_SECS: u64 = 120;

pub struct BashTool;

pub fn check_denylist(command: &str) -> Result<(), String> {
    let cmd_lower = command.to_lowercase();
    for dangerous in DANGEROUS_COMMANDS {
        if cmd_lower.contains(dangerous) {
            return Err(format!(
                "blocked dangerous command pattern: `{}`",
                dangerous
            ));
        }
    }
    for system in SYSTEM_COMMANDS {
        if cmd_lower.contains(system) {
            return Err(format!("blocked system command: `{}`", system));
        }
    }
    Ok(())
}

fn timeout_secs(args: &Value) -> u64 {
    args["timeout_secs"]
        .as_u64()
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .min(600)
}

#[async_trait::async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Executes a shell command and returns its combined stdout+stderr with exit code. \
Use for builds, tests, git and project inspection. Dangerous/system commands are blocked."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default 120, max 600)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        if ctx.abort.is_aborted() {
            return Err("aborted".to_string());
        }
        let command = args["command"]
            .as_str()
            .ok_or_else(|| "missing required argument: command".to_string())?
            .to_string();
        if command.trim().is_empty() {
            return Err("command is empty".to_string());
        }

        let secs = timeout_secs(&args);
        check_denylist(&command)?;

        let cwd = ctx.cwd.path().to_path_buf();

        let run = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .current_dir(&cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();

        let output = tokio::time::timeout(Duration::from_secs(secs), run)
            .await
            .map_err(|_| {
                format!(
                    "command timed out after {}s: {}",
                    secs,
                    preview(&command, 80)
                )
            })?
            .map_err(|e| format!("failed to spawn command: {}", e))?;

        let mut combined = String::new();
        if !output.stdout.is_empty() {
            combined.push_str(&String::from_utf8_lossy(&output.stdout));
        }
        if !output.stderr.is_empty() {
            if !combined.is_empty() {
                combined.push_str("\n[stderr]\n");
            }
            combined.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        let code = output.status.code().unwrap_or(-1);
        let full = format!("{}\n[exit: {}]", combined.trim_end(), code);
        let truncated = super::truncate::truncate_output(&full, MAX_OUTPUT_BYTES);

        Ok(ToolResult {
            title: preview(&command, 60),
            output: truncated,
            metadata: json!({"exit_code": code}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_denylist_blocks_dangerous() {
        assert!(check_denylist("rm -rf /").is_err());
        assert!(check_denylist("mkfs.ext4 /dev/sda").is_err());
        assert!(check_denylist("shutdown now").is_err());
    }

    #[test]
    fn test_denylist_allows_safe() {
        assert!(check_denylist("ls -la").is_ok());
        assert!(check_denylist("cargo test").is_ok());
        assert!(check_denylist("git status").is_ok());
    }

    #[test]
    fn test_timeout_parsing() {
        let args = json!({"command": "x", "timeout_secs": 9999});
        assert_eq!(timeout_secs(&args), 600);
        assert_eq!(timeout_secs(&json!({"command": "x"})), 120);
    }
}
