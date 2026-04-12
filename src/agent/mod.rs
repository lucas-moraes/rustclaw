//! RustClaw Agent Module
//!
//! The Agent is the core component that orchestrates the ReAct loop, memory management,
//! skill execution, and tool invocations.
//!
//! ## Submodules
//!
//! - `llm_client` - LLM HTTP calls and client management
//! - `response_parser` - Parse and sanitize LLM responses  
//! - `session` - Session management and history
//! - `plan_executor` - Execute development plans
//! - `build_validator` - Validate builds
//! - `output` - Output formatting

pub mod build_validator;
pub mod conversation_summarizer;
pub mod cost_tracker;
pub mod llm_client;
pub mod output;
pub mod plan_executor;
pub mod rate_limiter;
pub mod response_parser;
pub mod session;
pub mod token_counter;

pub use response_parser::{ParsedResponse, ResponseParser};

use crate::app_state::AppState;
use crate::app_store::Store;
use crate::config::Config;
use crate::memory::checkpoint::{
    CheckpointStore, DevelopmentCheckpoint, DevelopmentState, PlanPhase, PlanStage, ToolExecution,
};
use crate::memory::embeddings::EmbeddingService;
use crate::memory::search::{format_memories_for_prompt, search_similar_memories};
use crate::memory::skill_context::SkillContextStore;
use crate::memory::store::MemoryStore;
use crate::memory::{MemoryEntry, MemoryType};
use crate::security::SecurityManager;
use crate::skills::manager::SkillManager;
use crate::skills::prompt_builder::SkillPromptBuilder;
use crate::tools::ToolRegistry;
use crate::utils::build_detector::BuildDetector;
use crate::utils::colors::Colors;
use crate::utils::error_parser::{BuildValidation, ErrorParser};
use crate::utils::output::OutputManager;
use crate::utils::tmux::TmuxManager;
use crate::workspace_trust::{TrustEvaluator, WorkspaceTrustStore};
use crate::agent::conversation_summarizer::ConversationSummarizer;
use crate::agent::cost_tracker::CostTracker;
use crate::agent::rate_limiter::RateLimiter;
use chrono::{DateTime, Utc};
use regex::Regex;
use reqwest::Client;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use tracing::{debug, info};

static OUTPUT_MANAGER: OnceLock<OutputManager> = OnceLock::new();
static TMUX_MANAGER: OnceLock<TmuxManager> = OnceLock::new();

// Local regex patterns used in this module
#[allow(dead_code)]
static RE_PLAN_STEP: OnceLock<Regex> = OnceLock::new();
#[allow(dead_code)]
static RE_REVIEW: OnceLock<Regex> = OnceLock::new();
#[allow(dead_code)]
static RE_SUGGESTION: OnceLock<Regex> = OnceLock::new();

const USER_AGENT: &str = "RustClaw/1.0";
const SKILLS_DIR: &str = "skills";

fn create_http_client() -> anyhow::Result<reqwest::Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))
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
    #[allow(dead_code)]
    fallback_index: usize,
    #[allow(dead_code)]
    app_store: Store<AppState>,
    workspace_trust: Option<TrustEvaluator>,
    summarizer: ConversationSummarizer,
    compression_count: usize,
    cost_tracker: CostTracker,
    rate_limiter: RateLimiter,
}

/// Session details for display
#[allow(dead_code)]
pub struct SessionDetails {
    pub id: String,
    pub user_input: String,
    pub phase: String,
    pub state: String,
    pub plan_text: String,
    pub project_dir: String,
    pub message_count: usize,
    pub created_at: DateTime<Utc>,
}

impl Agent {
    pub fn new(config: Config, tools: ToolRegistry, memory_path: &Path) -> anyhow::Result<Self> {
        let memory_store = MemoryStore::new(memory_path)?;

        if let Err(e) = memory_store.delete_all_without_session() {
            tracing::warn!("Failed to cleanup old memories: {}", e);
        } else {
            tracing::info!("Cleaned up old memories without session_id");
        }

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

        // Load workspace trust store from file if exists
        let trust_config_dir = memory_path.parent().unwrap_or(memory_path);
        let trust_file = trust_config_dir.join("trust.json");
        let workspace_trust = if trust_file.exists() {
            match WorkspaceTrustStore::load_from(&trust_file) {
                Ok(store) => Some(TrustEvaluator::with_store(store)),
                Err(e) => {
                    tracing::warn!("Failed to load trust store: {}", e);
                    Some(TrustEvaluator::new())
                }
            }
        } else {
            Some(TrustEvaluator::new())
        };

        let max_context_tokens = config.max_context_tokens;

        Ok(Self {
            client: create_http_client()?,
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
            workspace_trust,
            summarizer: ConversationSummarizer::new(max_context_tokens, 10),
            compression_count: 0,
            cost_tracker: CostTracker::new(),
            rate_limiter: RateLimiter::from_env(),
        })
    }

    pub async fn prompt(&mut self, user_input: &str) -> anyhow::Result<String> {
        // ====== AUTO LOOP COMMAND ======
        if user_input.to_lowercase().starts_with("auto loop:")
            || user_input.to_lowercase().starts_with("iniciar loop:")
            || user_input.to_lowercase().starts_with("dev loop:")
        {
            // Extrai a tarefa após o comando
            let task = if let Some(idx) = user_input.find(':') {
                user_input[idx + 1..].trim()
            } else {
                return Ok("Formato: auto loop: <tarefa>".to_string());
            };

            if task.is_empty() {
                return Ok(
                    "Informe a tarefa. Ex: auto loop: implementar autenticação JWT".to_string(),
                );
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
        if user_input.eq_ignore_ascii_case("ativar loop")
            || user_input.eq_ignore_ascii_case("enable loop")
        {
            if let Some(mut checkpoint) = self.get_last_active_checkpoint() {
                checkpoint.set_auto_loop(true);
                self.checkpoint_store.save(&checkpoint)?;
                return Ok(
                    "🔄 Auto loop ativado! O sistema validará o build após cada ação.".to_string(),
                );
            }
            return Ok("Nenhum projeto ativo.".to_string());
        }

        if user_input.eq_ignore_ascii_case("desativar loop")
            || user_input.eq_ignore_ascii_case("disable loop")
        {
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
                    format!(
                        "🔄 Auto loop: ATIVADO\n📂 Diretório: {}\n🔢 Tentativas: {}/{}\n{}",
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
            // First, try to get any existing checkpoint to potentially clean up
            if let Some(old_checkpoint) = self.get_last_active_checkpoint() {
                // If there's an in_progress checkpoint from a previous plan, clean it up
                if old_checkpoint.state == DevelopmentState::InProgress
                    && old_checkpoint.phase != PlanPhase::Executing
                    && old_checkpoint.project_dir.is_empty()
                {
                    // Delete old checkpoint to start fresh
                    let _ = self.checkpoint_store.delete(&old_checkpoint.id);
                }
            }

            let mut checkpoint = DevelopmentCheckpoint::new("criar plano".to_string());
            checkpoint.set_phase(PlanPhase::AwaitingDir);
            checkpoint.set_project_dir(String::new());
            checkpoint.set_plan_file(String::new());
            self.checkpoint_store.save(&checkpoint)?;
            return Ok(
                "📁 Informe o diretório do projeto:\nEx: /Users/macbook/projects/meu-projeto"
                    .to_string(),
            );
        }

        // Handle plan flow based on current phase
        if let Some(mut checkpoint) = self.get_last_active_checkpoint() {
            match checkpoint.phase {
                PlanPhase::AwaitingDir => {
                    let dir = user_input.trim();
                    if dir.is_empty() {
                        return Ok(
                            "Diretório inválido. Informe o caminho do diretório.".to_string()
                        );
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
                    let dev_keywords = [
                        "criar",
                        "site",
                        "app",
                        "aplicativo",
                        "projeto",
                        "sistema",
                        "api",
                        "build",
                        "make",
                        "desenvolver",
                        "implementar",
                        "construir",
                        "funcionalidade",
                        "feature",
                    ];
                    let looks_like_idea = word_count >= 5
                        || dev_keywords
                            .iter()
                            .any(|kw| idea.to_lowercase().contains(kw));

                    if looks_like_idea {
                        checkpoint.set_plan_text(idea.to_string());
                        checkpoint.set_phase(PlanPhase::AwaitingPlanEdit);
                        self.checkpoint_store.save(&checkpoint)?;

                        let plan = self.generate_plan(idea).await?;
                        checkpoint.set_plan_text(plan.clone());
                        checkpoint.set_phase(PlanPhase::AwaitingApproval);
                        checkpoint.set_plan_text(plan.clone());

                        let plan_path = std::path::Path::new(&checkpoint.plan_file);
                        let _ = std::fs::create_dir_all(
                            plan_path.parent().unwrap_or(std::path::Path::new(".")),
                        );
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
                            if !checkpoint.plan_file.is_empty()
                                && std::path::Path::new(&checkpoint.plan_file).exists()
                            {
                                if let Ok(content) = std::fs::read_to_string(&checkpoint.plan_file)
                                {
                                    if let Some(steps_start) = content.find("## Passos\n\n") {
                                        if let Some(steps_end) =
                                            content[steps_start..].find("\n\n---")
                                        {
                                            let steps =
                                                &content[steps_start + 10..steps_start + steps_end];
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
                                let idea = checkpoint
                                    .plan_text
                                    .lines()
                                    .find(|l| l.starts_with("**Ideia:**"))
                                    .map(|l| l.replace("**Ideia:**", "").trim().to_string())
                                    .unwrap_or_default();
                                let skill_info = checkpoint
                                    .active_skill
                                    .as_ref()
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
                                    return Ok(format!(
                                        "📋 Plano em {}:\n\n{}",
                                        plan_file, content
                                    ));
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
                    if plan.project_dir.is_empty() {
                        "(nao definido)"
                    } else {
                        plan.project_dir.as_str()
                    },
                    if plan.plan_file.is_empty() {
                        "(nao definido)"
                    } else {
                        plan.plan_file.as_str()
                    }
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
                    if plan.project_dir.is_empty() {
                        "(nao definido)"
                    } else {
                        plan.project_dir.as_str()
                    },
                    if plan.plan_file.is_empty() {
                        "(nao definido)"
                    } else {
                        plan.plan_file.as_str()
                    },
                    if plan.plan_text.is_empty() {
                        "(sem plano)"
                    } else {
                        plan.plan_text.as_str()
                    }
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

        // Handle resume commands — resumes an Executing checkpoint
        // Also handles common typos like "cotinuar", "contiunar", etc.
        let lower = user_input.to_lowercase();
        let is_resume = lower.starts_with("contin")
            || lower.starts_with("cotin")
            || lower.starts_with("conti")
            || lower.starts_with("retom")
            || lower.starts_with("retome")
            || lower.starts_with("resum")
            || lower.starts_with("resume")
            || lower == "continue";

        // Extract directory from input if specified
        let mut target_dir = String::new();
        // Extract directory from various patterns
        if lower.contains("diretório:") || lower.contains("diretorio:") || lower.contains("dir:") {
            if let Some(dir) = user_input
                .split("diretório:")
                .nth(1)
                .or_else(|| user_input.split("diretorio:").nth(1))
                .or_else(|| user_input.split("dir:").nth(1))
            {
                target_dir = dir.trim().to_string();
            }
        } else if lower.contains("no diretório")
            || lower.contains("no diretorio")
            || lower.contains("no dir")
            || lower.contains("em ")
        {
            // Try to extract after "no diretório", "no diretorio", "em", etc.
            let query_lower = user_input.to_lowercase();
            if let Some(dir) = query_lower
                .split("no diretório")
                .nth(1)
                .or_else(|| query_lower.split("no diretorio").nth(1))
                .or_else(|| query_lower.split("em ").nth(1))
            {
                target_dir = dir.trim().to_string();
            }
        }

        // Clean up - remove trailing punctuation
        if !target_dir.is_empty() {
            target_dir = target_dir
                .trim_end_matches('.')
                .trim_end_matches(',')
                .trim_end_matches('!')
                .trim_end_matches('?')
                .to_string();
        }

        if is_resume {
            // Try to find any checkpoint to resume
            if let Ok(checkpoints) = self.checkpoint_store.get_active() {
                for active in checkpoints {
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

                        info!(
                            "Resuming checkpoint: {} in phase {:?}",
                            active.id, active.phase
                        );
                        let task_input = format!(
                            "Execute o plano de desenvolvimento no diretorio {}:\n\n{}",
                            active.project_dir, active.plan_text
                        );
                        return self.run_development(task_input, active).await;
                    }
                }
            }

            // If directory was specified and no checkpoint found, try to read PLANO.md
            if !target_dir.is_empty() {
                let plano_path = std::path::Path::new(&target_dir).join("PLANO.md");
                if plano_path.exists() {
                    if let Ok(plano_content) = std::fs::read_to_string(&plano_path) {
                        info!("Found PLANO.md in {}, resuming development", target_dir);
                        let task_input = format!(
                            "Execute o plano de desenvolvimento no diretorio {}.\n\nPLANO.md encontrado:\n{}",
                            target_dir,
                            plano_content
                        );

                        // Create a new checkpoint for this development
                        let mut checkpoint =
                            DevelopmentCheckpoint::new("resume from PLANO.md".to_string());
                        checkpoint.set_project_dir(target_dir.clone());
                        checkpoint.set_plan_text(plano_content);
                        checkpoint.set_phase(PlanPhase::Executing);

                        return self.run_development(task_input, checkpoint).await;
                    }
                }
            }

            // If directory specified but no PLANO.md found
            if !target_dir.is_empty() {
                let plano_path = std::path::Path::new(&target_dir).join("PLANO.md");
                if !plano_path.exists() {
                    return Ok(format!(
                        "Nenhum plano encontrado no diretório {}.\n\nPara criar um novo plano, use 'criar plano' ou crie um arquivo PLANO.md no diretório especificado.",
                        target_dir
                    ));
                }
            }

            // Check for any recent checkpoints with plans
            if let Ok(checkpoints) = self.checkpoint_store.get_recent_with_plans(1) {
                for active in checkpoints {
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

                        info!(
                            "Resuming from recent: {} in phase {:?}",
                            active.id, active.phase
                        );
                        let task_input = format!(
                            "Execute o plano de desenvolvimento no diretorio {}:\n\n{}",
                            active.project_dir, active.plan_text
                        );
                        return self.run_development(task_input, active).await;
                    }
                }
            }

            // No plan found - try to retry the last task (max 3 attempts)
            // Instead of recursive call, just warn and let user re-enter
            if let Ok(checkpoints) = self.checkpoint_store.get_active() {
                if let Some(mut last) = checkpoints.into_iter().max_by_key(|c| c.created_at) {
                    if !last.user_input.is_empty() && last.retry_count < 3 {
                        last.increment_retry();
                        info!("Retry attempt {}/3 for task", last.retry_count);
                        let _ = self.checkpoint_store.save(&last);
                        return Ok(format!(
                            "🔄 Tentativa {}/3 - Re-envie sua última solicitação para continuar: \"{}\"",
                            last.retry_count,
                            &last.user_input[..last.user_input.len().min(100)]
                        ));
                    } else if last.retry_count >= 3 {
                        return Ok("⚠️ Máximo de 3 tentativas excedido para esta tarefa. Crie um novo plano ou tente novamente mais tarde.".to_string());
                    }
                }
            }

            return Ok("Nenhum plano encontrado para continuar. Use 'criar plano' para iniciar um novo projeto.".to_string());
        }

        // Handle /desenvolver command - structured development mode
        let is_structured_dev = lower.starts_with("/desenvolver")
            || lower.starts_with("desenvolver ")
            || lower.starts_with("desenvolva ")
            || lower.starts_with("desenvolvedor ");

        if is_structured_dev {
            // Extract directory - try various patterns
            let mut dev_dir = String::new();

            // Pattern: /desenvolver /path
            if let Some(dir) = user_input.split("/desenvolver").nth(1) {
                let d = dir.trim();
                if !d.is_empty() {
                    dev_dir = d.to_string();
                }
            }

            // If not found, try other patterns
            if dev_dir.is_empty() {
                if let Some(dir) = lower.split("desenvolver ").nth(1) {
                    let d = dir.trim();
                    if !d.is_empty()
                        && (d.starts_with('/') || d.starts_with("em ") || d.starts_with("no "))
                    {
                        dev_dir = d
                            .trim_start_matches("em ")
                            .trim_start_matches("no ")
                            .trim_start_matches("no ")
                            .trim()
                            .to_string();
                    } else {
                        dev_dir = d.to_string();
                    }
                }
            }

            if dev_dir.is_empty() {
                return Ok("📂 Por favor, especifique o diretório do projeto:\n\nEx: /desenvolver /Users/macbook/projects/meu-projeto\n   ou\n   desenvolva o projeto no /Users/macbook/projects/meu-projeto".to_string());
            }

            // Clean up directory
            dev_dir = dev_dir
                .trim_end_matches('.')
                .trim_end_matches(',')
                .trim_end_matches('!')
                .trim_end_matches('?')
                .to_string();

            // Check if directory exists
            let path = std::path::Path::new(&dev_dir);
            if !path.exists() {
                if let Err(e) = std::fs::create_dir_all(path) {
                    return Ok(format!("❌ Erro ao criar diretório: {}", e));
                }
            }

            // Check for PLANO.md
            let plano_path = path.join("PLANO.md");
            if !plano_path.exists() {
                // Create default PLANO.md
                let default_plano = r#"# Plano de Desenvolvimento

## Etapa 1: Setup
- [ ] Configurar ambiente
- [ ] Instalar dependências

## Etapa 2: Desenvolvimento
- [ ] Implementar funcionalidades

## Etapa 3: Testes e Validação
- [ ] Criar testes
- [ ] Validar build

---
*Este plano foi criado automaticamente. Edite conforme necessário.*
"#;
                if let Err(e) = std::fs::write(&plano_path, default_plano) {
                    return Ok(format!("❌ Erro ao criar PLANO.md: {}", e));
                }

                return Ok(format!(
                    "✅ Diretório configurado: {}\n\n📄 Criei um PLANO.md básico no diretório.\n\nPor favor, edite o arquivo com as etapas do seu projeto e depois digite:\n\ncontinuar\n\npara iniciar o desenvolvimento estruturado.",
                    dev_dir
                ));
            }

            // Read PLANO.md and start structured development
            if let Ok(plano_content) = std::fs::read_to_string(&plano_path) {
                info!("Starting structured development in {}", dev_dir);

                // Create checkpoint for structured development
                let mut checkpoint =
                    DevelopmentCheckpoint::new("desenvolvimento estruturado".to_string());
                checkpoint.set_project_dir(dev_dir.clone());
                checkpoint.set_plan_text(plano_content.clone());
                checkpoint.set_phase(PlanPhase::Executing);

                // Save checkpoint
                self.checkpoint_store.save(&checkpoint)?;

                // Start structured development
                let task_input = format!(
                    "Desenvolva o projeto no diretório {} seguindo o PLANO.md:\n\n{}",
                    dev_dir, plano_content
                );

                return self
                    .run_structured_development(task_input, checkpoint)
                    .await;
            } else {
                return Ok(format!("❌ Erro ao ler PLANO.md em {}", dev_dir));
            }
        }

        // Handle clean memory commands
        let lower = user_input.to_lowercase();
        if lower.contains("limpar memória")
            || lower.contains("clean memory")
            || lower.contains("limpar memoria")
        {
            if lower.contains("confirm")
                || lower.contains("sim")
                || lower.contains("yes")
                || lower.contains("true")
            {
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
                checkpoint.project_dir, checkpoint.plan_text
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
        let skill_opt = self.skill_manager.process_message(user_input).cloned();
        let skill_name = skill_opt.as_ref().map(|s| s.name.clone());

        // 2. Recupera memórias
        let memories = self.retrieve_relevant_memories(user_input, None).await?;
        let memory_context = format_memories_for_prompt(&memories);

        // 3. Constrói system prompt com skill e defense instructions
        let mut system_prompt = self.build_system_prompt(&memory_context, skill_opt.as_ref());

        // SECURITY: Append defense prompt (minimal version for API compatibility)
        system_prompt.push_str(&SecurityManager::get_defense_prompt_minimal());

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
                return self
                    .execute_plan_steps(&mut checkpoint, &system_prompt, &steps)
                    .await;
            }
        }

        // 7. Build messages
        let mut current_messages = self
            .load_checkpoint_messages(&checkpoint)
            .unwrap_or_else(|| self.build_messages(&system_prompt));

        // 8. ReAct loop
        let start_iteration = checkpoint.current_iteration;
        for iteration in start_iteration..self.config.max_iterations {
            info!("ReAct iteration {}", iteration + 1);
            checkpoint.current_iteration = iteration;
            self.save_checkpoint(&mut checkpoint, &current_messages, &[])?;

            let response = self.call_llm(&current_messages).await?;
            debug!("LLM response:\n{}", response);

            let parsed = response_parser::ResponseParser::parse_response(&response)?;

            match parsed {
                ParsedResponse::FinalAnswer(answer) => {
                    info!("Final answer received");

                    // BUILD VALIDATION GATE: Para dev tasks com project_dir, valida build antes de aceitar
                    if DevelopmentCheckpoint::is_development_task(user_input)
                        && !checkpoint.project_dir.is_empty()
                    {
                        match self.validate_build(&checkpoint.project_dir).await? {
                            BuildValidation::Failed { errors } => {
                                let error_summary = errors
                                    .iter()
                                    .take(5) // Mostra até 5 erros
                                    .map(|e| {
                                        format!(
                                            "- {} ({}:{})",
                                            e.message,
                                            e.file,
                                            e.line.unwrap_or(0)
                                        )
                                    })
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

                    // Self-review loop - skip for plan execution completions
                    let final_answer = if checkpoint.is_plan_mode() {
                        // Skip self-review for plan mode - just use the answer as-is
                        answer.clone()
                    } else {
                        let (fa, _) = self.self_review(&answer, user_input).await?;
                        fa
                    };

                    self.save_conversation_to_memory(
                        user_input,
                        &final_answer,
                        Some(&checkpoint.id),
                    )
                    .await?;

                    self.finalize_checkpoint(
                        &mut checkpoint,
                        DevelopmentState::Completed,
                        &current_messages,
                        &[],
                    )?;

                    // Revisão final: listar arquivos criados no diretório
                    if !checkpoint.project_dir.is_empty() {
                        let project_path = std::path::Path::new(&checkpoint.project_dir);
                        if project_path.exists() {
                            let review_msg = format!(
                                "\n\n{}=== RESUMO DO DESENVOLVIMENTO ==={}\n\n📁 Diretório: {}\n\n📋 Arquivos/criados:\n",
                                Colors::AMBER, Colors::RESET, checkpoint.project_dir
                            );
                            // Usar echo para obter listagem
                            let _ls_result = self
                                .execute_tool(
                                    "shell",
                                    &format!("ls -la {}", checkpoint.project_dir),
                                )
                                .await?;
                            let final_with_review = format!(
                                "{}{}\n\n✅ Desenvolvimento concluído!",
                                final_answer, review_msg
                            );
                            return Ok(final_with_review);
                        }
                    }

                    // VALIDAÇÃO FINAL: Tentar build antes de finalizar
                    if !checkpoint.project_dir.is_empty() {
                        let project_path = std::path::Path::new(&checkpoint.project_dir);
                        if project_path.exists() {
                            let build_info = BuildDetector::detect(&checkpoint.project_dir);
                            if !build_info.build_command.is_empty() {
                                info!(
                                    "Executando validação final do build: {}",
                                    build_info.build_command
                                );

                                let build_result = self
                                    .execute_tool("shell", &build_info.build_command)
                                    .await?;

                                if build_result.to_lowercase().contains("error")
                                    || build_result.to_lowercase().contains("failed")
                                    || build_result.to_lowercase().contains("erro")
                                {
                                    // Build falhou - informar mas não bloquear (já que provavelmente vai continuar desenvolvendo)
                                    info!(
                                        "⚠️ Build final possui warnings/erros: {}",
                                        &build_result[..build_result.len().min(200)]
                                    );
                                } else {
                                    info!("✅ Build validado com sucesso!");
                                }
                            }
                        }
                    }

                    return Ok(final_answer);
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
                        self.save_tool_result_to_memory(
                            &action,
                            &action_input,
                            &observation,
                            Some(&checkpoint.id),
                        )
                        .await?;
                    }

                    // VERIFICATION: Verifica automaticamente o resultado da ação
                    let verification_result = self
                        .verify_action_result(&action, &action_input, &observation)
                        .await?;

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
        self.finalize_checkpoint(
            &mut checkpoint,
            DevelopmentState::Interrupted,
            &current_messages,
            &[],
        )?;
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

        if let ParsedResponse::FinalAnswer(answer) =
            ResponseParser::parse_response(&final_response)?
        {
            // Skip self-review for plan execution - just return answer
            let final_answer = answer.clone();
            self.save_conversation_to_memory(user_input, &final_answer, None)
                .await?;
            return Ok(final_answer);
        }

        Ok(final_response)
    }

    async fn run_development(
        &mut self,
        task_input: String,
        mut checkpoint: DevelopmentCheckpoint,
    ) -> anyhow::Result<String> {
        // Retrieve memories
        let memories = self.retrieve_relevant_memories(&task_input, None).await?;
        let memory_context = format_memories_for_prompt(&memories);

        // Build system prompt
        let mut system_prompt = self.build_system_prompt(&memory_context, None);
        system_prompt.push_str(&SecurityManager::get_defense_prompt_minimal());

        // Add user message
        self.conversation_history.push(json!({
            "role": "user",
            "content": &task_input
        }));

        // Check plan mode
        if checkpoint.is_plan_mode() {
            let steps = checkpoint.parse_plan_steps();
            if !steps.is_empty() {
                return self
                    .execute_plan_steps(&mut checkpoint, &system_prompt, &steps)
                    .await;
            }
        }

        // Build messages
        let mut current_messages = self
            .load_checkpoint_messages(&checkpoint)
            .unwrap_or_else(|| self.build_messages(&system_prompt));

        // ReAct loop
        let start_iteration = checkpoint.current_iteration;
        for iteration in start_iteration..self.config.max_iterations {
            info!("ReAct iteration {}", iteration + 1);
            checkpoint.current_iteration = iteration;

            if let Err(e) = self.maybe_summarize(&mut current_messages).await {
                tracing::warn!("Summarization check failed: {}", e);
            }

            self.save_checkpoint(&mut checkpoint, &current_messages, &[])?;

            let response = self.call_llm(&current_messages).await?;
            let parsed = response_parser::ResponseParser::parse_response(&response)?;

            match parsed {
                ParsedResponse::FinalAnswer(answer) => {
                    // BUILD VALIDATION GATE: Para dev tasks com project_dir, valida build antes de aceitar
                    if DevelopmentCheckpoint::is_development_task(&task_input)
                        && !checkpoint.project_dir.is_empty()
                    {
                        match self.validate_build(&checkpoint.project_dir).await? {
                            BuildValidation::Failed { errors } => {
                                let error_summary = errors
                                    .iter()
                                    .take(5)
                                    .map(|e| {
                                        format!(
                                            "- {} ({}:{})",
                                            e.message,
                                            e.file,
                                            e.line.unwrap_or(0)
                                        )
                                    })
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

                    // Skip self-review for plan execution - just return answer
                    let final_answer = answer.clone();

                    self.save_conversation_to_memory(
                        &task_input,
                        &final_answer,
                        Some(&checkpoint.id),
                    )
                    .await?;
                    self.finalize_checkpoint(
                        &mut checkpoint,
                        DevelopmentState::Completed,
                        &current_messages,
                        &[],
                    )?;

                    return Ok(final_answer);
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
                        self.save_tool_result_to_memory(
                            &action,
                            &action_input,
                            &observation,
                            Some(&checkpoint.id),
                        )
                        .await?;
                    }

                    // VERIFICATION: Verifica automaticamente o resultado da ação
                    let verification_result = self
                        .verify_action_result(&action, &action_input, &observation)
                        .await?;

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
                                    errors
                                        .iter()
                                        .enumerate()
                                        .map(|(i, e)| format!("{}. {}", i + 1, e))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                );

                                checkpoint.set_last_error(error_msg.clone());
                                let max_retries = self.config.agent_loop.max_retries_per_step;
                                info!(
                                    "❌ Build failed with {} errors (retry {}/{})",
                                    errors.len(),
                                    checkpoint.retry_count,
                                    max_retries
                                );

                                if checkpoint.retry_count < max_retries {
                                    // Adiciona feedback de erro ao LLM para corrigir
                                    current_messages.push(json!({
                                        "role": "system",
                                        "content": format!(
                                            "{}\n\n🔧 Por favor, corrija estes erros e execute as ações necessárias. Tentativa {}/{}", 
                                            error_msg, checkpoint.retry_count, max_retries
                                        )
                                    }));
                                } else {
                                    // Máximo de retries atingido
                                    let failure_msg = if self.config.agent_loop.exit_on_error
                                        == crate::config::ExitBehavior::Never
                                    {
                                        // Never exit - just warn and continue
                                        format!(
                                            "⚠️ Máximo de {} tentativas atingido para este passo, mas continuando por configuração.\n\nÚltimo erro:\n{}",
                                            max_retries, error_msg
                                        )
                                    } else {
                                        format!(
                                            "❌ Máximo de {} tentativas atingido. Último erro:\n\n{}",
                                            max_retries, error_msg
                                        )
                                    };

                                    if self.config.agent_loop.exit_on_error
                                        == crate::config::ExitBehavior::Task
                                    {
                                        self.finalize_checkpoint(
                                            &mut checkpoint,
                                            DevelopmentState::Failed,
                                            &current_messages,
                                            &[],
                                        )?;
                                    }

                                    return Ok(failure_msg);
                                }
                            }
                        }
                    }
                }
            }
        }

        info!("Max iterations reached");
        self.finalize_checkpoint(
            &mut checkpoint,
            DevelopmentState::Interrupted,
            &current_messages,
            &[],
        )?;
        Ok(format!(
            "Execução interrompida após {} iterações.",
            self.config.max_iterations
        ))
    }

    /// Structured development mode - parses PLANO.md and executes step by step
    async fn run_structured_development(
        &mut self,
        task_input: String,
        mut checkpoint: DevelopmentCheckpoint,
    ) -> anyhow::Result<String> {
        let project_dir = checkpoint.project_dir.clone();

        // Parse PLANO.md into structured stages
        let stages = self.parse_plano_md(&checkpoint.plan_text)?;

        if stages.is_empty() {
            return Ok("❌ Nenhuma etapa encontrada no PLANO.md. Por favor, defina as etapas usando headers ##.".to_string());
        }

        info!(
            "Starting structured development with {} stages",
            stages.len()
        );

        // Build system prompt for structured development
        let memories = self.retrieve_relevant_memories(&task_input, None).await?;
        let memory_context = format_memories_for_prompt(&memories);
        let mut system_prompt = self.build_system_prompt(&memory_context, None);
        system_prompt.push_str(&SecurityManager::get_defense_prompt_minimal());

        // Add structured development instructions
        system_prompt.push_str(
            r#"
        
MODO DESENVOLVIMENTO ESTRUTURADO:
- Execute as etapas uma por vez
- Para cada etapa, faça as ações necessárias
- Use ferramentas (file_write, shell, etc.) para criar arquivos
- Ao final da etapa, diga "Etapa X Concluída" para passar para a próxima
- Não mostre código completo, apenas confirme quando concluir
"#,
        );

        let mut current_messages = vec![
            json!({"role": "system", "content": system_prompt}),
            json!({"role": "user", "content": &task_input}),
        ];

        // Execute each stage
        let total = stages.len();
        for (stage_idx, stage) in stages.iter().enumerate() {
            let stage_num = stage_idx + 1;
            info!("Executing stage {}/{}: {}", stage_num, total, stage.name);

            // Stage prompt
            let stage_prompt = format!(
                "## ETAPA {}/{}: {}\n\n{}\n\n**DIRETÓRIO:** {}\n\nExecute as ações necessárias para completar esta etapa. Quando terminar, responda: \"Etapa {} Concluída\"",
                stage_num,
                total,
                stage.name,
                stage.description,
                project_dir,
                stage_num
            );

            current_messages.push(json!({
                "role": "user",
                "content": stage_prompt
            }));

            // ReAct loop for this stage (max 10 iterations per stage)
            let mut stage_complete = false;
            let mut stage_attempts = 0;

            for iteration in 0..10 {
                stage_attempts += 1;
                info!("Stage {} iteration {}", stage_num, iteration + 1);

                let response = self.call_llm(&current_messages).await?;
                let parsed = response_parser::ResponseParser::parse_response(&response)?;

                match parsed {
                    ParsedResponse::FinalAnswer(answer) => {
                        let lower = answer.to_lowercase();
                        if lower.contains(&format!("etapa {} concluída", stage_num).to_lowercase())
                            || lower
                                .contains(&format!("etapa {} concluida", stage_num).to_lowercase())
                        {
                            stage_complete = true;

                            // Validate stage
                            let build_info = BuildDetector::detect(&project_dir);
                            if !build_info.build_command.is_empty() {
                                info!("Validating build for stage {}...", stage_num);
                                let build_result = self
                                    .execute_tool("shell", &build_info.build_command)
                                    .await?;

                                if build_result.to_lowercase().contains("error")
                                    || build_result.to_lowercase().contains("failed")
                                {
                                    info!("Build has errors, reporting to model...");
                                    current_messages.push(json!({
                                        "role": "user",
                                        "content": format!("⚠️ Build tem erros:\n{}\n\nPor favor, corrija os erros antes de finalizar a etapa.", build_result)
                                    }));
                                    stage_complete = false;
                                    continue;
                                }
                            }

                            info!("Stage {} completed!", stage_num);
                            break;
                        }

                        // If answer doesn't contain "stage complete", continue
                        current_messages.push(json!({
                            "role": "user",
                            "content": "Por favor, finalize a etapa dizendo \"Etapa X Concluída\" quando terminar."
                        }));
                    }
                    ParsedResponse::Action {
                        action,
                        action_input,
                        ..
                    } => {
                        info!("Executing tool: {}", action);

                        let raw_observation = self.execute_tool(&action, &action_input).await?;
                        let observation =
                            SecurityManager::clean_tool_output(&raw_observation, &action);

                        current_messages.push(json!({
                            "role": "assistant",
                            "content": format!("Thought: Executando {}\nAction: {}\nAction Input: {}\n", action, action_input, observation)
                        }));
                    }
                }
            }

            if !stage_complete && stage_attempts >= 10 {
                info!(
                    "Stage {} reached max iterations without completing",
                    stage_num
                );
                return Ok(format!(
                    "⚠️ Etapa {} não pôde ser completada após 10 tentativas.\n\nEtapa: {}\n\nDeseja que eu:\n1. Continue tentando\n2. Pular para próxima etapa\n3. Parar aqui",
                    stage_num,
                    stage.name
                ));
            }

            // Save progress
            checkpoint.set_current_step(stage_idx);
            self.checkpoint_store.save(&checkpoint)?;
        }

        // All stages complete - final validation
        info!("All stages complete, doing final validation...");

        let build_info = BuildDetector::detect(&project_dir);
        if !build_info.build_command.is_empty() {
            let final_build = self
                .execute_tool("shell", &build_info.build_command)
                .await?;

            let build_status = if final_build.to_lowercase().contains("error")
                || final_build.to_lowercase().contains("failed")
            {
                "⚠️ Build final tem warnings/erros"
            } else {
                "✅ Build final passou!"
            };

            self.finalize_checkpoint(
                &mut checkpoint,
                DevelopmentState::Completed,
                &current_messages,
                &[],
            )?;

            // List created files
            let files_list = self
                .execute_tool("shell", &format!("ls -la {}", project_dir))
                .await?;

            return Ok(format!(
                "🎉 Desenvolvimento Concluído!\n\n{}\n\n📁 Diretório: {}\n\n📋 Arquivos criados:\n{}\n\n✅ Desenvolvimento estruturado finalizado com sucesso!",
                build_status,
                project_dir,
                files_list.lines().take(20).collect::<Vec<_>>().join("\n")
            ));
        }

        self.finalize_checkpoint(
            &mut checkpoint,
            DevelopmentState::Completed,
            &current_messages,
            &[],
        )?;

        Ok(format!(
            "🎉 Desenvolvimento Concluído!\n\n📁 Diretório: {}\n\n✅ Todas as {} etapas foram executadas.",
            project_dir,
            total
        ))
    }

    /// Parse PLANO.md into structured stages
    fn parse_plano_md(&self, content: &str) -> anyhow::Result<Vec<PlanStage>> {
        let mut stages = Vec::new();
        let mut current_stage_idx: Option<usize> = None;

        for line in content.lines() {
            let trimmed = line.trim();

            // Match ## Etapa N: Name or ## Etapa N
            // Also match ### Fase 1: Name or ### 🔴 Fase 1: Name
            if trimmed.starts_with("##") || trimmed.starts_with("###") {
                let header_content = trimmed.trim_start_matches('#').trim();
                let lower = header_content.to_lowercase();

                // Check if it's a stage/phase header
                if lower.contains("etapa")
                    || lower.contains("fase")
                    || lower.contains("stage")
                    || lower.contains("phase")
                {
                    // Clean up emojis and extra chars
                    let clean_name = header_content
                        .replace("🔴", "")
                        .replace("🟠", "")
                        .replace("🟡", "")
                        .replace("🟢", "")
                        .replace("🔵", "")
                        .trim()
                        .to_string();

                    stages.push(PlanStage {
                        id: stages.len() + 1,
                        name: clean_name.clone(),
                        description: String::new(),
                        validation: None,
                    });
                    current_stage_idx = Some(stages.len() - 1);
                }
            } else if !trimmed.is_empty()
                && !trimmed.starts_with('-')
                && !trimmed.starts_with('*')
                && !trimmed.starts_with('#')
                && !trimmed.starts_with("---")
            {
                // Add description to current stage
                if let Some(idx) = current_stage_idx {
                    if stages[idx].description.is_empty() {
                        stages[idx].description = trimmed.to_string();
                    } else {
                        stages[idx].description.push('\n');
                        stages[idx].description.push_str(trimmed);
                    }
                }
            } else if trimmed.starts_with("- [") || trimmed.starts_with("* [") {
                // Add checklist items to current stage
                if let Some(idx) = current_stage_idx {
                    if stages[idx].description.is_empty() {
                        stages[idx].description = trimmed.to_string();
                    } else {
                        stages[idx].description.push('\n');
                        stages[idx].description.push_str(trimmed);
                    }
                }
            }
        }

        Ok(stages)
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

            let plan_system_msg = "MODO PLANO DE DESENVOLVIMENTO:\nVocê está executando um plano de desenvolvimento estruturado. Foque APENAS na etapa atual. Use ferramentas necessárias (file_write, shell, file_read, etc.). Quando a etapa estiver completa, responda no formato:\nStep Complete: [resumo]\n\n**REGRA CRÍTICA**: O diretório do projeto é: {project_dir}\n- Use ONLY arquivos neste diretório\n- NUNCA crie arquivos fora deste diretório\n- **SEMPRE leia o arquivo {project_dir}/PLANO.md antes de começar**\n- Se precisar criar um arquivo, use o caminho completo (ex: {project_dir}/src/main.rs)\n\nIMPORTANTE: Quando todas as etapas estiverem concluídas e os arquivos forem criados, NÃO mostre o código completo na resposta. Apenas confirme que o trabalho foi realizado com uma mensagem de conclusão."
                .replace("{project_dir}", &project_dir);

            let mut step_messages = vec![
                json!({"role": "system", "content": system_prompt}),
                json!({"role": "system", "content": plan_system_msg}),
                json!({"role": "user", "content": step_prompt}),
            ];

            // ReAct loop for each step (max 5 iterations per step)
            let mut step_complete = false;
            for _iteration in 0..5 {
                let response = self.call_llm(&step_messages).await?;
                let parsed = response_parser::ResponseParser::parse_response(&response)?;

                match parsed {
                    ParsedResponse::FinalAnswer(answer) => {
                        if answer.to_lowercase().contains("step complete:") {
                            // BUILD VALIDATION: Valida build antes de marcar step como completo
                            if !checkpoint.project_dir.is_empty() {
                                match self.validate_build(&checkpoint.project_dir).await? {
                                    BuildValidation::Failed { errors } => {
                                        let error_summary = errors
                                            .iter()
                                            .take(3)
                                            .map(|e| {
                                                format!(
                                                    "- {} ({}:{})",
                                                    e.message,
                                                    e.file,
                                                    e.line.unwrap_or(0)
                                                )
                                            })
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
                                        info!(
                                            "✅ Build validation passed for step {}/{}",
                                            step_num, total
                                        );
                                    }
                                }
                            }

                            checkpoint.mark_step_done(step_idx);
                            self.checkpoint_store.save(checkpoint)?;
                            self.update_plan_progress(
                                &plan_file,
                                steps,
                                &checkpoint.completed_steps,
                            )?;
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
                        let observation =
                            SecurityManager::clean_tool_output(&raw_observation, &action);

                        // Save to memory
                        if action != "echo" {
                            self.save_tool_result_to_memory(
                                &action,
                                &action_input,
                                &observation,
                                Some(&checkpoint.id),
                            )
                            .await?;
                        }

                        // VERIFICATION: Verifica automaticamente o resultado da ação
                        let verification_result = self
                            .verify_action_result(&action, &action_input, &observation)
                            .await?;

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
                info!(
                    "Step {}/{} max iterations reached, marking as done anyway",
                    step_num, total
                );
                checkpoint.mark_step_done(step_idx);
                self.checkpoint_store.save(checkpoint)?;
                self.update_plan_progress(&plan_file, steps, &checkpoint.completed_steps)?;
            }
        }

        checkpoint.set_phase(PlanPhase::Completed);
        checkpoint.set_state(DevelopmentState::Completed);
        self.checkpoint_store.save(checkpoint)?;

        let summary = format!(
            "✅ Plano de desenvolvimento concluído!\n\n{} etapas executadas no diretório: {}",
            total, project_dir
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
        let step_re = RE_PLAN_STEP
            .get_or_init(|| Regex::new(r"(?m)^(\s*\d+)\.\s*(\[[ xX]\])\s+(.*)$").unwrap());

        let updated = step_re
            .replace_all(&content, |caps: &regex::Captures| {
                let number = &caps[1];
                let step_text = &caps[3];
                let step_idx: usize = number
                    .trim()
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_sub(1);

                if completed.contains(&step_idx) {
                    format!("{}. [x] {}", number, step_text)
                } else {
                    format!("{}. [ ] {}", number, step_text)
                }
            })
            .to_string();

        std::fs::write(plan_file, updated)?;

        Ok(())
    }

    async fn retrieve_relevant_memories(
        &self,
        query: &str,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<(MemoryEntry, f32)>> {
        let query_embedding = self.embedding_service.embed(query).await?;

        let mut all_memories = self.memory_store.get_all()?;

        let cross_session = self.memory_store.get_cross_session_memories(session_id)?;
        for memory in cross_session {
            if !all_memories.iter().any(|m| m.id == memory.id) {
                all_memories.push(memory);
            }
        }

        if all_memories.is_empty() {
            return Ok(vec![]);
        }

        let results = search_similar_memories(&query_embedding, &all_memories, 5, 0.3);

        for (memory, _) in &results {
            if let Err(e) = self.memory_store.touch_memory(&memory.id) {
                tracing::warn!("Failed to touch memory: {}", e);
            }
        }

        Ok(results)
    }

    async fn load_or_create_checkpoint(
        &self,
        user_input: &str,
    ) -> anyhow::Result<DevelopmentCheckpoint> {
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

        let messages = vec![json!({
            "role": "user",
            "content": plan_prompt
        })];

        let response = self.call_llm(&messages).await?;
        Ok(response.trim().to_string())
    }

    /// Valida o build do projeto no diretório especificado
    async fn validate_build(&mut self, project_dir: &str) -> anyhow::Result<BuildValidation> {
        // Detecta tipo de projeto e comando de build
        let build_info = BuildDetector::detect(project_dir);

        if build_info.build_command.is_empty() {
            info!(
                "No build command detected for {}, skipping validation",
                project_dir
            );
            return Ok(BuildValidation::Success);
        }

        info!(
            "Running build command: {} in {}",
            build_info.build_command, project_dir
        );

        // Executa o comando de build via shell tool
        let build_result = self
            .execute_tool("shell", &build_info.build_command)
            .await?;

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

        let mut all_tools: Vec<ToolExecution> =
            serde_json::from_str(&checkpoint.completed_tools_json).unwrap_or_default();
        all_tools.extend_from_slice(tool_execs);

        checkpoint.messages_json = serde_json::to_string(messages)?;
        checkpoint.completed_tools_json = serde_json::to_string(&all_tools)?;
        checkpoint.updated_at = chrono::Utc::now();

        self.checkpoint_store.save(checkpoint)?;

        // Update session summary with message count
        if let Err(e) = self
            .checkpoint_store
            .update_session_message(&checkpoint.id, &checkpoint.user_input)
        {
            tracing::warn!("Failed to update session: {}", e);
        }

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

        // Log para review
        if state == DevelopmentState::Completed {
            info!("=== REVISÃO DO DESENVOLVIMENTO ===");
            info!("Diretório: {}", checkpoint.project_dir);
            info!(
                "Plano: {}",
                checkpoint
                    .plan_text
                    .lines()
                    .take(5)
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            info!("Passos completados: {:?}", checkpoint.completed_steps);
            info!("===================================");
        }

        Ok(())
    }

    #[allow(dead_code)]
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
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        if user_input.len() < 10 {
            return Ok(());
        }

        let content = format!(
            "Usuário: {}\nAssistente: {}",
            user_input, assistant_response
        );

        let embedding = self.embedding_service.embed(&content).await?;

        let memory = if let Some(sid) = session_id {
            MemoryEntry::new(content, embedding, MemoryType::Episode, 0.6)
                .with_session(sid.to_string())
        } else {
            MemoryEntry::new(content, embedding, MemoryType::Episode, 0.6)
        };

        self.memory_store.save(&memory)?;
        info!("Saved conversation to long-term memory");

        Ok(())
    }

    async fn save_tool_result_to_memory(
        &self,
        tool_name: &str,
        input: &str,
        output: &str,
        session_id: Option<&str>,
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

        let memory = if let Some(sid) = session_id {
            MemoryEntry::new(content, embedding, MemoryType::ToolResult, 0.5)
                .with_session(sid.to_string())
        } else {
            MemoryEntry::new(content, embedding, MemoryType::ToolResult, 0.5)
        };

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

    const SUMMARIZE_THRESHOLD: f64 = 0.80;

    async fn maybe_summarize(&mut self, messages: &mut Vec<Value>) -> anyhow::Result<()> {
        let threshold = Self::SUMMARIZE_THRESHOLD;
        if !self.summarizer.should_summarize(messages, threshold) {
            return Ok(());
        }

        tracing::info!("Context usage exceeded {}%, triggering summarization", threshold * 100.0);

        let result = self
            .summarizer
            .summarize_with_llm(
                &self.client,
                &self.config.api_key,
                messages,
                &self.config.model,
                &self.config.base_url,
                &self.config.provider,
                500,
            )
            .await;

        match result {
            Ok(summarization) => {
                tracing::info!(
                    "Summarization complete: {} tokens -> {} tokens (removed {} messages)",
                    summarization.original_token_count,
                    summarization.summary_token_count,
                    summarization.messages_removed
                );

                let compressed = self.summarizer.compress_messages(messages, &summarization.summary);
                *messages = compressed;
                self.compression_count += 1;

                tracing::info!("Compression count: {}", self.compression_count);
            }
            Err(e) => {
                tracing::warn!("Summarization failed: {}, continuing without compression", e);
            }
        }

        Ok(())
    }

    async fn call_llm(&mut self, messages: &[Value]) -> anyhow::Result<String> {
        let prompt_tokens = self.summarizer.token_counter().count_messages_tokens(messages);
        let model = &self.config.model;

        let result = self
            .call_llm_with_config(
                messages,
                model,
                &self.config.base_url,
                &self.config.provider,
            )
            .await;

        match &result {
            Ok(response) => {
                let completion_tokens = self.summarizer.token_counter().count_tokens(response);
                self.cost_tracker.record_call(prompt_tokens, completion_tokens, model);
                self.cost_tracker.record_iteration();
            }
            Err(_) => {
                self.cost_tracker.record_iteration();
            }
        }

        if result.is_err() && !self.config.fallback_models.is_empty() {
            tracing::warn!("Primary model failed, trying fallbacks...");

            for fallback in &self.config.fallback_models {
                tracing::info!("Trying fallback model: {}", fallback.model);
                let prompt_tokens = self.summarizer.token_counter().count_messages_tokens(messages);
                match self
                    .call_llm_with_config(
                        messages,
                        &fallback.model,
                        &fallback.base_url,
                        "opencode-go",
                    )
                    .await
                {
                    Ok(response) => {
                        tracing::info!("Fallback model {} succeeded", fallback.model);
                        let completion_tokens = self.summarizer.token_counter().count_tokens(&response);
                        self.cost_tracker.record_call(prompt_tokens, completion_tokens, &fallback.model);
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

    /// Self-review loop: evaluates and refines the response
    async fn self_review(
        &self,
        draft_answer: &str,
        user_input: &str,
    ) -> anyhow::Result<(String, Vec<String>)> {
        let mut current_answer = draft_answer.to_string();
        let mut review_history = Vec::new();

        if !self.config.self_review.enabled {
            return Ok((current_answer, review_history));
        }

        let max_loops = self.config.self_review.max_loops;
        let show_process = self.config.self_review.show_process;

        let review_re =
            RE_REVIEW.get_or_init(|| Regex::new(r"(?i)REVIEW:\s*(ADEQUATE|INADEQUATE)").unwrap());
        let suggestion_re =
            RE_SUGGESTION.get_or_init(|| Regex::new(r"(?i)SUGGESTION:\s*(.+)").unwrap());

        for iteration in 1..=max_loops {
            let review_prompt = format!(
                r#"Você é um revisor crítico. Analise a resposta abaixo e determine se ela atende completamente ao pedido do usuário.

PEDIDO DO USUÁRIO:
{}

RESPOSTA A REVISAR:
{}

Analise criticamente:
1. A resposta está COMPLETA? (não faltou nada?)
2. A resposta ATENDE ao que foi pedido? (resolve o problema?)
3. O RACIOCÍNIO está correto? (lógica sem erros?)
4. **PARA TAREFAS DE DESENVOLVIMENTO**: Se o trabalho foi realizado (arquivos criados, código implementado), a resposta pode mostrar:
   - Estrutura de diretórios (ls ou tree)
   - Nomes dos arquivos criados
   - Uma mensagem breve de conclusão
   Isso é VÁLIDO e ACEITÁVEL.

**REGRA IMPORTANTE**: 
- Respostas que mostram código completo são INADEQUADAS (muito longo)
- Respostas breves confirmando conclusão OU com listagem de arquivos são ADEQUADAS
- Não exija "provas" do código em si - a ausência de código não é um problema

Responda EXATAMENTE neste formato:
REVIEW: ADEQUATE ou INADEQUATE
ANALYSIS: [explicação breve]
SUGGESTION: [se inadequada, sugira]

Seja justo. Aceite respostas de conclusão em qualquer formato."#,
                user_input, current_answer
            );

            let review_messages = vec![
                json!({
                    "role": "system",
                    "content": "Você é um revisor crítico mas JUSTO de respostas de IA.\n\nDIRETRIZES:\n- Para tarefas de desenvolvimento: quando arquivos forem criados, a resposta pode ser:\n  * Uma mensagem breve de conclusão (\"Desenvolvimento concluído!\")\n  * Uma listagem de arquivos criados (ls, tree)\n  * Ambos\n- Mostrar código completo = INADEQUADO (muito longo)\n- Mostrar apenas confirmação = ADEQUADO\n- Mostrar estrutura de diretórios = ADEQUADO\n- Não mostrar código não é um problema!\n\nSEJA JUSTO: Aceite qualquer uma das formas acima como resposta válida."
                }),
                json!({
                    "role": "user",
                    "content": review_prompt
                }),
            ];

            let review_response = self
                .call_llm_with_config(
                    &review_messages,
                    &self.config.model,
                    &self.config.base_url,
                    &self.config.provider,
                )
                .await?;

            let analysis =
                response_parser::ResponseParser::sanitize_model_response(&review_response);
            review_history.push(analysis.clone());

            if show_process {
                println!();
                println!(
                    "{}🤔 [Auto-Revisão {}/{}]{}",
                    Colors::AMBER,
                    iteration,
                    max_loops,
                    Colors::RESET
                );
                println!("  {}", Colors::LIGHT_GRAY);
                for line in analysis.lines().take(5) {
                    println!("    {}", line);
                }
                println!("{}", Colors::RESET);
            }

            // Parse the review response
            let is_adequate = review_re
                .captures(&analysis)
                .map(|c| c.get(1).map(|m| m.as_str() == "ADEQUATE").unwrap_or(false))
                .unwrap_or(true);

            if is_adequate {
                if show_process {
                    println!(
                        "{}  ✅ Revisão aprovada{} - continuando",
                        Colors::AMBER,
                        Colors::RESET
                    );
                }
                break;
            }

            // Get suggestion for improvement
            let suggestion = suggestion_re
                .captures(&analysis)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| "Revise a resposta".to_string());

            if show_process {
                println!(
                    "{}  ⚠️ Revisão encontrou problemas{} - refinando",
                    Colors::AMBER,
                    Colors::RESET
                );
                println!("  Sugestão: {}", Colors::LIGHT_GRAY);
                println!("    {}", suggestion);
            }

            // Generate improved answer
            let improve_prompt = format!(
                r#"Com base na seguinte análise, gere uma resposta melhorada:

PEDIDO ORIGINAL:
{}

RESPOSTA ANTERIOR:
{}

ANÁLISE DO PROBLEMA:
{}

Por favor, forneça a RESPOSTA MELHORADA que corrige os problemas identificados."#,
                user_input, current_answer, suggestion
            );

            let improve_messages = vec![
                json!({
                    "role": "system",
                    "content": "Você é um assistente que melhora respostas com base em feedback."
                }),
                json!({
                    "role": "user",
                    "content": improve_prompt
                }),
            ];

            let improved_response = self
                .call_llm_with_config(
                    &improve_messages,
                    &self.config.model,
                    &self.config.base_url,
                    &self.config.provider,
                )
                .await?;

            current_answer =
                response_parser::ResponseParser::sanitize_model_response(&improved_response);
        }

        if show_process {
            println!();
        }

        Ok((current_answer, review_history))
    }

    async fn call_llm_with_config(
        &self,
        messages: &[Value],
        model: &str,
        base_url: &str,
        provider: &str,
    ) -> anyhow::Result<String> {
        // Debug: log API key info
        let api_key_preview = if self.config.api_key.len() > 4 {
            format!(
                "{}...{}",
                &self.config.api_key[..4],
                &self.config.api_key[self.config.api_key.len() - 4..]
            )
        } else {
            "too_short".to_string()
        };
        tracing::debug!(
            "API Key preview: {}, URL: {}, Model: {}",
            api_key_preview,
            base_url,
            model
        );

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

        tracing::debug!("Sending request to URL: {}", url);
        tracing::debug!("Request body: {:?}", body);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("X-API-Key", &self.config.api_key)
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

        // OpenCode Go API returns content as an array with type "text" or "thinking"
        let content = if let Some(content_arr) = json_response["content"].as_array() {
            // Find the text content (not thinking)
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
                // If no text found, try the first item
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

        let cleaned = response_parser::ResponseParser::sanitize_model_response(&content)
            .trim()
            .to_string();

        Ok(cleaned)
    }

    async fn execute_tool(&mut self, action: &str, action_input: &str) -> anyhow::Result<String> {
        if let Some(trust) = &self.workspace_trust {
            let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            match action {
                "file_write" | "file_edit" => {
                    if let Ok(args) =
                        response_parser::ResponseParser::parse_action_input_json(action_input)
                    {
                        if let Some(path_str) = args["path"].as_str() {
                            let path = Path::new(path_str);
                            let decision =
                                trust.evaluate(path, &crate::workspace_trust::Operation::WriteFile);
                            if !decision.allowed {
                                return Ok(format!(
                                    "Acesso negado: operação '{}' não permitida em '{}' (trust: {:?}). {}",
                                    action,
                                    path_str,
                                    decision.trust_level,
                                    decision.reason.unwrap_or_default()
                                ));
                            }
                        }
                    }
                }
                "shell" => {
                    if !trust.can_execute_shell(&current_dir) {
                        return Ok(format!(
                            "Acesso negado: execução de comandos shell não permitida neste diretório (trust: {:?})",
                            trust.evaluate(&current_dir, &crate::workspace_trust::Operation::ExecuteShell).trust_level
                        ));
                    }
                }
                "http_get" | "http_post" => {
                    if !trust.can_access_network(&current_dir) {
                        return Ok(format!(
                            "Acesso negado: operações de rede não permitida neste diretório (trust: {:?})",
                            trust.evaluate(&current_dir, &crate::workspace_trust::Operation::NetworkRequest).trust_level
                        ));
                    }
                }
                _ => {}
            }
        }

        let tool = self
            .tools
            .get(action)
            .ok_or_else(|| anyhow::anyhow!("Ferramenta '{}' não encontrada", action))?;

        let args = match response_parser::ResponseParser::parse_action_input_json(action_input) {
            Ok(value) => value,
            Err(e) => {
                let err_msg = format!("Erro: {}", e);
                output_write_error(&err_msg);
                return Ok(
                    "Erro: Action Input inválido. Reenvie apenas o JSON válido para a ferramenta."
                        .to_string(),
                );
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
                // Verifica se arquivo foi criado (apenas verifica existência)
                if let Ok(args) =
                    response_parser::ResponseParser::parse_action_input_json(action_input)
                {
                    if let Some(path) = args["path"].as_str() {
                        if Path::new(path).exists() {
                            Ok(None) // Arquivo existe
                        } else {
                            Ok(Some(format!(
                                "Arquivo '{}' não foi criado após file_write",
                                path
                            )))
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
                let error_indicators = [
                    "❌ Erro",
                    "error:",
                    "ERROR:",
                    "FAILED",
                    "failed",
                    "panicked",
                    "exception",
                    "Exception",
                ];
                let has_error = error_indicators.iter().any(|ind| observation.contains(ind));

                if has_error {
                    Ok(Some(
                        "Comando shell falhou. Output contém indicadores de erro".to_string(),
                    ))
                } else {
                    Ok(None)
                }
            }
            "http_get" | "http_post" => {
                // Verifica status code (procura por padrões tipo "status: 4xx" ou "status: 5xx")
                if observation.contains("status: 4") || observation.contains("status: 5") {
                    Ok(Some(
                        "Requisição HTTP falhou com status code de erro".to_string(),
                    ))
                } else if observation.contains("❌ Erro") {
                    Ok(Some("Requisição HTTP retornou erro".to_string()))
                } else {
                    Ok(None)
                }
            }
            "file_read" => {
                // Se não retornou erro, passou
                if observation.starts_with("❌ Erro") {
                    Ok(Some("Falha ao ler arquivo".to_string()))
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
        self.memory_store
            .clear_all()
            .map_err(|e| format!("Erro ao limpar memória: {}", e))?;

        // Clear checkpoints (get all recent ones)
        if let Ok(checkpoints) = self.checkpoint_store.get_recent_with_plans(1000) {
            for cp in checkpoints {
                let _ = self.checkpoint_store.delete(&cp.id);
            }
        }

        Ok(
            "Todas as memórias foram limpas (conversas, planos, checkpoints, skills ativas)"
                .to_string(),
        )
    }

    #[allow(dead_code)]
    pub fn get_skill_manager(&self) -> &SkillManager {
        &self.skill_manager
    }

    pub fn list_skills(&self) -> Vec<String> {
        self.skill_manager.list_available_skills()
    }

    /// List all sessions (returns checkpoints for backward compat)
    #[allow(dead_code)]
    pub fn list_sessions(&self) -> anyhow::Result<Vec<DevelopmentCheckpoint>> {
        self.checkpoint_store
            .list_all(50)
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// List all session summaries (fast listing from session_summaries table)
    #[allow(dead_code)]
    pub fn list_session_summaries(
        &self,
    ) -> anyhow::Result<Vec<crate::memory::checkpoint::SessionSummary>> {
        self.checkpoint_store
            .list_session_summaries(50)
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// List session summaries with hierarchy depth for tree display
    pub fn list_sessions_with_hierarchy(
        &self,
    ) -> anyhow::Result<Vec<(crate::memory::checkpoint::SessionSummary, usize)>> {
        let sessions = self
            .checkpoint_store
            .list_session_summaries(100)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let mut result = Vec::new();
        let mut session_map: std::collections::HashMap<
            String,
            &crate::memory::checkpoint::SessionSummary,
        > = std::collections::HashMap::new();

        for session in &sessions {
            session_map.insert(session.session_id.clone(), session);
        }

        fn get_depth(
            session: &crate::memory::checkpoint::SessionSummary,
            session_map: &std::collections::HashMap<
                String,
                &crate::memory::checkpoint::SessionSummary,
            >,
        ) -> usize {
            let mut depth = 0;
            let mut current = session;
            while let Some(ref parent_id) = current.parent_id {
                if let Some(parent) = session_map.get(parent_id) {
                    depth += 1;
                    current = parent;
                } else {
                    break;
                }
            }
            depth
        }

        for session in &sessions {
            let depth = get_depth(session, &session_map);
            result.push((session.clone(), depth));
        }

        result.sort_by(|a, b| {
            let depth_cmp = a.1.cmp(&b.1);
            if depth_cmp == std::cmp::Ordering::Equal {
                b.0.updated_at.cmp(&a.0.updated_at)
            } else {
                depth_cmp
            }
        });

        Ok(result)
    }

    /// Get session details by ID (full ID or prefix)
    #[allow(dead_code)]
    pub fn get_session_details(&self, session_id: &str) -> anyhow::Result<Option<SessionDetails>> {
        // Try exact match first
        if let Ok(Some(cp)) = self.checkpoint_store.get(session_id) {
            let message_count = serde_json::from_str::<Vec<Value>>(&cp.messages_json)
                .map(|v| v.len())
                .unwrap_or(0);
            return Ok(Some(SessionDetails {
                id: cp.id,
                user_input: cp.user_input,
                phase: cp.phase.to_string(),
                state: cp.state.to_string(),
                plan_text: cp.plan_text,
                project_dir: cp.project_dir,
                message_count,
                created_at: cp.created_at,
            }));
        }
        // Try prefix match
        if let Ok(Some(cp)) = self.checkpoint_store.find_by_id_prefix(session_id) {
            let message_count = serde_json::from_str::<Vec<Value>>(&cp.messages_json)
                .map(|v| v.len())
                .unwrap_or(0);
            return Ok(Some(SessionDetails {
                id: cp.id,
                user_input: cp.user_input,
                phase: cp.phase.to_string(),
                state: cp.state.to_string(),
                plan_text: cp.plan_text,
                project_dir: cp.project_dir,
                message_count,
                created_at: cp.created_at,
            }));
        }
        Ok(None)
    }

    /// Resume a session by ID
    pub async fn resume_session(&mut self, session_id: &str) -> anyhow::Result<String> {
        // Try to find the checkpoint
        let checkpoint = if let Ok(Some(cp)) = self.checkpoint_store.get(session_id) {
            cp
        } else if let Ok(Some(cp)) = self.checkpoint_store.find_by_id_prefix(session_id) {
            cp
        } else {
            return Err(anyhow::anyhow!("Sessão não encontrada"));
        };

        // Resume the development task
        let task_input = if !checkpoint.plan_text.is_empty() {
            format!(
                "Execute o plano de desenvolvimento no diretorio {}:\n\n{}",
                checkpoint.project_dir, checkpoint.plan_text
            )
        } else {
            checkpoint.user_input.clone()
        };

        // Restore active skill if saved
        if let Some(ref skill_name) = checkpoint.active_skill {
            info!("Restoring skill: {}", skill_name);
            let _ = self.skill_manager.force_skill(skill_name);
        }

        // Create a new checkpoint for this session and run it
        let mut new_checkpoint = DevelopmentCheckpoint::new(task_input.clone());
        new_checkpoint.plan_text = checkpoint.plan_text.clone();
        new_checkpoint.project_dir = checkpoint.project_dir.clone();
        new_checkpoint.plan_file = checkpoint.plan_file.clone();
        new_checkpoint.active_skill = checkpoint.active_skill.clone();
        new_checkpoint.messages_json = checkpoint.messages_json.clone();

        self.run_development(task_input, new_checkpoint).await
    }

    /// Delete a session by ID
    pub async fn delete_session(&mut self, session_id: &str) -> Result<String, String> {
        tracing::debug!("delete_session called with: {}", session_id);

        // Try to delete from session_summaries directly first
        if let Err(e) = self.checkpoint_store.delete_session_summary(session_id) {
            tracing::warn!("Failed to delete session summary: {}", e);
        }

        // Try to find and delete from checkpoints
        if let Ok(Some(cp)) = self.checkpoint_store.get(session_id) {
            self.checkpoint_store
                .delete(&cp.id)
                .map_err(|e| format!("Erro ao excluir checkpoint: {}", e))?;
            return Ok(format!("Sessão {} excluída", &cp.id[..8]));
        }
        if let Ok(Some(cp)) = self.checkpoint_store.find_by_id_prefix(session_id) {
            self.checkpoint_store
                .delete(&cp.id)
                .map_err(|e| format!("Erro ao excluir checkpoint: {}", e))?;
            return Ok(format!("Sessão {} excluída", &cp.id[..8]));
        }

        // If we get here, session was deleted from summaries
        tracing::debug!(
            "Session {} not found in checkpoints but removed from summaries",
            session_id
        );
        Ok(format!(
            "Sessão {} excluída",
            &session_id[..8.min(session_id.len())]
        ))
    }

    /// Rename a session by ID
    pub async fn rename_session(
        &mut self,
        session_id: &str,
        new_name: &str,
    ) -> Result<String, String> {
        // Try to find and update
        if let Ok(Some(mut cp)) = self.checkpoint_store.get(session_id) {
            cp.session_name = Some(new_name.to_string());
            self.checkpoint_store
                .save(&cp)
                .map_err(|e| format!("Erro ao renomear: {}", e))?;
            return Ok(format!("Sessão renomeada para: {}", new_name));
        }
        if let Ok(Some(mut cp)) = self.checkpoint_store.find_by_id_prefix(session_id) {
            cp.session_name = Some(new_name.to_string());
            self.checkpoint_store
                .save(&cp)
                .map_err(|e| format!("Erro ao renomear: {}", e))?;
            return Ok(format!("Sessão renomeada para: {}", new_name));
        }
        Err("Sessão não encontrada".to_string())
    }

    pub fn force_skill(&mut self, skill_name: &str) -> Result<(), String> {
        self.skill_manager.force_skill(skill_name)
    }

    pub fn get_trust_level(&self, path: &Path) -> String {
        if let Some(ref trust) = self.workspace_trust {
            format!("{:?}", trust.get_store().get_trust(path))
        } else {
            "None".to_string()
        }
    }

    pub fn set_trust_level(&mut self, path: &Path, level: crate::workspace_trust::TrustLevel) -> Result<(), String> {
        if let Some(ref mut trust) = self.workspace_trust {
            trust.set_trust(path, level);
            Ok(())
        } else {
            Err("Trust system not initialized".to_string())
        }
    }

    pub fn list_workspaces(&self) -> Vec<String> {
        if let Some(ref trust) = self.workspace_trust {
            trust.get_store()
                .list_workspaces()
                .iter()
                .map(|(path, level)| format!("{:?} - {:?}", level, path.display()))
                .collect()
        } else {
            vec![]
        }
    }

    pub fn model_name(&self) -> String {
        self.config.model.clone()
    }

    #[allow(dead_code)]
    pub fn get_active_skill_name(&self) -> Option<String> {
        self.skill_manager.get_active_skill_name()
    }

    #[allow(dead_code)]
    pub fn get_app_store(&self) -> &Store<AppState> {
        &self.app_store
    }

    #[allow(dead_code)]
    pub fn get_app_state(&self) -> AppState {
        self.app_store.get_state()
    }

    #[allow(dead_code)]
    pub fn update_app_state<F>(&self, updater: F)
    where
        F: FnOnce(&AppState) -> AppState,
    {
        self.app_store.set_state(updater);
    }
}

#[derive(Debug, Clone)]
pub struct CompressionStats {
    pub compression_count: usize,
    pub current_tokens: usize,
    pub max_context_tokens: usize,
    pub usage_ratio: f64,
}

impl Agent {
    pub fn get_compression_stats(&self) -> CompressionStats {
        let current_tokens = self.summarizer.token_counter().count_messages_tokens(&self.conversation_history);
        let usage_ratio = if self.config.max_context_tokens > 0 {
            current_tokens as f64 / self.config.max_context_tokens as f64
        } else {
            0.0
        };

        CompressionStats {
            compression_count: self.compression_count,
            current_tokens,
            max_context_tokens: self.config.max_context_tokens,
            usage_ratio,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentStats {
    pub cost_tracker: CostTrackerStats,
    pub rate_limiter: RateLimiterStats,
    pub compression_stats: CompressionStats,
}

#[derive(Debug, Clone)]
pub struct CostTrackerStats {
    pub total_tokens_used: usize,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub api_calls: usize,
    pub iterations: usize,
    pub estimated_cost_usd: f64,
    pub rate_limit_hits: usize,
}

#[derive(Debug, Clone)]
pub struct RateLimiterStats {
    pub calls_remaining: usize,
    pub tokens_remaining: usize,
    pub max_calls_per_minute: usize,
    pub max_tokens_per_minute: usize,
}

impl Agent {
    pub fn get_stats(&mut self) -> AgentStats {
        let cost = &self.cost_tracker;
        let rate = &mut self.rate_limiter;

        AgentStats {
            cost_tracker: CostTrackerStats {
                total_tokens_used: cost.total_tokens_used,
                prompt_tokens: cost.prompt_tokens,
                completion_tokens: cost.completion_tokens,
                api_calls: cost.api_calls,
                iterations: cost.iterations,
                estimated_cost_usd: cost.estimated_cost_usd,
                rate_limit_hits: cost.rate_limit_hits,
            },
            rate_limiter: RateLimiterStats {
                calls_remaining: rate.calls_remaining(),
                tokens_remaining: rate.tokens_remaining(),
                max_calls_per_minute: 60,
                max_tokens_per_minute: 100_000,
            },
            compression_stats: self.get_compression_stats(),
        }
    }
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

#[allow(dead_code)]
pub fn get_tmux_manager() -> Option<&'static TmuxManager> {
    TMUX_MANAGER.get()
}

#[allow(dead_code)]
pub fn get_output_manager() -> Option<&'static OutputManager> {
    OUTPUT_MANAGER.get()
}

#[allow(dead_code)]
pub fn output_write(msg: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write(msg);
    }
    print!("{}", msg);
}

#[allow(dead_code)]
pub fn output_write_line(msg: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write_line(msg);
    }
    println!("{}", msg);
}

#[allow(dead_code)]
pub fn output_write_tool(tool: &str, input: &str, output: &str) {
    if let Some(out) = OUTPUT_MANAGER.get() {
        out.write_tool(tool, input, output);
    }
    print!("{}{}", Colors::CLEAR_LINE, Colors::ORANGE);
    print!("⬡ ");
    print!("{}{}", Colors::RESET, Colors::LIGHT_GRAY);
    println!("{}  {}{}", tool, input, Colors::RESET);
    println!("{}{}{}", Colors::LIGHT_GRAY, output, Colors::RESET);
}

#[allow(dead_code)]
pub fn output_write_thought(thought: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write_thought(thought);
    }
}

#[allow(dead_code)]
pub fn output_write_error(error: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write_error(error);
    }
    eprintln!(
        "{}⨯{} {}{}{}",
        Colors::RED,
        Colors::RESET,
        error,
        Colors::RESET,
        Colors::RESET
    );
}

#[allow(dead_code)]
pub fn output_write_debug(msg: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write_debug(msg);
    }
}

#[allow(dead_code)]
pub fn output_write_browser(path: &str, description: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write_browser(path, description);
    }
    println!("📸 {} - {}", description, path);
}
