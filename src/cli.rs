use crate::agent::init_tmux;
use crate::agent::Agent;
use crate::config::Config;
use crate::i18n;
use crate::tavily::tools::{TavilyQuickSearchTool, TavilySearchTool};
use crate::tools::browser::BrowserTool;
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
    skill_script::{SkillScriptTool, SkillScriptsListTool},
    system::SystemInfoTool,
    ToolRegistry,
};
use crate::utils::colors::Colors;
use crate::utils::spinner::Spinner;
use is_terminal::IsTerminal;
use rustyline::history::FileHistory;
use rustyline::Editor;
use std::io::{self, Write};
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

const COMMAND_COMMANDS: &[(&str, &str)] = &[
    ("/help", "Mostrar ajuda"),
    ("/clear", "Limpar contexto e memórias"),
    ("/skills", "Listar skills disponíveis"),
    ("/skill:", "Ativar skill por nome"),
    ("/sessions", "Listar sessões (interativo)"),
    ("/session ", "Resumir sessão por ID"),
    ("/desenvolver", "Desenvolvimento estruturado"),
    ("/trust", "Mostrar/atualizar trust level"),
    ("/trust ", "Definir trust level (trusted/untrusted/fullytrusted)"),
    ("/summarize", "Resumir contexto (compression)"),
    ("/compress", "Alias para /summarize"),
    ("/stats", "Mostrar estatísticas de uso e custos"),
];

fn print_command_suggestions(prefix: &str) {
    println!();
    println!(
        "{}{}Comandos:{}",
        Colors::LIGHT_GRAY,
        Colors::AMBER,
        Colors::RESET
    );

    for (cmd, desc) in COMMAND_COMMANDS.iter() {
        if cmd.starts_with(prefix) {
            println!("  {}{:<16}{} - {}", Colors::AMBER, cmd, Colors::RESET, desc);
        }
    }

    println!();
}

/// Reads multi-line input from the user.
/// Lines ending with `\` continue to the next line.
/// Empty line (or line with just spaces) submits the input.
fn read_multiline_input(rl: &mut Editor<(), FileHistory>, is_continuation: bool) -> Option<String> {
    let prompt = if is_continuation {
        format!("{}  · ", Colors::CONTINUATION)
    } else {
        format!("{}{} ", Colors::AMBER, "›")
    };

    let line = match rl.readline(&prompt) {
        Ok(l) => l,
        Err(_) => return None,
    };

    let trimmed = line.trim();

    // Empty line submits (unless it's the first line of continuation)
    if trimmed.is_empty() && is_continuation {
        return Some(String::new());
    }

    Some(line)
}

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
        "{}    ─────────────────────────────────────────────────────────────────────{}",
        Colors::LIGHT_GRAY,
        Colors::RESET
    );
    println!();
    print!("    ");
    print!("{}model{}", Colors::LIGHT_GRAY, Colors::RESET);
    print!("  {}", model);
    print!("      ");
    print!("{}memories{}", Colors::LIGHT_GRAY, Colors::RESET);
    print!("  {}", memory_count);
    print!("      ");
    print!("{}v{}", Colors::LIGHT_GRAY, Colors::RESET);
    println!("{}{}{}", Colors::RESET, VERSION, Colors::RESET);
    println!();
}

fn print_help() {
    println!();
    println!("{}⬡{}  Commands", Colors::ORANGE, Colors::RESET);
    println!();
    println!(
        "  {}{}/help{}        show this message",
        Colors::LIGHT_GRAY,
        Colors::AMBER,
        Colors::RESET
    );
    println!(
        "  {}{}/skills{}      list available skills",
        Colors::LIGHT_GRAY,
        Colors::AMBER,
        Colors::RESET
    );
    println!(
        "  {}{}/clear{}       clear context and memories",
        Colors::LIGHT_GRAY,
        Colors::AMBER,
        Colors::RESET
    );
    println!(
        "  {}{}/exit{}        exit RustClaw",
        Colors::LIGHT_GRAY,
        Colors::AMBER,
        Colors::RESET
    );
    println!();
    println!("{}⬡{}  Skills", Colors::ORANGE, Colors::RESET);
    println!();
    println!(
        "  {}{}/skill-name{}  activate a skill by name",
        Colors::LIGHT_GRAY,
        Colors::AMBER,
        Colors::RESET
    );
    println!(
        "  {}Use {}@reference.md{} to load references",
        Colors::LIGHT_GRAY,
        Colors::AMBER,
        Colors::RESET
    );
    println!();
    println!("{}⬡{}  Input", Colors::ORANGE, Colors::RESET);
    println!();
    println!(
        "  {}End a line with {}\\{} to continue on the next line",
        Colors::LIGHT_GRAY,
        Colors::AMBER,
        Colors::RESET
    );
    println!(
        "  {}Use {}<<<{} to start a multiline block, {}>>>{} to close it",
        Colors::LIGHT_GRAY,
        Colors::AMBER,
        Colors::RESET,
        Colors::AMBER,
        Colors::RESET
    );
    println!(
        "  {}↑↓{} for command history{}",
        Colors::LIGHT_GRAY,
        Colors::AMBER,
        Colors::RESET
    );
    println!();
}

pub async fn run(config: Config) -> anyhow::Result<()> {
    tracing::info!("Starting RustClaw CLI");
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
    tools.register(Box::new(SkillScriptTool::new(PathBuf::from("skills"))));
    tools.register(Box::new(SkillScriptsListTool::new(PathBuf::from("skills"))));

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
        "{}✻{}  Welcome to RustClaw! Type {}/help{} for commands",
        Colors::LIGHT_GRAY,
        Colors::RESET,
        Colors::AMBER,
        Colors::RESET
    );
    println!();

    loop {
        let mut full_input = String::new();
        let mut is_continuation = false;
        let mut line_count = 0;
        let mut input_lines: Vec<String> = Vec::new();

        #[allow(clippy::while_let_loop)]
        loop {
            let line = match read_multiline_input(&mut rl, is_continuation) {
                Some(l) => l,
                None => break,
            };

            // Empty line ends multi-line input (except for first iteration)
            if line.trim().is_empty() && is_continuation {
                break;
            }

            // Add line to collection
            if line_count > 0 {
                full_input.push('\n');
            }
            full_input.push_str(&line);
            input_lines.push(line.clone());
            line_count += 1;

            let trimmed = line.trim();

            // Check if line ends with backslash (continuation)
            if trimmed.ends_with('\\') && !trimmed.ends_with("\\\\") {
                // Remove the trailing backslash and continue
                full_input.pop(); // Remove '\'
                is_continuation = true;
            } else if trimmed.ends_with("\\\\") {
                // Double backslash = literal backslash at end, submit
                full_input.pop(); // Remove one '\'
                full_input.pop(); // Remove second '\'
                break;
            } else {
                break;
            }
        }

        // Check for exit command in full input
        if full_input.trim().eq_ignore_ascii_case("/exit")
            || full_input.trim().eq_ignore_ascii_case("sair")
        {
            println!("{}{}{}", Colors::LIGHT_GRAY, i18n::t(i18n::MessageKey::GoodbyeMessage), Colors::RESET);
            break;
        }

        // Skip empty input
        if full_input.trim().is_empty() {
            continue;
        }

        let trimmed = full_input.trim();

        // If user just types "/" (no other characters), show suggestions and get MORE input
        if trimmed == "/" {
            print_command_suggestions(trimmed);
            // Prompt for more input (don't execute yet)
            let prompt = format!("{}{} / ", Colors::AMBER, "›");
            if let Ok(more) = rl.readline(&prompt) {
                if more.trim().starts_with("/") {
                    // They started a new command - restart loop
                    full_input = more;
                } else {
                    // Continue the command
                    full_input.push_str(&more);
                }
                continue;
            }
        } else if trimmed.starts_with("/") {
            // Show suggestions for partial commands (but still execute)
            print_command_suggestions(trimmed);
        }

        // Simple newline before processing
        println!();

        if trimmed.eq_ignore_ascii_case("/help") {
            print_help();
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/clear")
            || trimmed.to_lowercase().contains("limpar memória")
            || trimmed.to_lowercase().contains("clean memory")
        {
            println!(
                "{}🧹 Limpando todas as memórias...{}",
                Colors::LIGHT_GRAY,
                Colors::RESET
            );
            match agent.clear_all_memory().await {
                Ok(msg) => println!("{}✓ {}{}", Colors::AMBER, msg, Colors::RESET),
                Err(e) => println!("{}✗ {}: {}{}", Colors::RED, i18n::t(i18n::MessageKey::ErrorClearing), e, Colors::RESET),
            }
            continue;
        }

        if trimmed.eq_ignore_ascii_case("/skills") || trimmed.eq_ignore_ascii_case("/skill") {
            let skills = agent.list_skills();
            println!();
            println!("{}⬡{}  {}", Colors::ORANGE, Colors::RESET, i18n::t(i18n::MessageKey::AvailableSkills));
            println!();
            if skills.is_empty() {
                println!("  {}{}{}", Colors::LIGHT_GRAY, i18n::t(i18n::MessageKey::NoSkillsFound), Colors::RESET);
            } else {
                for skill in skills {
                    println!(
                        "  {}{}/{}  {}",
                        Colors::LIGHT_GRAY,
                        Colors::AMBER,
                        skill,
                        Colors::RESET
                    );
                }
            }
            println!();
            continue;
        }

        // Session management commands - Interactive selector with arrow keys
        if trimmed.eq_ignore_ascii_case("/sessions")
            || trimmed.to_lowercase().starts_with("/session")
        {
            let mut target_session_id: Option<String> = None;

            // If /session <id>, extract the ID
            if trimmed.to_lowercase().starts_with("/session") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 2 {
                    target_session_id = Some(parts[1].to_string());
                }
            }

            println!();
            let mut sessions_with_depth = match agent.list_sessions_with_hierarchy() {
                Ok(s) => s,
                Err(e) => {
                    println!(
                        "  {}Erro ao listar sessões: {}{}",
                        Colors::RED,
                        e,
                        Colors::RESET
                    );
                    continue;
                }
            };

            if sessions_with_depth.is_empty() {
                println!(
                    "  {}Nenhuma sessão encontrada{}",
                    Colors::LIGHT_GRAY,
                    Colors::RESET
                );
                println!();
                continue;
            }

            // Interactive selection with arrow keys (only on real TTY)
            let is_tty = std::io::stdin().is_terminal();

            if is_tty {
                // Interactive mode with arrow keys
                let mut selected = 0;
                if let Some(ref tid) = target_session_id {
                    if let Some(idx) = sessions_with_depth
                        .iter()
                        .position(|(s, _)| s.session_id.starts_with(tid))
                    {
                        selected = idx;
                    }
                }

                // Save terminal settings and enable raw mode
                #[cfg(unix)]
                {
                    use std::os::unix::io::AsRawFd;
                    let stdin = std::io::stdin();
                    let fd = stdin.as_raw_fd();
                    let mut old_termios: libc::termios = unsafe { std::mem::zeroed() };
                    unsafe {
                        libc::tcgetattr(fd, &mut old_termios);
                        let mut new_termios = old_termios;
                        new_termios.c_lflag &= !(libc::ICANON | libc::ECHO);
                        libc::tcsetattr(fd, libc::TCSANOW, &new_termios);

                        let mut running = true;
                        while running {
                            // Clear and redraw
                            print!("\x1b[2J\x1b[H");
                            println!(
                                "{}Sessões anteriores{} - ↑↓ navegar, Enter continuar, q sair",
                                Colors::ORANGE,
                                Colors::RESET
                            );
                            println!();

                            for (idx, (session, depth)) in sessions_with_depth.iter().enumerate() {
                                let marker = if idx == selected { "▶" } else { "  " };
                                let short_id =
                                    &session.session_id[..8.min(session.session_id.len())];
                                let display_text = &session.title;
                                let task_short: String = display_text.chars().take(36).collect();
                                let task_short = if task_short.len() == 36 {
                                    format!("{}...", task_short)
                                } else {
                                    task_short
                                };

                                // Indentation based on depth
                                let indent: String = "  ".repeat(depth.saturating_sub(1).min(5));
                                let prefix = if *depth > 0 { "└─" } else { "" };

                                // Format date: dd/mm HH:MM
                                let date_str = session.updated_at.format("%d/%m %H:%M").to_string();

                                // Session type indicator
                                let type_indicator = match session.session_type {
                                    crate::memory::checkpoint::SessionType::Project => "[P]",
                                    crate::memory::checkpoint::SessionType::Subtask => "[S]",
                                    crate::memory::checkpoint::SessionType::Research => "[R]",
                                    crate::memory::checkpoint::SessionType::Chat => "[C]",
                                };

                                if idx == selected {
                                    println!(
                                        "{}{}{}{} │ {} │ {}{} │ {:36} │ {}{}",
                                        Colors::AMBER,
                                        marker,
                                        indent,
                                        prefix,
                                        short_id,
                                        type_indicator,
                                        Colors::RESET,
                                        task_short,
                                        date_str,
                                        Colors::RESET
                                    );
                                } else {
                                    println!(
                                        "{}  {}{} │ {} │ {} │ {:36} │ {}",
                                        Colors::LIGHT_GRAY,
                                        indent,
                                        prefix,
                                        short_id,
                                        type_indicator,
                                        task_short,
                                        date_str
                                    );
                                }
                            }
                            println!();
                            println!("{}Enter{} Cont.  │  {}D{}el Excluir  │  {}R{}en. Renomear  │  {}Q{} Sair", Colors::AMBER, Colors::RESET, Colors::AMBER, Colors::RESET, Colors::AMBER, Colors::RESET, Colors::AMBER, Colors::RESET);

                            // Read key - blocking read
                            let mut buf = [0u8; 1];
                            let n = libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, 1);

                            if n == 1 {
                                match buf[0] {
                                    27 => {
                                        // ESC - check for arrow sequence
                                        let mut seq = [0u8; 2];
                                        let m = libc::read(
                                            fd,
                                            seq.as_mut_ptr() as *mut libc::c_void,
                                            2,
                                        );
                                        if m >= 2 && seq[0] == 91 {
                                            match seq[1] {
                                                65 if selected > 0 => selected -= 1, // Up
                                                66 if selected < sessions_with_depth.len() - 1 => {
                                                    selected += 1
                                                } // Down
                                                _ => {}
                                            }
                                        }
                                    }
                                    10 | 13 => {
                                        // Enter (LF ou CR)
                                        let selected_id =
                                            sessions_with_depth[selected].0.session_id.clone();
                                        println!(
                                            "\n{}Continuando sessão...{}",
                                            Colors::LIGHT_GRAY,
                                            Colors::RESET
                                        );
                                        std::io::stdout().flush().ok();
                                        libc::tcsetattr(fd, libc::TCSANOW, &old_termios);
                                        match tokio::time::timeout(
                                            std::time::Duration::from_secs(60),
                                            agent.resume_session(&selected_id),
                                        )
                                        .await
                                        {
                                            Ok(Ok(response)) => {
                                                println!();
                                                println!("{}", response);
                                            }
                                            Ok(Err(e)) => {
                                                println!(
                                                    "{}Erro: {}{}",
                                                    Colors::RED,
                                                    e,
                                                    Colors::RESET
                                                );
                                            }
                                            Err(_) => {
                                                println!(
                                                    "{}Timeout ao continuar sessão{}",
                                                    Colors::RED,
                                                    Colors::RESET
                                                );
                                            }
                                        }
                                        running = false;
                                    }
                                    4 => {
                                        // Cmd+D - delete (macOS) or 'd'
                                        let selected_id =
                                            sessions_with_depth[selected].0.session_id.clone();
                                        tracing::debug!(
                                            "Delete pressed for session: {}",
                                            selected_id
                                        );
                                        match agent.delete_session(&selected_id).await {
                                            Ok(msg) => println!("{}{}", Colors::AMBER, msg),
                                            Err(e) => println!(
                                                "{}Erro ao excluir: {}{}",
                                                Colors::RED,
                                                e,
                                                Colors::RESET
                                            ),
                                        }
                                        match agent.list_sessions_with_hierarchy() {
                                            Ok(new_sessions) => {
                                                sessions_with_depth = new_sessions;
                                                if sessions_with_depth.is_empty() {
                                                    println!(
                                                        "\n{}Nenhuma sessão restante{}",
                                                        Colors::LIGHT_GRAY,
                                                        Colors::RESET
                                                    );
                                                    running = false;
                                                } else if selected >= sessions_with_depth.len() {
                                                    selected = sessions_with_depth.len() - 1;
                                                }
                                            }
                                            Err(_) => running = false,
                                        }
                                    }
                                    100 => {
                                        // lowercase 'd' - also delete
                                        let selected_id =
                                            sessions_with_depth[selected].0.session_id.clone();
                                        tracing::debug!(
                                            "Delete (lowercase) pressed for session: {}",
                                            selected_id
                                        );
                                        match agent.delete_session(&selected_id).await {
                                            Ok(msg) => println!("{}{}", Colors::AMBER, msg),
                                            Err(e) => println!(
                                                "{}Erro ao excluir: {}{}",
                                                Colors::RED,
                                                e,
                                                Colors::RESET
                                            ),
                                        }
                                        match agent.list_sessions_with_hierarchy() {
                                            Ok(new_sessions) => {
                                                sessions_with_depth = new_sessions;
                                                if sessions_with_depth.is_empty() {
                                                    println!(
                                                        "\n{}Nenhuma sessão restante{}",
                                                        Colors::LIGHT_GRAY,
                                                        Colors::RESET
                                                    );
                                                    running = false;
                                                } else if selected >= sessions_with_depth.len() {
                                                    selected = sessions_with_depth.len() - 1;
                                                }
                                            }
                                            Err(_) => running = false,
                                        }
                                    }
                                    18 => {
                                        // Cmd+R - rename (macOS)
                                        // Restore terminal first to allow input
                                        libc::tcsetattr(fd, libc::TCSANOW, &old_termios);
                                        print!("  {}Novo nome: {}", Colors::AMBER, Colors::RESET);
                                        io::stdout().flush()?;
                                        let new_name: String =
                                            rl.readline("").unwrap_or_default().trim().to_string();

                                        if !new_name.is_empty() {
                                            match agent
                                                .rename_session(
                                                    &sessions_with_depth[selected].0.session_id,
                                                    &new_name,
                                                )
                                                .await
                                            {
                                                Ok(_) => {
                                                    println!(
                                                        "{}Sessão renomeada para: {}{}",
                                                        Colors::AMBER,
                                                        new_name,
                                                        Colors::RESET
                                                    );
                                                    // Reload sessions
                                                    if let Ok(new_sessions) =
                                                        agent.list_sessions_with_hierarchy()
                                                    {
                                                        sessions_with_depth = new_sessions;
                                                        if selected >= sessions_with_depth.len() {
                                                            selected = sessions_with_depth
                                                                .len()
                                                                .saturating_sub(1);
                                                        }
                                                    }
                                                }
                                                Err(e) => println!(
                                                    "{}Erro ao renomear: {}{}",
                                                    Colors::RED,
                                                    e,
                                                    Colors::RESET
                                                ),
                                            }
                                        }

                                        // Re-enable raw mode
                                        let mut new_termios = old_termios;
                                        new_termios.c_lflag &= !(libc::ICANON | libc::ECHO);
                                        libc::tcsetattr(fd, libc::TCSANOW, &new_termios);
                                    }
                                    113 => {
                                        // 'q' - quit
                                        println!();
                                        running = false;
                                    }
                                    _ => {
                                        eprintln!(
                                            "Tecla pressionada: {} ({})",
                                            buf[0], buf[0] as char
                                        );
                                    }
                                }
                            }
                        }
                        // Restore terminal
                        libc::tcsetattr(fd, libc::TCSANOW, &old_termios);
                    }
                }

                #[cfg(not(unix))]
                {
                    // Non-Unix fallback - show list
                    println!("{}Sessões anteriores{}:", Colors::ORANGE, Colors::RESET);
                    println!();
                    for (idx, (session, depth)) in sessions_with_depth.iter().enumerate() {
                        let marker = if Some(session.session_id.clone()) == target_session_id {
                            "▶"
                        } else {
                            "  "
                        };
                        let short_id = &session.session_id[..8.min(session.session_id.len())];
                        let indent = "  ".repeat(depth.saturating_sub(1).min(5));
                        let prefix = if *depth > 0 { "└─" } else { "" };
                        let type_indicator = match session.session_type {
                            crate::memory::checkpoint::SessionType::Project => "[P]",
                            crate::memory::checkpoint::SessionType::Subtask => "[S]",
                            crate::memory::checkpoint::SessionType::Research => "[R]",
                            crate::memory::checkpoint::SessionType::Chat => "[C]",
                        };
                        println!(
                            "{}{}{} │ {} │ {} │ {:30} │ {} msgs │ {}",
                            Colors::AMBER,
                            marker,
                            indent,
                            short_id,
                            type_indicator,
                            session.title.chars().take(30).collect::<String>(),
                            session.message_count,
                            Colors::RESET
                        );
                    }
                }
            } else {
                // Non-TTY fallback - simple numbered list
                println!("{}Sessões anteriores{}:", Colors::ORANGE, Colors::RESET);
                println!();

                for (idx, (session, depth)) in sessions_with_depth.iter().enumerate() {
                    let marker = if Some(session.session_id.clone()) == target_session_id {
                        "▶"
                    } else {
                        "  "
                    };
                    let short_id = &session.session_id[..8.min(session.session_id.len())];
                    let display_text = &session.title;
                    let task_short: String = display_text.chars().take(36).collect();
                    let task_short = if task_short.len() == 36 {
                        format!("{}...", task_short)
                    } else {
                        task_short
                    };
                    let indent = "  ".repeat(depth.saturating_sub(1).min(5));
                    let prefix = if *depth > 0 { "└─" } else { "" };
                    let type_indicator = match session.session_type {
                        crate::memory::checkpoint::SessionType::Project => "[P]",
                        crate::memory::checkpoint::SessionType::Subtask => "[S]",
                        crate::memory::checkpoint::SessionType::Research => "[R]",
                        crate::memory::checkpoint::SessionType::Chat => "[C]",
                    };
                    let phase_display = match session.phase.as_str() {
                        "executing" => "▶ exec",
                        "awaiting_approval" => "⏳ agu",
                        "completed" => "✓ con",
                        _ => "out",
                    };
                    if Some(session.session_id.clone()) == target_session_id {
                        println!(
                            "{}{}{}{} │ {:2} │ {} │ {} │ {:36} │ {}{}",
                            Colors::AMBER,
                            marker,
                            indent,
                            prefix,
                            idx + 1,
                            short_id,
                            type_indicator,
                            task_short,
                            phase_display,
                            Colors::RESET
                        );
                    } else {
                        println!(
                            "{}  {}{} │ {:2} │ {} │ {} │ {:36} │ {}",
                            Colors::LIGHT_GRAY,
                            indent,
                            prefix,
                            idx + 1,
                            short_id,
                            type_indicator,
                            task_short,
                            phase_display
                        );
                    }
                }

                println!();
                if let Some(ref tid) = target_session_id {
                    if let Some(idx) = sessions_with_depth
                        .iter()
                        .position(|(s, _)| s.session_id.starts_with(tid))
                    {
                        println!(
                            "  {}Selecionada:{} {}",
                            Colors::AMBER,
                            Colors::RESET,
                            sessions_with_depth[idx].0.summary
                        );
                        println!();
                        print!(
                            "  {}▸{} Enter p/ continuar, delete p/ excluir: ",
                            Colors::AMBER,
                            Colors::RESET
                        );
                        io::stdout().flush()?;
                        let action: String =
                            rl.readline("").unwrap_or_default().trim().to_lowercase();
                        if action.is_empty() || action == "enter" {
                            let selected_id = sessions_with_depth[idx].0.session_id.clone();
                            println!(
                                "\n{}Continuando sessão...{}",
                                Colors::LIGHT_GRAY,
                                Colors::RESET
                            );
                            match agent.resume_session(&selected_id).await {
                                Ok(response) => {
                                    println!();
                                    println!("{}", response);
                                }
                                Err(e) => {
                                    println!("{}Erro: {}{}", Colors::RED, e, Colors::RESET);
                                }
                            }
                        } else if action == "delete" || action == "d" {
                            let selected_id = sessions_with_depth[idx].0.session_id.clone();
                            match agent.delete_session(&selected_id).await {
                                Ok(_) => {
                                    println!("{}Sessão excluída{}", Colors::AMBER, Colors::RESET)
                                }
                                Err(e) => println!("{}Erro: {}{}", Colors::RED, e, Colors::RESET),
                            }
                        }
                    } else {
                        println!(
                            "{}Sessão não encontrada: {}{}",
                            Colors::RED,
                            tid,
                            Colors::RESET
                        );
                    }
                } else {
                    println!(
                        "  {}Digite /session <id> para selecionar{}",
                        Colors::LIGHT_GRAY,
                        Colors::RESET
                    );
                }
            }

            println!();
            continue;
        }

        // Trust management command
        if trimmed.eq_ignore_ascii_case("/trust") || trimmed.to_lowercase().starts_with("/trust ") {
            println!();
            let current_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

            if trimmed.to_lowercase().starts_with("/trust ") {
                // Parse trust level
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 2 {
                    let level_str = parts[1].to_lowercase();
                    let level = match level_str.as_str() {
                        "untrusted" => Some(crate::workspace_trust::TrustLevel::Untrusted),
                        "readonly" | "ro" => Some(crate::workspace_trust::TrustLevel::UntrustedReadOnly),
                        "trusted" | "t" => Some(crate::workspace_trust::TrustLevel::Trusted),
                        "fullytrusted" | "ft" => Some(crate::workspace_trust::TrustLevel::FullyTrusted),
                        _ => {
                            println!("{}✗ Nível de trust inválido: {}{}", Colors::RED, level_str, Colors::RESET);
                            println!();
                            println!("Níveis disponíveis: untrusted, readonly, trusted, fullytrusted");
                            println!();
                            continue;
                        }
                    };

                    if let Some(level) = level {
                        match agent.set_trust_level(&current_dir, level) {
                            Ok(_) => {
                                println!("{}✓ Trust level definido para: {:?}{}", Colors::AMBER, level, Colors::RESET);
                            }
                            Err(e) => {
                                println!("{}✗ Erro ao definir trust: {}{}", Colors::RED, e, Colors::RESET);
                            }
                        }
                    }
                }
            } else {
                // Show current trust level
                let workspaces = agent.list_workspaces();
                let current_level = agent.get_trust_level(&current_dir);

                println!("{}⬡{}  Trust Status", Colors::ORANGE, Colors::RESET);
                println!();
                println!("  {}Diretório atual:{} {}", Colors::LIGHT_GRAY, Colors::RESET, current_dir.display());
                println!("  {}Trust atual:{} {}", Colors::LIGHT_GRAY, Colors::RESET, current_level);
                println!();

                if !workspaces.is_empty() {
                    println!("  {}Workspaces configurados:{}", Colors::LIGHT_GRAY, Colors::RESET);
                    for ws in workspaces {
                        println!("    - {}", ws);
                    }
                } else {
                    println!("  {}Nenhum workspace configurado{} (usando default: Untrusted)", Colors::LIGHT_GRAY, Colors::RESET);
                }
                println!();
                println!("  {}Usar:{} /trust <nivel>", Colors::LIGHT_GRAY, Colors::RESET);
                println!("  {}Níveis:{} untrusted, readonly, trusted, fullytrusted", Colors::LIGHT_GRAY, Colors::RESET);
            }
            println!();
            continue;
        }

        // Summarize/Compress command
        if trimmed.eq_ignore_ascii_case("/summarize") || trimmed.eq_ignore_ascii_case("/compress") {
            println!();
            let stats = agent.get_compression_stats();
            println!("{}⬡{}  {}", Colors::ORANGE, Colors::RESET, i18n::t(i18n::MessageKey::ContextCompression));
            println!();
            println!("  {}{}:{} {}", Colors::LIGHT_GRAY, i18n::t(i18n::MessageKey::CompressionsApplied), Colors::RESET, stats.compression_count);
            println!("  {}{}:{} {}", Colors::LIGHT_GRAY, i18n::t(i18n::MessageKey::CurrentContextTokens), Colors::RESET, stats.current_tokens);
            println!("  {}{}:{} {}", Colors::LIGHT_GRAY, i18n::t(i18n::MessageKey::MaxContextTokens), Colors::RESET, stats.max_context_tokens);
            println!("  {}{}:{} {:.1}%", Colors::LIGHT_GRAY, i18n::t(i18n::MessageKey::ContextUsage), Colors::RESET, stats.usage_ratio * 100.0);
            println!();

            if stats.compression_count == 0 {
                println!("  {}{}", i18n::t(i18n::MessageKey::CompressionNotNeeded), Colors::RESET);
            } else {
                println!("  {} {} {} {}", i18n::t(i18n::MessageKey::CompressionDone), stats.compression_count, i18n::t(i18n::MessageKey::CompressionTimes), Colors::RESET);
            }
            println!();
            continue;
        }

        // Stats command
        if trimmed.eq_ignore_ascii_case("/stats") {
            println!();
            let stats = agent.get_stats();
            println!("{}⬡{}  {}", Colors::ORANGE, Colors::RESET, i18n::t(i18n::MessageKey::UsageStatistics));
            println!();
            println!("  {}{}:{} {}", Colors::LIGHT_GRAY, i18n::t(i18n::MessageKey::ApiCalls), Colors::RESET, stats.cost_tracker.api_calls);
            println!("  {}{}:{} {}", Colors::LIGHT_GRAY, i18n::t(i18n::MessageKey::Iterations), Colors::RESET, stats.cost_tracker.iterations);
            println!("  {}{}:{} {}", Colors::LIGHT_GRAY, i18n::t(i18n::MessageKey::TotalTokens), Colors::RESET, stats.cost_tracker.total_tokens_used);
            println!("  {}  - {}:{} {}", Colors::LIGHT_GRAY, i18n::t(i18n::MessageKey::PromptTokens), Colors::RESET, stats.cost_tracker.prompt_tokens);
            println!("  {}  - Completion:{} {}", Colors::LIGHT_GRAY, Colors::RESET, stats.cost_tracker.completion_tokens);
            println!("  {}Est. Cost:{} ${:.4}", Colors::LIGHT_GRAY, Colors::RESET, stats.cost_tracker.estimated_cost_usd);
            println!();
            println!("  {}Rate Limiter:{} {}/{} calls remaining", 
                Colors::LIGHT_GRAY, Colors::RESET, 
                stats.rate_limiter.calls_remaining, 
                stats.rate_limiter.max_calls_per_minute);
            println!("  {}Tokens:{} {}/{} per min", 
                Colors::LIGHT_GRAY, Colors::RESET, 
                stats.rate_limiter.tokens_remaining, 
                stats.rate_limiter.max_tokens_per_minute);
            println!();
            println!("  {}Context Compression:{} {} ({}%)", 
                Colors::LIGHT_GRAY, Colors::RESET, 
                stats.compression_stats.compression_count,
                stats.compression_stats.usage_ratio * 100.0);
            println!();
            continue;
        }

        // Check if it's a skill command (starts with / and matches an actual skill name)
        if trimmed.starts_with('/') && !trimmed.starts_with("<<<") {
            let cmd = trimmed.trim_start_matches('/');
            if let Some(skill_name) = cmd.split_whitespace().next() {
                let available_skills = agent.list_skills();
                if available_skills.contains(&skill_name.to_string()) {
                    println!(
                        "{}Activating skill: {}{}",
                        Colors::LIGHT_GRAY,
                        skill_name,
                        Colors::RESET
                    );
                    let _ = agent.force_skill(skill_name);
                } else {
                    // Not a valid skill, show available skills
                    println!(
                        "{}{}: skill '{}' not found. Available skills:",
                        Colors::AMBER,
                        skill_name,
                        Colors::RESET
                    );
                    for skill in &available_skills {
                        println!("  {}• {}{}", Colors::LIGHT_GRAY, skill, Colors::RESET);
                    }
                }
            }
        }

        let input = if trimmed == "<<<" {
            let mut buf = String::new();
            let mut continuing = true;
            while continuing {
                print!("{}...{} ", Colors::LIGHT_GRAY, Colors::RESET);
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
            let mut buf = trimmed.strip_suffix('\\').unwrap_or(trimmed).to_string();
            buf.push('\n');
            let mut continuing = true;
            while continuing {
                print!("{}...{} ", Colors::LIGHT_GRAY, Colors::RESET);
                io::stdout().flush()?;
                match rl.readline("") {
                    Ok(cont_line) => {
                        let ct = cont_line.trim();
                        if ct.ends_with('\\') {
                            buf.push_str(ct.strip_suffix('\\').unwrap_or(ct));
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
                    println!("{}●{}  {}", Colors::AMBER, Colors::RESET, line);
                }
                println!();
            }
            Err(e) => {
                eprintln!(
                    "\n{}⨯{}  Error: {}{}\n",
                    Colors::RED,
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
