use crate::agent::Agent;
use crate::config::Config;
use crate::reminder_executor::ReminderExecutor;
use crate::tavily::tools::{TavilyQuickSearchTool, TavilySearchTool};
use crate::tools::{
    capabilities::CapabilitiesTool, datetime::DateTimeTool, echo::EchoTool,
    file_list::FileListTool, file_read::FileReadTool, file_search::FileSearchTool,
    file_write::FileWriteTool, http::{HttpGetTool, HttpPostTool},
    location::LocationTool, reminder::{AddReminderTool, CancelReminderTool, ListRemindersTool},
    shell::ShellTool, system::SystemInfoTool,
    skill_manager::{SkillCreateTool, SkillDeleteTool, SkillEditTool, SkillListTool, SkillRenameTool, SkillValidateTool},
    skill_import::SkillImportFromUrlTool,
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
    #[command(description = "Listar lembretes")]
    Reminders,
    #[command(description = "Cancelar lembrete", parse_with = "split")]
    CancelReminder(String),
    #[command(description = "Listar tarefas agendadas")]
    Tasks,
    #[command(description = "Pesquisar na internet", parse_with = "split")]
    Internet(String),
    #[command(description = "Ajuda")]
    Help,
}

pub struct BotState {
    // scheduler removed - module deleted
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
            BotCommand::new("reminders", "Listar lembretes"),
            BotCommand::new("cancel_reminder", "Cancelar lembrete"),
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
        
        // Start reminder executor
        if let Some(chat_id) = authorized_chat_id {
            let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id));
            let bot_clone = bot.clone();
            tokio::spawn(async move {
                let executor = ReminderExecutor::new(bot_clone, memory_path);
                executor.start().await;
            });
            info!("Reminder executor started for chat {}", chat_id);
        }
        
        // Scheduler removed - module deleted
        let state = Arc::new(Mutex::new(BotState {}));

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
                bot.send_message(chat_id, "Não autorizado").await?;
                return Ok(());
            }
        }

        match cmd {
            Command::Start => {
                bot.send_message(chat_id, Self::welcome_message()).await?;
            }
            Command::Help => {
                bot.send_message(chat_id, Self::help_message()).await?;
            }
            Command::Status => {
                let status = Self::get_status(chat_id).await;
                bot.send_message(chat_id, status).await?;
            }
            Command::ClearMemory => {
                let result = Self::clear_memory(chat_id).await;
                bot.send_message(chat_id, result).await?;
            }
            Command::Reminders => {
                let reminders = Self::get_reminders(chat_id).await;
                bot.send_message(chat_id, reminders).await?;
            }
            Command::CancelReminder(id) => {
                let result = Self::cancel_reminder(chat_id, &id).await;
                bot.send_message(chat_id, result).await?;
            }
            Command::Tasks => {
                let tasks = Self::get_tasks(chat_id).await;
                bot.send_message(chat_id, tasks).await?;
            }
            Command::Internet(query) => {
                if query.is_empty() {
                    bot.send_message(chat_id, "Use: /internet <consulta>").await?;
                    return Ok(());
                }
                
                bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;
                
                if let Some(ref tavily_key) = config.tavily_api_key {
                    let tool = TavilyQuickSearchTool::new(tavily_key.clone());
                    let args = serde_json::json!({ "query": query });
                    
                    match tool.call(args).await {
                        Ok(result) => {
                            bot.send_message(chat_id, format!("Resultados:\n\n{}", result)).await?;
                        }
                        Err(e) => {
                            bot.send_message(chat_id, format!("Erro: {}", e)).await?;
                        }
                    }
                } else {
                    bot.send_message(chat_id, "TAVILY_API_KEY não configurado").await?;
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
                bot.send_message(chat_id, "Não autorizado").await?;
                return Ok(());
            }
        }

        let text = match msg.text() {
            Some(t) => t,
            None => {
                bot.send_message(chat_id, "Envie texto").await?;
                return Ok(());
            }
        };

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
                bot.send_message(chat_id, format!("{}", text)).await?;
            }
            Ok(Err(e)) => {
                error!("Agent error: {}", e);
                bot.send_message(chat_id, format!("Erro: {}", e)).await?;
            }
            Err(e) => {
                error!("Task error: {}", e);
                bot.send_message(chat_id, "Erro interno").await?;
            }
        }

        Ok(())
    }

    fn create_agent(config: &Config, chat_id: ChatId) -> Agent {
        let mut tools = ToolRegistry::new();
        let config_arc = Arc::new(config.clone());
        let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));

        tools.register(Box::new(CapabilitiesTool::new()));
        tools.register(Box::new(DateTimeTool::new()));
        tools.register(Box::new(EchoTool));
        tools.register(Box::new(FileListTool::new()));
        tools.register(Box::new(FileReadTool::new()));
        tools.register(Box::new(FileSearchTool::new()));
        tools.register(Box::new(FileWriteTool::new()));
        tools.register(Box::new(HttpGetTool::new()));
        tools.register(Box::new(HttpPostTool::new()));
        tools.register(Box::new(LocationTool::new()));
        tools.register(Box::new(AddReminderTool::new(config_arc.clone(), &memory_path, chat_id.0)));
        tools.register(Box::new(ListRemindersTool::new(&memory_path, chat_id.0)));
        tools.register(Box::new(CancelReminderTool::new(&memory_path, chat_id.0)));
        tools.register(Box::new(ShellTool::new()));
        tools.register(Box::new(SystemInfoTool::new()));

        // Skill management tools
        tools.register(Box::new(SkillListTool::new()));
        tools.register(Box::new(SkillCreateTool::new()));
        tools.register(Box::new(SkillDeleteTool::new()));
        tools.register(Box::new(SkillEditTool::new("skills")));
        tools.register(Box::new(SkillRenameTool::new()));
        tools.register(Box::new(SkillValidateTool::new()));
        tools.register(Box::new(SkillImportFromUrlTool::new()));

        if let Some(ref tavily_key) = config.tavily_api_key {
            tools.register(Box::new(TavilySearchTool::new(tavily_key.clone())));
            tools.register(Box::new(TavilyQuickSearchTool::new(tavily_key.clone())));
        }

        Agent::new(config.clone(), tools, &memory_path).expect("Failed to create agent")
    }

    fn welcome_message() -> String {
        r#"Bem-vindo ao RustClaw!

Sou seu assistente AI com:
✓ Memória persistente
✓ Ferramentas de sistema
✓ Busca na internet
✓ Lembretes automáticos
✓ Browser automation

Comandos:
/start - Esta mensagem
/status - Status do sistema
/reminders - Ver lembretes
/cancel_reminder <id> - Cancelar
/tasks - Ver tarefas
/clear_memory - Limpar memórias
/internet <query> - Pesquisar
/help - Ajuda

Criar lembretes:
• "Me lembre amanhã às 10h"
• "Todo dia às 8h tomar remédio"

Vamos conversar!"#.to_string()
    }

    fn help_message() -> String {
        r#"Comandos disponíveis:

/reminders - Listar lembretes
/cancel_reminder <id> - Cancelar
/tasks - Listar tarefas
/status - Status do sistema
/clear_memory - Limpar memórias
/internet <query> - Pesquisar
/help - Esta mensagem

Exemplos de lembretes:
• "Me lembre amanhã às 10h"
• "Todo dia às 8h"
• "Daqui 2 horas"
• "Toda segunda às 9h""#.to_string()
    }

    async fn get_status(chat_id: ChatId) -> String {
        let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        let memory_count = if memory_path.exists() {
            match crate::memory::store::MemoryStore::new(&memory_path) {
                Ok(store) => store.count().unwrap_or(0),
                Err(_) => 0,
            }
        } else {
            0
        };

        let mut sys = System::new_all();
        sys.refresh_all();

        format!(
            "Status\n\nMemórias: {}\nRAM: {} MB / {} MB",
            memory_count,
            sys.used_memory() / 1024,
            sys.total_memory() / 1024
        )
    }

    async fn get_tasks(chat_id: ChatId) -> String {
        let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        
        // Scheduler module deleted - task listing disabled
        "📋 O agendador de tarefas foi removido.\n\nUse o sistema de lembretes com:\n• /reminders - Listar lembretes\n• /cancel_reminder - Cancelar lembrete".to_string()
    }

    async fn get_reminders(chat_id: ChatId) -> String {
        let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        
        match crate::memory::store::MemoryStore::new(&memory_path) {
            Ok(store) => {
                match store.get_pending_reminders(chat_id.0) {
                    Ok(reminders) => {
                        if reminders.is_empty() {
                            "📋 Nenhum lembrete.\n\nCrie um:\n• 'Me lembre amanhã às 10h'".to_string()
                        } else {
                            let mut output = String::from("📋 Lembretes:\n\n");
                            for (i, reminder) in reminders.iter().enumerate() {
                                let local_time = reminder.remind_at.with_timezone(&chrono::Local);
                                let icon = if reminder.is_recurring { "🔄" } else { "⏰" };
                                output.push_str(&format!(
                                    "{}. {} {}\n   📅 {}\n   🆔 {}\n\n",
                                    i + 1,
                                    icon,
                                    reminder.message,
                                    local_time.format("%d/%m %H:%M"),
                                    &reminder.id[..8]
                                ));
                            }
                            output
                        }
                    }
                    Err(e) => format!("❌ Erro: {}", e),
                }
            }
            Err(e) => format!("❌ Erro: {}", e),
        }
    }

    async fn cancel_reminder(chat_id: ChatId, id: &str) -> String {
        if id.is_empty() {
            return "Use: /cancel_reminder <id>".to_string();
        }
        
        let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        
        match crate::memory::store::MemoryStore::new(&memory_path) {
            Ok(store) => {
                match store.get_pending_reminders(chat_id.0) {
                    Ok(reminders) => {
                        let reminder_to_cancel = reminders.iter()
                            .find(|r| r.id.starts_with(id));
                        
                        match reminder_to_cancel {
                            Some(reminder) => {
                                if let Err(e) = store.delete_reminder(&reminder.id) {
                                    format!("❌ Erro: {}", e)
                                } else {
                                    format!("✅ Cancelado: {}", reminder.message)
                                }
                            }
                            None => format!("❌ ID '{}' não encontrado", id)
                        }
                    }
                    Err(e) => format!("❌ Erro: {}", e),
                }
            }
            Err(e) => format!("❌ Erro: {}", e),
        }
    }

    async fn clear_memory(chat_id: ChatId) -> String {
        let path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        match tokio::fs::remove_file(&path).await {
            Ok(_) => "🧹 Memória limpa!".to_string(),
            Err(_) => "🧹 Memória já estava limpa".to_string(),
        }
    }
}

use teloxide::utils::command::BotCommands;
