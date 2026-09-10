//! `write` tool: creates or overwrites files (with parent dir creation).

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};

pub struct WriteTool;

#[async_trait::async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Creates or overwrites a file with the given content. \
Parent directories are created automatically. Use `edit` to modify existing files."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (relative to cwd or absolute)"},
                "content": {"type": "string", "description": "Full file content to write"}
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        if ctx.abort.is_aborted() {
            return Err("aborted".to_string());
        }
        let raw_path = args["path"]
            .as_str()
            .ok_or_else(|| "missing required argument: path".to_string())?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| "missing required argument: content".to_string())?;

        let path = ctx.cwd.resolve(raw_path);

        // Serialize concurrent writes to the same file and hold the lock across
        // read → snapshot → write so the snapshot reflects the pre-write state.
        let lock = ctx.checkpoints.lock(&path);
        let _guard = lock.lock().await;

        // Capture prior content (for the diff) before overwriting.
        let before = tokio::fs::read_to_string(&path).await.unwrap_or_default();

        ctx.checkpoints.snapshot(&path);

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(&parent)
                .await
                .map_err(|e| format!("failed to create directory {}: {}", parent.display(), e))?;
        }

        let bytes = content.as_bytes();
        // Atomic write (temp file + rename) so a failure never leaves the file
        // truncated, and the write lands on the canonical path.
        tokio::task::spawn_blocking({
            let path = path.clone();
            let bytes = bytes.to_vec();
            move || super::checkpoint::atomic_write(&path, &bytes)
        })
        .await
        .map_err(|e| format!("write task failed: {}", e))?
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))?;

        let lines = content.lines().count();
        let diff = super::diff::unified_diff(&before, content);
        let metadata = serde_json::json!({
            "path": path.display().to_string(),
            "diff": diff,
            "diff_truncated": false,
            "new_file": before.is_empty(),
        });
        Ok(ToolResult {
            title: format!("wrote {}", preview(raw_path, 40)),
            output: format!(
                "Wrote {} bytes ({} lines) to {}",
                bytes.len(),
                lines,
                path.display()
            ),
            metadata,
        })
    }
}
