use crate::memory::reminder::Reminder;
use crate::memory::store::MemoryStore;
use chrono::Utc;
use std::path::PathBuf;
use std::time::Duration;
use teloxide::prelude::*;
use tracing::{error, info, warn};

pub struct ReminderExecutor {
    bot: Bot,
    memory_path: PathBuf,
}

impl ReminderExecutor {
    pub fn new(bot: Bot, memory_path: PathBuf) -> Self {
        Self { bot, memory_path }
    }

    pub async fn start(&self) {
        info!("Starting reminder executor...");

        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;

            match self.check_and_send_reminders().await {
                Ok(count) => {
                    if count > 0 {
                        info!("Processed {} reminder(s)", count);
                    }
                }
                Err(e) => {
                    error!("Error in reminder executor: {}", e);
                }
            }
        }
    }

    async fn check_and_send_reminders(&self) -> anyhow::Result<usize> {
        let store = MemoryStore::new(&self.memory_path)?;
        let now = Utc::now();

        let due_reminders = store.get_due_reminders(now)?;

        if due_reminders.is_empty() {
            return Ok(0);
        }

        let mut processed = 0;

        for reminder in due_reminders {
            match self.send_reminder(&reminder).await {
                Ok(_) => {
                    processed += 1;
                    
                    if reminder.is_recurring {
                        if let Some(next_time) = reminder.calculate_next_reminder() {
                            match store.update_reminder_time(&reminder.id, next_time) {
                                Ok(_) => {
                                    info!("Rescheduled reminder '{}' for {}", reminder.message, next_time);
                                }
                                Err(e) => {
                                    error!("Failed to reschedule: {}", e);
                                }
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

        if let Err(e) = store.cleanup_sent_reminders() {
            warn!("Failed to cleanup: {}", e);
        }

        Ok(processed)
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
