//! TUI surface for the harness (ratatui + crossterm).
//! Entry point selects TUI unless stdout is not a terminal or `--ui cli`.

pub mod anim;
pub mod app;
pub mod askers;
pub mod draw;
pub mod input;
pub mod markdown;
pub mod palette;
pub mod theme;

use crate::harness::permission::PermissionEngine;
use crate::harness::runtime::SessionRuntime;
use crate::harness::tool::context::UserAsker;
use anyhow::Result;
use std::sync::Arc;

/// Whether the current process should use the TUI (a real terminal on stdin+stdout).
pub fn is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal() && std::io::stdin().is_terminal()
}

/// UI mode from `RUSTCLAW_UI=cli|tui` (default tui when TTY).
pub fn ui_mode_from_env() -> &'static str {
    let m = std::env::var("RUSTCLAW_UI")
        .unwrap_or_default()
        .to_lowercase();
    match m.as_str() {
        "cli" | "tui" => Box::leak(m.into_boxed_str()),
        _ => "auto",
    }
}

/// Runs the interactive TUI.
pub async fn run(config: crate::config::Config, cwd: std::path::PathBuf) -> Result<()> {
    let db_path = data_db_path();
    let permission = Arc::new(PermissionEngine::default());

    let (asker, permission_rx) = askers::TuiAsker::new(permission.clone());
    let (user_asker, question_rx) = askers::TuiUserAsker::new();

    let registry = crate::harness::runtime::build_default_registry();
    let user_asker: Arc<dyn UserAsker> = user_asker;
    let runtime = SessionRuntime::from_legacy(&config, registry, &db_path, asker, user_asker)?;

    let session = runtime
        .create_session(&runtime.config.default_agent)
        .await?;

    app::run_tui(runtime, session, cwd, permission_rx, question_rx).await
}

fn data_db_path() -> std::path::PathBuf {
    if let Some(dir) = dirs::data_local_dir() {
        return dir.join("rustclaw").join("harness.db");
    }
    std::path::PathBuf::from("rustclaw-harness.db")
}
