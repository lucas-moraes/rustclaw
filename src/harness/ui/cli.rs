//! Streaming CLI for the harness. REPL with slash commands, live event output,
//! and stdin-based permission/user prompts.

use crate::harness::event::{EventReceiver, HarnessEvent, ToolStatus};
use crate::harness::permission::PermissionEngine;
use crate::harness::runtime::SessionRuntime;
use crate::harness::tool::context::{PermissionAskInput, PermissionAsker, UserAsker};
use crate::harness::ui::commands;
use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct CliAsker {
    pub engine: Arc<PermissionEngine>,
    always: AtomicBool,
}

impl CliAsker {
    pub fn new(engine: Arc<PermissionEngine>) -> Self {
        Self {
            engine,
            always: AtomicBool::new(false),
        }
    }
}

#[async_trait::async_trait]
impl PermissionAsker for CliAsker {
    async fn ask(&self, request: PermissionAskInput) -> bool {
        if self.always.load(Ordering::Relaxed) {
            return true;
        }
        eprint!("\n[permission] {} ", request.tool);
        if let Some(p) = &request.path {
            eprint!("(path: {}) ", p);
        }
        eprint!("{}\n  allow? [y]es/[n]o/[a]lways: ", request.args_summary);
        let answer = blocking_read_line().unwrap_or_default().to_lowercase();
        match answer.as_str() {
            "y" | "yes" | "" => true,
            "a" | "always" => {
                self.always.store(true, Ordering::Relaxed);
                self.engine.set_always_allow(&request.tool);
                true
            }
            _ => false,
        }
    }
}

pub struct CliUserAsker;

#[async_trait::async_trait]
impl UserAsker for CliUserAsker {
    async fn ask(&self, question: String, options: Vec<String>) -> Option<String> {
        eprint!("\n[question] {}\n", question);
        for (i, opt) in options.iter().enumerate() {
            eprintln!("  {}. {}", i + 1, opt);
        }
        eprint!("> ");
        let answer = blocking_read_line().unwrap_or_default();
        if answer.is_empty() {
            return None;
        }
        // Allow selecting an option by number.
        if let Ok(num) = answer.trim().parse::<usize>() {
            if num >= 1 && num <= options.len() {
                return Some(options[num - 1].clone());
            }
        }
        Some(answer)
    }
}

/// Reads a line on a blocking task to avoid stalling the runtime.
fn blocking_read_line() -> Option<String> {
    std::io::stdin().lines().next().and_then(|r| r.ok())
}

/// Consumes events and renders them to the terminal.
pub async fn print_events(mut rx: EventReceiver) {
    let mut streaming_text = false;
    while let Some(event) = rx.recv().await {
        match event {
            HarnessEvent::TextDelta { delta, .. } => {
                if !streaming_text {
                    print!("\nassistant: ");
                    streaming_text = true;
                }
                print!("{}", delta);
                flush_stdout();
            }
            HarnessEvent::ReasoningDelta { .. } => {
                // rendered faint; skip for the streaming CLI
            }
            HarnessEvent::MessageUpdated { .. } => {
                if streaming_text {
                    println!();
                    streaming_text = false;
                    flush_stdout();
                }
            }
            HarnessEvent::ToolStart { name, input, .. } => {
                if streaming_text {
                    println!();
                    streaming_text = false;
                }
                println!("\n● {}", tool_card(&name, &input.to_string()));
                flush_stdout();
            }
            HarnessEvent::ToolEnd {
                name,
                status,
                title,
                ..
            } => {
                let mark = match status {
                    ToolStatus::Completed => "✓",
                    ToolStatus::Error => "✗",
                    _ => "·",
                };
                let label = if title.is_empty() { name } else { title };
                println!("  {} {}", mark, label);
                flush_stdout();
            }
            HarnessEvent::CompactionStarted { .. } => {
                println!("\n[compacting context…]");
            }
            HarnessEvent::CompactionFinished {
                summarized_messages,
                ..
            } => {
                println!(
                    "[compaction: summarized {} message(s)]",
                    summarized_messages
                );
            }
            HarnessEvent::Error { message, .. } => {
                println!("\n[error] {}", message);
            }
            HarnessEvent::PermissionAsk { .. } | HarnessEvent::PermissionResolved { .. } => {}
            HarnessEvent::RunStarted { .. }
            | HarnessEvent::RunFinished { .. }
            | HarnessEvent::UserMessage { .. } => {}
        }
    }
}

fn tool_card(name: &str, args: &str) -> String {
    let preview = crate::harness::session::preview(args, 100);
    format!("{} {}", name, preview)
}

fn flush_stdout() {
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

/// Runs the interactive harness CLI.
pub async fn run(config: crate::config::Config, cwd: std::path::PathBuf) -> Result<()> {
    let db_path = data_db_path();
    let permission = Arc::new(PermissionEngine::default());
    let asker = Arc::new(CliAsker::new(permission.clone()));
    let user_asker = Arc::new(CliUserAsker);

    let registry = crate::harness::runtime::build_default_registry();
    let mut runtime = SessionRuntime::from_legacy_in(
        &cwd,
        &config,
        registry,
        &db_path,
        permission.clone(),
        asker,
        user_asker,
    )?;

    // Reopen the most recently used session of this project when one exists;
    // otherwise start a fresh session.
    let mut session = match runtime.load_last_session()? {
        Some(s) => s,
        None => {
            runtime
                .create_session(&runtime.config.default_agent)
                .await?
        }
    };

    // Auto-compact oversized sessions on open so the first turn doesn't start
    // already over the context budget.
    if runtime.config.is_configured() {
        match runtime.maybe_compact(&mut session, false, None).await {
            Ok(n) if n > 0 => {
                println!("[auto-compact] summarized {n} message(s) on open");
            }
            Ok(_) => {}
            Err(e) => eprintln!("[warn] auto-compact on open failed: {e}"),
        }
    }

    println!(
        "RustClaw harness — agent: {}, model: {}",
        session.agent, runtime.config.model
    );
    println!("Working directory: {}", cwd.display());

    // Choose skills for this session's memory (empty = none). Only prompt
    // interactively when stdin is a TTY; when input is piped, rely on the
    // RUSTCLAW_SKILLS env var so the first piped line is not consumed.
    if !runtime.skills.skills.is_empty() {
        let names = runtime.skills.names().join(", ");
        println!("Available skills (session memory): {}", names);
        if let Ok(env) = std::env::var("RUSTCLAW_SKILLS") {
            for id in env.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                if runtime.skills.get(id).is_some() {
                    session
                        .skills
                        .push(crate::harness::skill::SessionSkill::new(id, true));
                }
            }
            println!("skills from RUSTCLAW_SKILLS: {}", session.skills.len());
        } else if std::io::stdin().is_terminal() {
            println!("skills (comma-separated ids, empty = none):");
            if let Some(Ok(line)) = std::io::stdin().lines().next() {
                if !line.trim().is_empty() {
                    for id in line.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                        if runtime.skills.get(id).is_some() {
                            session
                                .skills
                                .push(crate::harness::skill::SessionSkill::new(id, true));
                        }
                    }
                }
            }
        }
        runtime.store.save_session(&session)?;
        if !session.skills.is_empty() {
            println!(
                "session skills: {}",
                session
                    .skills
                    .iter()
                    .map(|s| s.skill_id.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    println!("Type /help for commands.\n");

    let mut editor = DefaultEditor::new()?;

    loop {
        let line = match editor.readline(&format!("{} › ", session.agent)) {
            Ok(line) => line,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        // Slash commands are handled separately; everything else is a prompt.
        if line.trim().starts_with('/') {
            match commands::handle(&mut runtime, &mut session, line.trim()).await? {
                crate::harness::ui::commands::CommandOutcome::Exit => break,
                crate::harness::ui::commands::CommandOutcome::Continue(lines) => {
                    for l in lines {
                        println!("{}", l);
                    }
                }
            }
            continue;
        }

        let (tx, rx) = crate::harness::event::event_channel();
        let printer = tokio::spawn(print_events(rx));

        let result = runtime
            .prompt(
                &mut session,
                &tx,
                &line,
                crate::harness::tool::context::AbortSignal::new(),
                None,
            )
            .await;
        // Drop the sender so the printer's recv() returns and the task can end.
        drop(tx);
        let _ = printer.await;

        match result {
            Ok(r) => {
                println!("\n[iterations: {}]", r.iterations);
            }
            Err(e) => {
                eprintln!("\n[error] {}", e);
            }
        }
        let _ = editor.add_history_entry(&line);
    }

    println!("\nbye.");
    Ok(())
}

fn data_db_path() -> std::path::PathBuf {
    if let Some(dir) = dirs::data_local_dir() {
        return dir.join("rustclaw").join("harness.db");
    }
    std::path::PathBuf::from("rustclaw-harness.db")
}
