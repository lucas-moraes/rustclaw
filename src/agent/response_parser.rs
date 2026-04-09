use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

pub struct ResponseParser;

impl ResponseParser {
    pub fn parse_response(response: &str) -> anyhow::Result<ParsedResponse> {
        let sanitized = Self::sanitize_model_response(response);

        let final_answer_re =
            RE_FINAL_ANSWER.get_or_init(|| Regex::new(r"(?si)Final Answer:\s*(.+)$").unwrap());
        if let Some(caps) = final_answer_re.captures(&sanitized) {
            let answer = caps
                .get(1)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_else(|| sanitized.to_string());
            return Ok(ParsedResponse::FinalAnswer(answer));
        }

        let thought_re =
            RE_THOUGHT.get_or_init(|| Regex::new(r"(?i)Thought:\s*(.+?)(?:\n|$)").unwrap());
        let retrieved_memory_re = RE_RETRIEVED_MEMORY.get_or_init(|| {
            Regex::new(
                r"(?i)Retrieved Memory:\s*(.+?)(?:\n(?:Revise Memory:|Reasoning:|Verification:|Action:|Final Answer:)|$)",
            )
            .unwrap()
        });
        let revise_memory_re = RE_REVISE_MEMORY.get_or_init(|| {
            Regex::new(
                r"(?i)Revise Memory:\s*(.+?)(?:\n(?:Reasoning:|Verification:|Action:|Final Answer:)|$)",
            )
            .unwrap()
        });
        let reasoning_re = RE_REASONING.get_or_init(|| {
            Regex::new(r"(?i)Reasoning:\s*(.+?)(?:\n(?:Verification:|Action:|Final Answer:)|$)")
                .unwrap()
        });
        let verification_re = RE_VERIFICATION.get_or_init(|| {
            Regex::new(r"(?i)Verification:\s*(.+?)(?:\n(?:Action:|Final Answer:)|$)").unwrap()
        });
        let action_re =
            RE_ACTION.get_or_init(|| Regex::new(r"(?i)Action:\s*(.+?)(?:\n|$)").unwrap());

        let thought = thought_re
            .captures(&sanitized)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();

        let retrieved_memory = retrieved_memory_re
            .captures(&sanitized)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string());

        let revise_memory = revise_memory_re
            .captures(&sanitized)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string());

        let reasoning = reasoning_re
            .captures(&sanitized)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string());

        let verification = verification_re
            .captures(&sanitized)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string());

        let action = action_re
            .captures(&sanitized)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string());

        let action_input = if let Some(pos) = sanitized.to_lowercase().find("action input:") {
            let after = &sanitized[pos + "action input:".len()..];
            if let Some(json_block) = Self::extract_json_block(after) {
                json_block
            } else {
                after.trim().to_string()
            }
        } else {
            "{}".to_string()
        };

        if let Some(action) = action {
            return Ok(ParsedResponse::Action {
                thought,
                retrieved_memory,
                revise_memory,
                reasoning,
                verification,
                action,
                action_input,
            });
        }

        Ok(ParsedResponse::FinalAnswer(sanitized.trim().to_string()))
    }

    pub fn sanitize_model_response(response: &str) -> String {
        let reminder_re = RE_SYSTEM_REMINDER
            .get_or_init(|| Regex::new(r"(?is)<system-reminder>.*?</system-reminder>").unwrap());
        reminder_re.replace_all(response, "").to_string()
    }

    pub fn parse_action_input_json(action_input: &str) -> anyhow::Result<Value> {
        let trimmed = action_input.trim();

        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            return Ok(value);
        }

        let reminder_re = RE_SYSTEM_REMINDER
            .get_or_init(|| Regex::new(r"(?is)<system-reminder>.*?</system-reminder>").unwrap());
        let cleaned = reminder_re.replace_all(trimmed, "").to_string();

        let cleaned = cleaned
            .lines()
            .filter(|line| !line.trim_start().starts_with("<system-reminder>"))
            .collect::<Vec<_>>()
            .join("\n");

        let stripped = if cleaned.starts_with("```") {
            let mut lines: Vec<&str> = cleaned.lines().collect();
            if !lines.is_empty() {
                lines.remove(0);
            }
            if let Some(last) = lines.last() {
                if last.trim().starts_with("```") {
                    lines.pop();
                }
            }
            lines.join("\n")
        } else {
            trimmed.to_string()
        };

        if let Ok(value) = serde_json::from_str::<Value>(stripped.trim()) {
            return Ok(value);
        }

        if let Some(value) = Self::parse_heredoc_input(&stripped) {
            return Ok(value);
        }

        if let Some(json_block) = Self::extract_json_block(&stripped) {
            if let Ok(value) = serde_json::from_str::<Value>(&json_block) {
                return Ok(value);
            }
        }

        if let Some(value) = Self::recover_action_input(&stripped) {
            return Ok(value);
        }

        Err(anyhow::anyhow!("Action Input inválido: {}", action_input))
    }

    pub fn parse_heredoc_input(input: &str) -> Option<Value> {
        if input.contains("cat >") || input.contains("tee >") {
            let heredoc_re = RE_HEREDOC.get_or_init(|| {
                Regex::new(r#""command"\s*:\s*"cat\s+>\s+([^"]+)\s+<<\s*'?\w+'?\s*\n(.*?)\n\w+""#)
                    .unwrap()
            });

            if let Some(caps) = heredoc_re.captures(input) {
                let file_path = caps.get(1)?.as_str();
                let content = caps.get(2)?.as_str();

                return Some(serde_json::json!({
                    "path": file_path,
                    "content": content
                }));
            }

            let alt_re = RE_HEREDOC_ALT.get_or_init(|| {
                Regex::new(r#""command"\s*:\s*"([^"]*cat[^"]*\bEOF\b[^"]*)""#).unwrap()
            });

            if let Some(caps) = alt_re.captures(input) {
                let command = caps.get(1)?.as_str();

                let file_re =
                    RE_CAT_REDIRECT.get_or_init(|| Regex::new(r"cat\s+>\s+(\S+)").unwrap());
                if let Some(file_caps) = file_re.captures(command) {
                    let file_path = file_caps.get(1)?.as_str();

                    let eof_re = RE_EOF_MARKER
                        .get_or_init(|| Regex::new(r"<<\s*'?(\w+)'?\s*\n([\s\S]*?)\n\w+").unwrap());
                    if let Some(eof_caps) = eof_re.captures(input) {
                        let marker = eof_caps.get(1)?.as_str();
                        let content = eof_caps.get(2)?.as_str();
                        if content
                            .lines()
                            .last()
                            .map(|l| l.trim() == marker)
                            .unwrap_or(false)
                        {
                            return Some(serde_json::json!({
                                "path": file_path,
                                "content": content.lines().take(content.lines().count() - 1).collect::<Vec<_>>().join("\n")
                            }));
                        }
                    }
                }
            }
        }

        None
    }

    pub fn recover_action_input(input: &str) -> Option<Value> {
        let path_re =
            RE_JSON_PATH.get_or_init(|| Regex::new(r#"(?s)"path"\s*:\s*"([^"]*)""#).unwrap());
        let command_re =
            RE_JSON_COMMAND.get_or_init(|| Regex::new(r#"(?s)"command"\s*:\s*"([^"]*)""#).unwrap());

        if let Some(caps) = path_re.captures(input) {
            let path = caps
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            if let Some(content) = Self::extract_json_string_field(input, "content") {
                return Some(serde_json::json!({
                    "path": path,
                    "content": content,
                }));
            }
        }

        if let Some(caps) = command_re.captures(input) {
            let command = caps
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            if !command.is_empty() {
                return Some(serde_json::json!({
                    "command": command,
                }));
            }
        }

        None
    }

    pub fn extract_json_string_field(input: &str, field: &str) -> Option<String> {
        let key = format!("\"{}\"", field);
        let idx = input.find(&key)?;
        let after_key = &input[idx + key.len()..];
        let colon_idx = after_key.find(':')?;
        let mut rest = after_key[colon_idx + 1..].trim_start();

        if !rest.starts_with('"') {
            return None;
        }

        rest = &rest[1..];
        let mut end = rest.len();
        if let Some(pos) = rest.rfind("\"}") {
            end = pos;
        } else if let Some(pos) = rest.rfind("\"") {
            end = pos;
        }

        let raw = rest[..end].to_string();
        let unescaped = raw
            .replace("\\n", "\n")
            .replace("\\t", "\t")
            .replace("\\r", "\r")
            .replace("\\\"", "\"")
            .replace("\\\\", "\\");

        Some(unescaped)
    }

    pub fn extract_json_block(input: &str) -> Option<String> {
        let mut start_idx = None;
        let mut stack: Vec<char> = Vec::new();
        let mut in_string = false;
        let mut escape = false;

        for (i, c) in input.char_indices() {
            if start_idx.is_none() {
                if c == '{' || c == '[' {
                    start_idx = Some(i);
                    stack.push(c);
                }
                continue;
            }

            if in_string {
                if escape {
                    escape = false;
                    continue;
                }
                if c == '\\' {
                    escape = true;
                    continue;
                }
                if c == '"' {
                    in_string = false;
                }
                continue;
            }

            match c {
                '"' => in_string = true,
                '{' | '[' => stack.push(c),
                '}' => {
                    if let Some(last) = stack.pop() {
                        if last != '{' {
                            return None;
                        }
                    }
                }
                ']' => {
                    if let Some(last) = stack.pop() {
                        if last != '[' {
                            return None;
                        }
                    }
                }
                _ => {}
            }

            if stack.is_empty() {
                if let Some(start) = start_idx {
                    return Some(input[start..=i].to_string());
                }
            }
        }

        None
    }
}

pub enum ParsedResponse {
    FinalAnswer(String),
    Action {
        thought: String,
        retrieved_memory: Option<String>,
        revise_memory: Option<String>,
        reasoning: Option<String>,
        verification: Option<String>,
        action: String,
        action_input: String,
    },
}

static RE_SYSTEM_REMINDER: OnceLock<Regex> = OnceLock::new();
static RE_FINAL_ANSWER: OnceLock<Regex> = OnceLock::new();
static RE_THOUGHT: OnceLock<Regex> = OnceLock::new();
static RE_RETRIEVED_MEMORY: OnceLock<Regex> = OnceLock::new();
static RE_REVISE_MEMORY: OnceLock<Regex> = OnceLock::new();
static RE_REASONING: OnceLock<Regex> = OnceLock::new();
static RE_VERIFICATION: OnceLock<Regex> = OnceLock::new();
static RE_ACTION: OnceLock<Regex> = OnceLock::new();
static RE_REVIEW: OnceLock<Regex> = OnceLock::new();
static RE_SUGGESTION: OnceLock<Regex> = OnceLock::new();
static RE_PLAN_STEP: OnceLock<Regex> = OnceLock::new();
static RE_HEREDOC: OnceLock<Regex> = OnceLock::new();
static RE_HEREDOC_ALT: OnceLock<Regex> = OnceLock::new();
static RE_CAT_REDIRECT: OnceLock<Regex> = OnceLock::new();
static RE_EOF_MARKER: OnceLock<Regex> = OnceLock::new();
static RE_JSON_PATH: OnceLock<Regex> = OnceLock::new();
static RE_JSON_COMMAND: OnceLock<Regex> = OnceLock::new();
