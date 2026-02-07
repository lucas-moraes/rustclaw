mod agent;
mod cli;
mod config;
mod memory;
mod telegram;
mod tools;

use clap::Parser;
use dotenv::dotenv;
use tracing::info;

#[derive(Parser, Debug)]
#[command(
    name = "rustclaw",
    about = "RustClaw - Agente AI com memória persistente",
    version = "0.1.0"
)]
struct Args {
    /// Modo de execução: cli ou telegram
    #[arg(short, long, default_value = "cli")]
    mode: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load environment variables
    dotenv().ok();

    // Parse command line arguments
    let args = Args::parse();

    info!("Starting RustClaw in {} mode", args.mode);

    // Load config (needed for both modes)
    let config = config::Config::from_env()?;

    match args.mode.as_str() {
        "cli" => {
            // Run in CLI mode
            cli::run(config).await?;
        }
        "telegram" => {
            // Run in Telegram bot mode
            telegram::TelegramBot::run(config).await?;
        }
        _ => {
            eprintln!("❌ Modo inválido: {}", args.mode);
            eprintln!("Use: --mode cli ou --mode telegram");
            std::process::exit(1);
        }
    }

    Ok(())
}
