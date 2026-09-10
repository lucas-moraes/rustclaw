//! `diagnostics` tool: runs `cargo check --message-format=json` and returns
//! structured compile diagnostics (file, line, column, severity, message).

use super::{Tool, ToolResult};
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};
use std::path::PathBuf;

const TIMEOUT_SECS: u64 = 120;
const MAX_DIAGNOSTICS: usize = 50;
const MAX_BYTES: usize = 40_000;

pub struct DiagnosticsTool;

/// One parsed compiler diagnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub severity: String,
    pub code: Option<String>,
    pub message: String,
}

/// Parses `cargo check --message-format=json` output (one JSON per line).
/// Non-JSON lines (compilation status, progress) are ignored.
pub fn parse_cargo_check_output(output: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for line in output.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["reason"].as_str() != Some("compiler-message") {
            continue;
        }
        let msg = &v["message"];
        let severity = msg["level"].as_str().unwrap_or("note").to_string();
        let message = msg["message"].as_str().unwrap_or("").to_string();
        if message.is_empty() {
            continue;
        }
        let code = msg["code"]["code"]
            .as_str()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        // First renderable span with a real file (macro expansions may lack one).
        let span = msg["spans"]
            .as_array()
            .and_then(|spans| {
                spans
                    .iter()
                    .find(|s| s["file_name"].as_str().is_some_and(|f| !f.is_empty()))
            })
            .cloned()
            .unwrap_or(Value::Null);
        let file = span["file_name"]
            .as_str()
            .unwrap_or("<unknown>")
            .to_string();
        let line = span["line_start"].as_u64().unwrap_or(0) as u32;
        let column = span["column_start"].as_u64().unwrap_or(0) as u32;
        out.push(Diagnostic {
            file,
            line,
            column,
            severity,
            code,
            message,
        });
    }
    out
}

/// Formats diagnostics as `path:line:col: severity: message [code]` lines.
pub fn format_diagnostics(diags: &[Diagnostic]) -> String {
    if diags.is_empty() {
        return "no diagnostics".to_string();
    }
    let errors = diags.iter().filter(|d| d.severity == "error").count();
    let warnings = diags.iter().filter(|d| d.severity == "warning").count();
    let mut lines: Vec<String> = Vec::new();
    for d in diags.iter().take(MAX_DIAGNOSTICS) {
        let mut line = format!(
            "{}:{}:{}: {}: {}",
            d.file, d.line, d.column, d.severity, d.message
        );
        if let Some(code) = &d.code {
            line.push_str(&format!(" [{}]", code));
        }
        lines.push(line);
    }
    if diags.len() > MAX_DIAGNOSTICS {
        lines.push(format!(
            "[truncated: showing {} of {} diagnostics]",
            MAX_DIAGNOSTICS,
            diags.len()
        ));
    }
    lines.push(format!(
        "\n{} error{}, {} warning{}, {} total",
        errors,
        if errors == 1 { "" } else { "s" },
        warnings,
        if warnings == 1 { "" } else { "s" },
        diags.len()
    ));
    lines.join("\n")
}

#[async_trait::async_trait]
impl Tool for DiagnosticsTool {
    fn name(&self) -> &str {
        "diagnostics"
    }

    fn description(&self) -> &str {
        "Runs `cargo check --message-format=json` in the project and returns structured \
compile errors (file, line, column, message)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "workspace": {
                    "type": "string",
                    "description": "Workspace path (relative to cwd or absolute; default cwd)"
                }
            }
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        if ctx.abort.is_aborted() {
            return Err("aborted".to_string());
        }
        let workspace = match args["workspace"].as_str() {
            Some(raw) => ctx.cwd.resolve(raw),
            None => ctx.cwd.path().to_path_buf(),
        };
        if !workspace.join("Cargo.toml").exists() {
            return Err(format!(
                "no Cargo.toml found in {} (not a cargo project?)",
                workspace.display()
            ));
        }

        let dir: PathBuf = workspace.clone();
        // Run cargo check as a tokio child with kill_on_drop so a timeout
        // actually kills the process (a stray `cargo check` would hold the
        // target/ lock for minutes and block the next build).
        let mut child = tokio::process::Command::new("cargo")
            .arg("check")
            .arg("--message-format=json")
            .current_dir(&dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("failed to spawn cargo check: {e}"))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let out_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut out) = stdout {
                let _ = tokio::io::AsyncReadExt::read_to_end(&mut out, &mut buf).await;
            }
            buf
        });
        let err_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut err) = stderr {
                let _ = tokio::io::AsyncReadExt::read_to_end(&mut err, &mut buf).await;
            }
            buf
        });

        let status =
            match tokio::time::timeout(std::time::Duration::from_secs(TIMEOUT_SECS), child.wait())
                .await
            {
                Ok(status) => status.map_err(|e| format!("failed to wait for cargo check: {e}"))?,
                Err(_) => {
                    // kill_on_drop(true) kills the child when `child` is dropped.
                    return Err(format!(
                        "cargo check timed out after {}s (try a smaller workspace)",
                        TIMEOUT_SECS
                    ));
                }
            };

        let stdout = out_task
            .await
            .map_err(|e| format!("failed to join stdout task: {e}"))?;
        let stderr = err_task
            .await
            .map_err(|e| format!("failed to join stderr task: {e}"))?;

        let stdout = String::from_utf8_lossy(&stdout);
        let stderr = String::from_utf8_lossy(&stderr);

        let diags = parse_cargo_check_output(&stdout);
        let body = if diags.is_empty() && !status.success() {
            // No JSON diagnostics but the check failed: surface stderr.
            format!(
                "cargo check failed with no diagnostics\n\n{}",
                crate::harness::tool::truncate::truncate_output(stderr.trim(), MAX_BYTES)
            )
        } else {
            crate::harness::tool::truncate::truncate_output(&format_diagnostics(&diags), MAX_BYTES)
        };

        let title = format!("diagnostics {}", workspace.display());
        Ok(ToolResult::simple(title, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn error_line() -> String {
        r#"{"reason":"compiler-message","package_id":"x","target":{},"message":{"rendered":"error[E0425]: cannot find value `x`","level":"error","message":"cannot find value `x` in this scope","code":{"code":"E0425","explanation":"..."},"spans":[{"file_name":"src/main.rs","byte_start":10,"byte_end":11,"line_start":3,"line_end":3,"column_start":5,"column_end":6,"is_primary":true}]}}"#.to_string()
    }

    fn warning_line() -> String {
        r#"{"reason":"compiler-message","message":{"level":"warning","message":"unused variable: `y`","code":{"code":"unused_variables"},"spans":[{"file_name":"src/lib.rs","line_start":7,"line_end":7,"column_start":9,"column_end":10}]}}"#.to_string()
    }

    #[test]
    fn test_parses_error_and_warning() {
        let diags = parse_cargo_check_output(&format!("{}\n{}", error_line(), warning_line()));
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].file, "src/main.rs");
        assert_eq!(diags[0].line, 3);
        assert_eq!(diags[0].column, 5);
        assert_eq!(diags[0].severity, "error");
        assert_eq!(diags[0].code.as_deref(), Some("E0425"));
        assert_eq!(diags[0].message, "cannot find value `x` in this scope");
        assert_eq!(diags[1].severity, "warning");
        assert_eq!(diags[1].code.as_deref(), Some("unused_variables"));
    }

    #[test]
    fn test_ignores_non_json_and_other_reasons() {
        let input = "Compiling foo v0.1.0\nerror: could not compile\n\
{\"reason\":\"compiler-artifact\",\"target\":{}}\nnot json at all\n";
        assert!(parse_cargo_check_output(input).is_empty());
    }

    #[test]
    fn test_diagnostic_without_spans_defaults() {
        let input = r#"{"reason":"compiler-message","message":{"level":"error","message":"boom"}}"#;
        let diags = parse_cargo_check_output(input);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, "<unknown>");
        assert_eq!(diags[0].line, 0);
    }

    #[test]
    fn test_format_empty() {
        assert_eq!(format_diagnostics(&[]), "no diagnostics");
    }

    #[test]
    fn test_format_lines_and_summary() {
        let diags = parse_cargo_check_output(&format!("{}\n{}", error_line(), warning_line()));
        let out = format_diagnostics(&diags);
        assert!(out.contains("src/main.rs:3:5: error: cannot find value `x` in this scope [E0425]"));
        assert!(out.contains("src/lib.rs:7:9: warning: unused variable: `y` [unused_variables]"));
        assert!(out.contains("1 error, 1 warning, 2 total"));
    }

    #[test]
    fn test_format_truncates_to_max() {
        let many: Vec<Diagnostic> = (0..60)
            .map(|i| Diagnostic {
                file: format!("src/f{i}.rs"),
                line: 1,
                column: 1,
                severity: "error".into(),
                code: None,
                message: "boom".into(),
            })
            .collect();
        let out = format_diagnostics(&many);
        assert!(out.contains("[truncated: showing 50 of 60 diagnostics]"));
        assert!(!out.contains("src/f59.rs"));
    }

    #[tokio::test]
    async fn test_missing_workspace_arg_uses_cwd() {
        // Missing `workspace` is fine (defaults to cwd); a non-cargo dir errors.
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ctx_with_cwd(tmp.path().to_path_buf());
        let err = DiagnosticsTool.execute(json!({}), &ctx).await.unwrap_err();
        assert!(err.contains("no Cargo.toml found"), "got: {err}");
    }

    #[tokio::test]
    async fn test_invalid_workspace_arg() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ctx_with_cwd(tmp.path().to_path_buf());
        let err = DiagnosticsTool
            .execute(json!({"workspace": "definitely/not/here"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.contains("no Cargo.toml found"), "got: {err}");
    }

    fn ctx_with_cwd(cwd: std::path::PathBuf) -> ToolContext {
        use crate::harness::permission::PermissionEngine;
        struct AllowAsker;
        #[async_trait::async_trait]
        impl crate::harness::tool::context::PermissionAsker for AllowAsker {
            async fn ask(&self, _: crate::harness::tool::context::PermissionAskInput) -> bool {
                true
            }
        }
        struct NoUserAsker;
        #[async_trait::async_trait]
        impl crate::harness::tool::context::UserAsker for NoUserAsker {
            async fn ask(&self, _: String, _: Vec<String>) -> Option<String> {
                None
            }
        }
        ToolContext {
            session_id: "s".into(),
            agent: "build".into(),
            agent_tools: vec![],
            cwd: crate::harness::tool::context::PathBufGuard(cwd),
            abort: crate::harness::tool::context::AbortSignal::new(),
            permission: Arc::new(PermissionEngine::default()),
            asker: Arc::new(AllowAsker),
            user_asker: Arc::new(NoUserAsker),
            todos: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            task_runner: None,
            events: crate::harness::event::event_channel().0,
            project_memory: None,
            hooks: Default::default(),
            checkpoints: std::sync::Arc::new(
                crate::harness::tool::checkpoint::FileCheckpoints::new(),
            ),
            jobs: std::sync::Arc::new(crate::harness::tool::jobs::JobRegistry::new()),
        }
    }
}
