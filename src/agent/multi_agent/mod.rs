use crate::error::AgentError;
use crate::memory::store::MemoryStore;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubAgentConfig {
    pub name: String,
    pub role: String,
    pub tools: Vec<String>,
    pub max_iterations: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub description: String,
    pub assigned_agent: Option<String>,
    pub status: TaskStatus,
    pub result: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentMessage {
    pub from: String,
    pub to: String,
    pub content: String,
    pub timestamp: std::time::SystemTime,
}

impl AgentMessage {
    pub fn new(from: String, to: String, content: String) -> Self {
        Self {
            from,
            to,
            content,
            timestamp: std::time::SystemTime::now(),
        }
    }
}

#[async_trait]
pub trait SubAgent: Send + Sync {
    fn name(&self) -> &str;
    fn role(&self) -> &str;
    async fn execute(&self, task: &Task, context: &AgentContext) -> Result<String, AgentError>;
    fn config(&self) -> SubAgentConfig;
}

#[derive(Clone)]
pub struct AgentContext {
    pub memory: Arc<RwLock<MemoryStore>>,
    pub shared_state: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    pub messages: Arc<RwLock<Vec<AgentMessage>>>,
}

impl AgentContext {
    pub fn new(memory: Arc<RwLock<MemoryStore>>) -> Self {
        Self {
            memory,
            shared_state: Arc::new(RwLock::new(HashMap::new())),
            messages: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn send_message(&self, to: String, content: String) -> Result<(), AgentError> {
        let msg = AgentMessage::new("orchestrator".to_string(), to, content);
        self.messages.write().await.push(msg);
        Ok(())
    }

    pub async fn broadcast(&self, content: String) -> Result<(), AgentError> {
        let msg = AgentMessage::new("orchestrator".to_string(), "all".to_string(), content);
        self.messages.write().await.push(msg);
        Ok(())
    }

    pub async fn get_messages_for(&self, agent: &str) -> Vec<AgentMessage> {
        self.messages
            .read()
            .await
            .iter()
            .filter(|m| m.to == agent || m.to == "all")
            .cloned()
            .collect()
    }

    pub async fn set_shared(&self, key: String, value: serde_json::Value) {
        self.shared_state.write().await.insert(key, value);
    }

    pub async fn get_shared(&self, key: &str) -> Option<serde_json::Value> {
        self.shared_state.read().await.get(key).cloned()
    }
}

pub struct AgentPool {
    agents: HashMap<String, Box<dyn SubAgent>>,
    tasks: HashMap<String, Task>,
    completed_tasks: Vec<String>,
}

impl AgentPool {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            tasks: HashMap::new(),
            completed_tasks: Vec::new(),
        }
    }

    pub fn register(&mut self, agent: Box<dyn SubAgent>) {
        let name = agent.name().to_string();
        self.agents.insert(name, agent);
    }

    pub fn get_agent(&self, name: &str) -> Option<&dyn SubAgent> {
        self.agents.get(name).map(|a| a.as_ref())
    }

    pub fn create_task(&mut self, description: String) -> String {
        let id = format!("task_{}", self.tasks.len() + 1);
        let task = Task {
            id: id.clone(),
            description,
            assigned_agent: None,
            status: TaskStatus::Pending,
            result: None,
        };
        self.tasks.insert(id.clone(), task);
        id
    }

    pub fn assign_task(&mut self, task_id: &str, agent_name: &str) -> Result<(), AgentError> {
        if !self.agents.contains_key(agent_name) {
            return Err(AgentError::Internal(crate::error::InternalError::Unexpected(format!(
                "Agent {} not found",
                agent_name
            ))));
        }

        if let Some(task) = self.tasks.get_mut(task_id) {
            task.assigned_agent = Some(agent_name.to_string());
            task.status = TaskStatus::InProgress;
            Ok(())
        } else {
            Err(AgentError::Internal(crate::error::InternalError::Unexpected(format!(
                "Task {} not found",
                task_id
            ))))
        }
    }

    pub fn get_task(&self, task_id: &str) -> Option<&Task> {
        self.tasks.get(task_id)
    }

    pub fn get_task_mut(&mut self, task_id: &str) -> Option<&mut Task> {
        self.tasks.get_mut(task_id)
    }

    pub fn complete_task(&mut self, task_id: &str, result: String) {
        if let Some(task) = self.tasks.get_mut(task_id) {
            task.status = TaskStatus::Completed;
            task.result = Some(result);
            self.completed_tasks.push(task_id.to_string());
        }
    }

    pub fn fail_task(&mut self, task_id: &str, error: String) {
        if let Some(task) = self.tasks.get_mut(task_id) {
            task.status = TaskStatus::Failed;
            task.result = Some(error);
        }
    }

    pub fn get_pending_tasks(&self) -> Vec<&Task> {
        self.tasks
            .values()
            .filter(|t| t.status == TaskStatus::Pending)
            .collect()
    }

    pub fn get_in_progress_tasks(&self) -> Vec<&Task> {
        self.tasks
            .values()
            .filter(|t| t.status == TaskStatus::InProgress)
            .collect()
    }

    pub fn list_agents(&self) -> Vec<SubAgentConfig> {
        self.agents.values().map(|a| a.config()).collect()
    }

    pub fn aggregate_results(&self) -> HashMap<String, String> {
        let mut results = HashMap::new();
        for task_id in &self.completed_tasks {
            if let Some(task) = self.tasks.get(task_id) {
                if let Some(result) = &task.result {
                    results.insert(task_id.clone(), result.clone());
                }
            }
        }
        results
    }
}

impl Default for AgentPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSubAgent {
        name: String,
        role: String,
    }

    impl TestSubAgent {
        fn new(name: &str, role: &str) -> Self {
            Self {
                name: name.to_string(),
                role: role.to_string(),
            }
        }
    }

    #[async_trait]
    impl SubAgent for TestSubAgent {
        fn name(&self) -> &str {
            &self.name
        }

        fn role(&self) -> &str {
            &self.role
        }

        async fn execute(&self, task: &Task, _context: &AgentContext) -> Result<String, AgentError> {
            Ok(format!("Agent {} completed: {}", self.name, task.description))
        }

        fn config(&self) -> SubAgentConfig {
            SubAgentConfig {
                name: self.name.clone(),
                role: self.role.clone(),
                tools: vec![],
                max_iterations: 5,
            }
        }
    }

    #[test]
    fn test_agent_pool_register() {
        let mut pool = AgentPool::new();
        pool.register(Box::new(TestSubAgent::new("test_agent", "tester")));
        
        assert!(pool.get_agent("test_agent").is_some());
        assert_eq!(pool.get_agent("test_agent").unwrap().role(), "tester");
    }

    #[test]
    fn test_create_and_assign_task() {
        let mut pool = AgentPool::new();
        pool.register(Box::new(TestSubAgent::new("test_agent", "tester")));
        
        let task_id = pool.create_task("Test task".to_string());
        assert!(pool.assign_task(&task_id, "test_agent").is_ok());
        
        let task = pool.get_task(&task_id).unwrap();
        assert_eq!(task.status, TaskStatus::InProgress);
        assert_eq!(task.assigned_agent.as_deref(), Some("test_agent"));
    }

    #[test]
    fn test_complete_task() {
        let mut pool = AgentPool::new();
        let task_id = pool.create_task("Test task".to_string());
        
        pool.complete_task(&task_id, "Result".to_string());
        
        let task = pool.get_task(&task_id).unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.result.as_deref(), Some("Result"));
    }

    #[test]
    fn test_aggregate_results() {
        let mut pool = AgentPool::new();
        
        pool.create_task("Task 1".to_string());
        pool.create_task("Task 2".to_string());
        
        pool.complete_task("task_1", "Result 1".to_string());
        pool.complete_task("task_2", "Result 2".to_string());
        
        let results = pool.aggregate_results();
        assert_eq!(results.len(), 2);
    }
}