mod agent;
mod api;
mod app_state;
mod app_store;
mod auth;
mod browser;
mod cli;
mod config;
mod error;
mod features;
mod i18n;
mod memory;
mod plugins;
mod reminder_executor;
mod security;
mod skills;
mod tavily;
mod telegram;
mod tools;
mod utils;
mod workspace_trust;

use clap::Parser;
use dotenv::dotenv;
use std::path::Path;
use tracing::info;

#[derive(Parser, Debug)]
#[command(
    name = "rustclaw",
    about = "RustClaw - Agente AI com memória persistente",
    version = "0.1.0"
)]
struct Args {
    #[arg(short, long, default_value = "cli")]
    mode: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load environment variables FIRST, before any env::var calls
    // Try to find config/.env by traversing up from executable or current directory
    fn find_and_load_env() {
        // First, try to find config/.env starting from executable location
        if let Ok(exe_path) = std::env::current_exe() {
            let mut dir = exe_path.parent().map(|p| p.to_path_buf());
            while let Some(d) = dir {
                let config_env = d.join("config/.env");
                if config_env.exists() {
                    dotenv::from_path(&config_env).ok();
                    return;
                }
                // Go up one directory
                dir = d.parent().map(|p| p.to_path_buf());
            }
        }
        
        // Fallback: try current directory
        let config_env = Path::new("config/.env");
        if config_env.exists() {
            dotenv::from_path(config_env).ok();
        } else {
            dotenv().ok();
        }
    }
    
    find_and_load_env();

    let args = Args::parse();

    info!("Starting RustClaw in {} mode", args.mode);

    let config = config::Config::from_env()?;

    info!(
        "Config loaded - provider: {}, model: {}",
        config.provider, config.model
    );

    match args.mode.as_str() {
        "cli" => {
            cli::run(config).await?;
        }
        "telegram" => {
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
