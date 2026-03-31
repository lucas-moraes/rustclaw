use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const DEFAULT_MAX_TOKENS: usize = 100000;
const TOKENS_PER_CHAR: f32 = 0.25;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextWindow {
    pub max_tokens: usize,
    pub current_tokens: usize,
    pub messages: Vec<ContextMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMessage {
    pub role: String,
    pub content: String,
    pub token_count: usize,
    pub important: bool,
}

impl ContextWindow {
    pub fn new(max_tokens: usize) -> Self {
        Self {
            max_tokens,
            current_tokens: 0,
            messages: Vec::new(),
        }
    }

    pub fn add_message(&mut self, role: &str, content: &str, important: bool) {
        let token_count = Self::estimate_tokens(content);
        self.current_tokens += token_count;

        self.messages.push(ContextMessage {
            role: role.to_string(),
            content: content.to_string(),
            token_count,
            important,
        });

        while self.current_tokens > self.max_tokens {
            self.compact();
        }
    }

    pub fn estimate_tokens(text: &str) -> usize {
        (text.len() as f32 * TOKENS_PER_CHAR) as usize
    }

    pub fn compact(&mut self) {
        if self.messages.is_empty() {
            return;
        }

        let mut compressed: Vec<ContextMessage> = Vec::new();
        let mut saved_tokens = 0;

        for msg in &self.messages {
            if msg.important {
                compressed.push(msg.clone());
            } else if msg.token_count > 50 {
                let summary = Self::summarize(&msg.content, 100);
                let summary_tokens = Self::estimate_tokens(&summary);
                saved_tokens += msg.token_count - summary_tokens;

                compressed.push(ContextMessage {
                    role: msg.role.clone(),
                    content: summary,
                    token_count: summary_tokens,
                    important: false,
                });
            }
        }

        self.messages = compressed;

        self.current_tokens = self.messages.iter().map(|m| m.token_count).sum();

        if self.current_tokens > self.max_tokens {
            self.summarize_all();
        }
    }

    pub fn summarize_all(&mut self) {
        let important: Vec<_> = self
            .messages
            .iter()
            .filter(|m| m.important)
            .cloned()
            .collect();

        let others: Vec<_> = self
            .messages
            .iter()
            .filter(|m| !m.important)
            .cloned()
            .collect();

        let combined = others
            .iter()
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
            .join("\n");

        let summary = Self::summarize(&combined, self.max_tokens / 2);
        let summary_tokens = Self::estimate_tokens(&summary);

        self.messages = important;
        self.messages.push(ContextMessage {
            role: "system".to_string(),
            content: format!(
                "[Conversation summarized - {} tokens]\n{}",
                summary_tokens, summary
            ),
            token_count: summary_tokens,
            important: true,
        });

        self.current_tokens = self.messages.iter().map(|m| m.token_count).sum();
    }

    fn summarize(text: &str, max_len: usize) -> String {
        let sentences: Vec<&str> = text
            .split(|c| c == '.' || c == '!' || c == '?')
            .filter(|s| !s.trim().is_empty())
            .collect();

        if sentences.len() <= 5 {
            return text.to_string();
        }

        let mut result = String::new();
        let step = (sentences.len() as f32 / 5.0).ceil() as usize;

        for (i, sent) in sentences.iter().enumerate().step_by(step) {
            if !result.is_empty() {
                result.push_str(". ");
            }
            result.push_str(sent.trim());
        }

        if result.len() > max_len {
            result.truncate(max_len);
            if let Some(pos) = result.rfind('.') {
                result.truncate(pos + 1);
            }
        }

        result
    }

    pub fn to_messages(&self) -> Vec<serde_json::Value> {
        self.messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content
                })
            })
            .collect()
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.current_tokens = 0;
    }

    pub fn usage(&self) -> ContextUsage {
        ContextUsage {
            current_tokens: self.current_tokens,
            max_tokens: self.max_tokens,
            message_count: self.messages.len(),
            usage_percent: (self.current_tokens as f32 / self.max_tokens as f32 * 100.0) as u8,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextUsage {
    pub current_tokens: usize,
    pub max_tokens: usize,
    pub message_count: usize,
    pub usage_percent: u8,
}

pub struct ContextCompactor {
    strategies: Vec<Box<dyn CompactionStrategy>>,
}

impl ContextCompactor {
    pub fn new() -> Self {
        Self {
            strategies: vec![
                Box::new(RemoveRedundantStrategy),
                Box::new(SummarizeOldStrategy),
                Box::new(CompressWhitespaceStrategy),
            ],
        }
    }

    pub fn compact(&self, messages: &mut Vec<ContextMessage>) {
        for strategy in &self.strategies {
            strategy.compact(messages);
        }
    }

    pub fn add_strategy(&mut self, strategy: Box<dyn CompactionStrategy>) {
        self.strategies.push(strategy);
    }
}

pub trait CompactionStrategy: Send + Sync {
    fn compact(&self, messages: &mut Vec<ContextMessage>);
}

struct RemoveRedundantStrategy;

impl CompactionStrategy for RemoveRedundantStrategy {
    fn compact(&self, messages: &mut Vec<ContextMessage>) {
        let mut seen = std::collections::HashSet::new();
        messages.retain(|msg| {
            let key = format!(
                "{}:{}",
                msg.role,
                msg.content.chars().take(50).collect::<String>()
            );
            seen.insert(key)
        });
    }
}

struct SummarizeOldStrategy;

impl CompactionStrategy for SummarizeOldStrategy {
    fn compact(&self, messages: &mut Vec<ContextMessage>) {
        if messages.len() < 10 {
            return;
        }

        let keep = 5;
        let to_summarize: Vec<_> = messages[..messages.len() - keep]
            .iter()
            .map(|m| m.content.clone())
            .collect();

        if !to_summarize.is_empty() {
            let summary = to_summarize.join("\n");
            let summarized = ContextWindow::summarize(&summary, 500);

            for msg in messages.iter_mut().take(messages.len() - keep) {
                if !msg.important {
                    msg.content = summarized.clone();
                    msg.token_count = ContextWindow::estimate_tokens(&summarized);
                }
            }
        }
    }
}

struct CompressWhitespaceStrategy;

impl CompactionStrategy for CompressWhitespaceStrategy {
    fn compact(&self, messages: &mut Vec<ContextMessage>) {
        for msg in messages {
            let compressed = msg.content.split_whitespace().collect::<Vec<_>>().join(" ");

            let reduction = msg.content.len() - compressed.len();
            if reduction > 0 {
                msg.content = compressed;
                msg.token_count = ContextWindow::estimate_tokens(&msg.content);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_message() {
        let mut ctx = ContextWindow::new(1000);
        ctx.add_message("user", "Hello world", false);
        assert_eq!(ctx.messages.len(), 1);
    }

    #[test]
    fn test_compact() {
        let mut ctx = ContextWindow::new(100);
        ctx.add_message("user", "This is a very long message ".repeat(10), false);
        ctx.add_message("assistant", "Important response", true);

        ctx.compact();

        assert!(ctx.current_tokens <= ctx.max_tokens);
    }

    #[test]
    fn test_token_estimation() {
        let tokens = ContextWindow::estimate_tokens("Hello world");
        assert!(tokens > 0);
    }
}
