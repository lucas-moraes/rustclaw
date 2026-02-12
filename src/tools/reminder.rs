use crate::config::Config;
use crate::memory::reminder::{Reminder, ReminderType};
use crate::memory::store::MemoryStore;
use crate::tools::reminder_parser::ReminderParser;
use crate::tools::Tool;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

pub struct AddReminderTool {
    config: Arc<Config>,
    memory_path: String,
    chat_id: i64,
}

impl AddReminderTool {
    pub fn new(config: Arc<Config>, memory_path: &Path, chat_id: i64) -> Self {
        Self {
            config,
            memory_path: memory_path.to_string_lossy().to_string(),
            chat_id,
        }
    }
}

#[async_trait::async_trait]
impl Tool for AddReminderTool {
    fn name(&self) -> &str {
        "add_reminder"
    }

    fn description(&self) -> &str {
        "Cria um lembrete com data/hora. Input: {\"text\": \"mensagem com data\"} ou {\"message\": \"texto\", \"when\": \"amanhã às 10h\"}"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        // Parse input - can be either full text or structured
        let (message, _when) = if let Some(text) = args["text"].as_str() {
            // Parse from full text like "Me lembre de tomar remédio amanhã às 8h"
            (text.to_string(), None)
        } else if let (Some(msg), Some(when_str)) = (args["message"].as_str(), args["when"].as_str()) {
            // Structured input
            (format!("{} {}", msg, when_str), Some(when_str.to_string()))
        } else {
            return Err("Formato inválido. Use: {\"text\": \"mensagem com data\"} ou {\"message\": \"texto\", \"when\": \"amanhã às 10h\"}".to_string());
        };

        // Parse the reminder
        let parsed = match ReminderParser::parse(&message, &self.config.timezone) {
            Some(p) => p,
            None => {
                return Err(format!(
                    "Não consegui entender a data/hora. Tente formatos como:\n\
                    - 'amanhã às 10h'\n\
                    - 'daqui 2 horas'\n\
                    - 'todo dia às 8h'\n\
                    Timezone atual: {}",
                    self.config.timezone
                ));
            }
        };

        let reminder = match parsed.reminder_type {
            ReminderType::Single => {
                let datetime = parsed.datetime.ok_or("Data não parseada")?;
                Reminder::new(parsed.message.clone(), datetime, self.chat_id)
            }
            ReminderType::Recurring(cron) => {
                let datetime = parsed.datetime.ok_or("Data não parseada")?;
                Reminder::new_recurring(parsed.message.clone(), cron, datetime, self.chat_id)
            }
        };

        let path = std::path::Path::new(&self.memory_path);
        let store = MemoryStore::new(path).map_err(|e| format!("Erro ao acessar banco: {}", e))?;
        
        // Obter o próximo ID antes de salvar
        let reminder_id = store.get_next_reminder_id()
            .map_err(|e| format!("Erro ao gerar ID: {}", e))?;
        
        store.save_reminder(&reminder)
            .map_err(|e| format!("Erro ao salvar lembrete: {}", e))?;

        tracing::info!(
            "Reminder created: id={}, message='{}', chat_id={}, remind_at={}",
            reminder_id,
            reminder.message,
            reminder.chat_id,
            reminder.remind_at
        );

        let local_time = reminder.remind_at.with_timezone(&chrono::Local);
        let formatted_time = local_time.format("%d/%m/%Y às %H:%M").to_string();
        
        let response = if reminder.is_recurring {
            let cron_desc = Self::cron_to_description(&reminder.cron_expression);
            format!(
                "✅ Lembrete recorrente criado!\n\
                🆔 ID: {}\n\
                📝 Mensagem: {}\n\
                🔄 Frequência: {}\n\
                📅 Próximo: {} ({})\n\n\
                Para cancelar: /cancel_reminder {}",
                reminder_id,
                reminder.message,
                cron_desc,
                formatted_time,
                self.config.timezone,
                reminder_id
            )
        } else {
            format!(
                "✅ Lembrete criado!\n\
                🆔 ID: {}\n\
                📝 Mensagem: {}\n\
                📅 Quando: {} ({})\n\n\
                Para cancelar: /cancel_reminder {}",
                reminder_id,
                reminder.message,
                formatted_time,
                self.config.timezone,
                reminder_id
            )
        };

        Ok(response)
    }
}

impl AddReminderTool {
    fn cron_to_description(cron: &Option<String>) -> String {
        match cron {
            Some(c) => {
                if c == "0 0 8 * * *" || c == "0 8 * * *" {
                    "Todo dia às 8:00".to_string()
                } else if c.starts_with("0 ") && c.ends_with(" * * *") {
                    // Daily at specific time
                    let parts: Vec<&str> = c.split_whitespace().collect();
                    if parts.len() >= 3 {
                        format!("Todo dia às {}:{}", parts[2], parts[1])
                    } else {
                        c.clone()
                    }
                } else if c.ends_with(" * * 1") {
                    "Toda segunda-feira".to_string()
                } else if c.ends_with(" * * 2") {
                    "Toda terça-feira".to_string()
                } else if c.ends_with(" * * 3") {
                    "Toda quarta-feira".to_string()
                } else if c.ends_with(" * * 4") {
                    "Toda quinta-feira".to_string()
                } else if c.ends_with(" * * 5") {
                    "Toda sexta-feira".to_string()
                } else if c.ends_with(" * * 6") {
                    "Todo sábado".to_string()
                } else if c.ends_with(" * * 0") {
                    "Todo domingo".to_string()
                } else {
                    c.clone()
                }
            }
            None => "Desconhecida".to_string(),
        }
    }
}

pub struct ListRemindersTool {
    memory_path: String,
    chat_id: i64,
}

impl ListRemindersTool {
    pub fn new(memory_path: &Path, chat_id: i64) -> Self {
        Self {
            memory_path: memory_path.to_string_lossy().to_string(),
            chat_id,
        }
    }
}

#[async_trait::async_trait]
impl Tool for ListRemindersTool {
    fn name(&self) -> &str {
        "list_reminders"
    }

    fn description(&self) -> &str {
        "Lista todos os lembretes pendentes. Input: {}"
    }

    async fn call(&self, _args: Value) -> Result<String, String> {
        let path = std::path::Path::new(&self.memory_path);
        let store = MemoryStore::new(path).map_err(|e| format!("Erro ao acessar banco: {}", e))?;
        
        let reminders = store.get_pending_reminders(self.chat_id)
            .map_err(|e| format!("Erro ao buscar lembretes: {}", e))?;

        if reminders.is_empty() {
            return Ok("📋 Nenhum lembrete pendente.\n\nPara criar um, diga algo como:\n• 'Me lembre amanhã às 10h'\n• 'Todo dia às 8h tomar remédio'".to_string());
        }

        let mut output = String::from("📋 Seus Lembretes:\n\n");
        
        for reminder in reminders.iter() {
            let local_time = reminder.remind_at.with_timezone(&chrono::Local);
            let formatted_time = local_time.format("%d/%m/%Y %H:%M").to_string();
            
            let icon = if reminder.is_recurring { "🔄" } else { "⏰" };
            let rec_text = if reminder.is_recurring { " (recorrente)" } else { "" };
            
            output.push_str(&format!(
                "🆔 ID: {} {}\n   📝 {}\n   📅 {}\n\n",
                reminder.id.split('-').next().unwrap_or(&reminder.id),
                icon,
                reminder.message,
                formatted_time
            ));
        }

        output.push_str(&format!("Total: {} lembrete(s)\n\nPara cancelar: /cancel_reminder <ID>", reminders.len()));
        
        Ok(output)
    }
}

pub struct CancelReminderTool {
    memory_path: String,
    chat_id: i64,
}

impl CancelReminderTool {
    pub fn new(memory_path: &Path, chat_id: i64) -> Self {
        Self {
            memory_path: memory_path.to_string_lossy().to_string(),
            chat_id,
        }
    }
}

#[async_trait::async_trait]
impl Tool for CancelReminderTool {
    fn name(&self) -> &str {
        "cancel_reminder"
    }

    fn description(&self) -> &str {
        "Cancela um lembrete pelo ID. Input: {\"id\": \"abc-123\"}"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let id = args["id"].as_str()
            .ok_or("ID do lembrete é obrigatório. Use: {\"id\": \"abc-123\"}")?;

        let path = std::path::Path::new(&self.memory_path);
        let store = MemoryStore::new(path).map_err(|e| format!("Erro ao acessar banco: {}", e))?;
        
        // First try to find by full ID
        let reminders = store.get_pending_reminders(self.chat_id)
            .map_err(|e| format!("Erro ao buscar lembretes: {}", e))?;
        
        let reminder_to_cancel = reminders.iter()
            .find(|r| r.id.starts_with(id) || r.id == id);
        
        match reminder_to_cancel {
            Some(reminder) => {
                store.delete_reminder(&reminder.id)
                    .map_err(|e| format!("Erro ao cancelar lembrete: {}", e))?;
                
                Ok(format!(
                    "✅ Lembrete cancelado!\n\
                    📝 {}\n\
                    🆔 {}",
                    reminder.message,
                    reminder.id.split('-').next().unwrap_or(&reminder.id)
                ))
            }
            None => {
                Err(format!(
                    "❌ Lembrete não encontrado com ID '{}'.\n\
                    Use 'list_reminders' para ver os IDs disponíveis.",
                    id
                ))
            }
        }
    }
}
