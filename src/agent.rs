use crate::config::Config;
use crate::memory::embeddings::EmbeddingService;
use crate::memory::search::{format_memories_for_prompt, search_similar_memories};
use crate::memory::store::MemoryStore;
use crate::memory::{MemoryEntry, MemoryType};
use crate::security::SecurityManager;
use crate::tools::ToolRegistry;
use regex::Regex;
use reqwest::Client;
use serde_json::{json, Value};
use std::path::Path;
use tracing::{debug, info};

const USER_AGENT: &str = "RustClaw/1.0";

fn create_http_client() -> reqwest::Client {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .expect("Failed to create HTTP client")
}

pub struct Agent {
    client: Client,
    config: Config,
    tools: ToolRegistry,
    conversation_history: Vec<Value>,
    memory_store: MemoryStore,
    embedding_service: EmbeddingService,
}

impl Agent {
    pub fn new(config: Config, tools: ToolRegistry, memory_path: &Path) -> anyhow::Result<Self> {
        let memory_store = MemoryStore::new(memory_path)?;
        let embedding_service = EmbeddingService::new()?;

        Ok(Self {
            client: create_http_client(),
            config,
            tools,
            conversation_history: Vec::new(),
            memory_store,
            embedding_service,
        })
    }

    pub async fn prompt(&mut self, user_input: &str) -> anyhow::Result<String> {
        // SECURITY: Validate user input
        let validation = SecurityManager::validate_user_input(user_input);
        if !validation.valid {
            let error_msg = format!("Invalid input: {}", validation.errors.join(", "));
            tracing::warn!("Security validation failed: {}", error_msg);
            return Ok(error_msg);
        }

        // SECURITY: Sanitize user input
        let sanitized_input = SecurityManager::sanitize_user_input(user_input);
        if sanitized_input.was_modified {
            tracing::debug!("Input was sanitized: {} -> {}", 
                sanitized_input.original_length, 
                sanitized_input.sanitized_length);
        }
        
        let user_input = &sanitized_input.text;
        info!("User input: {}", user_input);

        
        let memories = self.retrieve_relevant_memories(user_input).await?;
        let memory_context = format_memories_for_prompt(&memories);

        // Build system prompt with defense instructions
        let mut system_prompt = self.build_system_prompt(&memory_context);
        
        // SECURITY: Append defense prompt
        system_prompt.push_str(&SecurityManager::get_defense_prompt());

        
        self.conversation_history.push(json!({
            "role": "user",
            "content": user_input
        }));

        
        let messages = self.build_messages(&system_prompt);

        
        let mut current_messages = messages;

        for iteration in 0..self.config.max_iterations {
            info!("ReAct iteration {}", iteration + 1);

            
            let response = self.call_llm(&current_messages).await?;
            debug!("LLM response:\n{}", response);

            
            let parsed = self.parse_response(&response)?;

            match parsed {
                ParsedResponse::FinalAnswer(answer) => {
                    info!("Final answer received");
                    
                    
                    self.conversation_history.push(json!({
                        "role": "assistant",
                        "content": answer.clone()
                    }));

                    
                    self.save_conversation_to_memory(user_input, &answer).await?;

                    return Ok(answer);
                }
                ParsedResponse::Action { thought, action, action_input } => {
                    info!("Action detected: {} with input: {}", action, action_input);

                    let raw_observation = self.execute_tool(&action, &action_input).await?;
                    
                    // SECURITY: Sanitize tool output
                    let observation = SecurityManager::clean_tool_output(&raw_observation, &action);
                    info!("Tool observation (sanitized): {}", observation);

                    
                    if action != "echo" {
                        self.save_tool_result_to_memory(&action, &action_input, &observation).await?;
                    }

                    
                    let tool_result = format!(
                        "Thought: {}\nAction: {}\nAction Input: {}\nObservation: {}",
                        thought, action, action_input, observation
                    );

                    current_messages.push(json!({
                        "role": "assistant",
                        "content": tool_result
                    }));
                }
            }
        }

        
        info!("Max iterations reached, forcing final answer");
        let final_prompt = self.build_messages(&system_prompt)
            .iter()
            .map(|m| m["content"].as_str().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");

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
            self.save_conversation_to_memory(user_input, &answer).await?;
            return Ok(answer);
        }

        Ok(final_response)
    }

    async fn retrieve_relevant_memories(&self, query: &str) -> anyhow::Result<Vec<(MemoryEntry, f32)>> {
        
        let query_embedding = self.embedding_service.embed(query).await?;

        
        let all_memories = self.memory_store.get_all()?;

        if all_memories.is_empty() {
            return Ok(vec![]);
        }

        
        let results = search_similar_memories(&query_embedding, &all_memories, 5, 0.5);

        
        for (memory, _) in &results {
            if let Err(e) = self.memory_store.increment_search_count(&memory.id) {
                tracing::warn!("Failed to increment search count: {}", e);
            }
        }

        Ok(results)
    }

    async fn save_conversation_to_memory(&self, user_input: &str, assistant_response: &str) -> anyhow::Result<()> {
        
        if user_input.len() < 10 {
            return Ok(());
        }

        
        let content = format!("Usuário: {}\nAssistente: {}", user_input, assistant_response);

        
        let embedding = self.embedding_service.embed(&content).await?;

        
        let memory = MemoryEntry::new(
            content,
            embedding,
            MemoryType::Episode,
            0.6, 
        );

        
        self.memory_store.save(&memory)?;
        info!("Saved conversation to long-term memory");

        Ok(())
    }

    async fn save_tool_result_to_memory(&self, tool_name: &str, input: &str, output: &str) -> anyhow::Result<()> {
        
        if output.starts_with("Erro:") || output.len() > 1000 {
            return Ok(());
        }

        let content = format!("Tool: {}\nInput: {}\nOutput: {}", tool_name, input, output.chars().take(200).collect::<String>());

        let embedding = self.embedding_service.embed(&content).await?;

        let memory = MemoryEntry::new(
            content,
            embedding,
            MemoryType::ToolResult,
            0.5,
        );

        self.memory_store.save(&memory)?;

        Ok(())
    }

    fn build_system_prompt(&self, memory_context: &str) -> String {
        let tool_list = if self.tools.is_empty() {
            "Nenhuma ferramenta disponível".to_string()
        } else {
            self.tools.list()
        };

        let context_info = format!(
            r#"Você é RustClaw, um assistente AI útil com memória de longo prazo.

Você tem acesso às seguintes ferramentas:
{}

CONTEXTO DO DISPOSITIVO:
- Para saber data e hora atual do sistema, use a ferramenta 'datetime'
- Para saber a localização geográfica do dispositivo, use a ferramenta 'location'
- Use essas informações quando o usuário perguntar sobre hora, data, clima local, ou serviços baseados em localização

DIRETRIZES IMPORTANTES:
1. Para BUSCAS NA INTERNET, use SEMPRE tavily_search (busca IA sem CAPTCHA) ou web_search (busca rápida)
2. Para saber DATA/HORA ATUAL, use 'datetime' - não presuma a hora
3. Para saber LOCALIZAÇÃO do dispositivo, use 'location' - útil para clima local, fuso horário, etc
4. Para CRIAR LEMBRETES, use 'add_reminder' quando o usuário pedir para ser lembrado de algo
   - Formatos aceitos: "amanhã às 10h", "daqui 2 horas", "todo dia às 8h", "toda segunda às 9h"
   - O sistema enviará mensagem automaticamente no horário marcado
5. Use http_get APENAS para APIs REST ou quando Tavily não for suficiente

Para usar uma ferramenta, responda EXATAMENTE neste formato:
Thought: [seu raciocínio sobre o que fazer]
Action: [nome_da_ferramenta]
Action Input: {{"arg": "valor"}}

Quando tiver a resposta final (ou não precisar de ferramentas), responda EXATAMENTE neste formato:
Thought: [seu raciocínio]
Final Answer: [sua resposta para o usuário]

Sempre pense passo a passo. Se houver memórias relevantes abaixo, use-as para contextualizar sua resposta.{}
"#,
            tool_list,
            memory_context
        );
        context_info
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
        
        let final_answer_re = Regex::new(r"(?i)Final Answer:\s*(.+)$").unwrap();
        if let Some(caps) = final_answer_re.captures(response) {
            let answer = caps.get(1).map(|m| m.as_str().trim().to_string())
                .unwrap_or_else(|| response.to_string());
            return Ok(ParsedResponse::FinalAnswer(answer));
        }

        
        let thought_re = Regex::new(r"(?i)Thought:\s*(.+?)(?:\n|$)").unwrap();
        let action_re = Regex::new(r"(?i)Action:\s*(.+?)(?:\n|$)").unwrap();
        let action_input_re = Regex::new(r"(?i)Action Input:\s*(\{.+
}|.+?)(?:\n|$)").unwrap();

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

    pub fn get_memory_count(&self) -> anyhow::Result<i64> {
        self.memory_store.count()
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
