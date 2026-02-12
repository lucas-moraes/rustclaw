mod agent;
mod cli;
mod config;
mod memory;
mod reminder_executor;
mod skills;
mod tavily;
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
    
    #[arg(short, long, default_value = "cli")]
    mode: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    
    dotenv().ok();

    
    let args = Args::parse();

    info!("Starting RustClaw in {} mode", args.mode);

    
    let config = config::Config::from_env()?;

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
