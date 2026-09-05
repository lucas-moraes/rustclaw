//! Shared slash-command handling for both the TUI and CLI surfaces.

pub mod memory;

use crate::harness::runtime::SessionRuntime;
use crate::harness::session::Session;
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

/// Human-readable label for a permission rule.
fn rule_label(rule: crate::harness::permission::Rule) -> &'static str {
    match rule {
        crate::harness::permission::Rule::Allow => "allow",
        crate::harness::permission::Rule::Ask => "ask",
        crate::harness::permission::Rule::Deny => "deny",
    }
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
                "commands: /help /new /sessions /agent <name> \
                  /compact /theme [name] /usage /memory /models /model <name> \
                  /provider <name> /provider add|rm|list /auth <provider> /settings \
                  /undo /permissions /exit"
                    .to_string(),
            );
            out.push("keys: Ctrl+P palette · Ctrl+T theme · ? help · Ctrl+L clear".to_string());
        }
        "/settings" => {
            let c = &runtime.config;
            if arg.is_empty() {
                out.push(format!(
                    "settings · iterations {} · context {} · provider {} · model {}",
                    c.max_iterations, c.max_context_tokens, c.provider, c.model
                ));
                out.push("usage: /settings iterations <n> · context <n>".to_string());
            } else {
                let mut parts = arg.split_whitespace();
                match parts.next() {
                    Some("iterations") => {
                        match parts.next().and_then(|v| v.parse::<usize>().ok()) {
                            Some(n) => match runtime.update_settings(Some(n), None) {
                                Ok(()) => out.push(format!("settings · max_iterations = {}", n)),
                                Err(e) => out.push(format!("[error] {}", e)),
                            },
                            None => out.push("usage: /settings iterations <n>".to_string()),
                        }
                    }
                    Some("context") => match parts.next().and_then(|v| v.parse::<usize>().ok()) {
                        Some(n) => match runtime.update_settings(None, Some(n)) {
                            Ok(()) => out.push(format!("settings · max_context_tokens = {}", n)),
                            Err(e) => out.push(format!("[error] {}", e)),
                        },
                        None => out.push("usage: /settings context <tokens>".to_string()),
                    },
                    Some(other) => {
                        out.push(format!("unknown setting: {} (iterations · context)", other))
                    }
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
            let pct = if max == 0 { 0 } else { (ctx * 100) / max };
            out.push(format!("context · ~{} / {} tokens ({}%)", ctx, max, pct));
            out.push("full usage breakdown is shown in the TUI status bar".to_string());
        }
        "/new" => {
            *session = runtime
                .create_session(&runtime.config.default_agent)
                .await?;
            out.push(format!(
                "new session: {} (agent: {})",
                session.id, session.agent
            ));
        }
        "/sessions" => {
            let sub = arg.split_whitespace().next().unwrap_or("");
            if sub.is_empty() {
                for s in runtime.list_sessions()? {
                    let label = s.title.clone().unwrap_or_else(|| s.preview.clone());
                    out.push(format!(
                        "{}  [{}]  {} msgs — {}",
                        s.id, s.agent, s.message_count, label
                    ));
                }
                if out.is_empty() {
                    out.push("no sessions yet".to_string());
                }
                out.push(
                    "usage: /sessions delete <id> · rename <id> <title> · select <id>".to_string(),
                );
            } else {
                let rest = arg.split_whitespace().collect::<Vec<_>>();
                match (sub, rest.get(1).copied()) {
                    ("delete", Some(id)) => match runtime.delete_session(id) {
                        Ok(()) => out.push(format!("deleted session {}", id)),
                        Err(e) => out.push(format!("[error] {}", e)),
                    },
                    ("rename", Some(id)) => {
                        let title = arg.split_whitespace().skip(2).collect::<Vec<_>>().join(" ");
                        if title.is_empty() {
                            out.push("usage: /sessions rename <id> <title>".to_string());
                        } else if let Err(e) = runtime.set_session_title(id, &title) {
                            out.push(format!("[error] {}", e));
                        } else {
                            if session.id == id {
                                session.title = Some(title.clone());
                            }
                            out.push(format!("renamed session {} → {}", id, title));
                        }
                    }
                    ("select", Some(id)) => match runtime.load_session(id)? {
                        Some(mut loaded) => {
                            let note = if runtime.config.is_configured() {
                                match runtime.maybe_compact(&mut loaded, false, None).await {
                                    Ok(n) if n > 0 => {
                                        format!(" · auto-compacted {n} message(s)")
                                    }
                                    Ok(_) => String::new(),
                                    Err(e) => format!(" · auto-compact failed: {e}"),
                                }
                            } else {
                                String::new()
                            };
                            *session = loaded;
                            out.push(format!(
                                "selected session {} ({}){note}",
                                session.id, session.agent
                            ));
                        }
                        None => out.push(format!("session not found: {}", id)),
                    },
                    _ => {
                        out.push(
                            "usage: /sessions [delete <id>|rename <id> <title>|select <id>]"
                                .to_string(),
                        );
                    }
                }
            }
        }
        "/agent" => {
            if arg.is_empty() {
                out.push(format!("current agent: {}", session.agent));
                out.push("available: build, plan, explore, general".to_string());
            } else {
                let spec = runtime.resolve_agent(arg);
                session.agent = spec.name.clone();
                out.push(format!("agent -> {}", spec.name));
            }
        }
        "/skills" => {
            if runtime.skills.skills.is_empty() {
                out.push("no skills discovered (look for .agents/skills/SKILL.md or RUSTCLAW_SKILLS_DIR)".to_string());
            } else {
                let current: Vec<String> =
                    session.skills.iter().map(|s| s.skill_id.clone()).collect();
                out.push(format!(
                    "session skills ({}): {}",
                    current.len(),
                    if current.is_empty() {
                        "none".to_string()
                    } else {
                        current.join(", ")
                    }
                ));
                out.push(format!("available: {}", runtime.skills.names().join(", ")));
            }
        }
        "/compact" => match runtime.maybe_compact(session, true, None).await {
            Ok(0) => out.push("nothing to compact".to_string()),
            Ok(n) => out.push(format!("compacted {} message(s)", n)),
            Err(e) => out.push(format!("[error] compact failed: {e}")),
        },
        "/memory" => {
            let args: Vec<&str> = arg.split_whitespace().collect();
            out.push(memory::handle_memory_command(runtime, &args)?);
        }
        "/models" => {
            use crate::harness::provider::catalog;
            use crate::harness::provider::user_store::UserProviders;
            let mut parts = arg.split_whitespace();
            let sub = parts.next().unwrap_or("");
            match sub {
                "" => {
                    out.push(format!(
                        "current: provider `{}` · model `{}`",
                        runtime.config.provider, runtime.config.model
                    ));
                    out.push(format!(
                        "providers: {} · usage: /models <provider> · /models add <provider> <model>",
                        catalog::provider_names().join(", ")
                    ));
                }
                "add" => {
                    let provider = parts.next().unwrap_or("");
                    let model = parts.next().unwrap_or("");
                    if provider.is_empty() || model.is_empty() {
                        out.push("usage: /models add <provider> <model>".to_string());
                    } else {
                        let mut store = UserProviders::load();
                        if store.add_model(provider, model) {
                            match store.save() {
                                Ok(()) => out.push(format!(
                                    "model `{}` added to provider `{}`",
                                    model, provider
                                )),
                                Err(e) => out.push(format!("[error] {}", e)),
                            }
                        } else {
                            out.push(format!(
                                "provider `{}` not found or model already present",
                                provider
                            ));
                        }
                    }
                }
                _ => {
                    let models = catalog::models_for(sub);
                    if models.is_empty() {
                        out.push(format!("unknown provider: {}", sub));
                    } else {
                        out.push(format!("models for {} ({}):", sub, models.len()));
                        for m in models {
                            out.push(format!("  {}", m));
                        }
                        out.push("switch with /provider <name> or /model <name>".to_string());
                    }
                }
            }
        }
        "/model" => {
            if arg.is_empty() {
                out.push(format!(
                    "current model: {} (provider {}) · usage: /model <name>",
                    runtime.config.model, runtime.config.provider
                ));
            } else {
                let provider = runtime.config.provider.clone();
                runtime.switch_model(&provider, arg)?;
                out.push(format!("model → {} ({})", arg, runtime.provider.name()));
                out.push("selection saved to rustclaw.json (this project)".to_string());
                if !runtime.has_token_for(&provider) {
                    out.push(format!(
                        "no token for provider `{}` — use /auth {} to add one",
                        provider, provider
                    ));
                }
            }
        }
        "/provider" => {
            use crate::harness::provider::catalog;
            use crate::harness::provider::user_store::{UserProvider, UserProviders};
            let mut parts = arg.split_whitespace();
            let sub = parts.next().unwrap_or("");
            match sub {
                "" => {
                    out.push(format!(
                        "current provider: {} (model {}) · usage: /provider <name>",
                        runtime.config.provider, runtime.config.model
                    ));
                    out.push(format!(
                        "providers: {} · manage: /provider add|rm|list",
                        catalog::provider_names().join(", ")
                    ));
                }
                "add" => {
                    let name = parts.next().unwrap_or("");
                    let base_url = parts.next().unwrap_or("");
                    let default_model = parts.next().unwrap_or("");
                    if name.is_empty() || base_url.is_empty() {
                        out.push(
                            "usage: /provider add <name> <base_url> [default_model]".to_string(),
                        );
                    } else {
                        let mut store = UserProviders::load();
                        let replaced = store.upsert(UserProvider {
                            name: name.to_string(),
                            base_url: base_url.to_string(),
                            default_model: default_model.to_string(),
                            models: if default_model.is_empty() {
                                Vec::new()
                            } else {
                                vec![default_model.to_string()]
                            },
                        });
                        match store.save() {
                            Ok(()) => out.push(format!(
                                "provider `{}` {} (providers.json)",
                                name,
                                if replaced { "updated" } else { "added" }
                            )),
                            Err(e) => out.push(format!("[error] {}", e)),
                        }
                    }
                }
                "rm" => {
                    let name = parts.next().unwrap_or("");
                    if name.is_empty() {
                        out.push("usage: /provider rm <name>".to_string());
                    } else {
                        let mut store = UserProviders::load();
                        if store.remove(name) {
                            match store.save() {
                                Ok(()) => out.push(format!("provider `{}` removed", name)),
                                Err(e) => out.push(format!("[error] {}", e)),
                            }
                        } else {
                            out.push(format!("no user provider named `{}`", name));
                        }
                    }
                }
                "list" => {
                    let all = catalog::all_providers();
                    out.push(format!("providers ({}):", all.len()));
                    for p in all {
                        let tag = if p.user_defined { " (custom)" } else { "" };
                        out.push(format!(
                            "  {}{} · {} · default `{}`",
                            p.name, tag, p.base_url, p.default_model
                        ));
                    }
                }
                _ => {
                    if let Some(default_model) = catalog::default_model(sub) {
                        runtime.switch_model(sub, &default_model)?;
                        out.push(format!("provider → {} · model → {}", sub, default_model));
                        out.push("selection saved to rustclaw.json (this project)".to_string());
                        if !runtime.has_token_for(sub) {
                            out.push(format!(
                                "no token for provider `{}` — use /auth {} to add one",
                                sub, sub
                            ));
                        }
                    } else {
                        out.push(format!(
                            "unknown provider: {} (options: {})",
                            sub,
                            catalog::provider_names().join(", ")
                        ));
                    }
                }
            }
        }
        "/auth" => {
            use crate::harness::auth::AuthStore;
            // No argument → update the token for the current provider/model.
            let provider = if arg.is_empty() {
                runtime.config.provider.clone()
            } else {
                arg.to_string()
            };
            if provider.is_empty() {
                let store = AuthStore::load();
                let names = store.entries.keys().cloned().collect::<Vec<_>>();
                out.push("usage: /auth <provider> — stored providers:".to_string());
                if names.is_empty() {
                    out.push("  (none)".to_string());
                } else {
                    out.push(format!("  {}", names.join(", ")));
                }
            } else {
                // Prompt goes straight to stdout so it shows before blocking on input.
                println!("paste the API key for `{}`: ", provider);
                let key = tokio::task::spawn_blocking(|| -> String {
                    let mut s = String::new();
                    if std::io::stdin().read_line(&mut s).is_ok() {
                        s.trim().to_string()
                    } else {
                        String::new()
                    }
                })
                .await
                .unwrap_or_default();
                if key.trim().is_empty() {
                    out.push("empty token — auth cancelled".to_string());
                } else {
                    let mut store = AuthStore::load();
                    store.set_key(&provider, key.trim());
                    match store.save() {
                        Ok(()) => out.push(format!(
                            "token saved for `{}` (auth.json, chmod 600)",
                            provider
                        )),
                        Err(e) => out.push(format!("[error] failed to save token: {}", e)),
                    }
                }
            }
        }
        "/permissions" => {
            let mut parts = arg.split_whitespace();
            let sub = parts.next().unwrap_or("");
            match sub {
                "" | "list" => {
                    let rules = runtime.permission_rules();
                    if rules.is_empty() {
                        out.push("no per-tool permission rules (defaults apply)".to_string());
                    } else {
                        out.push(format!("permission rules ({}):", rules.len()));
                        for (tool, rule) in rules {
                            out.push(format!("  {} = {}", tool, rule_label(rule)));
                        }
                    }
                    out.push(
                        "usage: /permissions set <tool> <allow|ask|deny> · rm <tool>".to_string(),
                    );
                }
                "set" => {
                    let tool = parts.next().unwrap_or("");
                    let rule = parts.next().unwrap_or("");
                    if tool.is_empty() || rule.is_empty() {
                        out.push("usage: /permissions set <tool> <allow|ask|deny>".to_string());
                    } else {
                        let parsed = match rule.to_lowercase().as_str() {
                            "allow" | "a" => Some(crate::harness::permission::Rule::Allow),
                            "ask" => Some(crate::harness::permission::Rule::Ask),
                            "deny" | "d" => Some(crate::harness::permission::Rule::Deny),
                            _ => None,
                        };
                        match parsed {
                            Some(r) => match runtime.set_permission_rule(tool, r) {
                                Ok(()) => out.push(format!(
                                    "permission: {} = {} (saved to rustclaw.json)",
                                    tool,
                                    rule_label(r)
                                )),
                                Err(e) => out.push(format!("[error] {}", e)),
                            },
                            None => {
                                out.push(format!("unknown rule: {} (allow · ask · deny)", rule))
                            }
                        }
                    }
                }
                "rm" | "remove" => {
                    let tool = parts.next().unwrap_or("");
                    if tool.is_empty() {
                        out.push("usage: /permissions rm <tool>".to_string());
                    } else {
                        match runtime.remove_permission_rule(tool) {
                            Ok(true) => out.push(format!(
                                "permission rule for `{}` removed (falls back to default)",
                                tool
                            )),
                            Ok(false) => out.push(format!("no rule for tool `{}`", tool)),
                            Err(e) => out.push(format!("[error] {}", e)),
                        }
                    }
                }
                _ => out.push(format!("unknown subcommand: {} (list · set · rm)", sub)),
            }
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
        "/exit" | "/quit" => return Ok(CommandOutcome::Exit),
        _ => out.push(format!("unknown command: {} (try /help)", cmd)),
    }
    Ok(CommandOutcome::Continue(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::runtime::HarnessConfig;
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
        let provider = crate::harness::provider::opencode_go::build_provider("deepinfra", http)?;
        let db = dir.join("test.db");
        SessionRuntime::new_in(
            dir,
            provider,
            ToolRegistry::builder().build(),
            HarnessConfig {
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
        assert!(proj.permission.tools.get("bash").is_none());
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
}
