use crate::agent::Agent;
use crate::config::Config;
use crate::tavily::tools::{TavilyQuickSearchTool, TavilySearchTool};
use crate::tools::{
    capabilities::CapabilitiesTool, echo::EchoTool, file_list::FileListTool,
    file_read::FileReadTool, file_search::FileSearchTool, file_write::FileWriteTool,
    http::{HttpGetTool, HttpPostTool}, shell::ShellTool, system::SystemInfoTool,
    ToolRegistry,
};
use std::io::{self, Write};
use std::path::Path;
use tracing::{info, Level};

pub async fn run(config: Config) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("Iniciando RustClaw em modo CLI...");

    let memory_path = Path::new("data/memory_cli.db");

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
    
    if let Some(ref tavily_key) = config.tavily_api_key {
        tools.register(Box::new(TavilySearchTool::new(tavily_key.clone())));
        tools.register(Box::new(TavilyQuickSearchTool::new(tavily_key.clone())));
        info!("Tavily search tools registered");
    } else {
        info!("TAVILY_API_KEY not set, Tavily search tools disabled");
    }
    
    info!("Ferramentas registradas: {}", tools.list().lines().count());

    let mut agent = Agent::new(config, tools, memory_path)?;
    let memory_count = agent.get_memory_count()?;
    info!("Memórias carregadas: {}", memory_count);

    println!("RustClaw v0.1.0 - Raspberry Pi Edition");
    println!("Modo: Terminal (CLI)");
    println!("Memórias salvas: {}", memory_count);
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
            println!("Até logo! Suas memórias foram salvas.");
            break;
        }

        if input.is_empty() {
            continue;
        }

        match agent.prompt(input).await {
            Ok(response) => {
                println!("\nRustClaw: {}\n", response);
            }
            Err(e) => {
                eprintln!("\nErro: {}\n", e);
            }
        }
    }

    Ok(())
}
