use crate::agent::Agent;
use crate::browser::tools::{
    BrowserExtractTool, BrowserNavigateTool, BrowserScreenshotTool, BrowserSearchTool,
    BrowserTestTool,
};
use crate::config::Config;
use crate::tavily::tools::{TavilyQuickSearchTool, TavilySearchTool};
use crate::tools::{
    capabilities::CapabilitiesTool, clear_memory::ClearMemoryTool, datetime::DateTimeTool,
    echo::EchoTool, file_list::FileListTool, file_read::FileReadTool,
    file_search::FileSearchTool, file_write::FileWriteTool, http::{HttpGetTool, HttpPostTool},
    location::LocationTool, shell::ShellTool, system::SystemInfoTool,
    skill_manager::{SkillCreateTool, SkillDeleteTool, SkillEditTool, SkillListTool, SkillRenameTool, SkillValidateTool},
    skill_import::SkillImportFromUrlTool,
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
    tools.register(Box::new(DateTimeTool::new()));
    tools.register(Box::new(LocationTool::new()));
    tools.register(Box::new(ClearMemoryTool::new(memory_path)));
    
    
    if let Some(ref tavily_key) = config.tavily_api_key {
        tools.register(Box::new(TavilySearchTool::new(tavily_key.clone())));
        tools.register(Box::new(TavilyQuickSearchTool::new(tavily_key.clone())));
        info!("✅ Tavily search tools registered");
    } else {
        info!("⚠️  TAVILY_API_KEY not set, Tavily search tools disabled");
    }
    
    
    tools.register(Box::new(BrowserNavigateTool::new()));
    tools.register(Box::new(BrowserSearchTool::new()));
    tools.register(Box::new(BrowserExtractTool::new()));
    tools.register(Box::new(BrowserScreenshotTool::new()));
    tools.register(Box::new(BrowserTestTool::new()));
    
    // Skill management tools
    tools.register(Box::new(SkillListTool::new()));
    tools.register(Box::new(SkillCreateTool::new()));
    tools.register(Box::new(SkillDeleteTool::new()));
    tools.register(Box::new(SkillEditTool::new("skills")));
    tools.register(Box::new(SkillRenameTool::new()));
    tools.register(Box::new(SkillValidateTool::new()));
    tools.register(Box::new(SkillImportFromUrlTool::new()));
    
    info!("Ferramentas registradas: {}", tools.list().lines().count());

    
    let mut agent = Agent::new(config, tools, memory_path)?;
    let memory_count = agent.get_memory_count()?;
    info!("Memórias carregadas: {}", memory_count);

    
    println!("╔════════════════════════════════════════════════╗");
    println!("║              RustClaw v0.1.0                   ║");
    println!("║   Fase 4: Telegram + CLI + Memória LTM         ║");
    println!("╚════════════════════════════════════════════════╝");
    println!();
    println!("🖥️  Modo: Terminal (CLI)");
    println!("🧠 Memórias salvas: {}", memory_count);
    println!();
    println!("Digite mensagens (ou 'sair' para terminar):");
    println!();
    println!("💡 Dica: Pergunte 'o que você pode fazer?' para ver todas as capacidades");
    println!("💡 Dica: Execute com --mode telegram para modo bot");
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
                println!("\n🤖 RustClaw: {}\n", response);
            }
            Err(e) => {
                eprintln!("\n❌ Erro: {}\n", e);
            }
        }
    }

    Ok(())
}
