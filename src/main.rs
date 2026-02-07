mod agent;
mod config;
mod memory;
mod tools;

use agent::Agent;
use config::Config;
use dotenv::dotenv;
use std::io::{self, Write};
use std::path::Path;
use tools::capabilities::CapabilitiesTool;
use tools::echo::EchoTool;
use tools::file_list::FileListTool;
use tools::file_read::FileReadTool;
use tools::file_search::FileSearchTool;
use tools::file_write::FileWriteTool;
use tools::http::{HttpGetTool, HttpPostTool};
use tools::shell::ShellTool;
use tools::system::SystemInfoTool;
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

    // Create data directory for memory
    let memory_path = Path::new("data/memory.db");

    // Create tool registry and register tools
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(CapabilitiesTool::new()));
    tools.register(Box::new(EchoTool));
    tools.register(Box::new(ShellTool::new()));
    tools.register(Box::new(FileReadTool::new()));
    tools.register(Box::new(FileWriteTool::new()));
    tools.register(Box::new(FileListTool::new()));
    tools.register(Box::new(FileSearchTool::new()));
    tools.register(Box::new(HttpGetTool::new()));
    tools.register(Box::new(HttpPostTool::new()));
    tools.register(Box::new(SystemInfoTool::new()));
    info!("Ferramentas registradas: {}", tools.list().lines().count());

    // Create agent with memory
    let mut agent = Agent::new(config, tools, memory_path)?;
    let memory_count = agent.get_memory_count()?;
    info!("Memórias carregadas: {}", memory_count);

    // CLI loop
    println!("╔════════════════════════════════════════════════╗");
    println!("║              RustClaw v0.1.0                   ║");
    println!("║   Fase 3: Memória Persistente de Longo Prazo   ║");
    println!("╚════════════════════════════════════════════════╝");
    println!();
    println!("🧠 Memórias salvas: {}", memory_count);
    println!();
    println!("Digite mensagens (ou 'sair' para terminar):");
    println!();
    println!("💡 Dica: Pergunte 'o que você pode fazer?' para ver todas as capacidades");
    println!("💡 Dica: Pergunte 'Qual API eu prefiro?' após dizer 'Prefiro usar Kimi'");
    println!("   e veja se ele lembra após reiniciar!");
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
            println!("Até logo! Suas memórias foram salvas.");
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
