//! `bash` tool: runs shell commands with denylist, timeout and output truncation.
//!
//! Security model:
//! - **Token-based denylist**: dangerous patterns are detected by splitting the
//!   command into tokens (whitespace-separated) and matching specific
//!   combinations, not by substring. This blocks `rm -rf --no-preserve-root /`
//!   even though the substring `rm -rf /` never appears.
//! - **Privilege escalation**: `sudo`/`su` commands are not blocked but are
//!   flagged to require an `Ask` permission escalation (even when `bash` is
//!   otherwise `Allow`).
//! - **Destructive redirection**: `> /dev/sd*`, `> /etc/...` are blocked.

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};
use std::time::Duration;

const MAX_OUTPUT_BYTES: usize = 20_000;
const DEFAULT_TIMEOUT_SECS: u64 = 120;

pub struct BashTool;

/// Result of the security check for a bash command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BashCheck {
    /// Command is safe to run.
    Ok,
    /// Command is blocked outright (dangerous/system).
    Blocked,
    /// Command requires elevated permission (sudo/su) — escalate to Ask.
    NeedsPrivilege,
}

/// Splits a shell command into tokens, handling quotes and escapes minimally.
/// This is intentionally simple: it splits on whitespace but keeps quoted
/// strings together so `rm -rf "/path with spaces"` is tokenized correctly.
fn tokenize(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for c in command.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' => {
                escaped = true;
                current.push(c);
            }
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(c);
            }
            '"' if !in_single => {
                in_double = !in_double;
                current.push(c);
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Checks whether a token is a destructive `rm` flag combination.
fn is_destructive_rm(tokens: &[String]) -> bool {
    // Find `rm` and check for recursive + force flags with an absolute path.
    let Some(rm_idx) = tokens.iter().position(|t| t == "rm") else {
        return false;
    };
    let mut recursive = false;

    // Critical system directories that must never be `rm -rf`'d.
    const CRITICAL_DIRS: &[&str] = &[
        "/", "/etc", "/boot", "/dev", "/usr", "/var", "/home", "/root", "/bin", "/sbin", "/lib",
        "/lib64", "/opt", "/proc", "/sys", "/run",
    ];

    for t in tokens.iter().skip(rm_idx + 1) {
        if t.starts_with('-') {
            // Combined flags like -rf, -fr, -r, -f
            for ch in t.trim_start_matches('-').chars() {
                match ch {
                    'r' | 'R' => recursive = true,
                    _ => {}
                }
            }
        } else if t.starts_with('/') {
            // Check if the path is a critical system directory (or a direct
            // child of one, e.g. `/etc/passwd`).
            let cleaned = t.trim_end_matches('/');
            if CRITICAL_DIRS.contains(&cleaned) {
                return true;
            }
            // Also block direct children of critical dirs (e.g. /etc/*).
            for dir in CRITICAL_DIRS {
                if *dir != "/" && cleaned.starts_with(dir) && cleaned[dir.len()..].starts_with('/')
                {
                    return true;
                }
            }
        }
        // Stop at the first non-flag, non-path argument (e.g. a file name).
        if !t.starts_with('-') && !t.starts_with('/') {
            break;
        }
    }

    // `rm -r /` (recursive on root) is always blocked, even without -f.
    if recursive {
        for t in tokens.iter().skip(rm_idx + 1) {
            if t.starts_with('/') {
                let cleaned = t.trim_end_matches('/');
                if cleaned == "/" || cleaned.is_empty() {
                    return true;
                }
            }
        }
    }

    false
}

/// Checks for destructive redirection like `> /dev/sda` or `> /etc/...`.
fn has_destructive_redirect(tokens: &[String]) -> bool {
    for (i, t) in tokens.iter().enumerate() {
        if t == ">" || t == ">>" || t == "2>" || t == "1>" {
            if let Some(target) = tokens.get(i + 1) {
                let target = target.trim_start_matches('"').trim_start_matches('\'');
                if target.starts_with("/dev/sd") || target.starts_with("/dev/nvme") {
                    return true;
                }
                if target.starts_with("/etc/") || target.starts_with("/boot/") {
                    return true;
                }
            }
        }
    }
    false
}

/// Checks whether the command uses `sudo` or `su` (privilege escalation).
fn needs_privilege(tokens: &[String]) -> bool {
    tokens.iter().any(|t| t == "sudo" || t == "su")
}

/// Token-based security check for a bash command.
pub fn check_denylist(command: &str) -> BashCheck {
    let tokens = tokenize(command);
    let lower_tokens: Vec<String> = tokens.iter().map(|t| t.to_lowercase()).collect();

    // Block system commands.
    for t in &lower_tokens {
        match t.as_str() {
            "shutdown" | "reboot" | "halt" | "poweroff" => return BashCheck::Blocked,
            _ => {}
        }
    }
    // `init 0` / `init 6` (system shutdown/reboot).
    if lower_tokens.len() >= 2 && lower_tokens[0] == "init" {
        if let Some(arg) = lower_tokens.get(1) {
            if arg == "0" || arg == "6" {
                return BashCheck::Blocked;
            }
        }
    }
    // `systemctl stop` / `systemctl disable` (service management).
    if lower_tokens.len() >= 2 && lower_tokens[0] == "systemctl" {
        if let Some(arg) = lower_tokens.get(1) {
            if arg == "stop" || arg == "disable" {
                return BashCheck::Blocked;
            }
        }
    }

    // Block mkfs / fdisk / wipe / shred (disk operations).
    for t in &lower_tokens {
        if t.starts_with("mkfs") || t == "fdisk" || t == "wipe" || t == "shred" {
            return BashCheck::Blocked;
        }
    }

    // Block `dd if=` (disk writing).
    if lower_tokens.iter().any(|t| t.starts_with("if=")) {
        return BashCheck::Blocked;
    }

    // Block destructive `rm` combinations.
    if is_destructive_rm(&lower_tokens) {
        return BashCheck::Blocked;
    }

    // Block destructive redirection.
    if has_destructive_redirect(&lower_tokens) {
        return BashCheck::Blocked;
    }

    // Block fork bomb.
    if command.contains(":(){") || command.contains(":(){:|:&};:") {
        return BashCheck::Blocked;
    }

    // `mv /` or `mv /*` (moving root).
    if lower_tokens.len() >= 2 && lower_tokens[0] == "mv" {
        if let Some(arg) = lower_tokens.get(1) {
            if arg == "/" || arg == "/*" {
                return BashCheck::Blocked;
            }
        }
    }

    // Privilege escalation: not blocked, but requires Ask.
    if needs_privilege(&lower_tokens) {
        return BashCheck::NeedsPrivilege;
    }

    BashCheck::Ok
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

        // Security check: block dangerous commands, escalate sudo/su to Ask.
        match check_denylist(&command) {
            BashCheck::Blocked => {
                return Err(format!(
                    "blocked dangerous command: `{}`",
                    preview(&command, 80)
                ));
            }
            BashCheck::NeedsPrivilege => {
                // Escalate to Ask: even if `bash` is Allow, sudo/su requires
                // explicit user approval. We route through the permission
                // asker with a synthetic path marker.
                let input = crate::harness::tool::context::PermissionAskInput {
                    tool: "bash".to_string(),
                    args_summary: format!("[privileged] {}", preview(&command, 200)),
                    path: None,
                };
                let allowed = ctx.asker.ask(input).await;
                if !allowed {
                    return Err("The user denied permission for the privileged command. \
                         Do not retry the same call; explain and ask how to proceed."
                        .to_string());
                }
            }
            BashCheck::Ok => {}
        }

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
    fn test_tokenize_basic() {
        assert_eq!(tokenize("ls -la"), vec!["ls", "-la"]);
        assert_eq!(tokenize("rm -rf /"), vec!["rm", "-rf", "/"]);
        assert_eq!(
            tokenize("echo 'hello world'"),
            vec!["echo", "'hello world'"]
        );
    }

    #[test]
    fn test_denylist_blocks_dangerous() {
        assert_eq!(check_denylist("rm -rf /"), BashCheck::Blocked);
        assert_eq!(
            check_denylist("rm -rf --no-preserve-root /"),
            BashCheck::Blocked
        );
        assert_eq!(check_denylist("rm -fr /"), BashCheck::Blocked);
        assert_eq!(check_denylist("rm -r /"), BashCheck::Blocked);
        assert_eq!(check_denylist("rm -rf /etc"), BashCheck::Blocked);
        assert_eq!(check_denylist("mkfs.ext4 /dev/sda"), BashCheck::Blocked);
        assert_eq!(check_denylist("shutdown now"), BashCheck::Blocked);
        assert_eq!(check_denylist("reboot"), BashCheck::Blocked);
        assert_eq!(
            check_denylist("dd if=/dev/zero of=/dev/sda"),
            BashCheck::Blocked
        );
        assert_eq!(check_denylist("fdisk /dev/sda"), BashCheck::Blocked);
        assert_eq!(check_denylist("wipe /dev/sda"), BashCheck::Blocked);
        assert_eq!(check_denylist("shred /dev/sda"), BashCheck::Blocked);
        assert_eq!(check_denylist("init 0"), BashCheck::Blocked);
        assert_eq!(check_denylist("systemctl stop nginx"), BashCheck::Blocked);
        assert_eq!(check_denylist("mv / /tmp"), BashCheck::Blocked);
        assert_eq!(check_denylist("mv /* /tmp"), BashCheck::Blocked);
        assert_eq!(check_denylist("echo x > /dev/sda"), BashCheck::Blocked);
        assert_eq!(check_denylist("echo x > /etc/passwd"), BashCheck::Blocked);
        assert_eq!(check_denylist(":(){ :|:& };:"), BashCheck::Blocked);
    }

    #[test]
    fn test_denylist_allows_safe() {
        assert_eq!(check_denylist("ls -la"), BashCheck::Ok);
        assert_eq!(check_denylist("cargo test"), BashCheck::Ok);
        assert_eq!(check_denylist("git status"), BashCheck::Ok);
        assert_eq!(check_denylist("rm -rf ./target"), BashCheck::Ok);
        assert_eq!(check_denylist("rm -rf target/"), BashCheck::Ok);
        assert_eq!(check_denylist("rm -rf build/"), BashCheck::Ok);
        assert_eq!(check_denylist("echo hello"), BashCheck::Ok);
        assert_eq!(check_denylist("cat /etc/hostname"), BashCheck::Ok);
    }

    #[test]
    fn test_sudo_requires_privilege() {
        assert_eq!(
            check_denylist("sudo apt install git"),
            BashCheck::NeedsPrivilege
        );
        assert_eq!(
            check_denylist("sudo rm -rf /tmp/x"),
            BashCheck::NeedsPrivilege
        );
        assert_eq!(check_denylist("su -c 'whoami'"), BashCheck::NeedsPrivilege);
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
            events: crate::harness::event::event_channel().0,
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
