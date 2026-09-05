//! Git-aware tools: `git_status`, `git_diff`, `git_log`.
//!
//! These give the model structured, truncated git output without relying on
//! the generic `bash` tool (which wastes tokens and is error-prone to parse).
//! All three are read-only and run in the session cwd.

use super::{Tool, ToolResult};
use crate::harness::tool::context::ToolContext;
use crate::harness::tool::truncate::truncate_lines;
use serde_json::{json, Value};

const MAX_OUTPUT_LINES: usize = 200;
const DEFAULT_LOG_N: usize = 20;
const MAX_LOG_N: usize = 100;

/// Runs `git <args>` in the session cwd, returning combined stdout+stderr
/// truncated by line count. Errors on non-zero exit with a friendly message.
async fn run_git(ctx: &ToolContext, args: &[&str]) -> Result<String, String> {
    if ctx.abort.is_aborted() {
        return Err("aborted".to_string());
    }
    let cwd = ctx.cwd.path().to_path_buf();
    let mut child = tokio::process::Command::new("git")
        .args(args)
        .current_dir(&cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to spawn git: {}", e))?;

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

    let status = child
        .wait()
        .await
        .map_err(|e| format!("failed to wait for git: {}", e))?;
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

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        let msg = combined.trim();
        return Err(if msg.is_empty() {
            format!("git {} failed with exit code {}", args.join(" "), code)
        } else {
            format!("git {} failed (exit {}): {}", args.join(" "), code, msg)
        });
    }

    Ok(truncate_lines(&combined, MAX_OUTPUT_LINES))
}

/// `git_status`: branch + dirty files in a compact, truncated form.
pub struct GitStatusTool;

#[async_trait::async_trait]
impl Tool for GitStatusTool {
    fn name(&self) -> &str {
        "git_status"
    }
    fn description(&self) -> &str {
        "Shows the git working tree status: current branch and dirty/untracked \
files. Read-only. Use instead of parsing `git status` via bash."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "porcelain": {
                    "type": "boolean",
                    "description": "Use --porcelain=v1 for machine-readable output (default false)"
                }
            }
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        ctx.check_permission("git_status", &args).await?;
        let porcelain = args["porcelain"].as_bool().unwrap_or(false);
        let out = if porcelain {
            run_git(ctx, &["status", "--porcelain=v1", "--branch"]).await?
        } else {
            run_git(ctx, &["status", "--short", "--branch"]).await?
        };
        Ok(ToolResult::simple("git_status", out))
    }
}

/// `git_diff`: staged/unstaged diff with optional path filter.
pub struct GitDiffTool;

#[async_trait::async_trait]
impl Tool for GitDiffTool {
    fn name(&self) -> &str {
        "git_diff"
    }
    fn description(&self) -> &str {
        "Shows the git diff: unstaged by default, or staged with `staged: true`. \
Optionally filter to a single path. Read-only and truncated."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "staged": {
                    "type": "boolean",
                    "description": "Show the staged (cached) diff instead of unstaged (default false)"
                },
                "path": {
                    "type": "string",
                    "description": "Restrict the diff to this path (relative to the project root)"
                }
            }
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        ctx.check_permission("git_diff", &args).await?;
        let staged = args["staged"].as_bool().unwrap_or(false);
        let path = args["path"].as_str().map(str::to_string);
        let mut cmd: Vec<&str> = vec!["diff"];
        if staged {
            cmd.push("--cached");
        }
        let out = match path {
            Some(p) => {
                let mut full = cmd;
                full.push("--");
                full.push(&p);
                run_git(ctx, &full).await?
            }
            None => run_git(ctx, &cmd).await?,
        };
        Ok(ToolResult::simple("git_diff", out))
    }
}

/// `git_log`: short commit log (`--oneline`) with a configurable limit.
pub struct GitLogTool;

#[async_trait::async_trait]
impl Tool for GitLogTool {
    fn name(&self) -> &str {
        "git_log"
    }
    fn description(&self) -> &str {
        "Shows a short git commit log (one line per commit). Read-only and truncated."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "n": {
                    "type": "integer",
                    "description": "Number of commits to show (default 20, max 100)"
                }
            }
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        ctx.check_permission("git_log", &args).await?;
        let n = args["n"]
            .as_u64()
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_LOG_N)
            .min(MAX_LOG_N);
        let out = run_git(ctx, &["log", "--oneline", &format!("-n {}", n)]).await?;
        Ok(ToolResult::simple("git_log", out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::permission::PermissionEngine;
    use crate::harness::tool::context::{
        AbortSignal, PathBufGuard, PermissionAskInput, PermissionAsker, ToolContext, UserAsker,
    };
    use std::sync::Arc;

    struct AllowAsker;
    #[async_trait::async_trait]
    impl PermissionAsker for AllowAsker {
        async fn ask(&self, _req: PermissionAskInput) -> bool {
            true
        }
    }
    struct NoUserAsker;
    #[async_trait::async_trait]
    impl UserAsker for NoUserAsker {
        async fn ask(&self, _q: String, _o: Vec<String>) -> Option<String> {
            None
        }
    }

    fn ctx(cwd: &std::path::Path) -> ToolContext {
        ToolContext {
            session_id: "s".into(),
            agent: "build".into(),
            agent_tools: vec![],
            cwd: PathBufGuard(cwd.to_path_buf()),
            abort: AbortSignal::new(),
            permission: Arc::new(PermissionEngine::default()),
            asker: Arc::new(AllowAsker),
            user_asker: Arc::new(NoUserAsker),
            todos: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            task_runner: None,
            project_memory: None,
        }
    }

    /// Creates a temp dir with a git repo and one commit.
    fn git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .expect("git command");
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test"]);
        std::fs::write(dir.path().join("file.txt"), "hello\n").unwrap();
        run(&["add", "file.txt"]);
        run(&["commit", "-q", "-m", "initial commit"]);
        dir
    }

    #[tokio::test]
    async fn test_git_status_reports_branch() {
        let dir = git_repo();
        let c = ctx(dir.path());
        let res = GitStatusTool
            .execute(json!({}), &c)
            .await
            .expect("git_status");
        assert!(
            res.output.contains("master") || res.output.contains("main"),
            "expected branch in output, got: {}",
            res.output
        );
    }

    #[tokio::test]
    async fn test_git_diff_shows_unstaged_change() {
        let dir = git_repo();
        std::fs::write(dir.path().join("file.txt"), "hello\nworld\n").unwrap();
        let c = ctx(dir.path());
        let res = GitDiffTool.execute(json!({}), &c).await.expect("git_diff");
        assert!(
            res.output.contains("+world"),
            "expected +world in diff, got: {}",
            res.output
        );
    }

    #[tokio::test]
    async fn test_git_log_shows_commit() {
        let dir = git_repo();
        let c = ctx(dir.path());
        let res = GitLogTool
            .execute(json!({"n": 5}), &c)
            .await
            .expect("git_log");
        assert!(
            res.output.contains("initial commit"),
            "expected commit in log, got: {}",
            res.output
        );
    }

    #[tokio::test]
    async fn test_git_tools_error_outside_repo() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let err = GitStatusTool
            .execute(json!({}), &c)
            .await
            .expect_err("expected error outside a git repo");
        assert!(
            err.contains("not a git repository") || err.contains("failed"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn test_preview_import() {
        // Sanity: preview helper is reachable from this module.
        assert_eq!(
            crate::harness::tool::truncate::preview("hello world", 5),
            "hello…"
        );
    }
}
