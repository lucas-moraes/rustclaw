//! `message_send` / `message_inbox`: point-to-point messaging between agents.
//!
//! A subagent can report a finding to its parent (or to a sibling) without
//! waiting for the parent's turn to end, and the parent can drain what arrived
//! with `message_inbox`. Delivery is pull-based, so `message_send` never blocks.

use super::{Tool, ToolResult};
use crate::harness::tool::context::ToolContext;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct SendArgs {
    to: String,
    body: String,
}

/// Sends a message to another session of the same project.
pub struct MessageSendTool;

#[async_trait::async_trait]
impl Tool for MessageSendTool {
    fn name(&self) -> &str {
        "message_send"
    }
    fn description(&self) -> &str {
        "Envia uma mensagem para outra sessão do mesmo projeto (ex.: o agente pai \
ou um subagente irmão). Não bloqueia: a entrega é feita quando o destinatário \
chamar `message_inbox`."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Session id do destinatário (o id do agente pai aparece no prompt do subagente)."
                },
                "body": {
                    "type": "string",
                    "description": "Conteúdo da mensagem (achado, pergunta, status)."
                }
            },
            "required": ["to", "body"]
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let parsed: SendArgs = serde_json::from_value(args)
            .map_err(|e| format!("invalid arguments for `message_send`: {}", e))?;
        let to = parsed.to.trim();
        let body = parsed.body.trim();
        if to.is_empty() {
            return Err("`message_send` requires a non-empty `to`.".to_string());
        }
        if body.is_empty() {
            return Err("`message_send` requires a non-empty `body`.".to_string());
        }
        let bus = ctx
            .message_bus
            .as_ref()
            .ok_or("message bus is not available in this session")?;
        let id = bus
            .send(&ctx.session_id, to, body)
            .map_err(|e| format!("failed to send message: {e}"))?;
        Ok(ToolResult::simple(
            format!("message_send → {to}"),
            format!("Message #{id} queued for session `{to}`."),
        ))
    }
}

/// Drains the unread messages addressed to the current session.
pub struct MessageInboxTool;

#[async_trait::async_trait]
impl Tool for MessageInboxTool {
    fn name(&self) -> &str {
        "message_inbox"
    }
    fn description(&self) -> &str {
        "Lê e esvazia as mensagens não lidas endereçadas a esta sessão. \
Chame periodicamente em tarefas longas para receber instruções do agente pai."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    async fn execute(&self, _args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let bus = ctx
            .message_bus
            .as_ref()
            .ok_or("message bus is not available in this session")?;
        let messages = bus
            .inbox(&ctx.session_id)
            .map_err(|e| format!("failed to read inbox: {e}"))?;
        if messages.is_empty() {
            return Ok(ToolResult::simple("message_inbox", "No unread messages."));
        }
        // `inbox` already drained everything, so this is only a sanity check
        // that the drain and the counter agree.
        debug_assert_eq!(bus.unread_count(&ctx.session_id).unwrap_or(0), 0);
        let count = messages.len();
        let mut out = format!("{count} unread message(s):\n");
        for m in messages {
            out.push_str(&format!(
                "\n[#{} from {} at {}]\n{}\n",
                m.id, m.from, m.created_at, m.body
            ));
        }
        Ok(ToolResult::simple(
            format!("message_inbox ({} message(s))", count),
            out,
        ))
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

    #[tokio::test]
    async fn send_then_inbox_roundtrips_between_sessions() {
        let d = tempfile::tempdir().unwrap();
        let parent = ctx(d.path(), "parent");
        let child = ctx(d.path(), "child");

        let out = MessageSendTool
            .execute(
                serde_json::json!({"to": "parent", "body": "found 3 modules"}),
                &child,
            )
            .await
            .unwrap();
        assert!(out.output.contains("queued"));

        let inbox = MessageInboxTool
            .execute(serde_json::json!({}), &parent)
            .await
            .unwrap();
        assert!(inbox.output.contains("found 3 modules"));
        assert!(inbox.output.contains("child"));

        // Drained: a second read is empty.
        let again = MessageInboxTool
            .execute(serde_json::json!({}), &parent)
            .await
            .unwrap();
        assert!(again.output.contains("No unread messages"));
    }

    #[tokio::test]
    async fn send_rejects_empty_fields() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path(), "s");
        assert!(MessageSendTool
            .execute(serde_json::json!({"to": "  ", "body": "x"}), &c)
            .await
            .is_err());
        assert!(MessageSendTool
            .execute(serde_json::json!({"to": "p", "body": "  "}), &c)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn tools_fail_cleanly_without_a_bus() {
        let d = tempfile::tempdir().unwrap();
        let mut c = ctx(d.path(), "s");
        c.message_bus = None;
        assert!(MessageSendTool
            .execute(serde_json::json!({"to": "p", "body": "x"}), &c)
            .await
            .is_err());
        assert!(MessageInboxTool
            .execute(serde_json::json!({}), &c)
            .await
            .is_err());
    }
}
