use serde_json::Value;

#[allow(dead_code)]
pub const DEFAULT_MAX_CONTEXT_TOKENS: usize = 128_000;
pub const TOKENS_PER_CHAR_RATIO: f64 = 0.25;

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct TokenCounter {
    pub max_context_tokens: usize,
}

#[allow(dead_code)]
impl TokenCounter {
    pub fn new(max_context_tokens: usize) -> Self {
        Self { max_context_tokens }
    }

    pub fn count_tokens<S: AsRef<str>>(&self, text: S) -> usize {
        let chars = text.as_ref().chars().count();
        Self::chars_to_tokens(chars)
    }

    pub fn count_messages_tokens(&self, messages: &[Value]) -> usize {
        messages
            .iter()
            .map(|msg| {
                let content = msg
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
                let role_prefix = format!("{}: ", role);
                Self::chars_to_tokens(role_prefix.chars().count() + content.chars().count())
            })
            .sum()
    }

    pub fn chars_to_tokens(chars: usize) -> usize {
        ((chars as f64) * TOKENS_PER_CHAR_RATIO).ceil() as usize
    }

    pub fn tokens_to_chars(tokens: usize) -> usize {
        (tokens as f64 / TOKENS_PER_CHAR_RATIO).ceil() as usize
    }

    pub fn context_usage_ratio(&self, messages: &[Value]) -> f64 {
        if self.max_context_tokens == 0 {
            return 0.0;
        }
        let used = self.count_messages_tokens(messages);
        used as f64 / self.max_context_tokens as f64
    }

    pub fn should_summarize(&self, messages: &[Value], threshold: f64) -> bool {
        self.context_usage_ratio(messages) >= threshold
    }

    pub fn reserve_space_for_response(&self, messages: &[Value], response_tokens: usize) -> usize {
        let available = self.max_context_tokens.saturating_sub(response_tokens);
        let used = self.count_messages_tokens(messages);
        available.saturating_sub(used)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_count_tokens() {
        let counter = TokenCounter::new(DEFAULT_MAX_CONTEXT_TOKENS);
        assert_eq!(counter.count_tokens("hello"), 2);
        assert_eq!(counter.count_tokens("hello world"), 3);
        assert_eq!(counter.count_tokens(""), 0);
    }

    #[test]
    fn test_chars_to_tokens() {
        assert_eq!(TokenCounter::chars_to_tokens(4), 1);
        assert_eq!(TokenCounter::chars_to_tokens(8), 2);
        assert_eq!(TokenCounter::chars_to_tokens(1), 1);
    }

    #[test]
    fn test_tokens_to_chars() {
        assert_eq!(TokenCounter::tokens_to_chars(1), 4);
        assert_eq!(TokenCounter::tokens_to_chars(2), 8);
    }

    #[test]
    fn test_context_usage_ratio() {
        let counter = TokenCounter::new(1000);
        let messages: Vec<Value> = vec![
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "hi"}),
        ];
        let ratio = counter.context_usage_ratio(&messages);
        assert!(ratio > 0.0);
        assert!(ratio < 1.0);
    }

    #[test]
    fn test_should_summarize() {
        let counter = TokenCounter::new(100);
        let short_messages: Vec<Value> = vec![json!({"role": "user", "content": "hi"})];
        assert!(!counter.should_summarize(&short_messages, 0.8));

        let long_text = "a".repeat(500);
        let long_messages: Vec<Value> = vec![json!({"role": "user", "content": long_text})];
        assert!(counter.should_summarize(&long_messages, 0.1));
    }
}
