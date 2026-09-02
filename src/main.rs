mod config;
mod error;
mod harness;

use clap::Parser;
use dotenv::dotenv;
use std::path::Path;
use tracing::info;

#[derive(Parser, Debug)]
#[command(
    name = "rustclaw",
    about = "RustClaw - Coding agent harness (OpenCode/Claude Code style)",
    version = "0.2.0"
)]
struct Args {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fn find_and_load_env() {
        if let Ok(exe_path) = std::env::current_exe() {
            let mut dir = exe_path.parent().map(|p| p.to_path_buf());
            while let Some(d) = dir {
                let config_env = d.join("config/.env");
                if config_env.exists() {
                    dotenv::from_path(&config_env).ok();
                    return;
                }
                dir = d.parent().map(|p| p.to_path_buf());
            }
        }

        let config_env = Path::new("config/.env");
        if config_env.exists() {
            dotenv::from_path(config_env).ok();
        } else {
            dotenv().ok();
        }
    }

    find_and_load_env();
    Args::parse();

    let config = config::Config::from_env()?;

    info!(
        "Config loaded - provider: {}, model: {}",
        config.provider, config.model
    );

    let cwd = std::env::current_dir()?;

    // UI selection: RUSTCLAW_UI=cli|tui, else TUI when stdout is a terminal.
    match harness::ui::tui::ui_mode_from_env() {
        "cli" => harness::ui::cli::run(config, cwd).await?,
        "tui" => harness::ui::tui::run(config, cwd).await?,
        _ => {
            if harness::ui::tui::is_tty() {
                harness::ui::tui::run(config, cwd).await?;
            } else {
                harness::ui::cli::run(config, cwd).await?;
            }
        }
    }

    Ok(())
}
