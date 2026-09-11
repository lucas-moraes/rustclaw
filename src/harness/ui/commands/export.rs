//! `/export md|json [path]` — dump the current session transcript to disk.

use crate::harness::runtime::SessionRuntime;
use crate::harness::session::{preview, Part, Session};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Parses `/export` arguments: `[md|json] [path]`.
/// Returns (format, output path).
fn parse_args(session: &Session, arg: &str) -> (String, PathBuf) {
    let mut parts = arg.split_whitespace();
    let first = parts.next().unwrap_or("");
    let (fmt, rest) = match first {
        "json" => ("json".to_string(), parts.next()),
        "md" | "markdown" => ("md".to_string(), parts.next()),
        // First token is not a format → it's the path; format defaults to md.
        "" => ("md".to_string(), None),
        other => ("md".to_string(), Some(other)),
    };
    let ext = if fmt == "json" { "json" } else { "md" };
    let path = match rest {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(format!(
            "rustclaw-session-{}.{}",
            session.id.get(..8).unwrap_or(&session.id),
            ext
        )),
    };
    (fmt, path)
}

/// Removes `<project-memory>...</project-memory>` blocks (runtime-injected
/// memory) from message text before export.
fn strip_memory_blocks(text: &str) -> String {
    crate::harness::project::memory::strip_memory_blocks(text)
}

/// Removes `<system-reminder>...</system-reminder>` blocks (runtime-injected
/// reminders) from message text before export.
fn strip_system_reminders(text: &str) -> String {
    const START: &str = "<system-reminder>";
    const END: &str = "</system-reminder>";
    if !text.contains(START) {
        return text.to_string();
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(START) {
        out.push_str(&rest[..start]);
        match rest[start..].find(END) {
            Some(end) => rest = &rest[start + START.len() + end + END.len()..],
            None => {
                // Unterminated block: drop the remainder.
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// Renders the session transcript as human-readable Markdown.
pub fn render_markdown(session: &Session) -> String {
    let mut out = String::new();
    out.push_str(&format!("# RustClaw session {}\n", session.id));
    out.push_str(&format!(
        "- agent: {} · model: {} · created: {}\n\n",
        session.agent,
        session.title.clone().unwrap_or_default(),
        session.created_at.to_rfc3339()
    ));
    for msg in &session.messages {
        out.push_str(&format!("## {}\n", msg.role.as_str()));
        for part in &msg.parts {
            match part {
                Part::Text { text } => {
                    let text = strip_memory_blocks(&strip_system_reminders(text));
                    if text.is_empty() {
                        continue;
                    }
                    out.push_str(&text);
                    out.push('\n');
                }
                Part::Reasoning { .. } => {}
                Part::Image { .. } => {}
                Part::Tool(t) => {
                    out.push_str(&format!("### tool: {}\n", t.name));
                    out.push_str(&format!(
                        "```\n{}\n→ {}\n```\n",
                        preview(&t.input.to_string(), 200),
                        preview(&t.output, 200)
                    ));
                }
            }
        }
        out.push('\n');
    }
    out
}

/// Handles `/export md|json [path]`. Returns feedback lines.
pub fn handle_export_command(
    runtime: &SessionRuntime,
    session: &Session,
    arg: &str,
) -> Result<Vec<String>> {
    let (fmt, path) = parse_args(session, arg);
    let messages = if session.messages.is_empty() {
        // Fall back to the persisted transcript (e.g. right after /resume).
        runtime
            .store
            .load_session(&session.id, &session.cwd)?
            .map(|s| s.messages)
            .unwrap_or_default()
    } else {
        session.messages.clone()
    };
    if messages.is_empty() {
        return Ok(vec!["nothing to export (session is empty)".to_string()]);
    }

    let export_session = Session {
        messages,
        ..session.clone()
    };
    let content = if fmt == "json" {
        serde_json::to_string_pretty(&export_session).context("failed to serialize session")?
    } else {
        render_markdown(&export_session)
    };

    let count = export_session.messages.len();
    std::fs::write(&path, content)
        .with_context(|| format!("failed to write {}", path.display()))?;
    let abs = absolute(&path);
    Ok(vec![format!(
        "exported {} message(s) → {} ({})",
        count,
        abs.display(),
        if fmt == "json" { "json" } else { "markdown" }
    )])
}

/// Best-effort absolute path for display.
fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuntimeConfig;
    use crate::harness::session::{Message, Role, ToolPart, ToolStatus};
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

    fn sample_session(cwd: &Path) -> Session {
        let mut session = Session::new("build", cwd.to_path_buf());
        session.push_message(Message::user("fix the login bug"));
        session.push_message(Message::new(
            Role::Assistant,
            vec![
                Part::text("looking into it"),
                Part::Tool(ToolPart {
                    id: "t1".to_string(),
                    name: "bash".to_string(),
                    input: serde_json::json!({"command": "ls src"}),
                    status: ToolStatus::Completed,
                    output: "main.rs\nconfig.rs".to_string(),
                    title: String::new(),
                    error: None,
                }),
            ],
        ));
        session
    }

    #[tokio::test]
    async fn test_export_markdown_to_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        let rt = test_runtime(dir.path()).unwrap();
        let session = sample_session(dir.path());
        let out_path = dir.path().join("out.md");

        let lines = handle_export_command(&rt, &session, &out_path.to_string_lossy()).unwrap();
        assert!(lines[0].contains("exported 2 message(s)"), "{lines:?}");

        let md = std::fs::read_to_string(&out_path).unwrap();
        assert!(md.contains("# RustClaw session"), "{md}");
        assert!(md.contains("## user"), "{md}");
        assert!(md.contains("fix the login bug"), "{md}");
        assert!(md.contains("## assistant"), "{md}");
        assert!(md.contains("### tool: bash"), "{md}");
        assert!(md.contains("→ main.rs"), "{md}");
    }

    #[tokio::test]
    async fn test_export_markdown_skips_memory_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let rt = test_runtime(dir.path()).unwrap();
        let mut session = sample_session(dir.path());
        session.push_message(Message::user(
            "<project-memory>\n- [x] fact\n</project-memory>\nreal question",
        ));
        let out_path = dir.path().join("out.md");
        handle_export_command(&rt, &session, &out_path.to_string_lossy()).unwrap();
        let md = std::fs::read_to_string(&out_path).unwrap();
        assert!(!md.contains("project-memory"), "{md}");
        assert!(md.contains("real question"), "{md}");
    }

    #[tokio::test]
    async fn test_export_json_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let rt = test_runtime(dir.path()).unwrap();
        let session = sample_session(dir.path());
        let out_path = dir.path().join("out.json");

        let lines = handle_export_command(
            &rt,
            &session,
            &format!("json {}", out_path.to_string_lossy()),
        )
        .unwrap();
        assert!(lines[0].contains("json"), "{lines:?}");

        let raw = std::fs::read_to_string(&out_path).unwrap();
        let parsed: Session = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].role, Role::User);
        assert_eq!(
            parsed.messages[0].parts[0].as_text(),
            Some("fix the login bug")
        );
        assert_eq!(parsed.messages[1].role, Role::Assistant);
    }

    #[tokio::test]
    async fn test_export_default_filename_and_empty_session() {
        let dir = tempfile::tempdir().unwrap();
        let rt = test_runtime(dir.path()).unwrap();
        let session = rt.create_session("build").await.unwrap();

        let lines = handle_export_command(&rt, &session, "").unwrap();
        assert!(lines[0].contains("nothing to export"), "{lines:?}");
    }
}
