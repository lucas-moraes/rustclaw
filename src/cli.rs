use crate::agent::Agent;
use crate::agent::init_tmux;
use crate::tools::browser::BrowserTool;
use crate::config::Config;
use crate::tavily::tools::{TavilyQuickSearchTool, TavilySearchTool};
use crate::tools::{
    capabilities::CapabilitiesTool,
    clear_memory::ClearMemoryTool,
    datetime::DateTimeTool,
    echo::EchoTool,
    file_list::FileListTool,
    file_read::FileReadTool,
    file_search::FileSearchTool,
    file_write::FileWriteTool,
    http::{HttpGetTool, HttpPostTool},
    location::LocationTool,
    shell::ShellTool,
    skill_import::SkillImportFromUrlTool,
    skill_manager::{
        SkillCreateTool, SkillDeleteTool, SkillEditTool, SkillListTool, SkillRenameTool,
        SkillValidateTool,
    },
    system::SystemInfoTool,
    ToolRegistry,
};
use crate::utils::spinner::Spinner;
use std::io::{self, Write};
use std::path::Path;
use tracing::info;
use tracing_subscriber::EnvFilter;

pub async fn run(config: Config) -> anyhow::Result<()> {
    // Configure logging with EnvFilter - defaults to WARN level
    // Users can override with RUST_LOG environment variable
    // Examples:
    //   RUST_LOG=info    - Show info, warn, error logs
    //   RUST_LOG=warn    - Show warn, error logs (default)
    //   RUST_LOG=error   - Show only error logs
    //   RUST_LOG=off     - Disable all logs
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    info!("Iniciando RustClaw em modo CLI...");

    let memory_path = Path::new("config/memory_cli.db");

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

    let config_dir = Path::new("config");
    tools.register(Box::new(BrowserTool::new(config_dir.to_path_buf())));

    if let Some(ref tavily_key) = config.tavily_api_key {
        tools.register(Box::new(TavilySearchTool::new(tavily_key.clone())));
        tools.register(Box::new(TavilyQuickSearchTool::new(tavily_key.clone())));
        info!("✅ Tavily search tools registered");
    } else {
        info!("⚠️  TAVILY_API_KEY not set, Tavily search tools disabled");
    }

    // Skill management tools
    tools.register(Box::new(SkillListTool::new()));
    tools.register(Box::new(SkillCreateTool::new()));
    tools.register(Box::new(SkillDeleteTool::new()));
    tools.register(Box::new(SkillEditTool::new("skills")));
    tools.register(Box::new(SkillRenameTool::new()));
    tools.register(Box::new(SkillValidateTool::new()));
    tools.register(Box::new(SkillImportFromUrlTool::new()));

    info!("Ferramentas registradas: {}", tools.list().lines().count());

    // Initialize TMUX if enabled
    init_tmux("cli");

    let mut agent = Agent::new(config, tools, memory_path)?;
    let memory_count = agent.get_memory_count()?;
    info!("Memórias carregadas: {}", memory_count);

    println!("╔════════════════════════════════════════════════╗");
    println!("║                RustClaw v1.0.0                 ║");
    println!("╚════════════════════════════════════════════════╝");
    println!();
    println!("Modelo: {}", agent.model_name());
    println!();
    println!("🖥️  Modo: Terminal (CLI)");
    println!("🧠 Memórias salvas: {}", memory_count);
    if crate::agent::get_tmux_manager().is_some() {
        println!("📺 TMUX: Ativo");
    }
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

        let spinner = Spinner::new();
        match spinner.run(agent.prompt(input)).await {
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
