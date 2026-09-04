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

        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .current_dir(&cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("failed to spawn command: {}", e))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut out) = stdout {
                let _ = tokio::io::AsyncReadExt::read_to_end(&mut out, &mut buf).await;
            }
            buf
        });
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut err) = stderr {
                let _ = tokio::io::AsyncReadExt::read_to_end(&mut err, &mut buf).await;
            }
            buf
        });

        let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
        let status = loop {
            if ctx.abort.is_aborted() {
                let _ = child.start_kill();
                let _ = child.wait().await;
                stdout_task.abort();
                stderr_task.abort();
                return Err("aborted".to_string());
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                let _ = child.start_kill();
                let _ = child.wait().await;
                stdout_task.abort();
                stderr_task.abort();
                return Err(format!(
                    "command timed out after {}s: {}",
                    secs,
                    preview(&command, 80)
                ));
            }
            tokio::select! {
                biased;
                status = child.wait() => {
                    break status.map_err(|e| format!("command failed: {}", e))?;
                }
                _ = tokio::time::sleep(Duration::from_millis(50).min(remaining)) => {}
            }
        };

        let stdout = stdout_task.await.unwrap_or_default();
        let stderr = stderr_task.await.unwrap_or_default();

        let mut combined = String::new();
        if !stdout.is_empty() {
            combined.push_str(&String::from_utf8_lossy(&stdout));
        }
        if !stderr.is_empty() {
            if !combined.is_empty() {
                combined.push_str("\n[stderr]\n");
            }
            combined.push_str(&String::from_utf8_lossy(&stderr));
        }
        let code = status.code().unwrap_or(-1);
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

    #[tokio::test]
    async fn test_bash_respects_abort_signal() {
        use crate::harness::permission::PermissionEngine;
        use crate::harness::tool::context::{
            AbortSignal, PathBufGuard, PermissionAsker, ToolContext, UserAsker,
        };
        use std::sync::Arc;

        struct AllowAsker;
        struct NoUserAsker;
        #[async_trait::async_trait]
        impl PermissionAsker for AllowAsker {
            async fn ask(&self, _req: crate::harness::tool::context::PermissionAskInput) -> bool {
                true
            }
        }
        #[async_trait::async_trait]
        impl UserAsker for NoUserAsker {
            async fn ask(&self, _q: String, _opts: Vec<String>) -> Option<String> {
                None
            }
        }

        let abort = AbortSignal::new();
        let ctx = ToolContext {
            session_id: "s".into(),
            agent: "build".into(),
            agent_tools: vec![],
            cwd: PathBufGuard(std::env::temp_dir()),
            abort: abort.clone(),
            permission: Arc::new(PermissionEngine::default()),
            asker: Arc::new(AllowAsker),
            user_asker: Arc::new(NoUserAsker),
            todos: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            task_runner: None,
            project_memory: None,
        };

        // Abort after a short delay while a long sleep is running.
        let abort2 = abort.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            abort2.abort();
        });

        let started = std::time::Instant::now();
        let err = BashTool
            .execute(json!({"command": "sleep 30", "timeout_secs": 60}), &ctx)
            .await
            .expect_err("expected abort");
        assert_eq!(err, "aborted");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "abort should kill sleep quickly, took {:?}",
            started.elapsed()
        );
    }
}
