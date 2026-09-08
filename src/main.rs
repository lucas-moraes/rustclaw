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

    // Set up tracing subscriber. Default: pretty format at INFO level.
    // RUSTCLAW_LOG=json enables JSON output; RUSTCLAW_LOG=debug|trace changes level.
    init_tracing();

    // File-based config only (auth.json + config.json + rustclaw.json).
    // A missing API key is tolerated: the TUI handles onboarding.
    let config = config::RuntimeConfig::load();

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

/// Initializes the `tracing` subscriber.
///
/// - `RUSTCLAW_LOG=json` → JSON output (machine-readable).
/// - `RUSTCLAW_LOG=debug|trace|warn|error` → sets the level (default: info).
/// - Otherwise → pretty format for dev.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let env = std::env::var("RUSTCLAW_LOG").unwrap_or_default();
    let level = if env.is_empty() || env == "json" {
        "info".to_string()
    } else {
        env.clone()
    };

    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));

    // Always write logs to a file: stderr writes corrupt the TUI alt-screen.
    let log_path = dirs::data_local_dir()
        .map(|d| d.join("rustclaw").join("log.txt"))
        .unwrap_or_else(|| std::path::PathBuf::from("rustclaw.log"));
    let _ = std::fs::create_dir_all(log_path.parent().unwrap_or(std::path::Path::new(".")));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .expect("failed to open rustclaw log file");

    if env == "json" {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .with_writer(std::sync::Mutex::new(file))
            .try_init();
    } else {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .pretty()
            .with_writer(std::sync::Mutex::new(file))
            .try_init();
    }
}
