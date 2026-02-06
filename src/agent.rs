use crate::config::Config;
use crate::tools::ToolRegistry;
use regex::Regex;
use reqwest::Client;
use serde_json::{json, Value};
use tracing::{debug, info};

pub struct Agent {
    client: Client,
    config: Config,
    tools: ToolRegistry,
    conversation_history: Vec<Value>,
}

impl Agent {
    pub fn new(config: Config, tools: ToolRegistry) -> Self {
        Self {
            client: Client::new(),
            config,
            tools,
            conversation_history: Vec::new(),
        }
    }

    pub async fn prompt(&mut self, user_input: &str) -> anyhow::Result<String> {
        info!("User input: {}", user_input);

        // Build system prompt with ReAct instructions
        let system_prompt = self.build_system_prompt();

        // Add user message to history
        self.conversation_history.push(json!({
            "role": "user",
            "content": user_input
        }));

        // Build messages for API call
        let messages = self.build_messages(&system_prompt);

        // ReAct loop
        for iteration in 0..self.config.max_iterations {
            info!("ReAct iteration {}", iteration + 1);

            // Call LLM
            let response = self.call_llm(&messages).await?;
            debug!("LLM response:\n{}", response);

            // Parse response
            let parsed = self.parse_response(&response)?;

            match parsed {
                ParsedResponse::FinalAnswer(answer) => {
                    info!("Final answer received");
                    self.conversation_history.push(json!({
                        "role": "assistant",
                        "content": answer.clone()
                    }));
                    return Ok(answer);
                }
                ParsedResponse::Action { thought, action, action_input } => {
                    info!("Action detected: {} with input: {}", action, action_input);
                    
                    // Execute tool
                    let observation = self.execute_tool(&action, &action_input).await?;
                    info!("Tool observation: {}", observation);

                    // Add to messages for next iteration
                    let tool_result = format!(
                        "Thought: {}\nAction: {}\nAction Input: {}\nObservation: {}",
                        thought, action, action_input, observation
                    );
                    
                    // Rebuild messages with the result
                    let mut new_messages = self.build_messages(&system_prompt);
                    new_messages.push(json!({
                        "role": "assistant",
                        "content": tool_result
                    }));
                    
                    // Update for next iteration
                    // We need to include this in the next API call
                }
            }
        }

        // If we reached max iterations, force a final answer
        info!("Max iterations reached, forcing final answer");
        let final_prompt = format!(
            "{}",
            self.build_messages(&system_prompt)
                .iter()
                .map(|m| m["content"].as_str().unwrap_or(""))
                .collect::<Vec<_>>()
                .join("\n")
        );
        
        let final_messages = vec![
            json!({
                "role": "system",
                "content": system_prompt.clone()
            }),
            json!({
                "role": "user",
                "content": format!("{}\n\nCom base nas observações anteriores, forneça sua resposta final usando o formato:\nFinal Answer: [sua resposta]", final_prompt)
            })
        ];
        let final_response = self.call_llm(&final_messages).await?;
        
        if let ParsedResponse::FinalAnswer(answer) = self.parse_response(&final_response)? {
            return Ok(answer);
        }
        
        Ok(final_response)
    }

    fn build_system_prompt(&self) -> String {
        let tool_list = if self.tools.is_empty() {
            "Nenhuma ferramenta disponível".to_string()
        } else {
            self.tools.list()
        };

        format!(
            r#"Você é RustClaw, um assistente AI útil.

Você tem acesso às seguintes ferramentas:
{}

Para usar uma ferramenta, responda EXATAMENTE neste formato:
Thought: [seu raciocínio sobre o que fazer]
Action: [nome_da_ferramenta]
Action Input: {{"arg": "valor"}}

Quando tiver a resposta final (ou não precisar de ferramentas), responda EXATAMENTE neste formato:
Thought: [seu raciocínio]
Final Answer: [sua resposta para o usuário]

Sempre pense passo a passo."#,
            tool_list
        )
    }

    fn build_messages(&self, system_prompt: &str) -> Vec<Value> {
        let mut messages = vec![json!({
            "role": "system",
            "content": system_prompt
        })];

        messages.extend(self.conversation_history.clone());

        messages
    }

    async fn call_llm(&self, messages: &[Value]) -> anyhow::Result<String> {
        let url = format!("{}/chat/completions", self.config.base_url);
        
        let body = json!({
            "model": self.config.model,
            "messages": messages,
            "max_tokens": 500,
            "temperature": 0.7
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("API error: {}", error_text));
        }

        let json_response: Value = response.json().await?;
        
        let content = json_response["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid response format from API"))?;

        Ok(content.to_string())
    }

    fn parse_response(&self, response: &str) -> anyhow::Result<ParsedResponse> {
        // Try to find Final Answer
        let final_answer_re = Regex::new(r"(?i)Final Answer:\s*(.+)$").unwrap();
        if let Some(caps) = final_answer_re.captures(response) {
            let answer = caps.get(1).map(|m| m.as_str().trim().to_string())
                .unwrap_or_else(|| response.to_string());
            return Ok(ParsedResponse::FinalAnswer(answer));
        }

        // Try to find Action
        let thought_re = Regex::new(r"(?i)Thought:\s*(.+?)(?:\n|$)").unwrap();
        let action_re = Regex::new(r"(?i)Action:\s*(.+?)(?:\n|$)").unwrap();
        let action_input_re = Regex::new(r"(?i)Action Input:\s*(\{.+\}|.+?)(?:\n|$)").unwrap();

        let thought = thought_re
            .captures(response)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();

        let action = action_re
            .captures(response)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string());

        let action_input = action_input_re
            .captures(response)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_else(|| "{}".to_string());

        if let Some(action) = action {
            return Ok(ParsedResponse::Action {
                thought,
                action,
                action_input,
            });
        }

        // If no action or final answer, treat entire response as final answer
        Ok(ParsedResponse::FinalAnswer(response.trim().to_string()))
    }

    async fn execute_tool(&self, action: &str, action_input: &str) -> anyhow::Result<String> {
        let tool = self
            .tools
            .get(action)
            .ok_or_else(|| anyhow::anyhow!("Ferramenta '{}' não encontrada", action))?;

        let args: Value = serde_json::from_str(action_input)
            .map_err(|_| anyhow::anyhow!("Action Input inválido: {}", action_input))?;

        match tool.call(args).await {
            Ok(result) => Ok(result),
            Err(e) => Ok(format!("Erro: {}", e)),
        }
    }
}

enum ParsedResponse {
    FinalAnswer(String),
    Action {
        thought: String,
        action: String,
        action_input: String,
    },
}
