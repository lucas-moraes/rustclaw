//! Session management commands: /new, /sessions, /agent, /skills, /compact.
//!
//! Extracted from `mod.rs`.

use crate::harness::runtime::SessionRuntime;
use crate::harness::session::Session;
use anyhow::Result;

pub(crate) async fn handle_session_cmd(
    runtime: &mut SessionRuntime,
    session: &mut Session,
    cmd: &str,
    arg: &str,
    out: &mut Vec<String>,
) -> Result<()> {
    match cmd {
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
                    let marker = if s.parent_id.is_some() { "↳ " } else { "" };
                    out.push(format!(
                        "{}{}  [{}]  {} msgs — {}",
                        marker, s.id, s.agent, s.message_count, label
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
                let mut names: Vec<String> = runtime.custom_agents.keys().cloned().collect();
                for b in ["build", "plan", "explore", "general", "chat-free"] {
                    if !runtime.custom_agents.contains_key(b) {
                        names.push(b.to_string());
                    }
                }
                names.sort();
                out.push(format!("available: {}", names.join(", ")));
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
        // Invariant: the dispatcher in `ui/commands/mod.rs` only routes
        // `/new`, `/sessions`, `/agent`, `/skills` and `/compact` here
        // (line ~110), so no other `cmd` can reach this match. Fires loudly
        // if a new session command is added to the help but not routed.
        _ => unreachable!("handle_session_cmd called with unknown cmd: {}", cmd),
    }
    Ok(())
}
