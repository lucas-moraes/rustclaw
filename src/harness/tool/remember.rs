//! `remember` tool: persists a fact/convention into project memory (SQLite).
//!
//! The agent calls this to record hidden conventions, tricky code patterns, or
//! specific build/test commands so future sessions can reuse them. Facts are
//! appended (never overwriting prior ones) into the project's `summary`.

use super::{Tool, ToolResult};
use crate::harness::project::ProjectMemoryStore;
use crate::harness::tool::context::ToolContext;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize)]
struct RememberArgs {
    fact: String,
}

pub struct RememberTool;

#[async_trait::async_trait]
impl Tool for RememberTool {
    fn name(&self) -> &str {
        "remember"
    }
    fn description(&self) -> &str {
        "Salva uma instrução, convenção, comando de build/teste ou aprendizado \
crítico sobre o projeto no banco SQLite para uso em sessões futuras."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "fact": {
                    "type": "string",
                    "description": "O fato ou aprendizado que deve ser memorizado."
                }
            },
            "required": ["fact"]
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let parsed: RememberArgs = serde_json::from_value(args)
            .map_err(|e| format!("invalid arguments for `remember`: {}", e))?;
        let fact = parsed.fact.trim();
        if fact.is_empty() {
            return Err("`remember` requires a non-empty `fact`.".to_string());
        }

        let store: Arc<ProjectMemoryStore> = ctx
            .project_memory
            .clone()
            .ok_or_else(|| "project memory store unavailable".to_string())?;

        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string();
        let line = format!("- [{}] {}", now, fact);
        store
            .append(ctx.cwd.path(), &line)
            .map_err(|e| format!("failed to persist project memory: {}", e))?;

        Ok(ToolResult::simple(
            "remember",
            format!("Memorizado: {}", fact),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::permission::PermissionEngine;
    use crate::harness::tool::context::{AbortSignal, PathBufGuard};
    use std::sync::Arc;

    fn test_ctx(store: Arc<ProjectMemoryStore>, cwd: &std::path::Path) -> ToolContext {
        ToolContext {
            session_id: "s".into(),
            agent: "build".into(),
            cwd: PathBufGuard(cwd.to_path_buf()),
            abort: AbortSignal::new(),
            permission: Arc::new(PermissionEngine::default()),
            asker: Arc::new(AllowAsker),
            user_asker: Arc::new(NoUserAsker),
            todos: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            task_runner: None,
            project_memory: Some(store),
        }
    }

    struct AllowAsker;
    struct NoUserAsker;

    #[async_trait::async_trait]
    impl crate::harness::tool::context::PermissionAsker for AllowAsker {
        async fn ask(&self, _req: crate::harness::tool::context::PermissionAskInput) -> bool {
            true
        }
    }

    #[async_trait::async_trait]
    impl crate::harness::tool::context::UserAsker for NoUserAsker {
        async fn ask(&self, _q: String, _o: Vec<String>) -> Option<String> {
            None
        }
    }

    #[tokio::test]
    async fn test_remember_appends_with_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(ProjectMemoryStore::open(&dir.path().join("test.db")).unwrap());
        let ctx = test_ctx(store.clone(), dir.path());

        let tool = RememberTool;
        let r1 = tool
            .execute(json!({"fact": "use cargo test -- --ignored"}), &ctx)
            .await
            .unwrap();
        assert!(r1.output.contains("use cargo test"));

        let r2 = tool
            .execute(json!({"fact": "log to stderr not stdout"}), &ctx)
            .await
            .unwrap();
        assert!(r2.output.contains("log to stderr"));

        let row = store.load(dir.path()).unwrap().unwrap();
        // Both facts persisted (appended, not overwritten).
        assert!(row.summary.contains("use cargo test -- --ignored"));
        assert!(row.summary.contains("log to stderr not stdout"));
        // Each line carries a timestamp prefix.
        assert!(row.summary.contains("- [20"));
        assert!(row.summary.lines().count() == 2);
    }

    #[tokio::test]
    async fn test_remember_missing_fact_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(ProjectMemoryStore::open(&dir.path().join("test.db")).unwrap());
        let ctx = test_ctx(store, dir.path());
        let tool = RememberTool;
        let err = tool.execute(json!({}), &ctx).await.unwrap_err();
        assert!(err.contains("invalid arguments"));
    }

    #[tokio::test]
    async fn test_remember_empty_fact_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(ProjectMemoryStore::open(&dir.path().join("test.db")).unwrap());
        let ctx = test_ctx(store, dir.path());
        let tool = RememberTool;
        let err = tool
            .execute(json!({"fact": "   "}), &ctx)
            .await
            .unwrap_err();
        assert!(err.contains("non-empty"));
    }
}
