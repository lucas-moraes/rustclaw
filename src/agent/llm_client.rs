use serde_json::{json, Value};

pub struct LlmClient;

impl LlmClient {
    pub fn build_system_prompt(
        tools: &crate::tools::ToolRegistry,
        memory_context: &str,
        skill: Option<&crate::skills::Skill>,
    ) -> String {
        let tool_list = if tools.is_empty() {
            "Nenhuma ferramenta disponível".to_string()
        } else if let Some(s) = skill {
            if s.preferred_tools.is_empty() {
                tools.list()
            } else {
                tools.list_filtered(&s.preferred_tools)
            }
        } else {
            tools.list()
        };

        let base_prompt = r#"Você é RustClaw, um assistente AI útil com memória de longo prazo e capacidade de adaptar sua personalidade conforme o contexto.

Você tem acesso às seguintes ferramentas:
{tools}

DIRETRIZES IMPORTANTES:
1. Para BUSCAS NA INTERNET, use SEMPRE tavily_search (busca IA sem CAPTCHA) ou web_search (busca rápida)
2. Use browser_navigate APENAS para acessar sites específicos quando necessário
3. Use browser_screenshot para capturar páginas
4. Use http_get APENAS para APIs REST ou quando Tavily não for suficiente
5. Crie lembretes quando o usuário pedir para ser lembrado de algo
6. **DESENVOLVIMENTO DE PROJETOS - REGRA CRÍTICA**: 
   - Quando o usuário especificar um diretório para o projeto, SEMPRE use esse diretório
   - **NUNCA crie arquivos em diretórios não especificados pelo usuário**
   - Se o usuário não informar um diretório, PERGUNTE antes de criar qualquer arquivo
   - Use ferramentas como file_write/shell APENAS com caminhos absolutos ou relativos ao diretório especificado
   - **SEMPRE leia o arquivo PLANO.md do diretório do projeto** antes de começar a desenvolver
   - Quando o trabalho estiver completo, NÃO mostre o código completo - apenas confirme que foi realizado

Para usar uma ferramenta, responda EXATAMENTE neste formato:
Thought: [seu raciocínio sobre o que fazer]
Retrieved Memory: [conteúdo relevante recuperado da memória, se houver]
Revise Memory: [seu raciocínio sobre se a memória recuperada é útil ou não]
Reasoning: [seu raciocínio passo a passo sobre qual ação tomar, baseado no input do usuário e na memória]
Verification: [como vou verificar se esta ação teve sucesso - seja específico]
Action: [nome_da_ferramenta]
Action Input: {{"arg": "valor"}}

Quando tiver a resposta final (ou não precisar de ferramentas), responda EXATAMENTE neste formato:
Thought: [seu raciocínio]
Verification: [o que foi verificado e confirmado como correto antes de finalizar]
Final Answer: [sua resposta para o usuário]

Sempre pense passo a passo. Se houver memórias relevantes abaixo, use-as para contextualizar sua resposta.{memory}"#;

        let base = base_prompt
            .replace("{tools}", &tool_list)
            .replace("{memory}", memory_context);

        crate::skills::prompt_builder::SkillPromptBuilder::build(&base, skill, &tool_list, memory_context)
    }

    pub fn build_messages(system_prompt: &str, conversation_history: &[Value]) -> Vec<Value> {
        let mut messages = vec![json!({
            "role": "system",
            "content": system_prompt
        })];

        messages.extend(conversation_history.to_vec());

        messages
    }

    pub async fn call_llm_with_config(
        client: &reqwest::Client,
        api_key: &str,
        messages: &[Value],
        model: &str,
        base_url: &str,
        provider: &str,
        max_tokens: usize,
    ) -> anyhow::Result<String> {
        let endpoint = if provider == "opencode-go" || provider == "opencode" {
            if model.contains("minimax") {
                "/messages"
            } else {
                "/chat/completions"
            }
        } else {
            "/chat/completions"
        };

        let url = format!("{}{}", base_url, endpoint);

        let filtered_messages: Vec<Value> = messages
            .iter()
            .filter(|m| {
                if let Some(content) = m["content"].as_str() {
                    !content.trim().is_empty()
                } else {
                    false
                }
            })
            .cloned()
            .collect();

        if filtered_messages.is_empty() {
            return Err(anyhow::anyhow!("No valid messages to send to API"));
        }

        let body = json!({
            "model": model,
            "messages": filtered_messages,
            "max_tokens": max_tokens,
            "temperature": 0.7
        });

        tracing::debug!("Sending request to URL: {}", url);
        tracing::debug!("Request body: {:?}", body);

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("X-API-Key", api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("HTTP request failed: {}", e);
                anyhow::anyhow!("HTTP request to {} failed: {}", url, e)
            })?;

        if !response.status().is_success() {
            let _status = response.status();
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("API error: {}", error_text));
        }

        let json_response: Value = response.json().await?;

        let content = if let Some(content_arr) = json_response["content"].as_array() {
            let mut text_content = String::new();
            for item in content_arr {
                if let Some(item_type) = item.get("type") {
                    if item_type.as_str() == Some("text") {
                        if let Some(text) = item.get("text") {
                            if let Some(t) = text.as_str() {
                                text_content.push_str(t);
                            }
                        }
                    }
                }
            }
            if text_content.is_empty() {
                content_arr
                    .first()
                    .and_then(|i| {
                        i.get("text")
                            .and_then(|v| v.as_str())
                            .or_else(|| i.get("thinking").and_then(|v| v.as_str()))
                    })
                    .unwrap_or("")
                    .to_string()
            } else {
                text_content
            }
        } else if let Some(c) = json_response["content"].as_str() {
            c.to_string()
        } else if let Some(choices) = json_response["choices"].as_array() {
            if let Some(choice) = choices.first() {
                if let Some(msg) = choice.get("message") {
                    if let Some(c) = msg.get("content").and_then(|v| v.as_str()) {
                        c.to_string()
                    } else if let Some(c) = msg.get("reasoning_content").and_then(|v| v.as_str()) {
                        c.to_string()
                    } else {
                        return Err(anyhow::anyhow!("No content in message"));
                    }
                } else {
                    return Err(anyhow::anyhow!("No message in choice"));
                }
            } else {
                return Err(anyhow::anyhow!("No choices in response"));
            }
        } else {
            return Err(anyhow::anyhow!("Invalid response format"));
        };

        let cleaned = crate::agent::response_parser::ResponseParser::sanitize_model_response(&content).trim().to_string();

        Ok(cleaned)
    }
}