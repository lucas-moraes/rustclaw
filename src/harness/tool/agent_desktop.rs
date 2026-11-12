//! `agent_desktop` tool: computer use via the `agent-desktop` CLI
//! (vercel-labs). The CLI exposes the OS accessibility tree: snapshot, act
//! on element refs, screenshot, and read embedded skills.
//!
//! The tool is a thin, allowlisted wrapper around `agent-desktop <subcommand>`.
//! Observation subcommands (`status`, `snapshot`, `screenshot`, `list-apps`,
//! `skills`) are treated as read-only; everything else (click, type, key,
//! scroll, focus, launch, quit, batch, ...) mutates UI state and follows the
//! normal Ask flow.

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use crate::harness::tool::env::{mask_secrets, sanitized_env};
use crate::harness::tool::truncate::truncate_output;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

const MAX_OUTPUT_BYTES: usize = 20_000;
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_TIMEOUT_SECS: u64 = 300;

/// Subcommands that only observe (no UI mutation). Everything else asks.
const READ_ONLY_SUBCOMMANDS: &[&str] = &["status", "snapshot", "screenshot", "list-apps", "skills"];

pub struct AgentDesktopTool;

/// One token, no shell metacharacters: the CLI takes a flat argv.
fn validate_subcommand(sub: &str) -> Result<(), String> {
    if sub.is_empty()
        || sub.contains(char::is_whitespace)
        || sub.contains(|c: char| {
            matches!(
                c,
                '|' | ';'
                    | '&'
                    | '<'
                    | '>'
                    | '$'
                    | '`'
                    | '('
                    | ')'
                    | '{'
                    | '}'
                    | '\n'
                    | '\r'
                    | '"'
                    | '\''
            )
        })
    {
        return Err(format!(
            "invalid subcommand `{}`: expected a single CLI subcommand token \
             (status, snapshot, screenshot, click, type, key, scroll, ...)",
            preview(sub, 40)
        ));
    }
    Ok(())
}

#[async_trait::async_trait]
impl Tool for AgentDesktopTool {
    fn name(&self) -> &str {
        "agent_desktop"
    }

    fn description(&self) -> &str {
        "See and operate desktop apps through the OS accessibility tree via the \
`agent-desktop` CLI. Observation: `status`, `snapshot` (refs of the focused app), \
`screenshot`, `list-apps`, `skills get desktop|platform`. Actions: `click`, `type`, \
`key`, `scroll`, `focus`, `launch`, `quit`, `batch` (see `agent-desktop --help`). \
Workflow: snapshot -> act on a ref -> re-snapshot to verify. Snapshots can contain \
private content; keep them out of logs and uploads."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "subcommand": {
                    "type": "string",
                    "description": "Single agent-desktop subcommand token, e.g. \"snapshot\", \
        \"click\", \"type\", \"key\", \"screenshot\", \"status\", \"skills\"."
                },
                "args": {
                    "type": "string",
                    "description": "Extra CLI arguments as a single string, split on whitespace. \
        Examples: \"--ref 12\", \"get desktop\", \"--text hello\", \"--key return\"."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default 60, max 300)."
                }
            },
            "required": ["subcommand"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let sub = args["subcommand"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or("missing required parameter: subcommand")?
            .trim()
            .to_string();
        validate_subcommand(&sub)?;

        let mut argv = vec!["agent-desktop".to_string(), sub.clone()];
        if let Some(extra) = args["args"].as_str() {
            let extra = extra.trim();
            if !extra.is_empty() {
                // Free-form argument string passed verbatim to the CLI
                // (e.g. "--ref 12", "get desktop"). Split on whitespace;
                // quoting is not supported.
                argv.extend(extra.split_whitespace().map(str::to_string));
            }
        }

        // Permission: observation subcommands are read-only (Ask is treated as
        // Allow — they never mutate UI state); actions follow the Ask flow.
        if READ_ONLY_SUBCOMMANDS.contains(&sub.as_str()) {
            use crate::harness::permission::PermissionDecision;
            let decision = ctx.permission.check("agent_desktop", None, ctx.cwd.path());
            match decision {
                PermissionDecision::Deny => {
                    return Err(
                        "Permission denied: tool `agent_desktop` is not allowed by policy."
                            .to_string(),
                    )
                }
                PermissionDecision::Allow | PermissionDecision::Ask => {}
            }
        } else {
            let perm_args = json!({ "subcommand": sub, "args": args["args"] });
            ctx.check_permission("agent_desktop", &perm_args).await?;
        }

        let secs = args["timeout_secs"]
            .as_u64()
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .min(MAX_TIMEOUT_SECS);

        let mut cmd = tokio::process::Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .current_dir(ctx.cwd.path())
            .env_clear()
            .envs(sanitized_env(&BTreeMap::new()))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let output = tokio::time::timeout(Duration::from_secs(secs), cmd.output())
            .await
            .map_err(|_| format!("agent-desktop {} timed out after {}s", sub, secs))?
            .map_err(|e| format!("failed to run agent-desktop: {}", e))?;

        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.stderr.is_empty() {
            if !combined.is_empty() {
                combined.push_str("\n[stderr]\n");
            }
            combined.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        let code = output.status.code().unwrap_or(-1);
        let combined = mask_secrets(combined.trim_end());
        let full = format!("{}\n[exit: {}]", combined, code);
        let output = truncate_output(&full, MAX_OUTPUT_BYTES);

        Ok(ToolResult {
            title: preview(&argv[1..].join(" "), 60),
            output,
            metadata: json!({"exit_code": code, "subcommand": sub}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_shell_metacharacters_in_subcommand() {
        for bad in ["snap;shot", "snap shot", "$(x)", "a|b", "a&b", "a>b", "a'b"] {
            assert!(validate_subcommand(bad).is_err(), "should reject: {bad}");
        }
        for good in ["snapshot", "click", "list-apps", "skills"] {
            assert!(validate_subcommand(good).is_ok(), "should accept: {good}");
        }
    }

    #[test]
    fn read_only_subcommands_are_classified() {
        for sub in READ_ONLY_SUBCOMMANDS {
            assert!(READ_ONLY_SUBCOMMANDS.contains(sub));
        }
        for sub in ["click", "type", "key", "launch", "batch"] {
            assert!(!READ_ONLY_SUBCOMMANDS.contains(&sub));
        }
    }
}
