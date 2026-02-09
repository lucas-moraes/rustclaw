use crate::agent::Agent;
use crate::config::Config;
use crate::tavily::tools::{TavilyQuickSearchTool, TavilySearchTool};
use crate::tools::{
    capabilities::CapabilitiesTool, echo::EchoTool, file_list::FileListTool,
    file_read::FileReadTool, file_search::FileSearchTool, file_write::FileWriteTool,
    http::{HttpGetTool, HttpPostTool}, shell::ShellTool, system::SystemInfoTool,
    Tool, ToolRegistry,
};
use teloxide::types::BotCommand;
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
    description = "RustClaw - Raspberry Pi Edition"
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

pub struct BotState;

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
        let state = Arc::new(Mutex::new(BotState));

        info!("Starting Telegram bot (Raspberry Pi Edition)...");

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
                let help_text = r#"RustClaw - Raspberry Pi Edition

COMANDOS DO BOT:
/start - Iniciar o bot e ver boas-vindas
/status - Status do sistema (memória, RAM, tarefas)
/tasks - Listar tarefas agendadas
/clear_memory - Limpar todas as memórias
/internet <consulta> - Pesquisar na internet via Tavily
/help - Mostrar esta mensagem

AGENDAMENTO (usar cron do Linux):
Tarefas devem ser configuradas via crontab do sistema
Exemplo: 0 8 * * * /usr/local/bin/rustclaw --mode telegram --task heartbeat

FERRAMENTAS DISPONÍVEIS (via conversa):
• Sistema de arquivos: ler, escrever, listar, buscar arquivos
• Shell: executar comandos (ls, cat, etc.)
• HTTP: fazer requisições web
• Tavily: busca IA na internet (sem CAPTCHA)
• Sistema: informações de RAM, CPU, disco

EXEMPLOS DE USO:
"Liste os arquivos da pasta atual"
"Busque preço do bitcoin"
"Qual o clima em São Paulo?"
"Execute df -h para ver espaço em disco"

Nota: Esta é a versão otimizada para Raspberry Pi 3"#;
                bot.send_message(chat_id, help_text).await?;
            }
            Command::Status => {
                let status = Self::get_status(chat_id).await;
                bot.send_message(chat_id, status).await?;
            }
            Command::ClearMemory => {
                let keyboard = teloxide::types::InlineKeyboardMarkup::new(vec![
                    vec![
                        teloxide::types::InlineKeyboardButton::callback("Sim, limpar memória", "clear_memory_confirm"),
                        teloxide::types::InlineKeyboardButton::callback("Cancelar", "clear_memory_cancel"),
                    ],
                ]);
                
                bot.send_message(chat_id, "Tem certeza que deseja limpar as memórias?\n\nIsso apagará todas as conversas salvas.")
                    .reply_markup(keyboard)
                    .await?;
            }
            Command::Tasks => {
                let tasks = Self::get_tasks(chat_id).await;
                bot.send_message(chat_id, tasks).await?;
            }
            Command::Internet(query) => {
                if query.is_empty() {
                    bot.send_message(chat_id, "Use: /internet <texto da consulta>\nExemplo: /internet preço do bitcoin").await?;
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
                            bot.send_message(chat_id, format!("Erro na pesquisa: {}", e)).await?;
                        }
                    }
                } else {
                    bot.send_message(chat_id, "TAVILY_API_KEY não configurado. Configure a variável de ambiente.").await?;
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

        if text.starts_with("/add_task ") || text.starts_with("/remove_task ") {
            bot.send_message(chat_id, "Agendamento deve ser configurado via crontab do sistema Linux.\nUse: crontab -e").await?;
            return Ok(());
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
        }

        let memory_path = PathBuf::from(format!("data/memories_{}.db", chat_id.0));
        Agent::new(config.clone(), tools, &memory_path).expect("Failed to create agent")
    }

    fn welcome_message() -> String {
        r#"Bem-vindo ao RustClaw - Raspberry Pi Edition!

Sou seu assistente AI otimizado para Raspberry Pi 3:
✓ Memória persistente (SQLite)
✓ Ferramentas de sistema (shell, arquivos, HTTP)
✓ Busca na internet via Tavily API
✓ Baixo consumo de RAM

Comandos:
/start - Esta mensagem
/status - Status do sistema  
/clear_memory - Limpar memórias
/tasks - Ver tarefas (via cron)
/help - Ajuda completa

Para agendamento, use o crontab do Linux.

Vamos conversar!"#.to_string()
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
            "Status - Raspberry Pi Edition\n\nMemórias: {}\nRAM: {} MB / {} MB",
            memory_count,
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
                            "Nenhuma tarefa agendada.\n\nPara agendar, use crontab -e no Linux.".to_string()
                        } else {
                            let mut output = String::from("Tarefas Agendadas:\n\n");
                            for task in tasks {
                                output.push_str(&format!(
                                    "{}\n   ID: {}\n   Cron: {}\n\n",
                                    task.name,
                                    task.id,
                                    task.cron_expression
                                ));
                            }
                            output
                        }
                    }
                    Err(e) => format!("Erro: {}", e),
                }
            }
            Err(e) => format!("Erro ao acessar banco: {}", e),
        }
    }
}

use teloxide::utils::command::BotCommands;
