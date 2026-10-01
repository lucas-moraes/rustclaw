//! `todo_read` / `todo_write` tools: session-scoped task list.

use super::{Tool, ToolResult};
use crate::harness::session::{TodoItem, TodoStatus};
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};

pub struct TodoReadTool;

#[async_trait::async_trait]
impl Tool for TodoReadTool {
    fn name(&self) -> &str {
        "todo_read"
    }
    fn description(&self) -> &str {
        "Lists the current session's task list with status. \
Use after todo_write to track progress."
    }
    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    async fn execute(&self, _args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let todos = ctx.todos.read().await.clone();
        if todos.is_empty() {
            return Ok(ToolResult::simple("todo_read", "(no todos)"));
        }
        let mut body = String::new();
        for (i, t) in todos.iter().enumerate() {
            body.push_str(&format!("{}. [{}] {}\n", i + 1, t.status, t.content));
        }
        Ok(ToolResult::simple("todo_read", body.trim_end().to_string()))
    }
}

pub struct TodoWriteTool;

#[async_trait::async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "todo_write"
    }
    fn description(&self) -> &str {
        "Replaces the session task list. Each item: {content, status} where status is \
pending|in_progress|completed|cancelled. Keep at most one in_progress."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {"type": "string"},
                            "status": {"type": "string", "enum": ["pending","in_progress","completed","cancelled"]}
                        },
                        "required": ["content", "status"]
                    }
                }
            },
            "required": ["todos"]
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let items = args["todos"]
            .as_array()
            .ok_or_else(|| "missing required argument: todos".to_string())?;
        let mut todos = Vec::new();
        for item in items {
            let content = item["content"]
                .as_str()
                .ok_or_else(|| "each todo needs a content string".to_string())?;
            let status = match item["status"].as_str().unwrap_or("pending") {
                "in_progress" => TodoStatus::InProgress,
                "completed" => TodoStatus::Completed,
                "cancelled" => TodoStatus::Cancelled,
                _ => TodoStatus::Pending,
            };
            todos.push(TodoItem {
                content: content.to_string(),
                status,
            });
        }
        *ctx.todos.write().await = todos.clone();
        Ok(ToolResult::simple(
            "todo_write",
            format!("Set {} todo item(s).", todos.len()),
        ))
    }
}
