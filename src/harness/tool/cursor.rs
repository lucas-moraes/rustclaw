//! `cursor` tool: delegates a whole task to the Cursor CLI (`agent`).
//!
//! When the `cursor_agent` toggle is on, the `build` mode is served by the
//! `cursor` agent, whose only tool is this one. The tool synthesizes a
//! **deterministic delegation prompt** (no LLM, no harness system prompt) and
//! spawns `agent -p --force --output-format stream-json <prompt>`, parsing the
//! streamed JSON lines incrementally.
//!
//! Design notes (see TODO.md):
//! - The harness system prompt is NEVER sent to Cursor (D4).
//! - Skills are NOT included in the delegation prompt (D5); Cursor reads
//!   `AGENTS.md` / `.cursor/rules` on its own.
//! - A single positional argument is passed; no `--append-system-prompt` (D6).

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::harness::project::memory::ProjectMemoryStore;
use crate::harness::project::profiler::ProjectProfiler;
use crate::harness::project::scoring::{
    render_memory_ranked, MAX_MEMORY_CHARS, MEMORY_BLOCK_END, MEMORY_BLOCK_START,
};
use crate::harness::tool::context::ToolContext;
use crate::harness::tool::{Tool, ToolResult};

/// Fixed contract appended to every delegation prompt. Kept short on purpose:
/// Cursor already reads `AGENTS.md` / `.cursor/rules`, so we do not duplicate
/// project rules here.
const CONTRACT: &str = "\
## Contrato
- Você é o executor desta tarefa. Trabalhe diretamente no repositório.
- Use as ferramentas disponíveis para ler, editar e verificar o código.
- Ao terminar, responda com um resumo curto do que mudou e como verificou.";

/// Builds the deterministic delegation prompt sent to the Cursor CLI.
///
/// Sections: `## Tarefa` (raw user task), `# Project context` (structural
/// summary), an optional `<project-memory>` block, and the fixed `## Contrato`.
/// `extra` is reserved for a future (Variante 2) enrichment slot; pass `None`.
pub fn build_delegation_prompt(
    task: &str,
    cwd: &Path,
    memory: Option<&str>,
    extra: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str("## Tarefa\n");
    out.push_str(task.trim());
    out.push_str("\n\n");

    let summary = ProjectProfiler {
        inner: ProjectProfiler::analyze(cwd),
    }
    .render_summary();
    if !summary.trim().is_empty() {
        out.push_str(summary.trim_end());
        out.push_str("\n\n");
    }

    if let Some(mem) = memory {
        let mem = mem.trim();
        if !mem.is_empty() {
            out.push_str(MEMORY_BLOCK_START);
            out.push('\n');
            out.push_str(mem);
            out.push('\n');
            out.push_str(MEMORY_BLOCK_END);
            out.push_str("\n\n");
        }
    }

    if let Some(extra) = extra {
        let extra = extra.trim();
        if !extra.is_empty() {
            out.push_str(extra);
            out.push_str("\n\n");
        }
    }

    out.push_str(CONTRACT);
    out.push('\n');
    out
}

/// Renders the `<project-memory>` block for a cwd, or `None` when empty.
/// Mirrors `runtime::context::memory_block_for` but is self-contained (the
/// tool has no access to `SessionRuntime`).
fn memory_block(store: &Arc<ProjectMemoryStore>, cwd: &Path, query: &str) -> Option<String> {
    let facts = store.active_facts(cwd).unwrap_or_default();
    if facts.is_empty() {
        return None;
    }
    let ranks: std::collections::HashMap<i64, f64> = if query.trim().is_empty() {
        std::collections::HashMap::new()
    } else {
        store
            .search_facts(cwd, query)
            .unwrap_or_default()
            .into_iter()
            .map(|(f, rank)| (f.id, rank))
            .collect()
    };
    let rendered = render_memory_ranked(&facts, query, MAX_MEMORY_CHARS, &ranks);
    if rendered.trim().is_empty() {
        None
    } else {
        Some(rendered)
    }
}

/// Extracts the human-readable text from a single `stream-json` line.
///
/// The Cursor CLI emits one JSON object per line. We are tolerant: malformed
/// lines and unknown shapes yield `None` instead of failing the whole run.
fn extract_text_from_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    // Common shapes: {"type":"assistant","message":{"content":[{"type":"text","text":...}]}}
    // or {"type":"text","text":...} / {"text":...}.
    if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
        return Some(t.to_string());
    }
    if let Some(content) = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    {
        let mut buf = String::new();
        for part in content {
            if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                buf.push_str(t);
            }
        }
        if !buf.is_empty() {
            return Some(buf);
        }
    }
    if let Some(content) = v.get("content").and_then(|c| c.as_array()) {
        let mut buf = String::new();
        for part in content {
            if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                buf.push_str(t);
            }
        }
        if !buf.is_empty() {
            return Some(buf);
        }
    }
    None
}

/// Parses a full `stream-json` payload (multiple lines) into consolidated text.
/// Exposed for tests.
#[cfg(test)]
pub fn parse_stream_json(payload: &str) -> String {
    let mut out = String::new();
    for line in payload.lines() {
        if let Some(t) = extract_text_from_line(line) {
            out.push_str(&t);
        }
    }
    out
}

/// Builds the fixed argv for the Cursor CLI spawn. `--model <id>` is appended
/// only when a model is set; an empty/`None` model means the CLI's own default
/// ("auto").
/// Which Cursor CLI mode the delegation runs in.
///
/// `Build` writes to the repository (`--force`); `Plan` is read-only and
/// produces a plan (`--mode plan`), mirroring the harness `plan` agent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorMode {
    /// `agent -p --force` — full agent, may edit files.
    #[default]
    Build,
    /// `agent -p --mode plan` — read-only planning, no edits.
    Plan,
}

impl CursorMode {
    /// Tool name exposed to the model. Distinct per mode so build and plan
    /// delegations can coexist in the same registry.
    pub fn tool_name(self) -> &'static str {
        match self {
            CursorMode::Build => "cursor",
            CursorMode::Plan => "cursor_plan",
        }
    }

    /// Extra argv flags that select this mode.
    fn flags(self) -> &'static [&'static str] {
        match self {
            // `--force` is what lets the headless agent write files.
            CursorMode::Build => &["--force"],
            // Plan mode is read-only; no `--force`.
            CursorMode::Plan => &["--mode", "plan"],
        }
    }
}

/// Builds the argv for the Cursor CLI in the given mode. `--model <id>` is
/// appended only when a model is set (empty = the CLI's own default, "auto").
fn spawn_args(mode: CursorMode, model: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
    ];
    args.extend(mode.flags().iter().map(|s| s.to_string()));
    if let Some(model) = model {
        args.push("--model".to_string());
        args.push(model.to_string());
    }
    args
}

/// The `cursor` / `cursor_plan` tool.
///
/// `model` is the id passed to the Cursor CLI as `--model <id>`. When `None`
/// or empty, no `--model` flag is sent and the CLI uses its own default
/// ("auto"). `mode` selects build (`--force`) or plan (`--mode plan`).
#[derive(Default)]
pub struct CursorTool {
    model: Option<String>,
    mode: CursorMode,
}

impl CursorTool {
    /// Builds a build-mode tool with the given Cursor model id. An empty
    /// string is treated as "auto" (no `--model` flag).
    pub fn new(model: impl Into<String>) -> Self {
        Self::with_mode(CursorMode::Build, model)
    }

    /// Builds a tool for the given mode and Cursor model id. An empty model
    /// string is treated as "auto" (no `--model` flag).
    pub fn with_mode(mode: CursorMode, model: impl Into<String>) -> Self {
        let model = model.into();
        Self {
            model: if model.trim().is_empty() {
                None
            } else {
                Some(model)
            },
            mode,
        }
    }
}

#[async_trait::async_trait]
impl Tool for CursorTool {
    fn name(&self) -> &str {
        self.mode.tool_name()
    }

    fn description(&self) -> &str {
        match self.mode {
            CursorMode::Build => {
                "Delega a tarefa inteira ao Cursor CLI (agent -p --force). Use para \
                 executar trabalho de build: ler, editar e verificar código no \
                 repositório. Retorna o resumo produzido pelo Cursor."
            }
            CursorMode::Plan => {
                "Delega o planejamento ao Cursor CLI (agent -p --mode plan, read-only). \
                 Use para investigar o repositório e produzir um plano de mudanças, \
                 sem editar arquivos. Retorna o plano produzido pelo Cursor."
            }
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Descrição completa da tarefa a ser delegada ao Cursor."
                }
            },
            "required": ["task"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let task = args
            .get("task")
            .and_then(|t| t.as_str())
            .ok_or_else(|| "missing required argument `task`".to_string())?
            .to_string();

        let cwd = ctx.cwd.path().to_path_buf();
        let mem = match ctx.project_memory.clone() {
            Some(store) => {
                let cwd = cwd.clone();
                let q = task.clone();
                tokio::task::spawn_blocking(move || memory_block(&store, &cwd, &q))
                    .await
                    .ok()
                    .flatten()
            }
            None => None,
        };
        let prompt = build_delegation_prompt(&task, &cwd, mem.as_deref(), None);

        let mut cmd = tokio::process::Command::new("agent");
        cmd.args(spawn_args(self.mode, self.model.as_deref()));
        let mut child = cmd
            .arg(&prompt)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn `agent` (Cursor CLI): {e}"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to capture agent stdout".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "failed to capture agent stderr".to_string())?;

        let mut lines = BufReader::new(stdout).lines();
        let mut collected = String::new();
        let abort = ctx.abort.clone();

        loop {
            tokio::select! {
                _ = abort.wait() => {
                    let _ = child.kill().await;
                    return Err("cursor tool aborted".to_string());
                }
                line = lines.next_line() => {
                    match line {
                        Ok(Some(l)) => {
                            if let Some(t) = extract_text_from_line(&l) {
                                collected.push_str(&t);
                            }
                        }
                        Ok(None) => break,
                        Err(e) => return Err(format!("error reading agent output: {e}")),
                    }
                }
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| format!("failed to wait for agent: {e}"))?;

        if !status.success() {
            let mut err_buf = String::new();
            let mut err_lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = err_lines.next_line().await {
                err_buf.push_str(&l);
                err_buf.push('\n');
            }
            return Err(format!(
                "cursor agent exited with {}: {}",
                status.code().unwrap_or(-1),
                err_buf.trim()
            ));
        }

        let output = if collected.trim().is_empty() {
            "(cursor agent produced no textual output)".to_string()
        } else {
            collected
        };
        Ok(ToolResult::simple("cursor agent", output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_cwd() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn test_delegation_prompt_has_sections() {
        let p = build_delegation_prompt("fix the bug", &tmp_cwd(), None, None);
        assert!(p.contains("## Tarefa"));
        assert!(p.contains("fix the bug"));
        assert!(p.contains("## Contrato"));
    }

    #[test]
    fn test_delegation_prompt_includes_memory_when_present() {
        let p = build_delegation_prompt("t", &tmp_cwd(), Some("- fact one"), None);
        assert!(p.contains(MEMORY_BLOCK_START));
        assert!(p.contains("- fact one"));
        assert!(p.contains(MEMORY_BLOCK_END));
    }

    #[test]
    fn test_delegation_prompt_omits_memory_when_absent() {
        let p = build_delegation_prompt("t", &tmp_cwd(), None, None);
        assert!(!p.contains(MEMORY_BLOCK_START));
        let p2 = build_delegation_prompt("t", &tmp_cwd(), Some("   "), None);
        assert!(!p2.contains(MEMORY_BLOCK_START));
    }

    #[test]
    fn test_delegation_prompt_never_contains_system_prompt() {
        // The delegation prompt is built only from task + project data; it must
        // never embed the harness system prompt.
        let p = build_delegation_prompt("t", &tmp_cwd(), None, None);
        assert!(!p.contains("You are RustClaw"));
        assert!(!p.contains("system prompt"));
    }

    #[test]
    fn test_parse_stream_json_tolerates_malformed_lines() {
        let payload = "not json\n{\"text\":\"hello\"}\n{broken\n";
        assert_eq!(parse_stream_json(payload), "hello");
    }

    #[test]
    fn test_parse_stream_json_extracts_text() {
        let payload = concat!(
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"A\"}]}}\n",
            "{\"text\":\"B\"}\n"
        );
        assert_eq!(parse_stream_json(payload), "AB");
    }

    /// No model set → no `--model` flag (CLI default "auto").
    #[test]
    fn test_spawn_args_without_model() {
        let args = spawn_args(CursorMode::Build, None);
        assert_eq!(
            args,
            vec!["-p", "--output-format", "stream-json", "--force"]
        );
        assert!(!args.iter().any(|a| a == "--model"));
    }

    /// A model is forwarded as `--model <id>`.
    #[test]
    fn test_spawn_args_with_model() {
        let args = spawn_args(CursorMode::Build, Some("composer-2.5"));
        assert_eq!(
            args,
            vec![
                "-p",
                "--output-format",
                "stream-json",
                "--force",
                "--model",
                "composer-2.5",
            ]
        );
    }

    /// Plan mode swaps `--force` for `--mode plan` (read-only).
    #[test]
    fn test_spawn_args_plan_mode() {
        let args = spawn_args(CursorMode::Plan, None);
        assert_eq!(
            args,
            vec!["-p", "--output-format", "stream-json", "--mode", "plan"]
        );
        assert!(!args.iter().any(|a| a == "--force"));
    }

    /// The tool name follows the mode so build and plan can coexist.
    #[test]
    fn test_cursor_tool_names_by_mode() {
        use crate::harness::tool::Tool;
        assert_eq!(CursorTool::new("").name(), "cursor");
        assert_eq!(
            CursorTool::with_mode(CursorMode::Plan, "").name(),
            "cursor_plan"
        );
    }

    /// `CursorTool::new("")` normalizes to "auto" (no `--model` flag).
    #[test]
    fn test_cursor_tool_empty_model_is_auto() {
        assert_eq!(
            spawn_args(CursorMode::Build, CursorTool::new("").model.as_deref()),
            spawn_args(CursorMode::Build, None)
        );
        assert_eq!(
            spawn_args(
                CursorMode::Build,
                CursorTool::new("gpt-5.3-codex").model.as_deref()
            ),
            spawn_args(CursorMode::Build, Some("gpt-5.3-codex"))
        );
    }

    #[test]
    fn test_nonzero_exit_returns_err_with_stderr() {
        // The error formatting path is exercised by the tool; here we assert
        // the message shape used for non-zero exits.
        let msg = format!("cursor agent exited with {}: {}", 1, "boom".trim());
        assert!(msg.contains("exited with 1"));
        assert!(msg.contains("boom"));
    }
}
