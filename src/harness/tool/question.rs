//! `question` tool: asks the end user a question and returns their answer.

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};

pub struct QuestionTool;

#[async_trait::async_trait]
impl Tool for QuestionTool {
    fn name(&self) -> &str {
        "question"
    }
    fn description(&self) -> &str {
        "Asks the user a clarifying question and returns their answer. \
Use when requirements are ambiguous and cannot be resolved by reading the code."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {"type": "string", "description": "The question to ask"},
                "options": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional predefined choices"
                }
            },
            "required": ["question"]
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let question = args["question"]
            .as_str()
            .ok_or_else(|| "missing required argument: question".to_string())?;
        let options: Vec<String> = args["options"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let answer = ctx
            .user_asker
            .ask(question.to_string(), options)
            .await
            .ok_or_else(|| "user did not answer".to_string())?;

        Ok(ToolResult::simple(
            "question",
            format!("User answered: {}", preview(&answer, 500)),
        ))
    }
}
