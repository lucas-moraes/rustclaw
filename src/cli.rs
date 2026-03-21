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
    file_edit::FileEditTool,
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
use crate::utils::colors::Colors;
use rustyline::history::FileHistory;
use rustyline::Editor;
use std::io::{self, Write};
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_splash(model: &str, memory_count: usize) {
    println!();

    let gradient = Colors::logo_gradient();
    let logo_lines = [
        "    ██████╗ ██╗   ██╗███████╗████████╗ ██████╗██╗      █████╗ ██╗    ██╗",
        "    ██╔══██╗██║   ██║██╔════╝╚══██╔══╝██╔════╝██║     ██╔══██╗██║    ██║",
        "    ██████╔╝██║   ██║███████╗   ██║   ██║     ██║     ███████║██║ █╗ ██║",
        "    ██╔══██╗██║   ██║╚════██║   ██║   ██║     ██║     ██╔══██║██║███╗██║",
        "    ██║  ██║╚██████╔╝███████║   ██║   ╚██████╗███████╗██║  ██║╚███╔███╔╝",
        "    ╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝    ╚═════╝╚══════╝╚═╝  ╚═╝ ╚══╝╚══╝ ",
    ];

    for (i, line) in logo_lines.iter().enumerate() {
        println!("{}{}{}{}", gradient[i], line, Colors::RESET, Colors::RESET);
    }

    println!(
        "{}{}{}",
        Colors::DIM,
        "    ─────────────────────────────────────────────────────────────────────",
        Colors::RESET
    );
    println!();
    print!("    ");
    print!("{}{}{}", Colors::DIM, "model", Colors::RESET);
    print!("  {}", model);
    print!("      ");
    print!("{}{}{}", Colors::DIM, "memories", Colors::RESET);
    print!("  {}", memory_count);
    print!("      ");
    print!("{}{}{}", Colors::DIM, "v", Colors::RESET);
    println!("{}{}{}", Colors::RESET, VERSION, Colors::RESET);
    println!();
}

fn print_help() {
    println!();
    println!(
        "{}{}{}  Commands",
        Colors::ORANGE,
        "⬡",
        Colors::RESET
    );
    println!();
    println!(
        "  {}{}/help{}        show this message",
        Colors::DIM, Colors::AMBER, Colors::RESET
    );
    println!(
        "  {}{}/clear{}       clear context and memories",
        Colors::DIM, Colors::AMBER, Colors::RESET
    );
    println!(
        "  {}{}/exit{}        exit RustClaw",
        Colors::DIM, Colors::AMBER, Colors::RESET
    );
    println!();
    println!(
        "{}{}{}  Input",
        Colors::ORANGE,
        "⬡",
        Colors::RESET
    );
    println!();
    println!(
        "  {}End a line with {}\\{} to continue on the next line",
        Colors::DIM, Colors::AMBER, Colors::RESET
    );
    println!(
        "  {}Use {}<<<{} to start a multiline block, {}>>>{} to close it",
        Colors::DIM, Colors::AMBER, Colors::RESET, Colors::AMBER, Colors::RESET
    );
    println!(
        "  {}↑↓{} for command history{}",
        Colors::DIM, Colors::AMBER, Colors::RESET
    );
    println!();
}

pub async fn run(config: Config) -> anyhow::Result<()> {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config_dir = current_dir.join("config");
    let memory_path = config_dir.join("memory_cli.db");

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    info!("Starting RustClaw in CLI mode...");

    let mut tools = ToolRegistry::new();
    tools.register(Box::new(CapabilitiesTool::new()));
    tools.register(Box::new(EchoTool));
    tools.register(Box::new(ShellTool::new()));
    tools.register(Box::new(FileReadTool::new()));
    tools.register(Box::new(FileWriteTool::new()));
    tools.register(Box::new(FileEditTool::new()));
    tools.register(Box::new(FileListTool::new()));
    tools.register(Box::new(FileSearchTool::new()));
    tools.register(Box::new(HttpGetTool::new()));
    tools.register(Box::new(HttpPostTool::new()));
    tools.register(Box::new(SystemInfoTool::new()));
    tools.register(Box::new(DateTimeTool::new()));
    tools.register(Box::new(LocationTool::new()));
    tools.register(Box::new(ClearMemoryTool::new(&memory_path)));

    tools.register(Box::new(BrowserTool::new(config_dir.to_path_buf())));

    if let Some(ref tavily_key) = config.tavily_api_key {
        tools.register(Box::new(TavilySearchTool::new(tavily_key.clone())));
        tools.register(Box::new(TavilyQuickSearchTool::new(tavily_key.clone())));
        info!("Tavily search tools registered");
    }

    tools.register(Box::new(SkillListTool::new()));
    tools.register(Box::new(SkillCreateTool::new()));
    tools.register(Box::new(SkillDeleteTool::new()));
    tools.register(Box::new(SkillEditTool::new("skills")));
    tools.register(Box::new(SkillRenameTool::new()));
    tools.register(Box::new(SkillValidateTool::new()));
    tools.register(Box::new(SkillImportFromUrlTool::new()));

    init_tmux("cli");

    let mut agent = Agent::new(config, tools, &memory_path)?;
    let memory_count = agent.get_memory_count()? as usize;
    info!("Memories loaded: {}", memory_count);

    let mut rl: Editor<(), FileHistory> = Editor::new().map_err(|e| anyhow::anyhow!("{}", e))?;
    let history_path = config_dir.join("history.txt");
    let _ = rl.load_history(&history_path);

    let model_name = agent.model_name();
    print_splash(&model_name, memory_count);

    println!(
        "{}{}{}  Welcome to RustClaw! Type {}/help{} for commands",
        Colors::DIM,
        "✻",
        Colors::RESET,
        Colors::AMBER,
        Colors::RESET
    );
    println!();

    loop {
        print!("{}{}{} ", Colors::AMBER, "›", Colors::RESET);
        io::stdout().flush()?;

        let line = match rl.readline("") {
            Ok(l) => l,
            Err(_) => break,
        };

        let trimmed = line.trim();

        if trimmed.eq_ignore_ascii_case("/exit") || trimmed.eq_ignore_ascii_case("sair") {
            println!("{}Goodbye.{}", Colors::DIM, Colors::RESET);
            break;
        }

        if trimmed.is_empty() {
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/help") {
            print_help();
            continue;
        }

        let input = if trimmed == "<<<" {
            let mut buf = String::new();
            let mut continuing = true;
            while continuing {
                print!("{}...{} ", Colors::DIM, Colors::RESET);
                io::stdout().flush()?;
                match rl.readline("") {
                    Ok(cont_line) => {
                        if cont_line.trim() == ">>>" {
                            continuing = false;
                        } else {
                            buf.push_str(&cont_line);
                            buf.push('\n');
                        }
                    }
                    Err(_) => break,
                }
            }
            buf
        } else if trimmed.ends_with('\\') {
            let mut buf = trimmed[..trimmed.len() - 1].to_string();
            buf.push('\n');
            let mut continuing = true;
            while continuing {
                print!("{}...{} ", Colors::DIM, Colors::RESET);
                io::stdout().flush()?;
                match rl.readline("") {
                    Ok(cont_line) => {
                        let ct = cont_line.trim();
                        if ct.ends_with('\\') {
                            buf.push_str(&ct[..ct.len() - 1]);
                            buf.push('\n');
                        } else {
                            buf.push_str(ct);
                            continuing = false;
                        }
                    }
                    Err(_) => break,
                }
            }
            buf
        } else {
            let _ = rl.add_history_entry(trimmed);
            trimmed.to_string()
        };

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        let spinner = Spinner::new();
        match spinner.run(agent.prompt(input)).await {
            Ok(response) => {
                println!();
                for line in response.lines() {
                    println!(
                        "{}{}{}  {}",
                        Colors::AMBER,
                        "●",
                        Colors::RESET,
                        line
                    );
                }
                println!();
            }
            Err(e) => {
                eprintln!(
                    "\n{}{}{}  Error: {}{}\n",
                    Colors::RED,
                    "⨯",
                    Colors::RESET,
                    e,
                    Colors::RESET
                );
            }
        }
    }

    let _ = rl.save_history(&history_path);
    Ok(())
}
