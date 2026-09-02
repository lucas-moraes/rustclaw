mod config;
mod error;
mod harness;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "rustclaw",
    about = "RustClaw - Coding agent harness (OpenCode/Claude Code style)",
    version = "0.2.0"
)]
struct Args {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    Args::parse();

    // File-based config only (auth.json + config.json + rustclaw.json).
    // A missing API key is tolerated: the TUI handles onboarding.
    let config = config::Config::load();

    let cwd = std::env::current_dir()?;

    // UI selection: RUSTCLAW_UI=cli|tui (env override, not agent config),
    // else TUI when stdout is a terminal.
    match harness::ui::tui::ui_mode_from_env() {
        "cli" => {
            if !config.is_configured() {
                anyhow::bail!(
                    "no API token configured.\nRun the TUI in a terminal for onboarding: \
                     `rustclaw` then `/models` and `/auth <provider>`"
                );
            }
            harness::ui::cli::run(config, cwd).await?
        }
        "tui" => harness::ui::tui::run(config, cwd).await?,
        _ => {
            if harness::ui::tui::is_tty() {
                harness::ui::tui::run(config, cwd).await?;
            } else if !config.is_configured() {
                anyhow::bail!(
                    "no API token configured and no TTY for onboarding.\n\
                     Run `rustclaw` in a terminal, or set a token via the auth store."
                )
            } else {
                harness::ui::cli::run(config, cwd).await?
            }
        }
    }

    Ok(())
}
