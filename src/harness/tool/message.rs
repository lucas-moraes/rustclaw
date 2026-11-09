//! `message_send` / `message_inbox`: point-to-point messaging between agents.
//!
//! A subagent can report a finding to its parent (or to a sibling) without
//! waiting for the parent's turn to end, and the parent can drain what arrived
//! with `message_inbox`. Delivery is pull-based, so `message_send` never blocks.

use super::{Tool, ToolResult};
use crate::harness::swarm::bus::MessageKind;
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
        let count = bus
            .unread_count(&ctx.session_id)
            .map_err(|e| format!("failed to read inbox: {e}"))?;
        let messages = bus
            .inbox(&ctx.session_id)
            .map_err(|e| format!("failed to read inbox: {e}"))?;
        if messages.is_empty() {
            return Ok(ToolResult::simple("message_inbox", "No unread messages."));
        }
        let mut out = format!("{count} unread message(s):\n");
        for m in messages {
            let tag = match m.kind {
                MessageKind::Ask => " — QUESTION, answer with `message_reply`",
                MessageKind::Reply => " — reply",
                MessageKind::Note => "",
            };
            out.push_str(&format!(
                "\n[#{} from {} at {}{}]\n{}\n",
                m.id, m.from, m.created_at, tag, m.body
            ));
        }
        Ok(ToolResult::simple(
            format!("message_inbox ({} message(s))", count),
            out,
        ))
    }
}

#[derive(Deserialize)]
struct AskArgs {
    to: String,
    question: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

/// Asks another session a question and waits for the answer.
pub struct MessageAskTool;

/// How long `message_ask` waits before giving up, in seconds.
const DEFAULT_ASK_TIMEOUT_SECS: u64 = 120;

#[async_trait::async_trait]
impl Tool for MessageAskTool {
    fn name(&self) -> &str {
        "message_ask"
    }
    fn description(&self) -> &str {
        "Faz uma pergunta a outra sessão (normalmente o agente pai) e espera a \
resposta. Use quando estiver bloqueado por uma decisão que não é sua: \
ambiguidade de requisito, escolha de abordagem, permissão."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Session id de quem deve responder."
                },
                "question": {
                    "type": "string",
                    "description": "A pergunta, objetiva e autocontida."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Quanto tempo esperar pela resposta (padrão 120s)."
                }
            },
            "required": ["to", "question"]
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let parsed: AskArgs = serde_json::from_value(args)
            .map_err(|e| format!("invalid arguments for `message_ask`: {}", e))?;
        let to = parsed.to.trim();
        let question = parsed.question.trim();
        if to.is_empty() {
            return Err("`message_ask` requires a non-empty `to`.".to_string());
        }
        if question.is_empty() {
            return Err("`message_ask` requires a non-empty `question`.".to_string());
        }
        let bus = ctx
            .message_bus
            .as_ref()
            .ok_or("message bus is not available in this session")?;
        let secs = parsed
            .timeout_secs
            .unwrap_or(DEFAULT_ASK_TIMEOUT_SECS)
            .min(600);
        let timeout = std::time::Duration::from_secs(secs);
        let answer = bus
            .ask(&ctx.session_id, to, question, timeout)
            .await
            .map_err(|e| format!("failed to ask `{to}`: {e}"))?;
        let title = format!("message_ask → {to}");
        match answer {
            Some(body) => Ok(ToolResult::simple(
                title,
                format!("Answer from `{to}`:\n\n{body}"),
            )),
            // Not an error: the recipient may simply be busy. The question
            // stays on the bus and can still be answered later.
            None => Ok(ToolResult::simple(
                title,
                format!(
                    "No answer from `{to}` within {secs}s. The question is still \
queued — call `message_inbox` later to check for a late reply, or proceed \
with your best judgement."
                ),
            )),
        }
    }
}

#[derive(Deserialize)]
struct ReplyArgs {
    ask_id: i64,
    answer: String,
}

/// Answers a pending `message_ask`.
pub struct MessageReplyTool;

#[async_trait::async_trait]
impl Tool for MessageReplyTool {
    fn name(&self) -> &str {
        "message_reply"
    }
    fn description(&self) -> &str {
        "Responde uma pergunta pendente recebida via `message_ask` (o #id aparece \
no `message_inbox`). Use sempre que um subagente estiver esperando por você."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ask_id": {
                    "type": "integer",
                    "description": "O #id da pergunta, como mostrado no `message_inbox`."
                },
                "answer": {
                    "type": "string",
                    "description": "A resposta, autocontida (o outro agente não vê seu contexto)."
                }
            },
            "required": ["ask_id", "answer"]
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let parsed: ReplyArgs = serde_json::from_value(args)
            .map_err(|e| format!("invalid arguments for `message_reply`: {}", e))?;
        let answer = parsed.answer.trim();
        if answer.is_empty() {
            return Err("`message_reply` requires a non-empty `answer`.".to_string());
        }
        let bus = ctx
            .message_bus
            .as_ref()
            .ok_or("message bus is not available in this session")?;
        let original = bus
            .get(parsed.ask_id)
            .map_err(|e| format!("failed to load question #{}: {e}", parsed.ask_id))?
            .ok_or_else(|| format!("question #{} does not exist", parsed.ask_id))?;
        if original.kind != MessageKind::Ask {
            return Err(format!(
                "message #{} is a {}, not a question",
                parsed.ask_id,
                original.kind.as_str()
            ));
        }
        bus.send_kind(
            &ctx.session_id,
            &original.from,
            answer,
            MessageKind::Reply,
            Some(parsed.ask_id),
        )
        .map_err(|e| format!("failed to send reply: {e}"))?;
        // The asker has the reply now; consuming the question keeps
        // `pending_asks` honest if it asks again later.
        bus.mark_read(parsed.ask_id)
            .map_err(|e| format!("failed to close question #{}: {e}", parsed.ask_id))?;
        Ok(ToolResult::simple(
            format!("message_reply → {}", original.from),
            format!(
                "Reply sent to `{}` for question #{}.",
                original.from, parsed.ask_id
            ),
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
            delivery: Default::default(),
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

    #[tokio::test]
    async fn ask_blocks_until_the_parent_replies() {
        let d = tempfile::tempdir().unwrap();
        let child = ctx(d.path(), "child");
        let parent = ctx(d.path(), "parent");

        // The parent answers once the question shows up in `pending_asks`,
        // exactly like the processor would when draining mid-turn.
        let bus = parent.message_bus.clone().unwrap();
        let responder = tokio::spawn(async move {
            for _ in 0..200 {
                let asks = bus.pending_asks("parent").unwrap();
                if let Some(ask) = asks.first() {
                    let out = MessageReplyTool
                        .execute(
                            serde_json::json!({"ask_id": ask.id, "answer": "use option B"}),
                            &parent,
                        )
                        .await
                        .unwrap();
                    assert!(out.output.contains("Reply sent"));
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            panic!("question never reached the parent");
        });

        let out = MessageAskTool
            .execute(
                serde_json::json!({"to": "parent", "question": "A or B?", "timeout_secs": 5}),
                &child,
            )
            .await
            .unwrap();
        responder.await.unwrap();
        assert!(out.output.contains("use option B"), "got: {}", out.output);
    }

    #[tokio::test]
    async fn ask_times_out_and_says_the_question_is_still_queued() {
        let d = tempfile::tempdir().unwrap();
        let child = ctx(d.path(), "child");
        let out = MessageAskTool
            .execute(
                serde_json::json!({"to": "parent", "question": "anyone?", "timeout_secs": 0}),
                &child,
            )
            .await
            .unwrap();
        assert!(out.output.contains("No answer"));
        assert!(out.output.contains("still"));
    }

    #[tokio::test]
    async fn inbox_surfaces_pending_questions() {
        let d = tempfile::tempdir().unwrap();
        let child = ctx(d.path(), "child");
        let parent = ctx(d.path(), "parent");

        // A note plus an unanswered question: the question must be called out.
        MessageSendTool
            .execute(
                serde_json::json!({"to": "parent", "body": "progress"}),
                &child,
            )
            .await
            .unwrap();
        let bus = child.message_bus.clone().unwrap();
        bus.send_kind(
            "child",
            "parent",
            "which crate?",
            crate::harness::swarm::bus::MessageKind::Ask,
            None,
        )
        .unwrap();

        let inbox = MessageInboxTool
            .execute(serde_json::json!({}), &parent)
            .await
            .unwrap();
        assert!(inbox.output.contains("QUESTION"));
        assert!(inbox.output.contains("which crate?"));
    }

    #[tokio::test]
    async fn reply_rejects_a_non_question() {
        let d = tempfile::tempdir().unwrap();
        let child = ctx(d.path(), "child");
        let parent = ctx(d.path(), "parent");
        MessageSendTool
            .execute(serde_json::json!({"to": "parent", "body": "note"}), &child)
            .await
            .unwrap();
        let id = parent
            .message_bus
            .as_ref()
            .unwrap()
            .inbox("parent")
            .unwrap()[0]
            .id;
        let err = MessageReplyTool
            .execute(serde_json::json!({"ask_id": id, "answer": "x"}), &parent)
            .await
            .unwrap_err();
        assert!(err.contains("not a question"), "got: {err}");
    }
}
