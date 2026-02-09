use crate::agent::Agent;
use crate::browser::tools::{
    BrowserExtractTool, BrowserNavigateTool, BrowserScreenshotTool, BrowserSearchTool,
    BrowserTestTool,
};
use crate::config::Config;
use crate::scheduler::task::{ScheduledTask, TaskType};
use crate::scheduler::SchedulerService;
use crate::tavily::tools::{TavilyQuickSearchTool, TavilySearchTool};
use crate::tools::{
    capabilities::CapabilitiesTool, echo::EchoTool, file_list::FileListTool,
    file_read::FileReadTool, file_search::FileSearchTool, file_write::FileWriteTool,
    http::{HttpGetTool, HttpPostTool}, shell::ShellTool, system::SystemInfoTool,
    Tool, ToolRegistry,
};
use teloxide::types::{BotCommand, InputFile};
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use sysinfo::System;
use teloxide::prelude::*;
use tokio::sync::Mutex;
use tracing::{error, info};

pub struct TelegramBot;

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "RustClaw - Agente AI com memória persistente"
)]
pub enum Command {
    #[command(description = "Iniciar o bot")]
    Start,
    #[command(description = "Status do sistema")]
    Status,
    #[command(description = "Limpar memória")]
    ClearMemory,
    #[command(description = "Listar tarefas agendadas")]
    Tasks,
    #[command(description = "Pesquisar na internet", parse_with = "split")]
    Internet(String),
    #[command(description = "Ajuda")]
    Help,
}

pub struct BotState {
    scheduler: Arc<Mutex<Option<SchedulerService>>>,
}

impl TelegramBot {
    pub async fn run(config: Config) -> anyhow::Result<()> {
        let token = env::var("TELEGRAM_TOKEN")
            .map_err(|_| anyhow::anyhow!("TELEGRAM_TOKEN not set"))?;

        let authorized_chat_id = env::var("TELEGRAM_CHAT_ID")
            .ok()
            .and_then(|id| id.parse::<i64>().ok());

        if let Some(id) = authorized_chat_id {
            info!("Bot restricted to chat ID: {}", id);
        }

        let bot = Bot::new(token);
        
        
        let commands = vec![
            BotCommand::new("start", "Iniciar o bot"),
            BotCommand::new("status", "Status do sistema"),
            BotCommand::new("tasks", "Listar tarefas agendadas"),
            BotCommand::new("clear_memory", "Limpar memórias"),
            BotCommand::new("internet", "Pesquisar na internet"),
            BotCommand::new("help", "Ajuda e comandos disponíveis"),
        ];
        
        if let Err(e) = bot.set_my_commands(commands).await {
            error!("Failed to set commands: {}", e);
        } else {
            info!("Commands registered successfully");
        }
        
        let config = Arc::new(config);
        
        
        let scheduler = if let Some(chat_id) = authorized_chat_id {
            let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id));
            match SchedulerService::new(&memory_path, chat_id).await {
                Ok(svc) => {
                    info!("Initializing scheduler for chat {}", chat_id);
                    
                    
                    svc.init_default_tasks().await?;
                    
                    
                    let bot_clone = bot.clone();
                    let callback = move |chat_id: i64, message: String| {
                        let bot = bot_clone.clone();
                        tokio::spawn(async move {
                            if let Err(e) = bot.send_message(ChatId(chat_id), message).await {
                                error!("Failed to send scheduled message: {}", e);
                            }
                        });
                    };
                    
                    svc.load_and_schedule_tasks(callback).await?;
                    svc.start().await?;
                    
                    Some(svc)
                }
                Err(e) => {
                    error!("Failed to create scheduler: {}", e);
                    None
                }
            }
        } else {
            info!("No authorized chat ID, scheduler disabled");
            None
        };

        let state = Arc::new(Mutex::new(BotState {
            scheduler: Arc::new(Mutex::new(scheduler)),
        }));

        info!("Starting Telegram bot...");

        let config_cmd = Arc::clone(&config);
        let config_msg = Arc::clone(&config);
        let state_cmd = Arc::clone(&state);
        let state_msg = Arc::clone(&state);

        let handler = dptree::entry()
            .branch(
                Update::filter_message()
                    .filter_command::<Command>()
                    .endpoint(
                        move |bot: Bot, msg: Message, cmd: Command| {
                            let config = Arc::clone(&config_cmd);
                            let state = Arc::clone(&state_cmd);
                            async move {
                                Self::handle_command(bot, msg, cmd, &config, authorized_chat_id, &state).await
                            }
                        }
                    ),
            )
            .branch(
                Update::filter_message()
                    .endpoint(
                        move |bot: Bot, msg: Message| {
                            let config = Arc::clone(&config_msg);
                            let state = Arc::clone(&state_msg);
                            async move {
                                Self::handle_message(bot, msg, &config, authorized_chat_id, &state).await
                            }
                        }
                    ),
            );

        Dispatcher::builder(bot, handler)
            .enable_ctrlc_handler()
            .build()
            .dispatch()
            .await;

        Ok(())
    }

    async fn handle_command(
        bot: Bot,
        msg: Message,
        cmd: Command,
        config: &Config,
        authorized_chat_id: Option<i64>,
        _state: &Arc<Mutex<BotState>>,
    ) -> ResponseResult<()> {
        let chat_id = msg.chat.id;

        if let Some(auth_id) = authorized_chat_id {
            if chat_id.0 != auth_id {
                bot.send_message(chat_id, "⛔ Não autorizado").await?;
                return Ok(());
            }
        }

        match cmd {
            Command::Start => {
                bot.send_message(chat_id, Self::welcome_message()).await?;
            }
            Command::Help => {
                let help_text = r#"🦀 RustClaw - Comandos Disponíveis

📱 COMANDOS DO BOT:
/start - Iniciar o bot e ver boas-vindas
/status - Status do sistema (memória, RAM, tarefas)
/tasks - Listar tarefas agendadas
/clear_memory - Limpar todas as memórias
/internet <consulta> - Pesquisar na internet
/help - Mostrar esta mensagem

⏰ AGENDAMENTO:
/add_task <nome> <cron> <tipo> - Adicionar tarefa agendada
   Exemplo: /add_task Backup "0 2 * * *" reminder "Fazer backup"
/remove_task <id> - Remover tarefa pelo ID

🛠️ FERRAMENTAS DISPONÍVEIS (via conversa):
• Sistema de arquivos: ler, escrever, listar, buscar arquivos
• Shell: executar comandos (ls, cat, etc.)
• HTTP: fazer requisições web
• Tavily: busca IA na internet (sem CAPTCHA)
• Browser: navegar em sites, tirar screenshots
• Sistema: informações de RAM, CPU, disco

💡 EXEMPLOS DE USO:
"Liste os arquivos da pasta atual"
"Busque preço do bitcoin"
"Acesse example.com e tire screenshot"
"Qual o clima em São Paulo?"
"Execute df -h para ver espaço em disco"

📊 O bot também envia Heartbeat automático às 8h com status do sistema."#;
                bot.send_message(chat_id, help_text).await?;
            }
            Command::Status => {
                let status = Self::get_status(chat_id).await;
                bot.send_message(chat_id, status).await?;
            }
            Command::ClearMemory => {
                let result = Self::clear_memory(chat_id).await;
                bot.send_message(chat_id, result).await?;
            }
            Command::Tasks => {
                let tasks = Self::get_tasks(chat_id).await;
                bot.send_message(chat_id, tasks).await?;
            }
            Command::Internet(query) => {
                if query.is_empty() {
                    bot.send_message(chat_id, "❌ Use: /internet <texto da consulta>\nExemplo: /internet preço do bitcoin").await?;
                    return Ok(());
                }
                
                bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;
                
                
                if let Some(ref tavily_key) = config.tavily_api_key {
                    let tool = TavilyQuickSearchTool::new(tavily_key.clone());
                    let args = serde_json::json!({ "query": query });
                    
                    match tool.call(args).await {
                        Ok(result) => {
                            bot.send_message(chat_id, format!("🔍 Resultados:\n\n{}", result)).await?;
                        }
                        Err(e) => {
                            bot.send_message(chat_id, format!("❌ Erro na pesquisa: {}", e)).await?;
                        }
                    }
                } else {
                    
                    let tool = BrowserSearchTool::new();
                    let args = serde_json::json!({ "query": query });
                    
                    match tool.call(args).await {
                        Ok(result) => {
                            bot.send_message(chat_id, format!("🔍 Resultados:\n\n{}", result)).await?;
                            
                            
                            if result.contains("data/screenshots/") {
                                if let Some(start) = result.find("data/screenshots/") {
                                    let path_end = result[start..].find('\n').unwrap_or(result[start..].len());
                                    let screenshot_path = &result[start..start + path_end];
                                    if std::path::Path::new(screenshot_path).exists() {
                                        let photo = InputFile::file(screenshot_path);
                                        bot.send_photo(chat_id, photo).await?;
                                        let _ = tokio::fs::remove_file(screenshot_path).await;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            bot.send_message(chat_id, format!("❌ Erro na pesquisa: {}", e)).await?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_message(
        bot: Bot,
        msg: Message,
        config: &Config,
        authorized_chat_id: Option<i64>,
        _state: &Arc<Mutex<BotState>>,
    ) -> ResponseResult<()> {
        let chat_id = msg.chat.id;

        if let Some(auth_id) = authorized_chat_id {
            if chat_id.0 != auth_id {
                bot.send_message(chat_id, "⛔ Não autorizado").await?;
                return Ok(());
            }
        }

        let text = match msg.text() {
            Some(t) => t,
            None => {
                bot.send_message(chat_id, "❌ Envie texto").await?;
                return Ok(());
            }
        };

        
        if text.starts_with("/add_task ") {
            return Self::handle_add_task(bot, msg.clone(), text).await;
        }

        
        if text.starts_with("/remove_task ") {
            return Self::handle_remove_task(bot, msg.clone(), text).await;
        }

        info!("Message from {}: {}", chat_id, text);
        bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;

        
        let config = config.clone();
        let text = text.to_string();

        let response = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                let mut agent = Self::create_agent(&config, chat_id);
                agent.prompt(&text).await
            })
        })
        .await;

        match response {
            Ok(Ok(text)) => {
                
                if text.contains("data/screenshots/") && text.contains(".png") {
                    
                    if let Some(start) = text.find("data/screenshots/") {
                        let path_end = text[start..].find('\n').unwrap_or(text[start..].len());
                        let screenshot_path = &text[start..start + path_end];
                        
                        
                        let clean_text = text.replace(screenshot_path, "[screenshot attached]");
                        bot.send_message(chat_id, format!("🤖 {}", clean_text)).await?;
                        
                        
                        if std::path::Path::new(screenshot_path).exists() {
                            let photo = InputFile::file(screenshot_path);
                            bot.send_photo(chat_id, photo).await?;
                            
                            
                            let _ = tokio::fs::remove_file(screenshot_path).await;
                        }
                    } else {
                        bot.send_message(chat_id, format!("🤖 {}", text)).await?;
                    }
                } else {
                    bot.send_message(chat_id, format!("🤖 {}", text)).await?;
                }
            }
            Ok(Err(e)) => {
                error!("Agent error: {}", e);
                bot.send_message(chat_id, format!("❌ Erro: {}", e)).await?;
            }
            Err(e) => {
                error!("Task error: {}", e);
                bot.send_message(chat_id, "❌ Erro interno").await?;
            }
        }

        Ok(())
    }

    async fn handle_add_task(
        bot: Bot,
        msg: Message,
        text: &str,
    ) -> ResponseResult<()> {
        let chat_id = msg.chat.id;
        
        
        let parts: Vec<&str> = text.split_whitespace().collect();
        if parts.len() < 4 {
            bot.send_message(
                chat_id,
                "❌ Formato: /add_task <nome> <cron> <tipo>\nEx: /add_task Teste '*/5 * * * *' reminder 'mensagem'",
            ).await?;
            return Ok(());
        }

        let name = parts[1];
        let cron = parts[2].trim_matches('\'').trim_matches('"');
        let task_type = parts[3];

        let task_type = if task_type == "heartbeat" {
            TaskType::Heartbeat
        } else if task_type == "system_check" {
            TaskType::SystemCheck
        } else if task_type.starts_with("reminder:") || task_type == "reminder" {
            let reminder_msg = if parts.len() > 4 {
                parts[4..].join(" ")
            } else {
                "Lembrete!".to_string()
            };
            TaskType::Reminder(reminder_msg)
        } else {
            TaskType::Custom(task_type.to_string())
        };

        let task = ScheduledTask::new(name.to_string(), cron.to_string(), task_type);
        
        // Save to database
        let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        if let Ok(store) = crate::memory::store::MemoryStore::new(&memory_path) {
            if let Err(e) = store.save_task(&task) {
                bot.send_message(chat_id, format!("❌ Erro ao salvar tarefa: {}", e)).await?;
            } else {
                bot.send_message(
                    chat_id,
                    format!("✅ Tarefa '{}' adicionada!\nCron: {}\nReinicie o bot para ativar.", name, cron),
                ).await?;
            }
        } else {
            bot.send_message(chat_id, "❌ Erro ao acessar banco de dados").await?;
        }

        Ok(())
    }

    async fn handle_remove_task(
        bot: Bot,
        msg: Message,
        text: &str,
    ) -> ResponseResult<()> {
        let chat_id = msg.chat.id;
        
        let parts: Vec<&str> = text.split_whitespace().collect();
        if parts.len() < 2 {
            bot.send_message(chat_id, "❌ Formato: /remove_task <id>").await?;
            return Ok(());
        }

        let task_id = parts[1];
        
        let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        if let Ok(store) = crate::memory::store::MemoryStore::new(&memory_path) {
            if let Err(e) = store.delete_task(task_id) {
                bot.send_message(chat_id, format!("❌ Erro ao remover: {}", e)).await?;
            } else {
                bot.send_message(chat_id, "✅ Tarefa removida!").await?;
            }
        } else {
            bot.send_message(chat_id, "❌ Erro ao acessar banco de dados").await?;
        }

        Ok(())
    }

    fn create_agent(config: &Config, chat_id: ChatId) -> Agent {
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
        
        // Tavily search tools (IA-powered search without CAPTCHAs)
        if let Some(ref tavily_key) = config.tavily_api_key {
            tools.register(Box::new(TavilySearchTool::new(tavily_key.clone())));
            tools.register(Box::new(TavilyQuickSearchTool::new(tavily_key.clone())));
        }
        
        // Browser automation tools (Fase 6)
        tools.register(Box::new(BrowserNavigateTool::new()));
        tools.register(Box::new(BrowserSearchTool::new()));
        tools.register(Box::new(BrowserExtractTool::new()));
        tools.register(Box::new(BrowserScreenshotTool::new()));
        tools.register(Box::new(BrowserTestTool::new()));

        let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        Agent::new(config.clone(), tools, &memory_path).expect("Failed to create agent")
    }

    fn welcome_message() -> String {
        r#"🦀 Bem-vindo ao RustClaw!

Sou seu assistente AI proativo com:
✅ Memória persistente
✅ Ferramentas de sistema (shell, arquivos, HTTP)
✅ Tarefas agendadas (Heartbeat às 8h)
🌐 Navegação web 
📸 Screenshots de páginas

Comandos:
/start - Esta mensagem
/status - Status do sistema  
/clear_memory - Limpar memórias
/tasks - Ver tarefas agendadas
/help - Ajuda

Tarefas:
/add_task <nome> <cron> <tipo> - Adicionar tarefa
/remove_task <id> - Remover tarefa

Exemplos de uso:
• "Busque preço do bitcoin"
• "Acesse example.com e tire screenshot"
• "Liste arquivos do diretório atual"

Vamos conversar!"#.to_string()
    }

    async fn get_status(chat_id: ChatId) -> String {
        let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        let (memory_count, task_count) = if memory_path.exists() {
            match crate::memory::store::MemoryStore::new(&memory_path) {
                Ok(store) => (
                    store.count().unwrap_or(0),
                    store.count_tasks().unwrap_or(0),
                ),
                Err(_) => (0, 0),
            }
        } else {
            (0, 0)
        };

        let mut sys = System::new_all();
        sys.refresh_all();

        format!(
            "📊 Status\n\n📝 Memórias: {}\n⏰ Tarefas: {}\n🧠 RAM: {} MB / {} MB",
            memory_count,
            task_count,
            sys.used_memory() / 1024,
            sys.total_memory() / 1024
        )
    }

    async fn get_tasks(chat_id: ChatId) -> String {
        let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        
        match crate::memory::store::MemoryStore::new(&memory_path) {
            Ok(store) => {
                match store.get_all_tasks() {
                    Ok(tasks) => {
                        if tasks.is_empty() {
                            "📋 Nenhuma tarefa agendada.\n\nUse /add_task para criar uma.".to_string()
                        } else {
                            let mut output = String::from("📋 Tarefas Agendadas:\n\n");
                            for task in tasks {
                                let status = if task.is_active { "✅" } else { "❌" };
                                output.push_str(&format!(
                                    "{} {}\n   ID: {}\n   Cron: {}\n   Tipo: {}\n\n",
                                    status,
                                    task.name,
                                    task.id,
                                    task.cron_expression,
                                    task.get_type_string()
                                ));
                            }
                            output.push_str("Use /remove_task <id> para remover");
                            output
                        }
                    }
                    Err(e) => format!("❌ Erro: {}", e),
                }
            }
            Err(e) => format!("❌ Erro ao acessar banco: {}", e),
        }
    }

    async fn clear_memory(chat_id: ChatId) -> String {
        let path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        match tokio::fs::remove_file(&path).await {
            Ok(_) => "🧹 Memória limpa! Reinicie o bot para recriar as tarefas padrão.".to_string(),
            Err(_) => "🧹 Memória já estava limpa".to_string(),
        }
    }
}

use teloxide::utils::command::BotCommands;
