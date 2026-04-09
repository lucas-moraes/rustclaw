use crate::app_store::Store;
use crate::config::Config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type AppState = AppStateV1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppStateV1 {
    pub settings: AppSettings,
    pub verbose: bool,
    pub tasks: HashMap<String, TaskState>,
    pub mcp: McpState,
    pub plugins: PluginState,
    pub notifications: NotificationState,
    pub session: SessionState,
    pub development: DevelopmentState,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    pub provider: String,
    pub model: String,
    pub max_tokens: usize,
    pub max_iterations: usize,
    pub auto_approve: bool,
    pub theme: String,
    pub editor: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskState {
    pub id: String,
    pub status: TaskStatus,
    pub input: String,
    pub output: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum TaskStatus {
    #[default]
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct McpState {
    pub clients: Vec<McpClientState>,
    pub tools: Vec<String>,
    pub commands: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct McpClientState {
    pub name: String,
    pub status: ConnectionStatus,
    pub server_info: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Error,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginState {
    pub enabled: Vec<PluginInfo>,
    pub disabled: Vec<PluginInfo>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub path: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NotificationState {
    pub current: Option<Notification>,
    pub queue: Vec<Notification>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub message: String,
    pub notification_type: NotificationType,
    pub timestamp: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum NotificationType {
    #[default]
    Info,
    Warning,
    Error,
    Success,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    pub started_at: i64,
    pub messages_count: i64,
    pub tokens_used: i64,
    pub current_skill: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DevelopmentState {
    pub active_checkpoint: Option<String>,
    pub completed_steps: Vec<usize>,
    pub auto_loop_enabled: bool,
}

#[allow(dead_code)]
pub fn create_app_store() -> Store<AppState> {
    Store::new(AppState::default())
}

#[allow(dead_code)]
pub fn get_default_settings() -> AppSettings {
    AppSettings {
        provider: "opencode-go".to_string(),
        model: "minimax-m2.7".to_string(),
        max_tokens: 4000,
        max_iterations: 20,
        auto_approve: false,
        theme: "default".to_string(),
        editor: "vim".to_string(),
    }
}

impl From<&Config> for AppSettings {
    fn from(config: &Config) -> Self {
        Self {
            provider: config.provider.clone(),
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            max_iterations: config.max_iterations,
            auto_approve: false,
            theme: "default".to_string(),
            editor: "vim".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_default() {
        let state = AppState::default();
        assert_eq!(state.verbose, false);
        assert!(state.tasks.is_empty());
    }

    #[test]
    fn test_task_status() {
        let task = TaskState {
            id: "1".to_string(),
            status: TaskStatus::Pending,
            input: "test".to_string(),
            output: None,
            created_at: 0,
            updated_at: 0,
        };
        assert!(matches!(task.status, TaskStatus::Pending));
    }
}
