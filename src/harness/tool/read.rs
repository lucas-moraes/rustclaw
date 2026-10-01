//! `read` tool: reads files with optional offset/limit windowing.

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};

const MAX_BYTES: usize = 40_000;
const MAX_LINES: usize = 2_000;

pub struct ReadTool;

#[async_trait::async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Reads a text file, optionally a window of lines (offset/limit). \
Returns lines numbered as `line: content`."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (relative to cwd or absolute)"},
                "offset": {"type": "integer", "description": "Start line (1-based, optional)"},
                "limit": {"type": "integer", "description": "Max lines to read (optional)"}
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let raw_path = args["path"]
            .as_str()
            .ok_or_else(|| "missing required argument: path".to_string())?;
        let path = ctx.cwd.resolve(raw_path);

        if !path.exists() {
            return Err(format!("file not found: {}", path.display()));
        }
        if !path.is_file() {
            return Err(format!("not a file: {}", path.display()));
        }

        let content = tokio::fs::read(&path)
            .await
            .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
        let text = String::from_utf8_lossy(&content).to_string();

        let offset = args["offset"].as_u64().unwrap_or(1).max(1) as usize;
        let limit = args["limit"].as_u64().unwrap_or(MAX_LINES as u64) as usize;

        let lines: Vec<&str> = text.lines().collect();
        if offset > lines.len() {
            return Err(format!(
                "offset {} beyond file ({} lines total)",
                offset,
                lines.len()
            ));
        }
        let start = offset - 1;
        let end = (start + limit).min(lines.len());

        let mut body = String::new();
        for (i, line) in lines[start..end].iter().enumerate() {
            body.push_str(&format!("{}: {}\n", start + i + 1, line));
        }
        if body.is_empty() {
            body.push_str("(empty file)\n");
        }
        let truncated = super::truncate::truncate_output(&body, MAX_BYTES);

        let title = format!(
            "read {}{}",
            preview(raw_path, 40),
            if offset > 1 {
                format!(" (from line {})", offset)
            } else {
                String::new()
            }
        );

        Ok(ToolResult::simple(title, truncated))
    }
}
