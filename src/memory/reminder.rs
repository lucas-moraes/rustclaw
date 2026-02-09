use chrono::{DateTime, Utc};
use cron::Schedule;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

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
            id: String::new(), // ID será atribuído pelo store
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
            id: String::new(), // ID será atribuído pelo store
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

        // Parse cron expression and calculate next run
        if let Some(cron_expr) = &self.cron_expression {
            if let Ok(schedule) = cron_expr.parse::<Schedule>() {
                return schedule.upcoming(Utc).next();
            }
        }

        None
    }
}

#[derive(Debug, Clone)]
pub enum ReminderType {
    Single,
    Recurring(String), // cron expression
}

#[derive(Debug, Clone)]
pub struct ParsedReminder {
    pub message: String,
    pub reminder_type: ReminderType,
    pub datetime: Option<DateTime<Utc>>,
}
