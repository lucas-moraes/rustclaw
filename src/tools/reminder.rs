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
        "Cria um lembrete com data/hora. Input: {\"text\": \"mensagem com data\"}"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let text = args["text"].as_str()
            .ok_or("Formato inválido. Use: {\"text\": \"mensagem com data\"}")?;

        let parsed = match ReminderParser::parse(text, &self.config.timezone) {
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
        
        store.save_reminder(&reminder)
            .map_err(|e| format!("Erro ao salvar lembrete: {}", e))?;

        let local_time = reminder.remind_at.with_timezone(&chrono::Local);
        let formatted_time = local_time.format("%d/%m/%Y às %H:%M").to_string();
        
        let response = if reminder.is_recurring {
            format!(
                "✅ Lembrete recorrente criado!\n\
                📝 Mensagem: {}\n\
                📅 Próximo: {} ({})",
                reminder.message,
                formatted_time,
                self.config.timezone
            )
        } else {
            format!(
                "✅ Lembrete criado!\n\
                📝 Mensagem: {}\n\
                📅 Quando: {} ({})",
                reminder.message,
                formatted_time,
                self.config.timezone
            )
        };

        Ok(response)
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
        
        for (i, reminder) in reminders.iter().enumerate() {
            let local_time = reminder.remind_at.with_timezone(&chrono::Local);
            let formatted_time = local_time.format("%d/%m/%Y %H:%M").to_string();
            
            let icon = if reminder.is_recurring { "🔄" } else { "⏰" };
            let rec_text = if reminder.is_recurring { " (recorrente)" } else { "" };
            
            output.push_str(&format!(
                "{}. {}\n   📝 {}{}\n   📅 {}\n   🆔 {}\n\n",
                i + 1,
                icon,
                reminder.message,
                rec_text,
                formatted_time,
                &reminder.id[..8]
            ));
        }

        output.push_str(&format!("Total: {} lembrete(s)", reminders.len()));
        
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
        "Cancela um lembrete pelo ID. Input: {\"id\": \"abc123\"}"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let id = args["id"].as_str()
            .ok_or("ID do lembrete é obrigatório")?;

        let path = std::path::Path::new(&self.memory_path);
        let store = MemoryStore::new(path).map_err(|e| format!("Erro ao acessar banco: {}", e))?;
        
        let reminders = store.get_pending_reminders(self.chat_id)
            .map_err(|e| format!("Erro ao buscar lembretes: {}", e))?;
        
        let reminder_to_cancel = reminders.iter()
            .find(|r| r.id.starts_with(id));
        
        match reminder_to_cancel {
            Some(reminder) => {
                store.delete_reminder(&reminder.id)
                    .map_err(|e| format!("Erro ao cancelar: {}", e))?;
                
                Ok(format!(
                    "✅ Lembrete cancelado!\n📝 {}\n🆔 {}",
                    reminder.message,
                    &reminder.id[..8]
                ))
            }
            None => {
                Err(format!(
                    "❌ Lembrete não encontrado com ID '{}'. Use list_reminders para ver os IDs.",
                    id
                ))
            }
        }
    }
}
