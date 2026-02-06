mod agent;
mod config;
mod tools;

use agent::Agent;
use config::Config;
use dotenv::dotenv;
use std::io::{self, Write};
use tools::echo::EchoTool;
use tools::ToolRegistry;
use tracing::{info, Level};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load environment variables
    dotenv().ok();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("Iniciando RustClaw...");

    // Load config
    let config = Config::from_env()?;
    info!("Configuração carregada. Modelo: {}", config.model);

    // Create tool registry and register tools
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(EchoTool));
    info!("Ferramentas registradas");

    // Create agent
    let mut agent = Agent::new(config, tools);

    // CLI loop
    println!("╔════════════════════════════════════╗");
    println!("║        RustClaw v0.1.0             ║");
    println!("║   Fase 1: Core Agent + Tools       ║");
    println!("╚════════════════════════════════════╝");
    println!();
    println!("Digite mensagens (ou 'sair' para terminar):");
    println!();

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("> ");
        stdout.flush()?;

        let mut input = String::new();
        stdin.read_line(&mut input)?;
        let input = input.trim();

        if input.eq_ignore_ascii_case("sair") {
            println!("Até logo!");
            break;
        }

        if input.is_empty() {
            continue;
        }

        // Process with agent
        match agent.prompt(input).await {
            Ok(response) => {
                println!("\n🤖 RustClaw: {}\n", response);
            }
            Err(e) => {
                eprintln!("\n❌ Erro: {}\n", e);
            }
        }
    }

    Ok(())
}
