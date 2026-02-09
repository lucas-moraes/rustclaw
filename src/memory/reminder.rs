use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reminder {
    pub id: String,
    pub message: String,
    pub remind_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub is_recurring: bool,
    pub cron_expression: Option<String>,
    pub chat_id: i64,
    pub is_sent: bool,
}

impl Reminder {
    pub fn new(message: String, remind_at: DateTime<Utc>, chat_id: i64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            message,
            remind_at,
            created_at: Utc::now(),
            is_recurring: false,
            cron_expression: None,
            chat_id,
            is_sent: false,
        }
    }

    pub fn new_recurring(
        message: String,
        cron_expression: String,
        next_remind_at: DateTime<Utc>,
        chat_id: i64,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            message,
            remind_at: next_remind_at,
            created_at: Utc::now(),
            is_recurring: true,
            cron_expression: Some(cron_expression),
            chat_id,
            is_sent: false,
        }
    }

    pub fn calculate_next_reminder(&self) -> Option<DateTime<Utc>> {
        if !self.is_recurring {
            return None;
        }

        if let Some(cron_expr) = &self.cron_expression {
            use std::str::FromStr;
            match cron::Schedule::from_str(cron_expr) {
                Ok(schedule) => {
                    return schedule.upcoming(Utc).next();
                }
                Err(_) => return None,
            }
        }

        None
    }
}

#[derive(Debug, Clone)]
pub enum ReminderType {
    Single,
    Recurring(String),
}

#[derive(Debug, Clone)]
pub struct ParsedReminder {
    pub message: String,
    pub reminder_type: ReminderType,
    pub datetime: Option<DateTime<Utc>>,
}
