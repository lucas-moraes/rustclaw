use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    pub cron_expression: String,
    pub task_type: TaskType,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskType {
    Heartbeat,
    SystemCheck,
    Custom(String),
    Reminder(String),
}

impl TaskType {
    pub fn from_string(s: &str) -> anyhow::Result<Self> {
        if s.starts_with("custom:") {
            Ok(TaskType::Custom(
                s.strip_prefix("custom:").unwrap_or("").to_string(),
            ))
        } else if s.starts_with("reminder:") {
            Ok(TaskType::Reminder(
                s.strip_prefix("reminder:").unwrap_or("").to_string(),
            ))
        } else if s == "heartbeat" {
            Ok(TaskType::Heartbeat)
        } else if s == "system_check" {
            Ok(TaskType::SystemCheck)
        } else {
            Err(anyhow::anyhow!("Unknown task type: {}", s))
        }
    }
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskType::Heartbeat => write!(f, "heartbeat"),
            TaskType::SystemCheck => write!(f, "system_check"),
            TaskType::Custom(cmd) => write!(f, "custom:{}", cmd),
            TaskType::Reminder(msg) => write!(f, "reminder:{}", msg),
        }
    }
}

impl ScheduledTask {
    pub fn new(name: String, cron_expression: String, task_type: TaskType) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            cron_expression,
            task_type,
            is_active: true,
            created_at: Utc::now(),
            last_run: None,
            next_run: None,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        }
    }

    pub fn from_task_type_string(
        name: String,
        cron: String,
        type_str: String,
    ) -> anyhow::Result<Self> {
        let task_type = if type_str.starts_with("custom:") {
            TaskType::Custom(type_str.strip_prefix("custom:").unwrap_or("").to_string())
        } else if type_str.starts_with("reminder:") {
            TaskType::Reminder(type_str.strip_prefix("reminder:").unwrap_or("").to_string())
        } else if type_str == "heartbeat" {
            TaskType::Heartbeat
        } else if type_str == "system_check" {
            TaskType::SystemCheck
        } else {
            return Err(anyhow::anyhow!("Tipo de tarefa inválido: {}", type_str));
        };

        Ok(Self::new(name, cron, task_type))
    }

    pub fn get_type_string(&self) -> String {
        self.task_type.to_string()
    }
}
