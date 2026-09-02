//! `glob` tool: list files matching a pattern, respecting common ignore dirs.

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};

const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
];

const MAX_RESULTS: usize = 200;

pub struct GlobTool;

#[async_trait::async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Lists files matching a glob pattern (e.g. `**/*.rs`, `src/*.rs`), \
relative to the working directory. Skips common ignored directories."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Glob pattern to match"},
                "base_path": {"type": "string", "description": "Base directory (default: cwd)"}
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| "missing required argument: pattern".to_string())?;
        let base = args["base_path"]
            .as_str()
            .map(|p| ctx.cwd.resolve(p))
            .unwrap_or_else(|| ctx.cwd.path().to_path_buf());

        if !base.is_dir() {
            return Err(format!("base_path is not a directory: {}", base.display()));
        }

        let matches = glob_in(&base, pattern);
        let mut results: Vec<String> = Vec::new();
        for path in matches.iter().take(MAX_RESULTS) {
            let rel = path
                .strip_prefix(&base)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| path.to_string_lossy().to_string());
            results.push(rel);
        }

        let truncated_flag = matches.len() > MAX_RESULTS;
        let mut body = results.join("\n");
        if body.is_empty() {
            body = "(no matches)".to_string();
        } else if truncated_flag {
            body.push_str(&format!(
                "\n\n[showing first {} of {} matches]",
                MAX_RESULTS,
                matches.len()
            ));
        }

        Ok(ToolResult::simple(
            format!("glob {}", preview(pattern, 40)),
            body,
        ))
    }
}

fn glob_in(base: &std::path::Path, pattern: &str) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let full_pattern = base.join(pattern);
    let pattern_str = full_pattern.to_string_lossy().to_string();

    if let Ok(paths) = glob::glob(&pattern_str) {
        for entry in paths.flatten() {
            if entry.is_file() && !is_ignored(&entry, base) {
                out.push(entry);
            }
        }
    }
    // Fallback: recursive walk with manual matching (glob crate handles ** only in some cases).
    out.sort();
    out
}

pub fn is_ignored(path: &std::path::Path, base: &std::path::Path) -> bool {
    let rel = match path.strip_prefix(base) {
        Ok(r) => r,
        Err(_) => return true,
    };
    rel.components().any(|c| {
        IGNORED_DIRS
            .iter()
            .any(|d| c.as_os_str().to_string_lossy() == *d)
    })
}
