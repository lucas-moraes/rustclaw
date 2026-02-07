use crate::agent::Agent;
use crate::config::Config;
use crate::tools::{
    capabilities::CapabilitiesTool, echo::EchoTool, file_list::FileListTool,
    file_read::FileReadTool, file_search::FileSearchTool, file_write::FileWriteTool,
    http::{HttpGetTool, HttpPostTool}, shell::ShellTool, system::SystemInfoTool,
    ToolRegistry,
};
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use sysinfo::System;
use teloxide::prelude::*;
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
    #[command(description = "Ajuda")]
    Help,
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
        let config = Arc::new(config);

        info!("Starting Telegram bot...");

        let config_cmd = Arc::clone(&config);
        let config_msg = Arc::clone(&config);

        let handler = dptree::entry()
            .branch(
                Update::filter_message()
                    .filter_command::<Command>()
                    .endpoint(
                        move |bot: Bot, msg: Message, cmd: Command| {
                            let config = Arc::clone(&config_cmd);
                            async move {
                                Self::handle_command(bot, msg, cmd, &config, authorized_chat_id).await
                            }
                        }
                    ),
            )
            .branch(
                Update::filter_message()
                    .endpoint(
                        move |bot: Bot, msg: Message| {
                            let config = Arc::clone(&config_msg);
                            async move {
                                Self::handle_message(bot, msg, &config, authorized_chat_id).await
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
        _config: &Config,
        authorized_chat_id: Option<i64>,
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
                bot.send_message(chat_id, Command::descriptions().to_string()).await?;
            }
            Command::Status => {
                let status = Self::get_status(chat_id).await;
                bot.send_message(chat_id, status).await?;
            }
            Command::ClearMemory => {
                let result = Self::clear_memory(chat_id).await;
                bot.send_message(chat_id, result).await?;
            }
        }

        Ok(())
    }

    async fn handle_message(
        bot: Bot,
        msg: Message,
        config: &Config,
        authorized_chat_id: Option<i64>,
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

        info!("Message from {}: {}", chat_id, text);
        bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;

        // Process in blocking task
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
                bot.send_message(chat_id, format!("🤖 {}", text)).await?;
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

        let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        Agent::new(config.clone(), tools, &memory_path).expect("Failed to create agent")
    }

    fn welcome_message() -> String {
        r#"🦀 Bem-vindo ao RustClaw!

Sou seu assistente AI com memória persistente.

Comandos:
/start - Esta mensagem
/status - Status do sistema  
/clear_memory - Limpar memórias
/help - Ajuda

Vamos conversar!"#.to_string()
    }

    async fn get_status(chat_id: ChatId) -> String {
        let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        let count = if memory_path.exists() {
            crate::memory::store::MemoryStore::new(&memory_path)
                .and_then(|s| s.count())
                .unwrap_or(0)
        } else {
            0
        };

        let mut sys = System::new_all();
        sys.refresh_all();

        format!(
            "📊 Status\n\n📝 Memórias: {}\n🧠 RAM: {} MB / {} MB",
            count,
            sys.used_memory() / 1024,
            sys.total_memory() / 1024
        )
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
