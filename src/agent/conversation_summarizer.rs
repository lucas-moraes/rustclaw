use serde_json::{json, Value};
use std::result::Result;

use crate::agent::token_counter::TokenCounter;
use crate::error::{AgentError, LLMError};

#[derive(Debug, Clone)]
pub struct SummarizationResult {
    pub summary: String,
    pub original_token_count: usize,
    pub summary_token_count: usize,
    pub messages_removed: usize,
}

pub struct ConversationSummarizer {
    token_counter: TokenCounter,
    max_messages_to_preserve: usize,
    summarization_prompt: String,
}

impl ConversationSummarizer {
    pub fn new(max_context_tokens: usize, max_messages_to_preserve: usize) -> Self {
        let summarization_prompt = r#"Você é um assistente de resumo de conversas. Sua tarefa é resumir uma conversa longa de forma clara e concisa, preservando as informações mais importantes.

INSTRUÇÕES:
1. Resuma os pontos principais da conversa
2. Mantenha decisões importantes tomadas
3. Preserve contexto relevante para continuar o trabalho
4. Seja conciso mas informativo
5. Responda APENAS com o resumo, sem introduções ou conclusões

CONVERSAÇÃO A RESUMIR:"#.to_string();

        Self {
            token_counter: TokenCounter::new(max_context_tokens),
            max_messages_to_preserve,
            summarization_prompt,
        }
    }

    pub fn should_summarize(&self, messages: &[Value], threshold: f64) -> bool {
        self.token_counter.should_summarize(messages, threshold)
    }

    #[allow(dead_code)]
    pub fn token_counter(&self) -> &TokenCounter {
        &self.token_counter
    }

    pub fn get_messages_to_summarize(&self, messages: &[Value]) -> Vec<Value> {
        if messages.len() <= self.max_messages_to_preserve {
            return vec![];
        }
        messages[1..messages.len().saturating_sub(1)].to_vec()
    }

    pub fn prepare_summary_messages(&self, messages: &[Value]) -> Vec<Value> {
        let to_summarize = self.get_messages_to_summarize(messages);
        if to_summarize.is_empty() {
            return vec![];
        }

        let conversation_text = to_summarize
            .iter()
            .filter_map(|m| {
                let role = m.get("role")?.as_str()?;
                let content = m.get("content")?.as_str().unwrap_or_default();
                Some(format!("{}: {}\n", role, content))
            })
            .collect::<Vec<_>>()
            .join("\n---\n");

        let system_msg = json!({
            "role": "system",
            "content": format!("{}\n\n{}", self.summarization_prompt, conversation_text)
        });

        let user_msg = json!({
            "role": "user", 
            "content": "Por favor, resuma esta conversa preservando as informações mais importantes."
        });

        vec![system_msg, user_msg]
    }

    pub async fn summarize_with_llm(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        messages: &[Value],
        model: &str,
        base_url: &str,
        provider: &str,
        max_tokens_response: usize,
    ) -> Result<SummarizationResult, crate::error::AgentError> {
        let summary_messages = self.prepare_summary_messages(messages);
        if summary_messages.is_empty() {
            return Err(LLMError::NoChoices.into());
        }

        let original_token_count = self.token_counter.count_messages_tokens(messages);

        let response = crate::agent::llm_client::LlmClient::call_llm_with_config(
            client,
            api_key,
            &summary_messages,
            model,
            base_url,
            provider,
            max_tokens_response,
        )
        .await?;

        let summary_token_count = self.token_counter.count_tokens(&response);
        let messages_removed = summary_messages.len().saturating_sub(1);

        Ok(SummarizationResult {
            summary: response,
            original_token_count,
            summary_token_count,
            messages_removed,
        })
    }

    pub fn compress_messages(
        &self,
        messages: &[Value],
        summary: &str,
    ) -> Vec<Value> {
        if messages.is_empty() {
            return vec![];
        }

        let mut compressed = Vec::new();

        if let Some(first) = messages.first() {
            compressed.push(first.clone());
        }

        let summary_msg = json!({
            "role": "system",
            "content": format!("[RESUMO DA CONVERSA ANTERIOR]\n{}\n[/RESUMO]", summary)
        });
        compressed.push(summary_msg);

        if let Some(last) = messages.last() {
            if last.get("role").and_then(|v| v.as_str()) != Some("system") {
                compressed.push(last.clone());
            }
        }

        compressed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_messages(count: usize) -> Vec<Value> {
        (0..count)
            .map(|i| {
                json!({
                    "role": if i % 2 == 0 { "user" } else { "assistant" },
                    "content": format!("Message number {} with some additional text to make it longer", i)
                })
            })
            .collect()
    }

    #[test]
    fn test_should_summarize() {
        let summarizer = ConversationSummarizer::new(200, 5);
        let messages = create_test_messages(10);
        assert!(summarizer.should_summarize(&messages, 0.5));
        
        let short_messages = create_test_messages(2);
        assert!(!summarizer.should_summarize(&short_messages, 0.9));
    }

    #[test]
    fn test_get_messages_to_summarize() {
        let summarizer = ConversationSummarizer::new(1000, 5);
        let messages = create_test_messages(10);
        
        let to_summarize = summarizer.get_messages_to_summarize(&messages);
        assert_eq!(to_summarize.len(), 8);
    }

    #[test]
    fn test_compress_messages() {
        let summarizer = ConversationSummarizer::new(1000, 5);
        let messages = create_test_messages(10);
        let summary = "This is a test summary";
        
        let compressed = summarizer.compress_messages(&messages, summary);
        assert!(compressed.len() < messages.len());
    }
}
