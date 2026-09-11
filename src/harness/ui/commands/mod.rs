//! Shared slash-command handling for both the TUI and CLI surfaces.

pub mod apply_plan;
pub mod export;
pub mod memory;
pub(crate) mod permissions_cmd;
pub(crate) mod provider_cmd;
pub mod replay;
pub(crate) mod session_cmd;

use crate::harness::runtime::SessionRuntime;
use crate::harness::session::{Message, Role, Session};
use anyhow::Result;

/// Processes a slash command (line starts with `/`).
/// Returns `Ok(Some(output))` with one or more feedback lines, or
/// `Ok(None)` when the command produced no feedback. Returns `Exit` when the
/// user asked to quit.
pub enum CommandOutcome {
    /// Continue running; carries optional feedback lines.
    Continue(Vec<String>),
    /// User requested to exit the session.
    Exit,
}

/// Dispatches a slash command. `runtime` is mutable so commands may switch
/// provider/model (`/model`, `/provider`); `session` is mutated in place when
/// the command changes the current session (e.g. `/new`, `/resume`, `/agent`).
pub async fn handle(
    runtime: &mut SessionRuntime,
    session: &mut Session,
    line: &str,
) -> Result<CommandOutcome> {
    let mut parts = line.splitn(2, char::is_whitespace);
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();

    let mut out: Vec<String> = Vec::new();
    match cmd {
        "/help" => {
            out.push(
                "commands: /help /new /sessions /agent <name> /skills \
                  /compact /theme [name] /usage /memory /models /model <name> \
                  /provider <name> /provider add|rm|list /auth <provider> /settings \
                  /undo /diff /restore /fork [N] /apply-plan /image <path> /permissions /allow-all-permissions /mcp /jobs /record on|off|status /replay <file> /exit"
                    .to_string(),
            );
            out.push("keys: Ctrl+P palette · Ctrl+T theme · ? help · Ctrl+L clear".to_string());
        }
        "/settings" => {
            let c = &runtime.config;
            if arg.is_empty() {
                out.push(format!(
                    "settings · iterations {} · context {} · turn_timeout {}s · provider {} · model {}",
                    c.max_iterations, c.max_context_tokens, c.turn_timeout_secs, c.provider, c.model
                ));
                out.push(
                    "usage: /settings iterations <n> · context <n> · turn_timeout <secs>"
                        .to_string(),
                );
            } else {
                let mut parts = arg.split_whitespace();
                match parts.next() {
                    Some("iterations") => {
                        match parts.next().and_then(|v| v.parse::<usize>().ok()) {
                            Some(n) => match runtime.update_settings(Some(n), None, None) {
                                Ok(()) => out.push(format!("settings · max_iterations = {}", n)),
                                Err(e) => out.push(format!("[error] {}", e)),
                            },
                            None => out.push("usage: /settings iterations <n>".to_string()),
                        }
                    }
                    Some("context") => match parts.next().and_then(|v| v.parse::<usize>().ok()) {
                        Some(n) => match runtime.update_settings(None, Some(n), None) {
                            Ok(()) => out.push(format!("settings · max_context_tokens = {}", n)),
                            Err(e) => out.push(format!("[error] {}", e)),
                        },
                        None => out.push("usage: /settings context <tokens>".to_string()),
                    },
                    Some("turn_timeout") => {
                        match parts.next().and_then(|v| v.parse::<u64>().ok()) {
                            Some(n) => match runtime.update_settings(None, None, Some(n)) {
                                Ok(()) => out.push(format!("settings · turn_timeout_secs = {}", n)),
                                Err(e) => out.push(format!("[error] {}", e)),
                            },
                            None => out.push("usage: /settings turn_timeout <secs>".to_string()),
                        }
                    }
                    Some(other) => out.push(format!(
                        "unknown setting: {} (iterations · context · turn_timeout)",
                        other
                    )),
                    None => {}
                }
            }
        }
        "/theme" => {
            out.push(
                "themes: cyberclaw, aurora, ember, mono (use TUI /theme <name> or Ctrl+T)"
                    .to_string(),
            );
        }
        "/usage" | "/tokens" => {
            let ctx = session.approx_tokens();
            let max = runtime.config.max_context_tokens;
            let pct = (ctx * 100).checked_div(max).unwrap_or(0);
            out.push(format!("context · ~{} / {} tokens ({}%)", ctx, max, pct));
            out.push("full usage breakdown is shown in the TUI status bar".to_string());
        }
        "/new" | "/sessions" | "/agent" | "/skills" | "/compact" => {
            session_cmd::handle_session_cmd(runtime, session, cmd, arg, &mut out).await?;
        }
        "/memory" => {
            let args: Vec<&str> = arg.split_whitespace().collect();
            out.push(memory::handle_memory_command(runtime, &args)?);
        }
        "/apply-plan" => match apply_plan::handle_apply_plan_command(runtime, session) {
            Ok(lines) => out.extend(lines),
            Err(e) => out.push(format!("[error] apply-plan failed: {e:#}")),
        },
        "/image" => {
            if arg.is_empty() {
                out.push(
                    "usage: /image <path> — attach an image (png/jpeg/gif/webp) \
                          to the next prompt"
                        .to_string(),
                );
            } else {
                match crate::harness::session::image::load_image(arg) {
                    Ok(_) => {
                        let msg = Message::new(
                            Role::User,
                            vec![
                                crate::harness::session::Part::image(arg.to_string()),
                                crate::harness::session::Part::text("describe this image"),
                            ],
                        );
                        runtime
                            .store
                            .save_message(&session.id, &session.cwd, &msg)?;
                        session.push_message(msg);
                        out.push(format!(
                            "image attached: {} — sent with your next prompt",
                            arg
                        ));
                    }
                    Err(reason) => out.push(format!("[error] {}", reason)),
                }
            }
        }
        "/export" => match export::handle_export_command(runtime, session, arg) {
            Ok(lines) => out.extend(lines),
            Err(e) => out.push(format!("[error] export failed: {e:#}")),
        },
        "/diff" => {
            let cp = &runtime.checkpoints;
            let paths = if arg.is_empty() {
                cp.list()
            } else {
                vec![session.cwd.join(arg)]
            };
            if paths.is_empty() {
                out.push("no file checkpoints yet (files change after write/edit)".to_string());
            }
            for path in paths {
                match cp.diff_since_snapshot(&path) {
                    Ok(diff) => {
                        out.push(format!("diff {} (since snapshot):", path.display()));
                        if diff.trim().is_empty() {
                            out.push("  (no changes)".to_string());
                        } else {
                            for line in diff.lines() {
                                out.push(format!("  {line}"));
                            }
                        }
                    }
                    Err(e) => out.push(format!("[error] {e:#}")),
                }
            }
        }
        "/restore" => {
            let cp = &runtime.checkpoints;
            if arg.is_empty() {
                let paths = cp.list();
                if paths.is_empty() {
                    out.push("no file checkpoints yet".to_string());
                } else {
                    out.push(format!("checkpointed files ({}):", paths.len()));
                    for p in paths {
                        out.push(format!("  {}", p.display()));
                    }
                    out.push("usage: /restore <path>".to_string());
                }
            } else {
                let path = session.cwd.join(arg);
                match cp.restore(&path) {
                    Ok(msg) => out.push(msg),
                    Err(e) => out.push(format!("[error] {e:#}")),
                }
            }
        }
        "/models" | "/model" | "/provider" | "/auth" => {
            provider_cmd::handle_provider_cmd(runtime, cmd, arg, &mut out).await?;
        }
        "/mcp" => {
            let mut parts = arg.split_whitespace();
            let sub = parts.next().unwrap_or("");
            match runtime.mcp.as_ref() {
                None => {
                    out.push(
                        "no MCP servers configured (add ~/.local/share/rustclaw/mcp.json \
                         or `mcp` section in rustclaw.json)"
                            .to_string(),
                    );
                }
                Some(mgr) => match sub {
                    "" | "list" => {
                        let configured = mgr.configured().await;
                        if configured.is_empty() {
                            out.push("no MCP servers configured".to_string());
                        } else {
                            out.push(format!("mcp servers ({}):", configured.len()));
                            for (name, target, enabled) in configured {
                                let tools = mgr
                                    .tools()
                                    .await
                                    .iter()
                                    .filter(|t| t.server == name)
                                    .count();
                                out.push(format!(
                                    "  {} {} ({}{})",
                                    if enabled { "•" } else { "○" },
                                    name,
                                    target,
                                    if enabled {
                                        format!(", {} tools", tools)
                                    } else {
                                        ", disabled".to_string()
                                    }
                                ));
                            }
                        }
                    }
                    "status" => {
                        for (name, status) in mgr.status_snapshot().await {
                            let label = match status {
                                crate::harness::mcp::McpServerStatus::Connected => {
                                    "connected".to_string()
                                }
                                crate::harness::mcp::McpServerStatus::Failed(e) => {
                                    format!("failed: {e}")
                                }
                                crate::harness::mcp::McpServerStatus::Disabled => {
                                    "disabled".to_string()
                                }
                            };
                            out.push(format!("  {name}: {label}"));
                        }
                    }
                    "restart" => {
                        let name = parts.next().unwrap_or("");
                        if name.is_empty() {
                            out.push("usage: /mcp restart <name>".to_string());
                        } else {
                            match mgr.restart(name).await {
                                Ok(()) => out.push(format!("mcp server `{name}` restarted")),
                                Err(e) => out.push(format!("[error] {e:#}")),
                            }
                        }
                    }
                    other => out.push(format!("usage: /mcp list|status|restart (got `{other}`)")),
                },
            }
        }
        "/permissions" | "/allow-all-permissions" => {
            permissions_cmd::handle_permissions_cmd(runtime, cmd, arg, &mut out)?;
        }
        "/undo" => {
            // Revert the last user prompt and everything after it (replies +
            // tool results). Reuses the same truncation the TUI's revert action
            // performs, so the DB and in-memory session stay consistent.
            let last_user = session
                .messages
                .iter()
                .rposition(|m| m.role.as_str() == "user");
            match last_user {
                None => out.push("nothing to undo".to_string()),
                Some(idx) => {
                    let msg_id = session.messages[idx].id.clone();
                    match runtime
                        .store
                        .delete_messages_from(&session.id, &session.cwd, &msg_id)
                    {
                        Ok(()) => {
                            session.messages.truncate(idx);
                            match runtime.store.save_session(session) {
                                Ok(()) => {
                                    out.push("session reverted to before last prompt".to_string());
                                }
                                Err(e) => out.push(format!("[error] failed to save: {}", e)),
                            }
                        }
                        Err(e) => out.push(format!("[error] failed to revert: {}", e)),
                    }
                }
            }
        }
        "/fork" => {
            // Fork: copy messages 0..N (default: all) of the active session
            // into a brand-new session, then switch to it (same mechanism as
            // /sessions select).
            let n = if arg.is_empty() {
                None
            } else {
                match arg.parse::<usize>() {
                    Ok(n) => Some(n),
                    Err(_) => {
                        out.push(
                            "usage: /fork [N] — N must be a number of messages to copy".to_string(),
                        );
                        return Ok(CommandOutcome::Continue(out));
                    }
                }
            };
            if session.messages.is_empty() {
                out.push("nothing to fork: this session has no messages".to_string());
            } else {
                let take = n
                    .unwrap_or(session.messages.len())
                    .min(session.messages.len());
                let mut forked = runtime.store.create_session(&session.agent, &session.cwd)?;
                forked.skills = session.skills.clone();
                forked.title = session.title.clone();
                for msg in &session.messages[..take] {
                    // Message ids are globally unique (PRIMARY KEY), so the
                    // fork gets fresh ids while preserving role/parts/time.
                    let mut copy = msg.clone();
                    copy.id = crate::harness::session::new_id();
                    runtime.store.save_message(&forked.id, &forked.cwd, &copy)?;
                    forked.push_message(copy);
                }
                runtime.store.save_session(&forked)?;
                match runtime.load_session(&forked.id)? {
                    Some(mut loaded) => {
                        let note = if runtime.config.is_configured() {
                            match runtime.maybe_compact(&mut loaded, false, None).await {
                                Ok(n) if n > 0 => format!(" · auto-compacted {n} message(s)"),
                                Ok(_) => String::new(),
                                Err(e) => format!(" · auto-compact failed: {e}"),
                            }
                        } else {
                            String::new()
                        };
                        *session = loaded;
                        out.push(format!(
                            "forked session {} → {} ({} message(s) copied){note}",
                            session.id, forked.id, take
                        ));
                    }
                    None => out.push(format!(
                        "forked session {} ({} message(s) copied) — use /sessions select {}",
                        forked.id, take, forked.id
                    )),
                }
            }
        }
        "/record" => {
            let mut parts = arg.split_whitespace();
            match parts.next().unwrap_or("") {
                "on" => {
                    let path = replay::recording_path(&session.id);
                    match runtime.event_recorder.start(path.clone()) {
                        Ok(existing) => out.push(format!(
                            "recording events → {} (append; {} line(s) already there)",
                            path.display(),
                            existing
                        )),
                        Err(e) => out.push(format!("[error] {e:#}")),
                    }
                }
                "off" => match runtime.event_recorder.stop() {
                    Some((path, n)) => out.push(format!(
                        "recording stopped: {} event(s) → {}",
                        n,
                        path.display()
                    )),
                    None => out.push("not recording".to_string()),
                },
                "status" => match runtime.event_recorder.status() {
                    Some((path, n)) => out.push(format!("recording: {} ({} event(s))", path, n)),
                    None => out.push("not recording (use /record on)".to_string()),
                },
                _ => out.push("usage: /record on|off|status".to_string()),
            }
        }
        "/replay" => {
            if arg.is_empty() {
                out.push(
                    "usage: /replay <events.jsonl> — re-render a recorded event stream".to_string(),
                );
            } else {
                let path = session.cwd.join(arg);
                let path = if path.exists() {
                    path
                } else {
                    std::path::PathBuf::from(arg)
                };
                match replay::read_events(&path) {
                    Ok((events, skipped)) => {
                        let mut rendered = 0usize;
                        for ev in &events {
                            for line in replay::render_event_lines(ev) {
                                out.push(line);
                            }
                            if !matches!(
                                ev,
                                crate::harness::event::HarnessEvent::MessageUpdated { .. }
                            ) {
                                rendered += 1;
                            }
                        }
                        let note = if skipped > 0 {
                            format!(" · {} malformed line(s) skipped", skipped)
                        } else {
                            String::new()
                        };
                        out.push(format!(
                            "replayed {} event(s) from {}{}",
                            rendered,
                            path.display(),
                            note
                        ));
                    }
                    Err(e) => out.push(format!("[error] {e:#}")),
                }
            }
        }
        "/exit" | "/quit" => return Ok(CommandOutcome::Exit),
        _ => out.push(format!("unknown command: {} (try /help)", cmd)),
    }
    Ok(CommandOutcome::Continue(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuntimeConfig;
    use crate::harness::session::{Message, Part, Role};
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
    async fn test_undo_reverts_last_turn() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        let mut session = rt.create_session("build").await.unwrap();

        // First user turn + assistant reply.
        let m1 = Message::user("first prompt");
        session.push_message(m1.clone());
        rt.store
            .save_message(&session.id, &session.cwd, &m1)
            .unwrap();
        let a1 = Message::new(Role::Assistant, vec![Part::text("first reply")]);
        session.push_message(a1.clone());
        rt.store
            .save_message(&session.id, &session.cwd, &a1)
            .unwrap();

        // Second user turn + assistant reply (the one to undo).
        let m2 = Message::user("second prompt");
        session.push_message(m2.clone());
        rt.store
            .save_message(&session.id, &session.cwd, &m2)
            .unwrap();
        let a2 = Message::new(Role::Assistant, vec![Part::text("second reply")]);
        session.push_message(a2.clone());
        rt.store
            .save_message(&session.id, &session.cwd, &a2)
            .unwrap();

        assert_eq!(session.messages.len(), 4);

        let outcome = handle(&mut rt, &mut session, "/undo").await.unwrap();
        let CommandOutcome::Continue(lines) = outcome else {
            panic!("expected Continue");
        };
        assert!(lines.iter().any(|l| l.contains("reverted")));

        // Only the first user + assistant remain.
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].parts[0].as_text(), Some("first prompt"));
        assert_eq!(session.messages[1].parts[0].as_text(), Some("first reply"));

        // Persisted state matches.
        let loaded = rt
            .store
            .load_session(&session.id, &session.cwd)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.messages.len(), 2);
    }

    #[tokio::test]
    async fn test_undo_empty_session_reports_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        let mut session = rt.create_session("build").await.unwrap();

        let outcome = handle(&mut rt, &mut session, "/undo").await.unwrap();
        let CommandOutcome::Continue(lines) = outcome else {
            panic!("expected Continue");
        };
        assert!(lines.iter().any(|l| l.contains("nothing to undo")));
    }

    #[tokio::test]
    async fn test_permissions_set_list_rm_persists() {
        use crate::harness::permission::Rule;
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        let mut session = rt.create_session("build").await.unwrap();

        // set
        let outcome = handle(&mut rt, &mut session, "/permissions set bash allow")
            .await
            .unwrap();
        let CommandOutcome::Continue(lines) = outcome else {
            panic!("expected Continue");
        };
        assert!(lines.iter().any(|l| l.contains("bash = allow")));

        // Persisted in rustclaw.json.
        let proj = crate::harness::project::config_file::ProjectConfig::load(dir.path());
        assert_eq!(proj.permission.tools.get("bash"), Some(&Rule::Allow));

        // list
        let outcome = handle(&mut rt, &mut session, "/permissions").await.unwrap();
        let CommandOutcome::Continue(lines) = outcome else {
            panic!("expected Continue");
        };
        assert!(lines.iter().any(|l| l.contains("bash = allow")));

        // rm
        let outcome = handle(&mut rt, &mut session, "/permissions rm bash")
            .await
            .unwrap();
        let CommandOutcome::Continue(lines) = outcome else {
            panic!("expected Continue");
        };
        assert!(lines.iter().any(|l| l.contains("removed")));

        let proj = crate::harness::project::config_file::ProjectConfig::load(dir.path());
        assert!(!proj.permission.tools.contains_key("bash"));
    }

    #[tokio::test]
    async fn test_allow_all_permissions_grants_and_persists() {
        use crate::harness::permission::Rule;
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        let mut session = rt.create_session("build").await.unwrap();

        let outcome = handle(&mut rt, &mut session, "/allow-all-permissions")
            .await
            .unwrap();
        let CommandOutcome::Continue(lines) = outcome else {
            panic!("expected Continue");
        };
        assert!(lines.iter().any(|l| l.contains("all permissions granted")));

        // Every builtin tool is persisted as allow in rustclaw.json.
        let proj = crate::harness::project::config_file::ProjectConfig::load(dir.path());
        for tool in crate::harness::permission::ALL_TOOLS {
            assert_eq!(
                proj.permission.tools.get(*tool),
                Some(&Rule::Allow),
                "tool `{}` should be persisted as allow",
                tool
            );
        }

        // The live engine now allows a mutating tool inside the project.
        let cwd = std::path::Path::new(dir.path());
        assert_eq!(
            rt.permission.check(
                "edit",
                Some(&dir.path().join("x.rs").to_string_lossy()),
                cwd
            ),
            crate::harness::permission::PermissionDecision::Allow
        );
        // Paths outside the project still escalate to Ask.
        assert_eq!(
            rt.permission.check("edit", Some("/etc/passwd"), cwd),
            crate::harness::permission::PermissionDecision::Ask
        );
    }

    #[tokio::test]
    async fn test_permissions_set_rejects_unknown_rule() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        let mut session = rt.create_session("build").await.unwrap();

        let outcome = handle(&mut rt, &mut session, "/permissions set bash maybe")
            .await
            .unwrap();
        let CommandOutcome::Continue(lines) = outcome else {
            panic!("expected Continue");
        };
        assert!(lines.iter().any(|l| l.contains("unknown rule")));
    }

    async fn seed_session(rt: &mut SessionRuntime, session: &mut Session, n: usize) {
        for i in 0..n {
            let m = Message::user(format!("prompt {i}"));
            session.push_message(m.clone());
            rt.store
                .save_message(&session.id, &session.cwd, &m)
                .unwrap();
        }
    }

    #[tokio::test]
    async fn test_fork_copies_first_n_messages() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        let mut session = rt.create_session("build").await.unwrap();
        seed_session(&mut rt, &mut session, 5).await;

        let original_id = session.id.clone();
        let outcome = handle(&mut rt, &mut session, "/fork 3").await.unwrap();
        let CommandOutcome::Continue(lines) = outcome else {
            panic!("expected Continue");
        };
        assert!(lines.iter().any(|l| l.contains("3 message(s) copied")));

        // Active session switched to the fork, with exactly 3 messages.
        assert_ne!(session.id, original_id);
        assert_eq!(session.messages.len(), 3);
        assert_eq!(session.messages[0].parts[0].as_text(), Some("prompt 0"));
        assert_eq!(session.messages[2].parts[0].as_text(), Some("prompt 2"));

        // Original session intact with all 5 messages.
        let original = rt
            .store
            .load_session(&original_id, &session.cwd)
            .unwrap()
            .unwrap();
        assert_eq!(original.messages.len(), 5);
    }

    #[tokio::test]
    async fn test_fork_default_copies_all_messages() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        let mut session = rt.create_session("build").await.unwrap();
        seed_session(&mut rt, &mut session, 5).await;

        let outcome = handle(&mut rt, &mut session, "/fork").await.unwrap();
        let CommandOutcome::Continue(lines) = outcome else {
            panic!("expected Continue");
        };
        assert!(lines.iter().any(|l| l.contains("5 message(s) copied")));
        assert_eq!(session.messages.len(), 5);
    }

    #[tokio::test]
    async fn test_fork_empty_session_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        let mut session = rt.create_session("build").await.unwrap();

        let outcome = handle(&mut rt, &mut session, "/fork").await.unwrap();
        let CommandOutcome::Continue(lines) = outcome else {
            panic!("expected Continue");
        };
        assert!(lines.iter().any(|l| l.contains("nothing to fork")));
    }

    #[tokio::test]
    async fn test_fork_invalid_n_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        let mut session = rt.create_session("build").await.unwrap();
        seed_session(&mut rt, &mut session, 2).await;

        let outcome = handle(&mut rt, &mut session, "/fork abc").await.unwrap();
        let CommandOutcome::Continue(lines) = outcome else {
            panic!("expected Continue");
        };
        assert!(lines.iter().any(|l| l.contains("must be a number")));
    }
}
