use crate::app_store::Store;
use crate::app_state::AppState;
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
use crate::utils::build_detector::BuildDetector;
use crate::utils::error_parser::{ErrorParser, BuildValidation};
use crate::utils::colors::Colors;
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
    fallback_index: usize,
    app_store: Store<AppState>,
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
            fallback_index: 0,
            app_store: Store::new(AppState::default()),
        })
    }

    pub async fn prompt(&mut self, user_input: &str) -> anyhow::Result<String> {
        // ====== AUTO LOOP COMMAND ======
        if user_input.to_lowercase().starts_with("auto loop:") 
            || user_input.to_lowercase().starts_with("iniciar loop:") 
            || user_input.to_lowercase().starts_with("dev loop:") {
            
            // Extrai a tarefa após o comando
            let task = if let Some(idx) = user_input.find(':') {
                user_input[idx + 1..].trim()
            } else {
                return Ok("Formato: auto loop: <tarefa>".to_string());
            };
            
            if task.is_empty() {
                return Ok("Informe a tarefa. Ex: auto loop: implementar autenticação JWT".to_string());
            }
            
            // Verifica se tem um checkpoint ativo com project_dir
            let checkpoint = if let Some(mut cp) = self.get_last_active_checkpoint() {
                if cp.project_dir.is_empty() {
                    return Ok("⚠️  Nenhum diretório de projeto configurado. Use 'criar plano' primeiro ou defina o diretório.".to_string());
                }
                cp.set_auto_loop(true);
                cp.reset_retry();
                cp
            } else {
                return Ok("⚠️  Nenhum projeto ativo. Use 'criar plano' primeiro para definir o diretório.".to_string());
            };
            
            info!("🔄 Auto loop iniciado: {}", task);
            return self.run_development(task.to_string(), checkpoint).await;
        }
        
        // Comandos para ativar/desativar loop em checkpoint existente
        if user_input.eq_ignore_ascii_case("ativar loop") || user_input.eq_ignore_ascii_case("enable loop") {
            if let Some(mut checkpoint) = self.get_last_active_checkpoint() {
                checkpoint.set_auto_loop(true);
                self.checkpoint_store.save(&checkpoint)?;
                return Ok("🔄 Auto loop ativado! O sistema validará o build após cada ação.".to_string());
            }
            return Ok("Nenhum projeto ativo.".to_string());
        }
        
        if user_input.eq_ignore_ascii_case("desativar loop") || user_input.eq_ignore_ascii_case("disable loop") {
            if let Some(mut checkpoint) = self.get_last_active_checkpoint() {
                checkpoint.set_auto_loop(false);
                self.checkpoint_store.save(&checkpoint)?;
                return Ok("⏸️  Auto loop desativado.".to_string());
            }
            return Ok("Nenhum projeto ativo.".to_string());
        }
        
        if user_input.eq_ignore_ascii_case("status loop") {
            if let Some(checkpoint) = self.get_last_active_checkpoint() {
                let status = if checkpoint.auto_loop_enabled {
                    format!("🔄 Auto loop: ATIVADO\n📂 Diretório: {}\n🔢 Tentativas: {}/{}\n{}",
                        checkpoint.project_dir,
                        checkpoint.retry_count,
                        self.config.max_retries,
                        if let Some(err) = &checkpoint.last_error {
                            format!("⚠️  Último erro:\n{}", err)
                        } else {
                            "✅ Nenhum erro recente".to_string()
                        }
                    )
                } else {
                    "⏸️  Auto loop: DESATIVADO".to_string()
                };
                return Ok(status);
            }
            return Ok("Nenhum projeto ativo.".to_string());
        }
        
        // ====== NEW PLAN FLOW ======
        // Check for "criar plano" or "novo plano" or "criar novo" - more flexible matching
        let lower_input = user_input.to_lowercase();
        if lower_input.contains("criar plano") 
            || lower_input.contains("novo plano") 
            || lower_input.contains("criar novo")
            || lower_input.contains("outro plano")
            || lower_input.trim() == "criar"
            || lower_input.trim() == "novo"
        {
            // Create a fresh checkpoint, ignoring any existing one
            let mut checkpoint = DevelopmentCheckpoint::new("criar plano".to_string());
            checkpoint.set_phase(PlanPhase::AwaitingDir);
            checkpoint.set_project_dir(String::new());
            checkpoint.set_plan_file(String::new());
            self.checkpoint_store.save(&checkpoint)?;
            return Ok("📁 Informe o diretório do projeto:\nEx: /Users/macbook/projects/meu-projeto".to_string());
        }

        // Handle plan flow based on current phase
        if let Some(mut checkpoint) = self.get_last_active_checkpoint() {
            match checkpoint.phase {
                PlanPhase::AwaitingDir => {
                    let dir = user_input.trim();
                    if dir.is_empty() {
                        return Ok("Diretório inválido. Informe o caminho do diretório.".to_string());
                    }

                    // If it looks like a path (starts with / or contains common path chars), treat as directory
                    let looks_like_path = dir.starts_with('/') 
                        || dir.starts_with('.') 
                        || dir.contains("Users") 
                        || dir.contains("home")
                        || dir.contains("project")
                        || dir.contains("src")
                        || dir.contains("C:\\")
                        || dir.contains("D:\\");

                    if looks_like_path {
                        let path = std::path::Path::new(dir);
                        if !path.exists() {
                            if let Err(e) = std::fs::create_dir_all(path) {
                                return Ok(format!("Erro ao criar diretório: {}", e));
                            }
                        }

                        checkpoint.set_project_dir(dir.to_string());
                        let ai_dev_dir = path.join(".ai-dev");
                        let _ = std::fs::create_dir_all(&ai_dev_dir);
                        let plan_path = ai_dev_dir.join("plain.md");
                        checkpoint.set_plan_file(plan_path.to_string_lossy().to_string());
                        checkpoint.set_phase(PlanPhase::AwaitingIdea);
                        self.checkpoint_store.save(&checkpoint)?;

                        return Ok(format!(
                            "✅ Diretório salvo: {}\n\n💡 Agora descreva a ideia do projeto:\nEx: Um site de tarefas com Rust e React",
                            dir
                        ));
                    }

                    // Not a path — fall through to normal chat
                }

                PlanPhase::AwaitingIdea => {
                    let idea = user_input.trim();
                    if idea.is_empty() {
                        return Ok("Idea inválida. Descreva o que deseja desenvolver.".to_string());
                    }

                    // Only accept as idea if it looks like a project description
                    // (at least 5 words, or contains keywords like criar, site, app, etc.)
                    let word_count = idea.split_whitespace().count();
                    let dev_keywords = ["criar", "site", "app", "aplicativo", "projeto", "sistema", "api", "build", "make", "desenvolver", "implementar", "construir", "funcionalidade", "feature"];
                    let looks_like_idea = word_count >= 5 || dev_keywords.iter().any(|kw| idea.to_lowercase().contains(kw));

                    if looks_like_idea {
                        checkpoint.set_plan_text(idea.to_string());
                        checkpoint.set_phase(PlanPhase::AwaitingPlanEdit);
                        self.checkpoint_store.save(&checkpoint)?;

                        let plan = self.generate_plan(idea).await?;
                        checkpoint.set_plan_text(plan.clone());
                        checkpoint.set_phase(PlanPhase::AwaitingApproval);
                        checkpoint.set_plan_text(plan.clone());

                        let plan_path = std::path::Path::new(&checkpoint.plan_file);
                        let _ = std::fs::create_dir_all(plan_path.parent().unwrap_or(std::path::Path::new(".")));
                        let plan_content = format!(
                            "# Plano de Desenvolvimento\n\n**Ideia:** {}\n**Status:** Pendente de aprovação\n\n## Passos\n\n{}\n\n---\n*Edite o plano acima como desejar, depois digite: sincronizar plano",
                            idea, plan
                        );
                        let _ = std::fs::write(&checkpoint.plan_file, &plan_content);

                        self.checkpoint_store.save(&checkpoint)?;

                        return Ok(format!(
                            "✅ Plano criado em: {}\n\n{}\n\n📝 Edite o arquivo acima como desejar.\nQuando pronto, digite: sincronizar plano",
                            checkpoint.plan_file,
                            plan
                        ));
                    }

                    // Doesn't look like an idea — let it pass through to normal chat
                }

                PlanPhase::AwaitingApproval | PlanPhase::AwaitingPlanEdit => {
                    let lower = user_input.to_lowercase();
                    let is_plan_command = lower.contains("sincronizar")
                        || lower.contains("aprovar plano")
                        || lower.contains("cancelar plano")
                        || lower.contains("editar plano")
                        || lower.contains("mostrar plano")
                        || lower.contains("continuar");

                    if is_plan_command {
                        if lower.contains("cancelar") {
                            let id = checkpoint.id.clone();
                            self.checkpoint_store.delete(&id)?;
                            return Ok("Plano cancelado. Retornando ao modo normal.".to_string());
                        }

                        if lower.contains("sincronizar") || lower.contains("aprovar") {
                            if !checkpoint.plan_file.is_empty() && std::path::Path::new(&checkpoint.plan_file).exists() {
                                if let Ok(content) = std::fs::read_to_string(&checkpoint.plan_file) {
                                    if let Some(steps_start) = content.find("## Passos\n\n") {
                                        if let Some(steps_end) = content[steps_start..].find("\n\n---") {
                                            let steps = &content[steps_start + 10..steps_start + steps_end];
                                            checkpoint.set_plan_text(steps.to_string());
                                        }
                                    }
                                }
                            }

                            checkpoint.set_phase(PlanPhase::Executing);

                            // Save active skill
                            if let Some(skill_name) = self.skill_manager.get_active_skill_name() {
                                checkpoint.active_skill = Some(skill_name);
                            }

                            if !checkpoint.plan_file.is_empty() {
                                let idea = checkpoint.plan_text.lines()
                                    .find(|l| l.starts_with("**Ideia:**"))
                                    .map(|l| l.replace("**Ideia:**", "").trim().to_string())
                                    .unwrap_or_default();
                                let skill_info = checkpoint.active_skill.as_ref()
                                    .map(|s| format!("\n\n## Skill Ativa\n\n{}\n", s))
                                    .unwrap_or_default();
                                let plan_content = format!(
                                    "# Plano de Desenvolvimento\n\n**Ideia:** {}\n**Status:** ✅ Aprovado{}\n## Passos\n\n{}\n\n---\n*Plano aprovado e em execução*",
                                    idea, skill_info, checkpoint.plan_text
                                );
                                let _ = std::fs::write(&checkpoint.plan_file, &plan_content);
                            }

                            self.checkpoint_store.save(&checkpoint)?;

                            return Ok(format!(
                                "✅ Plano aprovado e em execução!\n\n📁 Diretório: {}\n📋 Plano: {}\n\nDigite 'continuar' para iniciar o desenvolvimento.",
                                checkpoint.project_dir,
                                checkpoint.plan_file
                            ));
                        }

                        if lower.contains("mostrar") {
                            let plan_file = &checkpoint.plan_file;
                            if !plan_file.is_empty() && std::path::Path::new(plan_file).exists() {
                                if let Ok(content) = std::fs::read_to_string(plan_file) {
                                    return Ok(format!("📋 Plano em {}:\n\n{}", plan_file, content));
                                }
                            }
                        }

                        return Ok(format!(
                            "Plano em '{}'. Digite 'sincronizar plano' quando terminar de editar, ou 'cancelar plano' para abortar.",
                            checkpoint.phase
                        ));
                    }

                    // Not a plan command — let it pass through to normal chat
                }

                PlanPhase::Completed => {
                    // Clean up old completed checkpoint
                    self.checkpoint_store.delete(&checkpoint.id)?;
                }

                _ => {}
            }
        }

        // ====== LEGACY COMMANDS ======
        if let Some(dir) = user_input.strip_prefix("diretorio:") {
            let dir = dir.trim();
            if dir.is_empty() {
                return Ok("Diretório inválido.".to_string());
            }

            let path = std::path::Path::new(dir);
            if !path.exists() {
                if let Err(e) = std::fs::create_dir_all(path) {
                    return Ok(format!("Erro ao criar diretório: {}", e));
                }
            }

            if let Some(mut checkpoint) = self.get_last_active_checkpoint() {
                checkpoint.set_project_dir(dir.to_string());
                let ai_dev_dir = path.join(".ai-dev");
                let _ = std::fs::create_dir_all(&ai_dev_dir);
                let plan_path = ai_dev_dir.join("plain.md");
                checkpoint.set_plan_file(plan_path.to_string_lossy().to_string());
                self.checkpoint_store.save(&checkpoint)?;
                return Ok(format!("Diretório salvo: {}", checkpoint.project_dir));
            }

            return Ok("Nenhum projeto ativo.".to_string());
        }
        if let Some(list_args) = user_input.strip_prefix("listar planos") {
            let limit = list_args
                .trim()
                .strip_prefix(':')
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(5);

            let plans = self
                .checkpoint_store
                .get_recent_with_plans(limit)
                .unwrap_or_default();

            if plans.is_empty() {
                return Ok("Nenhum plano encontrado.".to_string());
            }

            let mut output = String::from("Planos salvos:\n\n");
            for plan in plans {
                let id_short = plan.id.chars().take(8).collect::<String>();
                let step_count = self.count_plan_steps(&plan.plan_text);
                output.push_str(&format!(
                    "- {} | fase: {} | passos: {}\n  tarefa: {}\n  dir: {}\n  plano: {}\n",
                    id_short,
                    plan.phase,
                    step_count,
                    plan.user_input,
                    if plan.project_dir.is_empty() { "(nao definido)" } else { plan.project_dir.as_str() },
                    if plan.plan_file.is_empty() { "(nao definido)" } else { plan.plan_file.as_str() }
                ));
            }

            return Ok(output.trim().to_string());
        }

        if let Some(show_args) = user_input.strip_prefix("mostrar plano") {
            let id = show_args.trim().trim_start_matches(':').trim();
            if id.is_empty() {
                return Ok("Informe o id do plano. Ex: mostrar plano: abc123".to_string());
            }

            if let Ok(Some(plan)) = self.checkpoint_store.find_by_id_prefix(id) {
                return Ok(format!(
                    "Plano {}:\nTarefa: {}\nFase: {}\nDiretorio: {}\nArquivo: {}\n\n{}",
                    plan.id,
                    plan.user_input,
                    plan.phase,
                    if plan.project_dir.is_empty() { "(nao definido)" } else { plan.project_dir.as_str() },
                    if plan.plan_file.is_empty() { "(nao definido)" } else { plan.plan_file.as_str() },
                    if plan.plan_text.is_empty() { "(sem plano)" } else { plan.plan_text.as_str() }
                ));
            }

            return Ok("Plano não encontrado.".to_string());
        }
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

        // Handle "continuar projeto" — resumes an Executing checkpoint
        // Also handles common typos like "cotinuar", "contiunar", etc.
        let lower = user_input.to_lowercase();
        if lower.starts_with("contin") || lower.starts_with("cotin") || lower.starts_with("conti") {
            // Try to find any checkpoint to resume
            if let Some(checkpoints) = self.checkpoint_store.get_active().ok() {
                for mut active in checkpoints {
                    // Can resume from any phase with a plan
                    if !active.plan_text.is_empty() {
                        // Ensure .ai-dev directory exists
                        if let Some(parent) = std::path::Path::new(&active.plan_file).parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        
                        // Restore active skill if saved
                        if let Some(ref skill_name) = active.active_skill {
                            info!("Restoring skill: {}", skill_name);
                            let _ = self.skill_manager.force_skill(skill_name);
                        }
                        
                        info!("Resuming checkpoint: {} in phase {:?}", active.id, active.phase);
                        let task_input = format!(
                            "Execute o plano de desenvolvimento no diretorio {}:\n\n{}",
                            active.project_dir,
                            active.plan_text
                        );
                        return self.run_development(task_input, active).await;
                    }
                }
            }
            
            // Check for any recent checkpoints with plans
            if let Ok(checkpoints) = self.checkpoint_store.get_recent_with_plans(1) {
                for mut active in checkpoints {
                    if !active.plan_text.is_empty() {
                        // Ensure .ai-dev directory exists
                        if let Some(parent) = std::path::Path::new(&active.plan_file).parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        
                        // Restore active skill if saved
                        if let Some(ref skill_name) = active.active_skill {
                            info!("Restoring skill: {}", skill_name);
                            let _ = self.skill_manager.force_skill(skill_name);
                        }
                        
                        info!("Resuming from recent: {} in phase {:?}", active.id, active.phase);
                        let task_input = format!(
                            "Execute o plano de desenvolvimento no diretorio {}:\n\n{}",
                            active.project_dir,
                            active.plan_text
                        );
                        return self.run_development(task_input, active).await;
                    }
                }
            }
            
            return Ok("Nenhum plano encontrado para continuar. Use 'criar plano' para iniciar um novo projeto.".to_string());
        }

        // Handle clean memory commands
        let lower = user_input.to_lowercase();
        if lower.contains("limpar memória") || lower.contains("clean memory") || lower.contains("limpar memoria") {
            if lower.contains("confirm") || lower.contains("sim") || lower.contains("yes") || lower.contains("true") {
                match self.clear_all_memory().await {
                    Ok(msg) => return Ok("✓ ".to_string() + &msg),
                    Err(e) => return Ok("✗ Erro ao limpar: ".to_string() + &e),
                }
            }
            return Ok("⚠️ Para confirmar que deseja limpar TODAS as memórias (conversas, planos, checkpoints), digite:\n\n'clean memory confirm' ou 'limpar memória confirmar'".to_string());
        }

        let mut checkpoint = self.load_or_create_checkpoint(user_input).await?;
        let mut task_input = user_input.to_string();

        // If checkpoint is Executing, use plan_text as the development task
        if checkpoint.phase == PlanPhase::Executing && !checkpoint.plan_text.is_empty() {
            task_input = format!(
                "Execute o plano de desenvolvimento no diretorio {}:\n\n{}",
                checkpoint.project_dir,
                checkpoint.plan_text
            );
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

        // 6. Check if in plan mode with steps to execute
        if checkpoint.is_plan_mode() {
            let steps = checkpoint.parse_plan_steps();
            if !steps.is_empty() {
                return self.execute_plan_steps(&mut checkpoint, &system_prompt, &steps).await;
            }
        }

        // 7. Build messages
        let mut current_messages = self.load_checkpoint_messages(&checkpoint)
            .unwrap_or_else(|| self.build_messages(&system_prompt));

        // 8. ReAct loop
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

                    // BUILD VALIDATION GATE: Para dev tasks com project_dir, valida build antes de aceitar
                    if DevelopmentCheckpoint::is_development_task(user_input)
                        && !checkpoint.project_dir.is_empty()
                    {
                        match self.validate_build(&checkpoint.project_dir).await? {
                            BuildValidation::Failed { errors } => {
                                let error_summary = errors.iter()
                                    .take(5) // Mostra até 5 erros
                                    .map(|e| format!("- {} ({}:{})", e.message, e.file, e.line.unwrap_or(0)))
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                
                                current_messages.push(json!({
                                    "role": "user",
                                    "content": format!(
                                        "❌ Antes de finalizar, o build falhou com {} erro(s):\n\n{}\n\nPor favor, corrija os erros acima antes de dar Final Answer.",
                                        errors.len(),
                                        error_summary
                                    )
                                }));
                                continue; // Bloqueia FinalAnswer, loop continua
                            }
                            BuildValidation::Success => {
                                info!("✅ Build validation passed before final answer");
                            }
                        }
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
                    retrieved_memory,
                    revise_memory,
                    reasoning,
                    verification,
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

                    // VERIFICATION: Verifica automaticamente o resultado da ação
                    let verification_result = self.verify_action_result(&action, &action_input, &observation).await?;

                    let tool_execution = ToolExecution {
                        tool_name: action.clone(),
                        input: action_input.clone(),
                        output: observation.clone(),
                        iteration: iteration + 1,
                        timestamp: chrono::Utc::now(),
                    };

                    let verification_status = match &verification_result {
                        None => "✅ Verificação passou".to_string(),
                        Some(err) => format!("❌ Verificação falhou: {}", err),
                    };

                    let tool_result = format!(
                        "Thought: {}\nRetrieved Memory: {}\nRevise Memory: {}\nReasoning: {}\nVerification Plan: {}\nAction: {}\nAction Input: {}\nObservation: {}\nVerification Result: {}",
                        thought,
                        retrieved_memory.as_deref().unwrap_or("N/A"),
                        revise_memory.as_deref().unwrap_or("N/A"),
                        reasoning.as_deref().unwrap_or("N/A"),
                        verification.as_deref().unwrap_or("N/A"),
                        action,
                        action_input,
                        observation,
                        verification_status
                    );

                    current_messages.push(json!({
                        "role": "assistant",
                        "content": tool_result
                    }));

                    // Se a verificação falhou, injeta mensagem de erro para o LLM corrigir
                    if let Some(error) = verification_result {
                        current_messages.push(json!({
                            "role": "user",
                            "content": format!(
                                "⚠️ A ação '{}' falhou na verificação automática: {}\n\nPor favor, corrija o problema e tente novamente. Verifique se:\n- O arquivo/recurso foi criado corretamente\n- Não há erros de sintaxe ou execução\n- O comando foi executado com sucesso",
                                action, error
                            )
                        }));
                    }

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

    async fn run_development(
        &mut self,
        task_input: String,
        mut checkpoint: DevelopmentCheckpoint,
    ) -> anyhow::Result<String> {
        // Retrieve memories
        let memories = self.retrieve_relevant_memories(&task_input).await?;
        let memory_context = format_memories_for_prompt(&memories);

        // Build system prompt
        let mut system_prompt = self.build_system_prompt(&memory_context, None);
        system_prompt.push_str(&SecurityManager::get_defense_prompt());

        // Add user message
        self.conversation_history.push(json!({
            "role": "user",
            "content": &task_input
        }));

        // Check plan mode
        if checkpoint.is_plan_mode() {
            let steps = checkpoint.parse_plan_steps();
            if !steps.is_empty() {
                return self.execute_plan_steps(&mut checkpoint, &system_prompt, &steps).await;
            }
        }

        // Build messages
        let mut current_messages = self.load_checkpoint_messages(&checkpoint)
            .unwrap_or_else(|| self.build_messages(&system_prompt));

        // ReAct loop
        let start_iteration = checkpoint.current_iteration;
        let mut forced_tool_use = false;
        for iteration in start_iteration..self.config.max_iterations {
            info!("ReAct iteration {}", iteration + 1);
            checkpoint.current_iteration = iteration;
            self.save_checkpoint(&mut checkpoint, &current_messages, &[])?;

            current_messages.push(json!({
                "role": "system",
                "content": "IMPORTANTE: para tarefas de desenvolvimento, sempre execute pelo menos uma ferramenta por etapa. Responda APENAS nos formatos especificados. Nunca inclua <system-reminder> na resposta."
            }));

            let response = self.call_llm(&current_messages).await?;
            let parsed = self.parse_response(&response)?;

            match parsed {
                ParsedResponse::FinalAnswer(answer) => {
                    if DevelopmentCheckpoint::is_development_task(&task_input)
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

                    // BUILD VALIDATION GATE: Para dev tasks com project_dir, valida build antes de aceitar
                    if DevelopmentCheckpoint::is_development_task(&task_input)
                        && !checkpoint.project_dir.is_empty()
                    {
                        match self.validate_build(&checkpoint.project_dir).await? {
                            BuildValidation::Failed { errors } => {
                                let error_summary = errors.iter()
                                    .take(5)
                                    .map(|e| format!("- {} ({}:{})", e.message, e.file, e.line.unwrap_or(0)))
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                
                                current_messages.push(json!({
                                    "role": "user",
                                    "content": format!(
                                        "❌ Antes de finalizar, o build falhou com {} erro(s):\n\n{}\n\nPor favor, corrija os erros antes de dar Final Answer.",
                                        errors.len(),
                                        error_summary
                                    )
                                }));
                                continue;
                            }
                            BuildValidation::Success => {
                                info!("✅ Build validation passed before final answer");
                            }
                        }
                    }

                    self.conversation_history.push(json!({
                        "role": "assistant",
                        "content": answer.clone()
                    }));

                    self.save_conversation_to_memory(&task_input, &answer).await?;
                    self.finalize_checkpoint(&mut checkpoint, DevelopmentState::Completed, &current_messages, &[])?;

                    return Ok(answer);
                }
                ParsedResponse::Action {
                    thought,
                    retrieved_memory,
                    revise_memory,
                    reasoning,
                    verification,
                    action,
                    action_input,
                } => {
                    info!("Action detected: {} with input: {}", action, action_input);

                    let raw_observation = self.execute_tool(&action, &action_input).await?;
                    let observation = SecurityManager::clean_tool_output(&raw_observation, &action);

                    if action != "echo" {
                        self.save_tool_result_to_memory(&action, &action_input, &observation).await?;
                    }

                    // VERIFICATION: Verifica automaticamente o resultado da ação
                    let verification_result = self.verify_action_result(&action, &action_input, &observation).await?;

                    let tool_execution = ToolExecution {
                        tool_name: action.clone(),
                        input: action_input.clone(),
                        output: observation.clone(),
                        iteration: iteration + 1,
                        timestamp: chrono::Utc::now(),
                    };

                    let verification_status = match &verification_result {
                        None => "✅ Verificação passou".to_string(),
                        Some(err) => format!("❌ Verificação falhou: {}", err),
                    };

                    let tool_result = format!(
                        "Thought: {}\nRetrieved Memory: {}\nRevise Memory: {}\nReasoning: {}\nVerification Plan: {}\nAction: {}\nAction Input: {}\nObservation: {}\nVerification Result: {}",
                        thought,
                        retrieved_memory.as_deref().unwrap_or("N/A"),
                        revise_memory.as_deref().unwrap_or("N/A"),
                        reasoning.as_deref().unwrap_or("N/A"),
                        verification.as_deref().unwrap_or("N/A"),
                        action,
                        action_input,
                        observation,
                        verification_status
                    );

                    current_messages.push(json!({
                        "role": "assistant",
                        "content": tool_result
                    }));

                    // Se a verificação falhou, injeta mensagem de erro para o LLM corrigir
                    if let Some(error) = verification_result {
                        current_messages.push(json!({
                            "role": "user",
                            "content": format!(
                                "⚠️ A ação '{}' falhou na verificação automática: {}\n\nPor favor, corrija o problema e tente novamente.",
                                action, error
                            )
                        }));
                    }

                    self.save_checkpoint(&mut checkpoint, &current_messages, &[tool_execution])?;

                    // AUTO LOOP: Valida build se habilitado
                    if checkpoint.auto_loop_enabled && !checkpoint.project_dir.is_empty() {
                        info!("Auto loop enabled, validating build...");
                        
                        match self.validate_build(&checkpoint.project_dir).await? {
                            BuildValidation::Success => {
                                info!("✅ Build passed!");
                                checkpoint.reset_retry();
                                
                                // Adiciona feedback positivo ao LLM
                                current_messages.push(json!({
                                    "role": "system",
                                    "content": "✅ Build passou com sucesso! Continue com a próxima ação ou finalize se tudo estiver pronto."
                                }));
                            }
                            BuildValidation::Failed { errors } => {
                                checkpoint.increment_retry();
                                let error_msg = format!(
                                    "❌ Build falhou com {} erro(s):\n\n{}",
                                    errors.len(),
                                    errors.iter()
                                        .enumerate()
                                        .map(|(i, e)| format!("{}. {}", i + 1, e))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                );
                                
                                checkpoint.set_last_error(error_msg.clone());
                                info!("❌ Build failed with {} errors (retry {}/{})", 
                                      errors.len(), checkpoint.retry_count, self.config.max_retries);
                                
                                if checkpoint.should_retry(self.config.max_retries) {
                                    // Adiciona feedback de erro ao LLM para corrigir
                                    current_messages.push(json!({
                                        "role": "system",
                                        "content": format!(
                                            "{}\n\n🔧 Por favor, corrija estes erros e execute as ações necessárias. Tentativa {}/{}", 
                                            error_msg, checkpoint.retry_count, self.config.max_retries
                                        )
                                    }));
                                } else {
                                    // Máximo de retries atingido
                                    let failure_msg = format!(
                                        "❌ Máximo de {} tentativas atingido. Último erro:\n\n{}",
                                        self.config.max_retries, error_msg
                                    );
                                    
                                    self.finalize_checkpoint(
                                        &mut checkpoint, 
                                        DevelopmentState::Failed, 
                                        &current_messages, 
                                        &[]
                                    )?;
                                    
                                    return Ok(failure_msg);
                                }
                            }
                        }
                    }
                }
            }
        }

        info!("Max iterations reached");
        self.finalize_checkpoint(&mut checkpoint, DevelopmentState::Interrupted, &current_messages, &[])?;
        Ok(format!("Execução interrompida após {} iterações.", self.config.max_iterations))
    }

    async fn execute_plan_steps(
        &mut self,
        checkpoint: &mut DevelopmentCheckpoint,
        system_prompt: &str,
        steps: &[String],
    ) -> anyhow::Result<String> {
        let total = steps.len();
        let project_dir = checkpoint.project_dir.clone();
        let plan_file = checkpoint.plan_file.clone();

        for (step_idx, step) in steps.iter().enumerate() {
            let step_num = step_idx + 1;
            checkpoint.set_current_step(step_idx);

            info!("Executing plan step {}/{}: {}", step_num, total, step);

            let step_prompt = format!(
                "## PROGRESSO DO PLANO\nEtapa {}/{} de {}\n\n**ETAPA ATUAL:**\n{}\n\n**DIRETÓRIO DO PROJETO:** {}\n\n**INSTRUÇÕES:**\n- Você está executando UM passo do plano de cada vez\n- Use as ferramentas necessárias para completar ESTA etapa\n- Após completar, responda com: Step Complete: [breve resumo do que foi feito]\n- Se precisar de mais ações, continue usando ferramentas\n\n**PLANO COMPLETO (para contexto):**\n{}",
                step_num,
                total,
                total,
                step,
                project_dir,
                steps.join("\n")
            );

            let mut step_messages = vec![
                json!({"role": "system", "content": system_prompt}),
                json!({"role": "system", "content": "MODO PLANO DE DESENVOLVIMENTO:\nVocê está executando um plano de desenvolvimento estruturado. Foque APENAS na etapa atual. Use ferramentas necessárias (file_write, shell, file_read, etc.). Quando a etapa estiver completa, responda no formato:\nStep Complete: [resumo]"}),
                json!({"role": "user", "content": step_prompt}),
            ];

            // ReAct loop for each step (max 5 iterations per step)
            let mut step_complete = false;
            for _iteration in 0..5 {
                let response = self.call_llm(&step_messages).await?;
                let parsed = self.parse_response(&response)?;

                match parsed {
                    ParsedResponse::FinalAnswer(answer) => {
                        if answer.to_lowercase().contains("step complete:") {
                            // BUILD VALIDATION: Valida build antes de marcar step como completo
                            if !checkpoint.project_dir.is_empty() {
                                match self.validate_build(&checkpoint.project_dir).await? {
                                    BuildValidation::Failed { errors } => {
                                        let error_summary = errors.iter()
                                            .take(3)
                                            .map(|e| format!("- {} ({}:{})", e.message, e.file, e.line.unwrap_or(0)))
                                            .collect::<Vec<_>>()
                                            .join("\n");
                                        
                                        step_messages.push(json!({
                                            "role": "user",
                                            "content": format!(
                                                "❌ Step não pode ser marcado como completo. Build falhou:\n\n{}\n\nCorrija os erros e tente novamente.",
                                                error_summary
                                            )
                                        }));
                                        continue; // Não marca como completo, continua o loop
                                    }
                                    BuildValidation::Success => {
                                        info!("✅ Build validation passed for step {}/{}", step_num, total);
                                    }
                                }
                            }

                            checkpoint.mark_step_done(step_idx);
                            self.checkpoint_store.save(checkpoint)?;
                            self.update_plan_progress(&plan_file, &steps, &checkpoint.completed_steps)?;
                            info!("Step {}/{} completed: {}", step_num, total, answer);
                            step_complete = true;
                            break;
                        } else {
                            // LLM gave a final answer but didn't say "Step Complete"
                            // Prompt it to continue with tools
                            step_messages.push(json!({
                                "role": "assistant",
                                "content": answer
                            }));
                            step_messages.push(json!({
                                "role": "user",
                                "content": "Use ferramentas para executar as ações necessárias. Quando terminar, responda: Step Complete: [resumo]"
                            }));
                        }
                    }
                    ParsedResponse::Action {
                        thought,
                        retrieved_memory,
                        revise_memory,
                        reasoning,
                        verification,
                        action,
                        action_input,
                    } => {
                        info!("Step {}/{} executing tool: {}", step_num, total, action);

                        // Execute the tool
                        let raw_observation = self.execute_tool(&action, &action_input).await?;
                        let observation = SecurityManager::clean_tool_output(&raw_observation, &action);

                        // Save to memory
                        if action != "echo" {
                            self.save_tool_result_to_memory(&action, &action_input, &observation).await?;
                        }

                        // VERIFICATION: Verifica automaticamente o resultado da ação
                        let verification_result = self.verify_action_result(&action, &action_input, &observation).await?;

                        // Add tool execution to checkpoint
                        let tool_execution = ToolExecution {
                            tool_name: action.clone(),
                            input: action_input.clone(),
                            output: observation.clone(),
                            iteration: checkpoint.current_iteration + 1,
                            timestamp: chrono::Utc::now(),
                        };

                        let verification_status = match &verification_result {
                            None => "✅ Verificação passou".to_string(),
                            Some(err) => format!("❌ Verificação falhou: {}", err),
                        };

                        // Add to message history
                        let tool_result = format!(
                            "Thought: {}\nRetrieved Memory: {}\nRevise Memory: {}\nReasoning: {}\nVerification Plan: {}\nAction: {}\nAction Input: {}\nObservation: {}\nVerification Result: {}",
                            thought,
                            retrieved_memory.as_deref().unwrap_or("N/A"),
                            revise_memory.as_deref().unwrap_or("N/A"),
                            reasoning.as_deref().unwrap_or("N/A"),
                            verification.as_deref().unwrap_or("N/A"),
                            action,
                            action_input,
                            observation,
                            verification_status
                        );

                        step_messages.push(json!({
                            "role": "assistant",
                            "content": tool_result
                        }));

                        // Se a verificação falhou, injeta mensagem de erro
                        if let Some(error) = verification_result {
                            step_messages.push(json!({
                                "role": "user",
                                "content": format!(
                                    "⚠️ A ação '{}' falhou na verificação: {}\n\nCorrija o problema antes de continuar.",
                                    action, error
                                )
                            }));
                        }

                        // Save checkpoint with tool execution
                        self.save_checkpoint(checkpoint, &step_messages, &[tool_execution])?;
                    }
                }
            }

            if !step_complete {
                info!("Step {}/{} max iterations reached, marking as done anyway", step_num, total);
                checkpoint.mark_step_done(step_idx);
                self.checkpoint_store.save(checkpoint)?;
                self.update_plan_progress(&plan_file, &steps, &checkpoint.completed_steps)?;
            }
        }

        checkpoint.set_phase(PlanPhase::Completed);
        checkpoint.set_state(DevelopmentState::Completed);
        self.checkpoint_store.save(checkpoint)?;

        let summary = format!(
            "✅ Plano de desenvolvimento concluído!\n\n{} etapas executadas no diretório: {}",
            total,
            project_dir
        );

        Ok(summary)
    }

    fn update_plan_progress(
        &self,
        plan_file: &str,
        _steps: &[String],
        completed: &[usize],
    ) -> anyhow::Result<()> {
        if plan_file.is_empty() || !std::path::Path::new(plan_file).exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(plan_file)?;
        let step_re = Regex::new(r"(?m)^(\s*\d+)\.\s*(\[[ xX]\])\s+(.*)$").unwrap();
        let done_re = Regex::new(r"\[x\]|\[X\]").unwrap();

        let updated = step_re.replace_all(&content, |caps: &regex::Captures| {
            let number = &caps[1];
            let step_text = &caps[3];
            let step_idx: usize = number.trim().parse::<usize>().unwrap_or(1).saturating_sub(1);

            if completed.contains(&step_idx) {
                format!("{}. [x] {}", number, step_text)
            } else {
                format!("{}. [ ] {}", number, step_text)
            }
        }).to_string();

        std::fs::write(plan_file, updated)?;

        Ok(())
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

    async fn generate_plan(&mut self, user_input: &str) -> anyhow::Result<String> {
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

    /// Valida o build do projeto no diretório especificado
    async fn validate_build(&mut self, project_dir: &str) -> anyhow::Result<BuildValidation> {
        // Detecta tipo de projeto e comando de build
        let build_info = BuildDetector::detect(project_dir);
        
        if build_info.build_command.is_empty() {
            info!("No build command detected for {}, skipping validation", project_dir);
            return Ok(BuildValidation::Success);
        }
        
        info!("Running build command: {} in {}", build_info.build_command, project_dir);
        
        // Executa o comando de build via shell tool
        let build_result = self.execute_tool("shell", &build_info.build_command).await?;
        
        // Verifica se o build foi bem-sucedido através do status code
        let success = !build_result.contains("❌ Erro");
        
        if success {
            info!("Build successful for {}", project_dir);
            return Ok(BuildValidation::Success);
        }
        
        // Se falhou, parseia os erros
        info!("Build failed, parsing errors...");
        let project_type = format!("{:?}", build_info.project_type);
        let validation = ErrorParser::parse(&build_result, &project_type);
        
        Ok(validation)
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
        } else if let Some(s) = skill {
            if s.preferred_tools.is_empty() {
                self.tools.list()
            } else {
                self.tools.list_filtered(&s.preferred_tools)
            }
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

    async fn call_llm(&mut self, messages: &[Value]) -> anyhow::Result<String> {
        // Try primary model first
        let result = self.call_llm_with_config(messages, &self.config.model, &self.config.base_url, &self.config.provider).await;
        
        // If primary fails, try fallback models
        if result.is_err() && !self.config.fallback_models.is_empty() {
            tracing::warn!("Primary model failed, trying fallbacks...");
            
            for fallback in &self.config.fallback_models {
                tracing::info!("Trying fallback model: {}", fallback.model);
                match self.call_llm_with_config(messages, &fallback.model, &fallback.base_url, "default").await {
                    Ok(response) => {
                        tracing::info!("Fallback model {} succeeded", fallback.model);
                        return Ok(response);
                    }
                    Err(e) => {
                        tracing::warn!("Fallback model {} failed: {}", fallback.model, e);
                    }
                }
            }
        }
        
        result
    }

    async fn call_llm_with_config(&self, messages: &[Value], model: &str, base_url: &str, provider: &str) -> anyhow::Result<String> {
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

        let message = &json_response["choices"][0]["message"];
        
        let content = if let Some(c) = message["content"].as_str() {
            if !c.is_empty() {
                c
            } else if let Some(r) = message["reasoning_content"].as_str() {
                r
            } else {
                return Err(anyhow::anyhow!("Empty response from API"));
            }
        } else if let Some(r) = message["reasoning_content"].as_str() {
            r
        } else {
            return Err(anyhow::anyhow!("Invalid response format from API"));
        };

        let reminder_re = Regex::new(r"(?is)<system-reminder>.*?</system-reminder>").unwrap();
        let cleaned = reminder_re.replace_all(content, "").trim().to_string();

        Ok(cleaned)
    }

    fn parse_response(&self, response: &str) -> anyhow::Result<ParsedResponse> {
        let sanitized = self.sanitize_model_response(response);
        let final_answer_re = Regex::new(r"(?si)Final Answer:\s*(.+)$").unwrap();
        if let Some(caps) = final_answer_re.captures(&sanitized) {
            let answer = caps
                .get(1)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_else(|| sanitized.to_string());
            return Ok(ParsedResponse::FinalAnswer(answer));
        }

        let thought_re = Regex::new(r"(?i)Thought:\s*(.+?)(?:\n|$)").unwrap();
        let retrieved_memory_re = Regex::new(r"(?i)Retrieved Memory:\s*(.+?)(?:\n(?:Revise Memory:|Reasoning:|Verification:|Action:|Final Answer:)|$)").unwrap();
        let revise_memory_re = Regex::new(r"(?i)Revise Memory:\s*(.+?)(?:\n(?:Reasoning:|Verification:|Action:|Final Answer:)|$)").unwrap();
        let reasoning_re = Regex::new(r"(?i)Reasoning:\s*(.+?)(?:\n(?:Verification:|Action:|Final Answer:)|$)").unwrap();
        let verification_re = Regex::new(r"(?i)Verification:\s*(.+?)(?:\n(?:Action:|Final Answer:)|$)").unwrap();
        let action_re = Regex::new(r"(?i)Action:\s*(.+?)(?:\n|$)").unwrap();

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
                    // Safe UTF-8 truncation: find the last valid char boundary
                    let mut end = 200.min(result.len());
                    while end > 0 && !result.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}...", &result[..end])
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

    /// Verifica automaticamente se uma ação executada teve sucesso
    /// Retorna None se passou, Some(erro) se falhou
    async fn verify_action_result(
        &mut self,
        action: &str,
        action_input: &str,
        observation: &str,
    ) -> anyhow::Result<Option<String>> {
        // Se observation começa com erro, falhou
        if observation.starts_with("❌ Erro") || observation.starts_with("Erro:") {
            return Ok(Some(format!("A ferramenta retornou erro: {}", observation)));
        }

        match action {
            "file_write" => {
                // Verifica se arquivo foi criado/modificado lendo-o de volta
                if let Ok(args) = self.parse_action_input_json(action_input) {
                    if let Some(path) = args["path"].as_str() {
                        match self.execute_tool("file_read", &format!(r#"{{"path": "{}"}}"#, path)).await {
                            Ok(content) if !content.starts_with("❌ Erro") && !content.is_empty() => {
                                Ok(None) // Arquivo existe e tem conteúdo
                            }
                            _ => Ok(Some(format!(
                                "Arquivo '{}' não foi criado ou está vazio após file_write",
                                path
                            ))),
                        }
                    } else {
                        Ok(None) // Sem path para verificar
                    }
                } else {
                    Ok(None)
                }
            }
            "shell" => {
                // Verifica se há indicadores de erro no output
                let error_indicators = ["❌ Erro", "error:", "ERROR:", "FAILED", "failed", "panicked", "exception", "Exception"];
                let has_error = error_indicators.iter().any(|ind| observation.contains(ind));
                
                if has_error {
                    Ok(Some(format!("Comando shell falhou. Output contém indicadores de erro")))
                } else {
                    Ok(None)
                }
            }
            "http_get" | "http_post" => {
                // Verifica status code (procura por padrões tipo "status: 4xx" ou "status: 5xx")
                if observation.contains("status: 4") || observation.contains("status: 5") {
                    Ok(Some(format!("Requisição HTTP falhou com status code de erro")))
                } else if observation.contains("❌ Erro") {
                    Ok(Some(format!("Requisição HTTP retornou erro")))
                } else {
                    Ok(None)
                }
            }
            "file_read" => {
                // Se não retornou erro, passou
                if observation.starts_with("❌ Erro") {
                    Ok(Some(format!("Falha ao ler arquivo")))
                } else {
                    Ok(None)
                }
            }
            _ => {
                // Para outras ferramentas, apenas verifica se não tem erro explícito
                if observation.starts_with("❌ Erro") {
                    Ok(Some(format!("A ferramenta '{}' retornou erro", action)))
                } else {
                    Ok(None)
                }
            }
        }
    }

    pub fn get_memory_count(&self) -> anyhow::Result<i64> {
        self.memory_store.count()
    }

    pub async fn clear_all_memory(&self) -> Result<String, String> {
        // Clear all memories
        self.memory_store.clear_all()
            .map_err(|e| format!("Erro ao limpar memória: {}", e))?;

        // Clear checkpoints (get all recent ones)
        if let Ok(checkpoints) = self.checkpoint_store.get_recent_with_plans(1000) {
            for cp in checkpoints {
                let _ = self.checkpoint_store.delete(&cp.id);
            }
        }

        Ok("Todas as memórias foram limpas (conversas, planos, checkpoints, skills ativas)".to_string())
    }

    pub fn get_skill_manager(&self) -> &SkillManager {
        &self.skill_manager
    }

    pub fn list_skills(&self) -> Vec<String> {
        self.skill_manager.list_available_skills()
    }

    pub fn force_skill(&mut self, skill_name: &str) -> Result<(), String> {
        self.skill_manager.force_skill(skill_name)
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

        // Try to extract heredoc content for shell commands
        if let Some(value) = Self::parse_heredoc_input(&stripped) {
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

    fn parse_heredoc_input(input: &str) -> Option<Value> {
        // Check if this is a shell command trying to create a file
        if input.contains("cat >") || input.contains("tee >") {
            // Try to find heredoc pattern: cat > file << EOF ... EOF
            let heredoc_re = Regex::new(
                r#""command"\s*:\s*"cat\s+>\s+([^"]+)\s+<<\s*'?\w+'?\s*\n(.*?)\n\w+""#
            ).ok()?;
            
            if let Some(caps) = heredoc_re.captures(input) {
                let file_path = caps.get(1)?.as_str();
                let content = caps.get(2)?.as_str();
                
                // Use file_write instead of shell for creating files
                return Some(json!({
                    "path": file_path,
                    "content": content
                }));
            }
            
            // Try alternative pattern without quotes around EOF
            let alt_re = Regex::new(
                r#""command"\s*:\s*"([^"]*cat[^"]*\bEOF\b[^"]*)""#
            ).ok()?;
            
            if let Some(caps) = alt_re.captures(input) {
                let command = caps.get(1)?.as_str();
                
                // Check if it's trying to write a file
                let file_re = Regex::new(r"cat\s+>\s+(\S+)").ok()?;
                if let Some(file_caps) = file_re.captures(command) {
                    let file_path = file_caps.get(1)?.as_str();
                    
                    // Try to extract content between EOF markers
                    let eof_re = Regex::new(r"<<\s*'?(\w+)'?\s*\n(.*?)\n\1").ok()?;
                    if let Some(eof_caps) = eof_re.captures(input) {
                        let content = eof_caps.get(2)?.as_str();
                        
                        return Some(json!({
                            "path": file_path,
                            "content": content
                        }));
                    }
                }
            }
        }
        
        None
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

    pub fn get_app_store(&self) -> &Store<AppState> {
        &self.app_store
    }

    pub fn get_app_state(&self) -> AppState {
        self.app_store.get_state()
    }

    pub fn update_app_state<F>(&self, updater: F)
    where
        F: FnOnce(&AppState) -> AppState,
    {
        self.app_store.set_state(updater);
    }
}

enum ParsedResponse {
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
    print!("{}{}", Colors::CLEAR_LINE, Colors::ORANGE);
    print!("{} ", "⬡");
    print!("{}{}", Colors::RESET, Colors::DIM);
    println!("{}  {}{}", tool, input, Colors::RESET);
    println!(
        "{}{}{}",
        Colors::DIM, output, Colors::RESET
    );
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
    eprintln!(
        "{}{}{} {}{}{}",
        Colors::RED, "⨯", Colors::RESET,
        error,
        Colors::RESET,
        Colors::RESET
    );
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
