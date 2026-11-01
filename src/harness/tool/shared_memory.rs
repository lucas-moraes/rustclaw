//! `shared_memory`: project-wide key/value store shared by all agents.
//!
//! Subagents use it to publish findings (a file map, a decision, a command
//! that worked) that siblings or the parent can read later, instead of copying
//! everything through the parent's context window.

use super::{Tool, ToolResult};
use crate::harness::tool::context::ToolContext;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct MemoryArgs {
    action: String,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    value: Option<String>,
}

/// Reads and writes the project's shared memory namespace.
pub struct SharedMemoryTool;

#[async_trait::async_trait]
impl Tool for SharedMemoryTool {
    fn name(&self) -> &str {
        "shared_memory"
    }
    fn description(&self) -> &str {
        "Memória compartilhada do projeto entre agentes (chave/valor). \
Use `set` para publicar um achado, `get` para ler uma chave, `list` para ver \
todas e `delete` para remover. Sobrevive ao fim da sessão."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["set", "get", "list", "delete"],
                    "description": "Operação a executar."
                },
                "key": {
                    "type": "string",
                    "description": "Chave (obrigatória em set/get/delete)."
                },
                "value": {
                    "type": "string",
                    "description": "Valor a gravar (obrigatório em set)."
                }
            },
            "required": ["action"]
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let parsed: MemoryArgs = serde_json::from_value(args)
            .map_err(|e| format!("invalid arguments for `shared_memory`: {}", e))?;
        let mem = ctx
            .shared_memory
            .as_ref()
            .ok_or("shared memory is not available in this session")?;

        let key = || -> Result<&str, String> {
            parsed
                .key
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .ok_or_else(|| {
                    format!(
                        "`shared_memory` action `{}` requires a `key`.",
                        parsed.action
                    )
                })
        };

        match parsed.action.as_str() {
            "set" => {
                let key = key()?;
                let value = parsed
                    .value
                    .as_deref()
                    .ok_or("`shared_memory` action `set` requires a `value`.")?;
                let replaced = mem
                    .set(key, value, &ctx.session_id)
                    .map_err(|e| format!("failed to write shared memory: {e}"))?;
                let verb = if replaced { "Updated" } else { "Stored" };
                Ok(ToolResult::simple(
                    format!("shared_memory set {key}"),
                    format!("{verb} `{key}` ({} bytes).", value.len()),
                ))
            }
            "get" => {
                let key = key()?;
                match mem
                    .get(key)
                    .map_err(|e| format!("failed to read shared memory: {e}"))?
                {
                    Some(e) => Ok(ToolResult::simple(
                        format!("shared_memory get {key}"),
                        format!(
                            "`{}` (by {}, {}):\n{}",
                            e.key, e.author, e.updated_at, e.value
                        ),
                    )),
                    None => Ok(ToolResult::simple(
                        format!("shared_memory get {key}"),
                        format!("No entry for `{key}`."),
                    )),
                }
            }
            "list" => {
                let entries = mem
                    .list()
                    .map_err(|e| format!("failed to list shared memory: {e}"))?;
                if entries.is_empty() {
                    return Ok(ToolResult::simple(
                        "shared_memory list",
                        "Shared memory is empty.",
                    ));
                }
                let count = entries.len();
                let mut out = format!("{count} entr(y/ies):\n");
                for e in entries {
                    out.push_str(&format!(
                        "\n- `{}` (by {}, {}) — {} bytes",
                        e.key,
                        e.author,
                        e.updated_at,
                        e.value.len()
                    ));
                }
                Ok(ToolResult::simple(
                    format!("shared_memory list ({} entries)", count),
                    out,
                ))
            }
            "delete" => {
                let key = key()?;
                let removed = mem
                    .delete(key)
                    .map_err(|e| format!("failed to delete shared memory: {e}"))?;
                let msg = if removed {
                    format!("Deleted `{key}`.")
                } else {
                    format!("No entry for `{key}`.")
                };
                Ok(ToolResult::simple(
                    format!("shared_memory delete {key}"),
                    msg,
                ))
            }
            other => Err(format!(
                "unknown `shared_memory` action `{other}` (expected set|get|list|delete)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::permission::PermissionEngine;
    use crate::harness::swarm::{MessageBus, SharedMemory};
    use std::sync::Arc;

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
        async fn ask(&self, _question: String, _options: Vec<String>) -> Option<String> {
            None
        }
    }

    fn ctx(dir: &std::path::Path, session: &str) -> ToolContext {
        let db = dir.join("swarm.db");
        ToolContext {
            session_id: session.into(),
            agent: "build".into(),
            agent_tools: vec![],
            cwd: crate::harness::tool::context::PathBufGuard(dir.to_path_buf()),
            abort: crate::harness::tool::context::AbortSignal::new(),
            permission: Arc::new(PermissionEngine::default()),
            asker: Arc::new(AllowAsker),
            user_asker: Arc::new(NoUserAsker),
            todos: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            task_runner: None,
            depth: 0,
            events: crate::harness::event::event_channel().0,
            project_memory: None,
            hooks: Default::default(),
            checkpoints: Arc::new(crate::harness::tool::checkpoint::FileCheckpoints::new()),
            jobs: Arc::new(crate::harness::tool::jobs::JobRegistry::new()),
            semantic_index: None,
            embedder: None,
            sandbox_policy: None,
            message_bus: Some(Arc::new(MessageBus::open(&db, dir).unwrap())),
            shared_memory: Some(Arc::new(SharedMemory::open(&db, dir).unwrap())),
        }
    }

    async fn run(ctx: &ToolContext, args: serde_json::Value) -> Result<String, String> {
        SharedMemoryTool.execute(args, ctx).await.map(|r| r.output)
    }

    #[tokio::test]
    async fn set_get_list_delete_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let a = ctx(d.path(), "agent-a");
        let b = ctx(d.path(), "agent-b");

        let out = run(
            &a,
            serde_json::json!({"action": "set", "key": "map", "value": "a.rs -> A"}),
        )
        .await
        .unwrap();
        assert!(out.contains("Stored"));

        // Another agent reads the same namespace.
        let got = run(&b, serde_json::json!({"action": "get", "key": "map"}))
            .await
            .unwrap();
        assert!(got.contains("a.rs -> A"));
        assert!(got.contains("agent-a"));

        let list = run(&b, serde_json::json!({"action": "list"}))
            .await
            .unwrap();
        assert!(list.contains("map"));

        let del = run(&a, serde_json::json!({"action": "delete", "key": "map"}))
            .await
            .unwrap();
        assert!(del.contains("Deleted"));
        let gone = run(&b, serde_json::json!({"action": "get", "key": "map"}))
            .await
            .unwrap();
        assert!(gone.contains("No entry"));
    }

    #[tokio::test]
    async fn set_reports_update_on_existing_key() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path(), "s");
        run(
            &c,
            serde_json::json!({"action": "set", "key": "k", "value": "1"}),
        )
        .await
        .unwrap();
        let out = run(
            &c,
            serde_json::json!({"action": "set", "key": "k", "value": "2"}),
        )
        .await
        .unwrap();
        assert!(out.contains("Updated"));
    }

    #[tokio::test]
    async fn missing_key_or_value_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path(), "s");
        assert!(run(&c, serde_json::json!({"action": "get"})).await.is_err());
        assert!(run(&c, serde_json::json!({"action": "set", "key": "k"}))
            .await
            .is_err());
        assert!(run(&c, serde_json::json!({"action": "bogus", "key": "k"}))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn empty_list_is_reported() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path(), "s");
        let out = run(&c, serde_json::json!({"action": "list"}))
            .await
            .unwrap();
        assert!(out.contains("empty"));
    }
}
