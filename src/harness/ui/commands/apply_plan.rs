//! `/apply-plan` — plan → build handoff: takes the last assistant reply (the
//! plan produced by the `plan` agent), injects it as a user context message and
//! switches the active agent to `build`.

use crate::harness::runtime::SessionRuntime;
use crate::harness::session::{Message, Part, Role, Session};
use anyhow::Result;

/// Wraps the plan text as the user message injected into the session.
pub fn plan_prompt(plan: &str) -> String {
    format!("Here is the plan to implement:\n\n{plan}\n\nImplement it step by step.")
}

/// Finds the last assistant message with non-empty textual content.
fn last_plan_text(session: &Session) -> Option<String> {
    session
        .messages
        .iter()
        .rev()
        .filter(|m| m.role == Role::Assistant)
        .filter_map(|m| {
            let text: String = m
                .parts
                .iter()
                .filter_map(|p| match p {
                    Part::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            let text = text.trim().to_string();
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        })
        .next()
}

/// Handles `/apply-plan`. Returns feedback lines.
pub fn handle_apply_plan_command(
    runtime: &mut SessionRuntime,
    session: &mut Session,
) -> Result<Vec<String>> {
    let plan = match last_plan_text(session) {
        Some(p) => p,
        None => return Ok(vec!["no plan found — run /agent plan first".to_string()]),
    };

    let msg = Message::user(plan_prompt(&plan));
    runtime
        .store
        .save_message(&session.id, &session.cwd, &msg)?;
    session.push_message(msg);

    let spec = runtime.resolve_agent("build");
    session.agent = spec.name.clone();

    Ok(vec![
        "plan injected as context (user message)".to_string(),
        "agent -> build".to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuntimeConfig;
    use crate::harness::tool::context::{PermissionAskInput, PermissionAsker, UserAsker};
    use crate::harness::tool::registry::ToolRegistry;
    use std::sync::Arc;

    struct AllowAsker;
    #[async_trait::async_trait]
    impl PermissionAsker for AllowAsker {
        async fn ask(&self, _req: PermissionAskInput) -> bool {
            true
        }
    }
    struct NoUserAsker;
    #[async_trait::async_trait]
    impl UserAsker for NoUserAsker {
        async fn ask(&self, _q: String, _o: Vec<String>) -> Option<String> {
            None
        }
    }

    fn test_runtime(dir: &std::path::Path) -> Result<SessionRuntime> {
        let http = crate::harness::provider::HttpConfig {
            client: crate::harness::provider::build_http_client(),
            base_url: "https://api.deepinfra.com/v1/openai".to_string(),
            api_key: "sk-initial-test-key-123456".to_string(),
        };
        let provider =
            crate::harness::provider::opencode_go::build_provider("deepinfra", http, false)?;
        let db = dir.join("test.db");
        SessionRuntime::new_in(
            dir,
            provider,
            ToolRegistry::builder().build(),
            RuntimeConfig {
                model: "deepseek-ai/DeepSeek-V4-Flash-0731".to_string(),
                provider: "deepinfra".to_string(),
                base_url: "https://api.deepinfra.com/v1/openai".to_string(),
                api_key: "sk-initial-test-key-123456".to_string(),
                ..Default::default()
            },
            &db,
            Arc::new(crate::harness::permission::PermissionEngine::default()),
            Arc::new(AllowAsker),
            Arc::new(NoUserAsker),
        )
    }

    #[tokio::test]
    async fn test_apply_plan_injects_message_and_switches_agent() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        let mut session = rt.create_session("plan").await.unwrap();

        session.push_message(Message::user("design the feature"));
        let plan = Message::new(Role::Assistant, vec![Part::text("1. do X\n2. do Y")]);
        session.push_message(plan.clone());
        rt.store
            .save_message(&session.id, &session.cwd, &session.messages[0])
            .unwrap();
        rt.store
            .save_message(&session.id, &session.cwd, &plan)
            .unwrap();

        let lines = handle_apply_plan_command(&mut rt, &mut session).unwrap();
        assert!(
            lines.iter().any(|l| l.contains("plan injected")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("agent -> build")),
            "{lines:?}"
        );

        assert_eq!(session.agent, "build");
        assert_eq!(session.messages.len(), 3);
        let injected = session.messages.last().unwrap();
        assert_eq!(injected.role, Role::User);
        let text = injected.parts[0].as_text().unwrap();
        assert!(text.starts_with("Here is the plan to implement:"), "{text}");
        assert!(text.contains("1. do X"), "{text}");
        assert!(text.ends_with("Implement it step by step."), "{text}");

        // Persisted transcript contains the injected message.
        let loaded = rt
            .store
            .load_session(&session.id, &session.cwd)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.messages.len(), 3);
        assert_eq!(
            loaded.messages[2].parts[0].as_text(),
            injected.parts[0].as_text()
        );
    }

    #[tokio::test]
    async fn test_apply_plan_without_assistant_reply_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        let mut session = rt.create_session("plan").await.unwrap();
        session.push_message(Message::user("just a question"));

        let lines = handle_apply_plan_command(&mut rt, &mut session).unwrap();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("no plan found — run /agent plan first")),
            "{lines:?}"
        );
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.agent, "plan");
    }

    #[tokio::test]
    async fn test_apply_plan_uses_last_assistant_message() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        let mut session = rt.create_session("plan").await.unwrap();

        session.push_message(Message::user("q1"));
        session.push_message(Message::new(Role::Assistant, vec![Part::text("old plan")]));
        session.push_message(Message::user("q2"));
        session.push_message(Message::new(Role::Assistant, vec![Part::text("new plan")]));

        handle_apply_plan_command(&mut rt, &mut session).unwrap();
        let injected = session.messages.last().unwrap().parts[0]
            .as_text()
            .unwrap()
            .to_string();
        assert!(injected.contains("new plan"), "{injected}");
        assert!(!injected.contains("old plan"), "{injected}");
    }
}
