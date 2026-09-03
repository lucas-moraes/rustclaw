//! `edit` tool: exact string replacement in files.

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};

pub struct EditTool;

#[async_trait::async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Replaces an exact `old_string` with `new_string` in a file. \
`old_string` must match exactly once unless `replace_all` is true. \
Read the file first to get exact content including whitespace."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (relative to cwd or absolute)"},
                "old_string": {"type": "string", "description": "Exact text to replace"},
                "new_string": {"type": "string", "description": "Replacement text"},
                "replace_all": {"type": "boolean", "description": "Replace every occurrence (default false)"}
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        if ctx.abort.is_aborted() {
            return Err("aborted".to_string());
        }
        let raw_path = args["path"]
            .as_str()
            .ok_or_else(|| "missing required argument: path".to_string())?;
        let old_string = args["old_string"]
            .as_str()
            .ok_or_else(|| "missing required argument: old_string".to_string())?;
        let new_string = args["new_string"]
            .as_str()
            .ok_or_else(|| "missing required argument: new_string".to_string())?;
        let replace_all = args["replace_all"].as_bool().unwrap_or(false);

        if old_string.is_empty() {
            return Err("old_string is empty".to_string());
        }

        let path = ctx.cwd.resolve(raw_path);
        if !path.is_file() {
            return Err(format!("file not found: {}", path.display()));
        }

        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
        let content = String::from_utf8_lossy(&bytes).to_string();

        if !content.contains(old_string) {
            return Err(format!(
                "old_string not found in {}. Read the file first and use exact content.",
                path.display()
            ));
        }

        let occurrences = content.matches(old_string).count();
        if occurrences > 1 && !replace_all {
            return Err(format!(
                "old_string matches {} locations; make it unique or pass replace_all=true",
                occurrences
            ));
        }

        let updated = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        tokio::fs::write(&path, updated.as_bytes())
            .await
            .map_err(|e| format!("failed to write {}: {}", path.display(), e))?;

        let count = if replace_all { occurrences } else { 1 };
        let diff = super::diff::unified_diff(&content, &updated);
        let metadata = serde_json::json!({
            "path": path.display().to_string(),
            "diff": diff,
            "diff_truncated": false,
        });
        Ok(ToolResult {
            title: format!("edit {}", preview(raw_path, 40)),
            output: format!(
                "Replaced {} occurrence(s) in {} ({} -> {} chars).",
                count,
                path.display(),
                old_string.len(),
                new_string.len()
            ),
            metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::permission::PermissionEngine;
    use crate::harness::tool::context::{
        AbortSignal, PathBufGuard, PermissionAsker, ToolContext, UserAsker,
    };
    use std::sync::Arc;

    struct AllowAsker;
    struct NoUserAsker;

    #[async_trait::async_trait]
    impl PermissionAsker for AllowAsker {
        async fn ask(&self, _r: crate::harness::tool::context::PermissionAskInput) -> bool {
            true
        }
    }
    #[async_trait::async_trait]
    impl UserAsker for NoUserAsker {
        async fn ask(&self, _q: String, _o: Vec<String>) -> Option<String> {
            None
        }
    }

    #[tokio::test]
    async fn test_edit_produces_diff_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.txt");
        std::fs::write(&file, "line1\nline2\nline3\n").unwrap();

        let ctx = ToolContext {
            session_id: "s".into(),
            agent: "build".into(),
            agent_tools: vec![],
            cwd: PathBufGuard(dir.path().to_path_buf()),
            abort: AbortSignal::new(),
            permission: Arc::new(PermissionEngine::default()),
            asker: Arc::new(AllowAsker),
            user_asker: Arc::new(NoUserAsker),
            todos: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            task_runner: None,
            project_memory: None,
        };

        let tool = EditTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": "x.txt",
                    "old_string": "line2",
                    "new_string": "CHANGED"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.output.contains("Replaced 1 occurrence"));
        let diff = result.metadata["diff"].as_str().unwrap();
        assert!(diff.contains("- line2"), "diff:\n{}", diff);
        assert!(diff.contains("+ CHANGED"), "diff:\n{}", diff);
    }
}
