use crate::config::Config;
use crate::memory::checkpoint::{CheckpointStore, DevelopmentCheckpoint, DevelopmentState, PlanPhase, ToolExecution};
use crate::memory::embeddings::EmbeddingService;
use crate::memory::search::{format_memories_for_prompt, search_similar_memories};
use crate::memory::skill_context::SkillContextStore;
use crate::memory::store::MemoryStore;
use crate::memory::{MemoryEntry, MemoryType};
use crate::security::SecurityManager;
use crate::skills::manager::SkillManager;
use crate::skills::prompt_builder::SkillPromptBuilder;
use crate::tools::ToolRegistry;
use crate::utils::output::{OutputManager, OutputSink};
use crate::utils::tmux::TmuxManager;
use regex::Regex;
use reqwest::Client;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info};
use std::sync::OnceLock;

static OUTPUT_MANAGER: OnceLock<OutputManager> = OnceLock::new();
static TMUX_MANAGER: OnceLock<TmuxManager> = OnceLock::new();

const USER_AGENT: &str = "RustClaw/1.0";
const SKILLS_DIR: &str = "skills";

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
    checkpoint_store: CheckpointStore,
    embedding_service: EmbeddingService,
    skill_manager: SkillManager,
    skill_context_store: SkillContextStore,
    chat_id: Option<i64>,
}

impl Agent {
    pub fn new(config: Config, tools: ToolRegistry, memory_path: &Path) -> anyhow::Result<Self> {
        let memory_store = MemoryStore::new(memory_path)?;
        let checkpoint_store = CheckpointStore::new(memory_path)?;
        let embedding_service = EmbeddingService::new()?;
        let skill_context_store = SkillContextStore::new(memory_path)?;

        // Extrai chat_id do nome do arquivo (memories_<chat_id>.db)
        let chat_id = memory_path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("memories_"))
            .and_then(|s| s.parse::<i64>().ok());

        // Carrega skill ativa do banco
        let active_skill = if let Some(cid) = chat_id {
            skill_context_store.get_active_skill(cid).unwrap_or(None)
        } else {
            None
        };

        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let skills_dir = current_dir.join(SKILLS_DIR);
        let mut skill_manager = SkillManager::new(skills_dir)?;
        if let Some(skill) = active_skill {
            let _ = skill_manager.force_skill(&skill);
        }

        Ok(Self {
            client: create_http_client(),
            config,
            tools,
            conversation_history: Vec::new(),
            memory_store,
            checkpoint_store,
            embedding_service,
            skill_manager,
            skill_context_store,
            chat_id,
        })
    }

    pub async fn prompt(&mut self, user_input: &str) -> anyhow::Result<String> {
        if user_input.eq_ignore_ascii_case("status projeto") {
            if let Some(checkpoint) = self.get_last_active_checkpoint() {
                let step_count = self.count_plan_steps(&checkpoint.plan_text);
                let tool_count = self.count_tool_execs(&checkpoint);
                return Ok(format!(
                    "Projeto ativo:\n- tarefa: {}\n- fase: {}\n- iteracao: {}\n- passos no plano: {}\n- ferramentas usadas: {}\n\nPlano:\n{}",
                    checkpoint.user_input,
                    checkpoint.phase,
                    checkpoint.current_iteration + 1,
                    step_count,
                    tool_count,
                    if checkpoint.plan_text.is_empty() { "(sem plano)" } else { checkpoint.plan_text.as_str() }
                ));
            }

            return Ok("Nenhum projeto ativo encontrado.".to_string());
        }

        let mut checkpoint = self.load_or_create_checkpoint(user_input).await?;
        let mut task_input = user_input.to_string();
        let lower_input = user_input.to_lowercase();
        let wants_new_project = lower_input.starts_with("novo projeto")
            || lower_input.starts_with("iniciar novo projeto")
            || lower_input.starts_with("novo trabalho");

        if !wants_new_project && DevelopmentCheckpoint::is_development_task(user_input) {
            if let Some(active) = self.get_last_active_checkpoint() {
                if active.id != checkpoint.id
                    && !user_input.eq_ignore_ascii_case("aprovar plano")
                    && !user_input.eq_ignore_ascii_case("cancelar plano")
                    && !user_input.to_lowercase().starts_with("editar plano:")
                {
                    checkpoint = active;
                    task_input = checkpoint.user_input.clone();
                }
            }
        }

        if user_input.eq_ignore_ascii_case("continuar projeto") {
            if let Some(active) = self.get_last_active_checkpoint() {
                checkpoint = active;
                task_input = checkpoint.user_input.clone();
            } else {
                return Ok("Nenhum projeto ativo para continuar.".to_string());
            }
        }

        if DevelopmentCheckpoint::is_development_task(&task_input) {
            if user_input.eq_ignore_ascii_case("cancelar plano") {
                checkpoint.set_plan_text(String::new());
                checkpoint.set_phase(PlanPhase::Planning);
                self.checkpoint_store.save(&checkpoint)?;
                return Ok("Plano cancelado. Descreva a nova tarefa para gerar outro plano.".to_string());
            }

            if checkpoint.plan_text.is_empty() {
                let plan = self.generate_plan(&task_input).await?;
                checkpoint.set_plan_text(plan.clone());
                let step_count = self.count_plan_steps(&plan);

                if step_count <= self.config.plan_auto_threshold {
                    checkpoint.set_phase(PlanPhase::Executing);
                    self.checkpoint_store.save(&checkpoint)?;
                } else {
                    checkpoint.set_phase(PlanPhase::AwaitingApproval);
                    self.checkpoint_store.save(&checkpoint)?;
                    return Ok(format!(
                        "Plano proposto:\n{}\n\nConfirme para executar (responda: aprovar plano)\nOu edite: editar plano: <sua versao>",
                        plan
                    ));
                }
            }

            if checkpoint.phase == PlanPhase::AwaitingApproval {
                if let Some(edited) = user_input.strip_prefix("editar plano:") {
                    let new_plan = edited.trim().to_string();
                    if !new_plan.is_empty() {
                        checkpoint.set_plan_text(new_plan.clone());
                        let step_count = self.count_plan_steps(&new_plan);
                        if step_count <= self.config.plan_auto_threshold {
                            checkpoint.set_phase(PlanPhase::Executing);
                            self.checkpoint_store.save(&checkpoint)?;
                        } else {
                            self.checkpoint_store.save(&checkpoint)?;
                            return Ok(format!(
                                "Plano atualizado:\n{}\n\nConfirme para executar (responda: aprovar plano)",
                                new_plan
                            ));
                        }
                    }
                }

                if user_input.eq_ignore_ascii_case("aprovar plano") {
                    checkpoint.set_phase(PlanPhase::Executing);
                    self.checkpoint_store.save(&checkpoint)?;
                } else {
                    return Ok(format!(
                        "Plano pendente de aprovacao. Responda: aprovar plano ou editar plano: <sua versao>\n\n{}",
                        checkpoint.plan_text
                    ));
                }
            }
        }

        // SECURITY: Validate user input
        let validation = SecurityManager::validate_user_input(&task_input);
        if !validation.valid {
            let error_msg = format!("Invalid input: {}", validation.errors.join(", "));
            tracing::warn!("Security validation failed: {}", error_msg);
            return Ok(error_msg);
        }

        // SECURITY: Sanitize user input
        let sanitized_input = SecurityManager::sanitize_user_input(&task_input);
        if sanitized_input.was_modified {
            tracing::debug!(
                "Input was sanitized: {} -> {}",
                sanitized_input.original_length,
                sanitized_input.sanitized_length
            );
        }

        let user_input = &sanitized_input.text;
        info!("User input: {}", user_input);

        // 1. Detecta skill (com hot reload automático)
        // Clone skill data immediately to avoid borrow issues
        let skill_opt = self
            .skill_manager
            .process_message(user_input)
            .map(|s| s.clone());
        let skill_name = skill_opt.as_ref().map(|s| s.name.clone());

        // 2. Recupera memórias
        let memories = self.retrieve_relevant_memories(user_input).await?;
        let memory_context = format_memories_for_prompt(&memories);

        // 3. Constrói system prompt com skill e defense instructions
        let mut system_prompt = self.build_system_prompt(&memory_context, skill_opt.as_ref());

        // SECURITY: Append defense prompt
        system_prompt.push_str(&SecurityManager::get_defense_prompt());

        // 4. Se skill mudou, salva no banco (after skill_manager borrow is done)
        if let Some(cid) = self.chat_id {
            if let Some(ref name) = skill_name {
                if let Err(e) = self.skill_context_store.save_active_skill(cid, name) {
                    tracing::error!("Failed to save skill context: {}", e);
                }
            }
        }

        // 5. Adiciona mensagem do usuário ao histórico
        self.conversation_history.push(json!({
            "role": "user",
            "content": user_input
        }));

        // 6. Build messages
        let mut current_messages = self.load_checkpoint_messages(&checkpoint)
            .unwrap_or_else(|| self.build_messages(&system_prompt));

        // 7. ReAct loop
        let start_iteration = checkpoint.current_iteration;
        let mut forced_tool_use = false;
        for iteration in start_iteration..self.config.max_iterations {
            info!("ReAct iteration {}", iteration + 1);
            checkpoint.current_iteration = iteration;
            self.save_checkpoint(&mut checkpoint, &current_messages, &[])?;

            if DevelopmentCheckpoint::is_development_task(user_input) {
                current_messages.push(json!({
                    "role": "system",
                    "content": "IMPORTANTE: para tarefas de desenvolvimento, sempre execute pelo menos uma ferramenta por etapa. Responda APENAS nos formatos especificados. Nunca inclua <system-reminder> na resposta."
                }));
            }

            let response = self.call_llm(&current_messages).await?;
            debug!("LLM response:\n{}", response);

            let parsed = self.parse_response(&response)?;

            match parsed {
                ParsedResponse::FinalAnswer(answer) => {
                    info!("Final answer received");

                    if DevelopmentCheckpoint::is_development_task(user_input)
                        && !self.checkpoint_has_tools(&checkpoint)
                        && !forced_tool_use
                    {
                        forced_tool_use = true;
                        current_messages.push(json!({
                            "role": "user",
                            "content": "Você ainda não executou nenhuma ferramenta. Para tarefas de desenvolvimento, use ferramentas (file_write, shell, file_read, etc.) antes de responder com a solução final. Prossiga com a primeira ação agora."
                        }));
                        continue;
                    }

                    self.conversation_history.push(json!({
                        "role": "assistant",
                        "content": answer.clone()
                    }));

                    self.save_conversation_to_memory(user_input, &answer)
                        .await?;

                    self.finalize_checkpoint(&mut checkpoint, DevelopmentState::Completed, &current_messages, &[])?;

                    return Ok(answer);
                }
                ParsedResponse::Action {
                    thought,
                    action,
                    action_input,
                } => {
                    info!("Action detected: {} with input: {}", action, action_input);

                    let raw_observation = self.execute_tool(&action, &action_input).await?;

                    // SECURITY: Sanitize tool output
                    let observation = SecurityManager::clean_tool_output(&raw_observation, &action);
                    info!("Tool observation (sanitized): {}", observation);

                    if action != "echo" {
                        self.save_tool_result_to_memory(&action, &action_input, &observation)
                            .await?;
                    }

                    let tool_execution = ToolExecution {
                        tool_name: action.clone(),
                        input: action_input.clone(),
                        output: observation.clone(),
                        iteration: iteration + 1,
                        timestamp: chrono::Utc::now(),
                    };

                    let tool_result = format!(
                        "Thought: {}\nAction: {}\nAction Input: {}\nObservation: {}",
                        thought, action, action_input, observation
                    );

                    current_messages.push(json!({
                        "role": "assistant",
                        "content": tool_result
                    }));

                    self.save_checkpoint(&mut checkpoint, &current_messages, &[tool_execution])?;
                }
            }
        }

        info!("Max iterations reached, forcing final answer");
        self.finalize_checkpoint(&mut checkpoint, DevelopmentState::Interrupted, &current_messages, &[])?;
        let final_prompt = self
            .build_messages(&system_prompt)
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
            }),
        ];
        let final_response = self.call_llm(&final_messages).await?;

        if let ParsedResponse::FinalAnswer(answer) = self.parse_response(&final_response)? {
            self.save_conversation_to_memory(user_input, &answer)
                .await?;
            return Ok(answer);
        }

        Ok(final_response)
    }

    async fn retrieve_relevant_memories(
        &self,
        query: &str,
    ) -> anyhow::Result<Vec<(MemoryEntry, f32)>> {
        let query_embedding = self.embedding_service.embed(query).await?;

        let all_memories = self.memory_store.get_all()?;

        if all_memories.is_empty() {
            return Ok(vec![]);
        }

        let results = search_similar_memories(&query_embedding, &all_memories, 5, 0.3);

        for (memory, _) in &results {
            if let Err(e) = self.memory_store.increment_search_count(&memory.id) {
                tracing::warn!("Failed to increment search count: {}", e);
            }
        }

        Ok(results)
    }

    async fn load_or_create_checkpoint(&self, user_input: &str) -> anyhow::Result<DevelopmentCheckpoint> {
        if DevelopmentCheckpoint::is_development_task(user_input) {
            if let Ok(Some(existing)) = self.checkpoint_store.find_by_input(user_input) {
                info!("Resuming development checkpoint: {}", existing.id);
                return Ok(existing);
            }
        }

        Ok(DevelopmentCheckpoint::new(user_input.to_string()))
    }

    async fn generate_plan(&self, user_input: &str) -> anyhow::Result<String> {
        let plan_prompt = format!(
            "Voce e um planejador. Crie um plano em passos numerados, conciso e executavel, para a tarefa abaixo.\n\nTarefa: {}\n\nRegras:\n- Use 5-10 passos\n- Cada passo deve ser uma acao concreta\n- Nao execute nada, apenas planeje\n\nFormato:\n1) ...\n2) ...\n3) ...",
            user_input
        );

        let messages = vec![
            json!({
                "role": "user",
                "content": plan_prompt
            })
        ];

        let response = self.call_llm(&messages).await?;
        Ok(response.trim().to_string())
    }

    fn count_plan_steps(&self, plan: &str) -> usize {
        plan.lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                trimmed.starts_with(|c: char| c.is_ascii_digit()) && trimmed.contains(')')
            })
            .count()
    }

    fn get_last_active_checkpoint(&self) -> Option<DevelopmentCheckpoint> {
        if let Ok(checkpoints) = self.checkpoint_store.get_active() {
            return checkpoints.into_iter().next();
        }

        None
    }

    fn count_tool_execs(&self, checkpoint: &DevelopmentCheckpoint) -> usize {
        serde_json::from_str::<Vec<ToolExecution>>(&checkpoint.completed_tools_json)
            .map(|tools| tools.len())
            .unwrap_or(0)
    }

    fn load_checkpoint_messages(&self, checkpoint: &DevelopmentCheckpoint) -> Option<Vec<Value>> {
        if checkpoint.messages_json == "[]" {
            return None;
        }

        serde_json::from_str(&checkpoint.messages_json).ok()
    }

    fn save_checkpoint(
        &self,
        checkpoint: &mut DevelopmentCheckpoint,
        messages: &[Value],
        tool_execs: &[ToolExecution],
    ) -> anyhow::Result<()> {
        if !DevelopmentCheckpoint::is_development_task(&checkpoint.user_input) {
            return Ok(());
        }

        let mut all_tools: Vec<ToolExecution> = serde_json::from_str(&checkpoint.completed_tools_json)
            .unwrap_or_default();
        all_tools.extend_from_slice(tool_execs);

        checkpoint.messages_json = serde_json::to_string(messages)?;
        checkpoint.completed_tools_json = serde_json::to_string(&all_tools)?;
        checkpoint.updated_at = chrono::Utc::now();

        self.checkpoint_store.save(checkpoint)?;
        Ok(())
    }

    fn finalize_checkpoint(
        &self,
        checkpoint: &mut DevelopmentCheckpoint,
        state: DevelopmentState,
        messages: &[Value],
        tool_execs: &[ToolExecution],
    ) -> anyhow::Result<()> {
        if !DevelopmentCheckpoint::is_development_task(&checkpoint.user_input) {
            return Ok(());
        }

        self.save_checkpoint(checkpoint, messages, tool_execs)?;
        checkpoint.set_state(state);
        self.checkpoint_store.save(checkpoint)?;
        Ok(())
    }

    fn checkpoint_has_tools(&self, checkpoint: &DevelopmentCheckpoint) -> bool {
        if checkpoint.completed_tools_json == "[]" {
            return false;
        }

        serde_json::from_str::<Vec<ToolExecution>>(&checkpoint.completed_tools_json)
            .map(|tools| !tools.is_empty())
            .unwrap_or(false)
    }

    async fn save_conversation_to_memory(
        &self,
        user_input: &str,
        assistant_response: &str,
    ) -> anyhow::Result<()> {
        if user_input.len() < 10 {
            return Ok(());
        }

        let content = format!(
            "Usuário: {}\nAssistente: {}",
            user_input, assistant_response
        );

        let embedding = self.embedding_service.embed(&content).await?;

        let memory = MemoryEntry::new(content, embedding, MemoryType::Episode, 0.6);

        self.memory_store.save(&memory)?;
        info!("Saved conversation to long-term memory");

        Ok(())
    }

    async fn save_tool_result_to_memory(
        &self,
        tool_name: &str,
        input: &str,
        output: &str,
    ) -> anyhow::Result<()> {
        if output.starts_with("Erro:") || output.len() > 1000 {
            return Ok(());
        }

        let content = format!(
            "Tool: {}\nInput: {}\nOutput: {}",
            tool_name,
            input,
            output.chars().take(200).collect::<String>()
        );

        let embedding = self.embedding_service.embed(&content).await?;

        let memory = MemoryEntry::new(content, embedding, MemoryType::ToolResult, 0.5);

        self.memory_store.save(&memory)?;

        Ok(())
    }

    fn build_system_prompt(
        &self,
        memory_context: &str,
        skill: Option<&crate::skills::Skill>,
    ) -> String {
        let tool_list = if self.tools.is_empty() {
            "Nenhuma ferramenta disponível".to_string()
        } else {
            self.tools.list()
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

Para usar uma ferramenta, responda EXATAMENTE neste formato:
Thought: [seu raciocínio sobre o que fazer]
Action: [nome_da_ferramenta]
Action Input: {{"arg": "valor"}}

Quando tiver a resposta final (ou não precisar de ferramentas), responda EXATAMENTE neste formato:
Thought: [seu raciocínio]
Final Answer: [sua resposta para o usuário]

Sempre pense passo a passo. Se houver memórias relevantes abaixo, use-as para contextualizar sua resposta.{memory}"#;

        let base = base_prompt
            .replace("{tools}", &tool_list)
            .replace("{memory}", memory_context);

        // Adiciona contexto da skill usando o builder
        SkillPromptBuilder::build(&base, skill, &tool_list, memory_context)
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
            "max_tokens": self.config.max_tokens,
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
        let sanitized = self.sanitize_model_response(response);
        let final_answer_re = Regex::new(r"(?i)Final Answer:\s*(.+)$").unwrap();
        if let Some(caps) = final_answer_re.captures(&sanitized) {
            let answer = caps
                .get(1)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_else(|| sanitized.to_string());
            return Ok(ParsedResponse::FinalAnswer(answer));
        }

        let thought_re = Regex::new(r"(?i)Thought:\s*(.+?)(?:\n|$)").unwrap();
        let action_re = Regex::new(r"(?i)Action:\s*(.+?)(?:\n|$)").unwrap();

        let thought = thought_re
            .captures(&sanitized)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();

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
                action,
                action_input,
            });
        }

        Ok(ParsedResponse::FinalAnswer(sanitized.trim().to_string()))
    }

    async fn execute_tool(&self, action: &str, action_input: &str) -> anyhow::Result<String> {
        let tool = self
            .tools
            .get(action)
            .ok_or_else(|| anyhow::anyhow!("Ferramenta '{}' não encontrada", action))?;

        let args = match self.parse_action_input_json(action_input) {
            Ok(value) => value,
            Err(e) => {
                let err_msg = format!("Erro: {}", e);
                output_write_error(&err_msg);
                return Ok("Erro: Action Input inválido. Reenvie apenas o JSON válido para a ferramenta.".to_string());
            }
        };

        output_write_tool(action, action_input, "...");
        
        match tool.call(args).await {
            Ok(result) => {
                let preview = if result.len() > 200 {
                    format!("{}...", &result[..200])
                } else {
                    result.clone()
                };
                output_write_tool(action, action_input, &preview);
                Ok(result)
            }
            Err(e) => {
                let err_msg = format!("Erro: {}", e);
                output_write_error(&err_msg);
                Ok(err_msg)
            }
        }
    }

    pub fn get_memory_count(&self) -> anyhow::Result<i64> {
        self.memory_store.count()
    }

    fn sanitize_model_response(&self, response: &str) -> String {
        let reminder_re = Regex::new(r"(?is)<system-reminder>.*?</system-reminder>").unwrap();
        reminder_re.replace_all(response, "").to_string()
    }

    fn parse_action_input_json(&self, action_input: &str) -> anyhow::Result<Value> {
        let trimmed = action_input.trim();

        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            return Ok(value);
        }

        let reminder_re = Regex::new(r"(?is)<system-reminder>.*?</system-reminder>").unwrap();
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

        if let Some(json_block) = Self::extract_json_block(&stripped) {
            if let Ok(value) = serde_json::from_str::<Value>(&json_block) {
                return Ok(value);
            }
        }

        if let Some(value) = self.recover_action_input(&stripped) {
            return Ok(value);
        }

        Err(anyhow::anyhow!("Action Input inválido: {}", action_input))
    }

    fn recover_action_input(&self, input: &str) -> Option<Value> {
        let path_re = Regex::new(r#"(?s)"path"\s*:\s*"([^"]*)""#).unwrap();
        let command_re = Regex::new(r#"(?s)"command"\s*:\s*"([^"]*)""#).unwrap();

        if let Some(caps) = path_re.captures(input) {
            let path = caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            if let Some(content) = Self::extract_json_string_field(input, "content") {
                return Some(json!({
                    "path": path,
                    "content": content,
                }));
            }
        }

        if let Some(caps) = command_re.captures(input) {
            let command = caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            if !command.is_empty() {
                return Some(json!({
                    "command": command,
                }));
            }
        }

        None
    }

    fn extract_json_string_field(input: &str, field: &str) -> Option<String> {
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

    fn extract_json_block(input: &str) -> Option<String> {
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

    pub fn model_name(&self) -> String {
        self.config.model.clone()
    }

    #[allow(dead_code)]
    pub fn get_active_skill_name(&self) -> Option<String> {
        self.skill_manager.get_active_skill_name()
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

pub fn init_tmux(skill_name: &str) {
    if TmuxManager::is_enabled() {
        let mut manager = TmuxManager::new(skill_name);
        if let Err(e) = manager.create_sessions() {
            eprintln!("⚠️  Erro ao criar sessões TMUX: {}", e);
        }
        
        let mut output = OutputManager::new();
        output.add_sink(Arc::new(crate::utils::output::ConsoleSink::new()));
        
        let _ = TMUX_MANAGER.set(manager);
        let _ = OUTPUT_MANAGER.set(output);
    }
}

pub fn get_tmux_manager() -> Option<&'static TmuxManager> {
    TMUX_MANAGER.get()
}

pub fn get_output_manager() -> Option<&'static OutputManager> {
    OUTPUT_MANAGER.get()
}

pub fn output_write(msg: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write(msg);
    }
    print!("{}", msg);
}

pub fn output_write_line(msg: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write_line(msg);
    }
    println!("{}", msg);
}

pub fn output_write_tool(tool: &str, input: &str, output: &str) {
    if let Some(out) = OUTPUT_MANAGER.get() {
        out.write_tool(tool, input, output);
    }
    println!("🛠️  TOOL: {}", tool);
    println!("📦 Args: {}", input);
}

pub fn output_write_thought(thought: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write_thought(thought);
    }
}

pub fn output_write_error(error: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write_error(error);
    }
    eprintln!("❌ {}", error);
}

pub fn output_write_debug(msg: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write_debug(msg);
    }
}

pub fn output_write_browser(path: &str, description: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write_browser(path, description);
    }
    println!("📸 {} - {}", description, path);
}
