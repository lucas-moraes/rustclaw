use crate::memory::reminder::Reminder;
use crate::memory::store::MemoryStore;
use chrono::Utc;
use std::path::PathBuf;
use std::time::Duration;
use teloxide::prelude::*;
use tracing::{error, info, warn};
use std::collections::HashMap;
use tokio::sync::Mutex;

pub struct ReminderExecutor {
    bot: Bot,
}

impl ReminderExecutor {
    pub fn new(bot: Bot) -> Self {
        Self { bot }
    }

    pub async fn start(&self) {
        info!("Starting reminder executor...");

        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;

            // Check all memory files in data directory
            let data_dir = std::path::Path::new("data");
            if let Ok(entries) = std::fs::read_dir(data_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                        if filename.starts_with("memories_") && filename.ends_with(".db") {
                            if let Some(chat_id_str) = filename.strip_prefix("memories_").and_then(|s| s.strip_suffix(".db")) {
                                if let Ok(chat_id) = chat_id_str.parse::<i64>() {
                                    if let Err(e) = self.check_reminders_for_chat(chat_id, &path).await {
                                        error!("Error checking reminders for chat {}: {}", chat_id, e);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    async fn check_reminders_for_chat(&self, chat_id: i64, memory_path: &PathBuf) -> anyhow::Result<()> {
        let store = MemoryStore::new(memory_path)?;
        let now = Utc::now();

        let due_reminders = store.get_due_reminders(now)?;

        for reminder in due_reminders {
            match self.send_reminder(&reminder).await {
                Ok(_) => {
                    if reminder.is_recurring {
                        if let Some(next_time) = reminder.calculate_next_reminder() {
                            if let Err(e) = store.update_reminder_time(&reminder.id, next_time) {
                                error!("Failed to reschedule: {}", e);
                            }
                        }
                    } else {
                        if let Err(e) = store.mark_reminder_sent(&reminder.id) {
                            warn!("Failed to mark as sent: {}", e);
                        }
                        if let Err(e) = store.delete_reminder(&reminder.id) {
                            warn!("Failed to delete: {}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to send reminder: {}", e);
                }
            }
        }

        Ok(())
    }

    async fn send_reminder(&self, reminder: &Reminder) -> anyhow::Result<()> {
        let chat_id = ChatId(reminder.chat_id);
        let icon = if reminder.is_recurring { "🔄" } else { "⏰" };
        let message = format!("{} Lembrete: {}", icon, reminder.message);

        match self.bot.send_message(chat_id, message).await {
            Ok(_) => {
                info!("Sent reminder '{}' to chat {}", reminder.message, reminder.chat_id);
                Ok(())
            }
            Err(e) => {
                error!("Failed to send: {}", e);
                Err(anyhow::anyhow!("Telegram error: {}", e))
            }
        }
    }
}
