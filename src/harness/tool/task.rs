//! `task` tool: spawns a subagent (child session) to complete a task.

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};

pub struct TaskTool;

#[async_trait::async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }
    fn description(&self) -> &str {
        "Spawns a subagent (default: explore, or build/plan/general) to complete a \
delegated task. Returns the subagent's summary. Use for parallelizable or \
self-contained research/implementation work."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {"type": "string", "description": "Short description of the task"},
                "prompt": {"type": "string", "description": "Full instructions for the subagent"},
                "agent": {"type": "string", "description": "explore|build|plan|general (default explore)"}
            },
            "required": ["description", "prompt"]
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let prompt = args["prompt"]
            .as_str()
            .ok_or_else(|| "missing required argument: prompt".to_string())?;
        let agent = args["agent"].as_str().unwrap_or("explore").to_string();

        let runner = ctx
            .task_runner
            .clone()
            .ok_or_else(|| "subagent runner not available".to_string())?;

        let result = runner.run_task(agent.clone(), prompt.to_string()).await?;

        Ok(ToolResult::simple(
            format!("task ({})", preview(&agent, 20)),
            format!("Subagent `{}` result:\n{}", agent, preview(&result, 4000)),
        ))
    }
}
