//! SessionRuntime: the harness facade connecting provider, tools, processor,
//! permissions and sessions.

use crate::harness::agent::{build_system_prompt, AgentSpec};
use crate::harness::event::{EventSender, HarnessEvent};
use crate::harness::permission::PermissionEngine;
use crate::harness::project::{ProjectMemoryStore, ProjectProfiler};
use crate::harness::provider::opencode_go::build_provider as build_provider_from;
use crate::harness::provider::{HttpConfig, Provider};
use crate::harness::session::processor::{ProcessorConfig, SessionProcessor, TurnOutcome};
use crate::harness::session::store::SessionStore;
use crate::harness::session::Session;
use crate::harness::skill::{inject, SkillCatalog};
use crate::harness::tool::context::{
    PathBufGuard, PermissionAsker, SubagentRunner, TaskOutcome, ToolContext, UserAsker,
};
use crate::harness::tool::registry::ToolRegistry;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Effective prompt-caching flag for a provider:
/// global kill-switch (`config.json prompt_caching`) AND the per-provider
/// override from `providers.json` (when present) OR the provider default
/// (`true` for anthropic, `false` otherwise).
pub fn effective_prompt_cache(provider: &str) -> bool {
    let global = crate::config::GlobalSettings::load().prompt_caching;
    if !global {
        return false;
    }
    let store = crate::harness::provider::user_store::UserProviders::load();
    store
        .find(provider)
        .and_then(|p| p.prompt_cache)
        .unwrap_or_else(|| crate::harness::provider::opencode_go::default_prompt_cache(provider))
}

/// Result of a prompt call.
pub struct PromptResult {
    pub final_text: String,
    pub iterations: usize,
    pub usage: crate::harness::provider::Usage,
    /// True when the turn was cancelled by the user (Esc / abort signal).
    pub aborted: bool,
}

pub struct SessionRuntime {
    pub store: Arc<SessionStore>,
    pub provider: Arc<dyn Provider>,
    pub registry: ToolRegistry,
    pub permission: Arc<PermissionEngine>,
    pub asker: Arc<dyn PermissionAsker>,
    pub user_asker: Arc<dyn UserAsker>,
    pub config: crate::config::RuntimeConfig,
    /// Discovered skills catalog (this session's available "memory").
    pub skills: Arc<SkillCatalog>,
    /// Auto-discovered project profiler (stack/commands).
    pub project: Arc<std::sync::Mutex<ProjectProfiler>>,
    /// SQLite-backed project memory cache.
    pub project_memory: Arc<ProjectMemoryStore>,
    /// Project root (cwd); used to scope sessions and permission rules.
    pub project_root: std::path::PathBuf,
    /// Custom agents injected by the CLI (overrides builtins).
    pub custom_agents: std::collections::HashMap<String, AgentSpec>,
    /// MCP manager (None when no servers are configured).
    pub mcp: Option<Arc<crate::harness::mcp::McpManager>>,
    /// Frozen per-session structural summaries (session_id → summary).
    ///
    /// The system prompt must stay byte-stable across turns so provider
    /// prompt caches (Anthropic `cache_control` / OpenAI automatic) keep
    /// hitting; the structural summary is therefore computed once per
    /// session (and re-frozen after compaction rewrites the context).
    pub summary_cache: Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
    /// File checkpoints (pre-agent snapshots) powering `/diff` and `/restore`.
    pub checkpoints: Arc<crate::harness::tool::checkpoint::FileCheckpoints>,
    /// Background bash jobs (`bash --background`); shared across sessions.
    pub jobs: Arc<crate::harness::tool::jobs::JobRegistry>,
    /// Daily USD budget tracker (per-day cost, persisted to usage-*.json).
    pub budget: Arc<tokio::sync::Mutex<crate::harness::budget::BudgetTracker>>,
    /// Event recording (`/record`): shared tee target for the UI event loops.
    pub event_recorder: Arc<crate::harness::ui::commands::replay::EventRecorder>,
}

impl SessionRuntime {
    /// Builds a runtime. Convenience wrapper over [`new_in`] that resolves the
    /// cwd from the environment; the UI entry points use `new_in` directly.
    #[allow(dead_code)]
    pub fn new(
        provider: Arc<dyn Provider>,
        registry: ToolRegistry,
        config: crate::config::RuntimeConfig,
        db_path: &std::path::Path,
        permission: Arc<PermissionEngine>,
        asker: Arc<dyn PermissionAsker>,
        user_asker: Arc<dyn UserAsker>,
    ) -> Result<Self> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new_in(
            &cwd, provider, registry, config, db_path, permission, asker, user_asker,
        )
    }

    /// Builds a runtime with an explicit project root (used by tests and by
    /// the UI entry points, which know the session cwd up front).
    #[allow(clippy::too_many_arguments)]
    pub fn new_in(
        project_root: &std::path::Path,
        provider: Arc<dyn Provider>,
        registry: ToolRegistry,
        config: crate::config::RuntimeConfig,
        db_path: &std::path::Path,
        permission: Arc<PermissionEngine>,
        asker: Arc<dyn PermissionAsker>,
        user_asker: Arc<dyn UserAsker>,
    ) -> Result<Self> {
        let cwd = project_root.to_path_buf();
        let store = Arc::new(SessionStore::open(db_path).context("failed to open session store")?);
        let skills = Arc::new(crate::harness::skill::loader::load_catalog(&cwd));
        let custom_agents = crate::harness::agent::custom::load_custom_agents(&cwd);
        if !custom_agents.is_empty() {
            tracing::info!(
                "loaded {} custom agent(s) from .agents/agents",
                custom_agents.len()
            );
        }
        let project_memory = Arc::new(
            ProjectMemoryStore::open(db_path).context("failed to open project memory store")?,
        );

        // Load persistent per-tool permission rules from the project config and
        // install a callback so "always allow" decisions are persisted too.
        let proj = crate::harness::project::config_file::ProjectConfig::load(&cwd);
        permission.apply_project_config(&proj.permission);
        let persist_root = cwd.clone();
        // Serialize load+save of rustclaw.json so concurrent "always allow"
        // decisions don't lose updates (read-modify-write race).
        let persist_lock = Arc::new(std::sync::Mutex::new(()));
        permission.set_persist(Some(Arc::new(move |tool: &str| {
            let _guard = persist_lock.lock().unwrap_or_else(|e| e.into_inner());
            let mut p = crate::harness::project::config_file::ProjectConfig::load(&persist_root);
            p.permission
                .tools
                .insert(tool.to_string(), crate::harness::permission::Rule::Allow);
            p.save(&persist_root).map_err(|e| e.to_string())
        })));

        Ok(Self {
            store,
            provider,
            registry,
            permission,
            asker,
            user_asker,
            config,
            skills,
            project: Arc::new(std::sync::Mutex::new(ProjectProfiler::new(&cwd))),
            project_memory,
            project_root: cwd,
            custom_agents,
            mcp: None,
            summary_cache: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            checkpoints: Arc::new(crate::harness::tool::checkpoint::FileCheckpoints::new()),
            jobs: Arc::new(crate::harness::tool::jobs::JobRegistry::new()),
            event_recorder: Arc::new(crate::harness::ui::commands::replay::EventRecorder::new()),
            budget: Arc::new(tokio::sync::Mutex::new(
                crate::harness::budget::BudgetTracker::default(),
            )),
        })
    }

    /// Builds a runtime directly from a resolved [`RuntimeConfig`], building
    /// the provider from its provider/base_url/api_key. Only used by tests.
    #[cfg(test)]
    pub fn from_config(
        cfg: &crate::config::RuntimeConfig,
        registry: ToolRegistry,
        db_path: &std::path::Path,
        permission: Arc<PermissionEngine>,
        asker: Arc<dyn PermissionAsker>,
        user_asker: Arc<dyn UserAsker>,
    ) -> Result<Self> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::from_config_in(&cwd, cfg, registry, db_path, permission, asker, user_asker)
    }

    /// `from_config` with an explicit project root (see `new_in`).
    #[allow(clippy::too_many_arguments)]
    pub fn from_config_in(
        project_root: &std::path::Path,
        cfg: &crate::config::RuntimeConfig,
        registry: ToolRegistry,
        db_path: &std::path::Path,
        permission: Arc<PermissionEngine>,
        asker: Arc<dyn PermissionAsker>,
        user_asker: Arc<dyn UserAsker>,
    ) -> Result<Self> {
        // `RuntimeConfig` already resolved provider/model/base_url and picked
        // the token from the auth store; we just build the provider from it.
        let http = HttpConfig {
            client: crate::harness::provider::build_http_client(),
            base_url: cfg.base_url.clone(),
            api_key: cfg.api_key.clone(),
        };
        let provider =
            build_provider_from(&cfg.provider, http, effective_prompt_cache(&cfg.provider))?;
        Self::new_in(
            project_root,
            provider,
            registry,
            cfg.clone(),
            db_path,
            permission,
            asker,
            user_asker,
        )
    }

    /// Switches provider/model at runtime (opencode-style `/models`).
    ///
    /// Rebuilds the provider with the API key from the auth store (falling
    /// back to the current key) and persists the selection in the global
    /// `config.json`. Applies from the next turn on.
    pub fn switch_model(&mut self, provider: &str, model: &str) -> Result<()> {
        let auth = crate::harness::auth::AuthStore::load();
        let root = self.project_root.clone();
        self.switch_model_with_auth(&root, &auth, provider, model)
    }

    /// Like [`switch_model`] but persists to an explicit project root (testable).
    /// Kept as a public convenience; the UI uses `switch_model` directly.
    #[allow(dead_code)]
    pub fn switch_model_at(
        &mut self,
        project_root: &std::path::Path,
        provider: &str,
        model: &str,
    ) -> Result<()> {
        let auth = crate::harness::auth::AuthStore::load();
        self.switch_model_with_auth(project_root, &auth, provider, model)
    }

    /// Model switch with an explicit auth store (hermetic tests).
    ///
    /// The selection is persisted globally (config.json), not per-project.
    /// `project_root` is unused for persistence and kept only to preserve the
    /// testable signature.
    pub fn switch_model_with_auth(
        &mut self,
        _project_root: &std::path::Path,
        auth: &crate::harness::auth::AuthStore,
        provider: &str,
        model: &str,
    ) -> Result<()> {
        let base_url = crate::harness::provider::catalog::default_base_url(provider)
            .unwrap_or_else(|| self.config.base_url.clone());
        // Only use a token that actually belongs to the *new* provider. Never
        // fall back to the previous provider's token: that would leak the old
        // credential to a different API. If the new provider has no token, the
        // caller (TUI/CLI) is expected to prompt for one.
        let api_key = auth
            .get_key(provider)
            .filter(|k| !k.trim().is_empty())
            .unwrap_or_default();

        let http = HttpConfig {
            client: crate::harness::provider::build_http_client(),
            base_url: base_url.clone(),
            api_key: api_key.clone(),
        };
        self.config.api_key = api_key;
        self.provider = build_provider_from(provider, http, effective_prompt_cache(provider))?;
        self.config.provider = provider.to_string();
        self.config.model = model.to_string();
        self.config.base_url = base_url.clone();

        // Persist the selection globally (config.json), not in the project's
        // rustclaw.json. base_url follows the provider catalog default.
        let mut settings = crate::config::GlobalSettings::load();
        settings.provider = provider.to_string();
        settings.model = model.to_string();
        settings.base_url = base_url;
        settings
            .save()
            .context("failed to persist global config.json")?;
        Ok(())
    }

    /// True when the auth store holds a usable token for `provider`.
    pub fn has_token_for(&self, provider: &str) -> bool {
        crate::harness::auth::AuthStore::load()
            .get_key(provider)
            .map(|k| k.trim().len() >= 10)
            .unwrap_or(false)
    }

    /// Sets a persistent per-tool permission rule in the project config and
    /// applies it to the live engine.
    pub fn set_permission_rule(
        &self,
        tool: &str,
        rule: crate::harness::permission::Rule,
    ) -> Result<()> {
        self.permission.set_rule(tool, rule);
        let mut proj =
            crate::harness::project::config_file::ProjectConfig::load(&self.project_root);
        proj.permission.tools.insert(tool.to_string(), rule);
        proj.save(&self.project_root)
            .context("failed to persist rustclaw.json")
    }

    /// Removes a persistent per-tool permission rule from the project config
    /// and the live engine. Returns true if a rule existed.
    pub fn remove_permission_rule(&self, tool: &str) -> Result<bool> {
        let removed = self.permission.remove_rule(tool);
        let mut proj =
            crate::harness::project::config_file::ProjectConfig::load(&self.project_root);
        let existed = proj.permission.tools.remove(tool).is_some();
        proj.save(&self.project_root)
            .context("failed to persist rustclaw.json")?;
        Ok(removed || existed)
    }

    /// Grants the harness full freedom to run any tool on files inside the
    /// project. Marks every known tool as `always_allow` in the live engine and
    /// persists an `allow` rule for each in `rustclaw.json`, so the freedom
    /// survives restarts. Paths outside the project still require approval.
    pub fn allow_all_permissions(&self) -> Result<()> {
        self.permission.allow_all();
        let mut proj =
            crate::harness::project::config_file::ProjectConfig::load(&self.project_root);
        for tool in crate::harness::permission::ALL_TOOLS {
            proj.permission
                .tools
                .insert(tool.to_string(), crate::harness::permission::Rule::Allow);
        }
        proj.save(&self.project_root)
            .context("failed to persist rustclaw.json")
    }

    /// Snapshot of the current per-tool permission rules.
    pub fn permission_rules(&self) -> Vec<(String, crate::harness::permission::Rule)> {
        self.permission.rules_snapshot()
    }

    pub fn resolve_agent(&self, name: &str) -> AgentSpec {
        if let Some(spec) = self.custom_agents.get(name) {
            return spec.clone();
        }
        crate::harness::agent::find_builtin(name)
            .unwrap_or_else(crate::harness::agent::builtin::build)
    }

    /// Effective sampling temperature for an agent (spec override wins,
    /// else the calibrated default for the mode). Public convenience; the
    /// processor reads the temperature via `resolve_agent` directly.
    #[allow(dead_code)]
    pub fn turn_temperature(&self, agent_name: &str) -> f32 {
        self.resolve_agent(agent_name).turn_temperature()
    }

    /// Updates global runtime limits (persisted in `config.json`) and
    /// applies them to the live config. `None` = leave unchanged.
    pub fn update_settings(
        &mut self,
        max_iterations: Option<usize>,
        max_context_tokens: Option<usize>,
        turn_timeout_secs: Option<u64>,
    ) -> Result<()> {
        if let Some(n) = max_iterations {
            anyhow::ensure!(n > 0, "max_iterations must be > 0");
            self.config.max_iterations = n;
        }
        if let Some(n) = max_context_tokens {
            anyhow::ensure!(n >= 1000, "max_context_tokens must be at least 1000");
            self.config.max_context_tokens = n;
        }
        if let Some(n) = turn_timeout_secs {
            anyhow::ensure!(n >= 30, "turn_timeout_secs must be at least 30");
            self.config.turn_timeout_secs = n as usize;
        }
        let mut s = crate::config::GlobalSettings::load();
        s.max_iterations = self.config.max_iterations;
        s.max_context_tokens = self.config.max_context_tokens;
        s.turn_timeout_secs = self.config.turn_timeout_secs;
        s.provider = self.config.provider.clone();
        s.model = self.config.model.clone();
        s.save().context("failed to persist config.json")?;
        Ok(())
    }

    pub async fn create_session(&self, agent_name: &str) -> Result<Session> {
        let store = self.store.clone();
        let agent = agent_name.to_string();
        let root = self.project_root.clone();
        tokio::task::spawn_blocking(move || store.create_session(&agent, &root))
            .await
            .map_err(|e| anyhow::anyhow!("join error: {e}"))?
    }

    pub fn load_session(&self, id: &str) -> Result<Option<Session>> {
        self.store.load_session(id, &self.project_root)
    }

    /// Loads the most recently used session of the current project, if any.
    /// `list_sessions` orders by `updated_at DESC`, so the first entry is the
    /// latest. Returns `None` when the project has no sessions yet.
    pub fn load_last_session(&self) -> Result<Option<Session>> {
        self.load_last_session_at(&self.project_root)
    }

    /// Compacts `session` when it exceeds the configured context budget.
    ///
    /// Used on session open/resume (auto-compact) and by `/compact` (force).
    /// When `force` is true the token budget is treated as zero so any session
    /// with enough messages is summarized. Returns the number of messages
    /// summarized away (`0` when nothing changed). Persists the result.
    pub async fn maybe_compact(
        &self,
        session: &mut Session,
        force: bool,
        events: Option<&EventSender>,
    ) -> Result<usize> {
        let summarized = crate::harness::session::compaction::compact_if_needed(
            session,
            self.provider.clone(),
            &self.store,
            self.config.max_context_tokens,
            force,
            events,
        )
        .await?;
        // Compaction rewrote the conversation context: drop the frozen
        // structural summary so the next turn recomputes (and re-freezes) it.
        if summarized > 0 {
            self.summary_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&session.id);
        }
        Ok(summarized)
    }

    /// Like [`load_last_session`] but scoped to an explicit project root
    /// (testable).
    pub fn load_last_session_at(&self, cwd: &Path) -> Result<Option<Session>> {
        let latest = self
            .store
            .list_sessions(cwd)?
            .into_iter()
            .next()
            .map(|s| s.id);
        match latest {
            Some(id) => self.store.load_session(&id, cwd),
            None => Ok(None),
        }
    }

    pub fn list_sessions(&self) -> Result<Vec<crate::harness::session::store::SessionSummary>> {
        self.store.list_sessions(&self.project_root)
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        // Garbage-collect child (subagent) sessions of the deleted parent.
        let _ = self.store.delete_children_of(id, &self.project_root);
        self.store.delete_session(id, &self.project_root)
    }

    /// Sets a user-defined title for a session.
    pub fn set_session_title(&self, id: &str, title: &str) -> Result<()> {
        self.store.set_session_title(id, &self.project_root, title)
    }

    /// Runs one user turn against `session`. `abort` lets the caller cancel the
    /// run (e.g. Ctrl+C in the TUI); the processor checks it between iterations.
    ///
    /// `enabled_skills` is the set of skill ids to inject into the system prompt
    /// for this turn. When `None`, falls back to the session's `include_by_default`.
    pub async fn prompt(
        &self,
        session: &mut Session,
        events: &EventSender,
        user_text: &str,
        abort: crate::harness::tool::context::AbortSignal,
        enabled_skills: Option<&[String]>,
    ) -> Result<PromptResult> {
        self.prompt_with_parts(
            session,
            events,
            vec![crate::harness::session::Part::text(user_text)],
            abort,
            enabled_skills,
        )
        .await
    }

    /// Like `prompt`, but the user message is built from explicit parts
    /// (used by `/image <path>` to attach a `Part::Image`).
    pub async fn prompt_with_parts(
        &self,
        session: &mut Session,
        events: &EventSender,
        mut user_parts: Vec<crate::harness::session::Part>,
        abort: crate::harness::tool::context::AbortSignal,
        enabled_skills: Option<&[String]>,
    ) -> Result<PromptResult> {
        let user_text = user_parts
            .iter()
            .filter_map(|p| p.as_text())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = events.send(HarnessEvent::RunStarted {
            session_id: session.id.clone(),
        });

        // 1. Append + persist user message. The per-turn project memory block
        //    is injected as a leading part (kept out of the system prompt so
        //    the prompt prefix stays byte-stable for provider prompt caches).
        let memory_block = self.memory_block_for(&session.cwd, &user_text).await?;
        if !memory_block.is_empty() {
            user_parts.insert(0, crate::harness::session::Part::text(memory_block));
        }
        let user_msg =
            crate::harness::session::Message::new(crate::harness::session::Role::User, user_parts);
        let _ = events.send(HarnessEvent::UserMessage {
            session_id: session.id.clone(),
            message_id: user_msg.id.clone(),
        });
        session.push_message(user_msg.clone());
        {
            let store = self.store.clone();
            let sid = session.id.clone();
            let cwd = session.cwd.clone();
            let msg = user_msg.clone();
            tokio::task::spawn_blocking(move || store.save_message(&sid, &cwd, &msg))
                .await
                .map_err(|e| anyhow::anyhow!("join error: {e}"))??;
        }

        // 2. Resolve agent + build system prompt (with enabled skills).
        let agent = self.resolve_agent(&session.agent);
        let enabled: Vec<String> = match enabled_skills {
            Some(ids) => ids.to_vec(),
            None => inject::enabled_for_turn(&session.skills, None),
        };
        let skills_block = inject::render_enabled(&self.skills, &session.skills, &enabled);
        let project_context = self.frozen_summary_for(&session.id, &session.cwd).await?;
        let available_tools = self.registry.specs(&agent.tools);
        let system_prompt = build_system_prompt(
            &agent,
            &session.cwd,
            &skills_block,
            None,
            &project_context,
            &available_tools,
        );

        // 3. Build tool context.
        let ctx = ToolContext {
            session_id: session.id.clone(),
            agent: session.agent.clone(),
            agent_tools: agent.tools.clone(),
            cwd: PathBufGuard(session.cwd.clone()),
            abort,
            permission: self.permission.clone(),
            asker: self.asker.clone(),
            user_asker: self.user_asker.clone(),
            todos: Arc::new(tokio::sync::RwLock::new(session.todos.clone())),
            task_runner: Some(Arc::new(TaskRunner {
                runtime: Arc::new(self.clone_shareable()),
                parent_session_id: session.id.clone(),
                allow_write: session.agent == crate::harness::agent::builtin::BUILD,
            })),
            events: events.clone(),
            jobs: self.jobs.clone(),
            project_memory: Some(self.project_memory.clone()),
            hooks: crate::harness::hooks::HooksConfig::load_for_cwd(&session.cwd),
            checkpoints: self.checkpoints.clone(),
        };

        // 4. Run the processor turn.
        let processor = SessionProcessor {
            provider: self.provider.clone(),
            registry: self.registry.clone(),
            events: events.clone(),
            store: self.store.clone(),
            config: ProcessorConfig {
                model: agent
                    .model
                    .clone()
                    .unwrap_or_else(|| self.config.model.clone()),
                max_iterations: self.config.max_iterations,
                max_context_tokens: self.config.max_context_tokens,
                turn_timeout_secs: self.config.turn_timeout_secs as u64,
            },
        };

        let TurnOutcome {
            final_text,
            iterations,
            continuations: _,
            usage,
            aborted,
        } = processor
            .run_turn(session, &agent, &system_prompt, &ctx)
            .await
            .context("agent turn failed")?;

        // If the user cancelled mid-turn (Esc), drop the just-submitted user
        // message so an accidental Enter can be retried cleanly. Only roll back
        // when no assistant reply was persisted yet (abort before first save).
        if aborted {
            // Prefer rolling back the user message we just appended when the
            // turn produced no assistant content worth keeping.
            let only_user_pending = session
                .messages
                .last()
                .map(|m| m.id == user_msg.id)
                .unwrap_or(false);
            if only_user_pending {
                let store = self.store.clone();
                let sid = session.id.clone();
                let cwd = session.cwd.clone();
                let msg_id = user_msg.id.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    store.delete_messages_from(&sid, &cwd, &msg_id)
                })
                .await;
                session.messages.pop();
            }
        }

        // 5. Sync todos back and persist session.
        session.todos = ctx.todos.read().await.clone();
        {
            let store = self.store.clone();
            let snapshot = session.clone();
            tokio::task::spawn_blocking(move || store.save_session(&snapshot))
                .await
                .map_err(|e| anyhow::anyhow!("join error: {e}"))??;
        }

        // Daily budget: accumulate the turn cost and warn at 80%/100%.
        // Never breaks the turn; warn is emitted as a system event.
        if usage.input_tokens + usage.output_tokens > 0 {
            let cost = crate::harness::budget::turn_cost(
                &self.config.provider,
                &self.config.model,
                &usage,
            );
            let limit = self.config.daily_budget_usd;
            // `record` does disk I/O (persist); run it on a blocking thread so
            // the async executor isn't stalled, and treat a poisoned lock by
            // recovering its inner value rather than silently disabling tracking.
            let budget = self.budget.clone();
            let (warn, spent) = tokio::task::spawn_blocking(move || {
                let mut t = budget.blocking_lock();
                let w = t.record(cost, limit);
                (w, t.spent_today())
            })
            .await
            .unwrap_or((None, 0.0));
            if let Some(w) = warn {
                let _ = events.send(HarnessEvent::BudgetWarn {
                    session_id: session.id.clone(),
                    message: w.message(spent, limit),
                });
            }
        }

        let _ = events.send(HarnessEvent::RunFinished {
            session_id: session.id.clone(),
        });

        Ok(PromptResult {
            final_text,
            iterations,
            usage,
            aborted,
        })
    }

    /// Connects to configured MCP servers (global + project config) and
    /// registers their tools into the runtime registry. Safe to call once at
    /// startup; a no-op when no servers are configured.
    pub async fn init_mcp(&mut self) {
        let cfg = match crate::harness::mcp::config::McpConfig::load_merged(&self.project_root) {
            Ok(cfg) if !cfg.servers.is_empty() => cfg,
            Ok(_) => return,
            Err(e) => {
                eprintln!("[warn] failed to load mcp config: {e:#}");
                return;
            }
        };
        let manager = crate::harness::mcp::McpManager::connect_all(cfg).await;
        manager.start_health_checks();
        let mut registry = self.registry.clone();
        for tool in manager.tools().await {
            registry = registry.with_tool(tool);
        }
        self.registry = registry;
        self.mcp = Some(manager);
    }

    /// Returns an Arc to this runtime (for subagent spawning). Clones shared fields.
    pub fn clone_shareable(&self) -> Self {
        Self {
            store: self.store.clone(),
            provider: self.provider.clone(),
            registry: self.registry.clone(),
            permission: self.permission.clone(),
            asker: self.asker.clone(),
            user_asker: self.user_asker.clone(),
            config: self.config.clone(),
            skills: self.skills.clone(),
            project: self.project.clone(),
            project_memory: self.project_memory.clone(),
            project_root: self.project_root.clone(),
            custom_agents: self.custom_agents.clone(),
            mcp: self.mcp.clone(),
            summary_cache: self.summary_cache.clone(),
            checkpoints: self.checkpoints.clone(),
            jobs: self.jobs.clone(),
            budget: self.budget.clone(),
            event_recorder: self.event_recorder.clone(),
        }
    }

    /// Returns the frozen structural summary for a session, computing (and
    /// persisting) it on first use. The summary is cached per session so the
    /// system prompt stays byte-stable across turns — a prerequisite for
    /// provider prompt caches (Anthropic `cache_control`, OpenAI automatic),
    /// which require an identical prefix. Compaction invalidates the entry
    /// (the context is rewritten anyway).
    fn frozen_summary_for<'a>(
        &'a self,
        session_id: &'a str,
        cwd: &'a std::path::Path,
    ) -> impl std::future::Future<Output = Result<String>> + Send + 'a {
        let cached = self
            .summary_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(session_id)
            .cloned();
        async move {
            if let Some(cached) = cached {
                return Ok(cached);
            }
            let cwd = cwd.to_path_buf();
            let memory = self.project_memory.clone();
            let (needs_regen, loaded, profiler_inner) = tokio::task::spawn_blocking(move || {
                let profiler = ProjectProfiler {
                    inner: ProjectProfiler::analyze(&cwd),
                };
                let needs_regen = memory.needs_regen(&cwd, &profiler.inner).unwrap_or(true);
                let loaded = if needs_regen {
                    None
                } else {
                    memory.load(&cwd).ok().flatten()
                };
                (needs_regen, loaded, profiler.inner)
            })
            .await
            .map_err(|e| anyhow::anyhow!("join error: {e}"))?;
            let profiler = ProjectProfiler {
                inner: profiler_inner,
            };
            let summary = if needs_regen {
                let rendered = profiler.render_summary();
                let memory = self.project_memory.clone();
                let ctx = profiler.inner.clone();
                let rendered_clone = rendered.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    memory.upsert_summary(&ctx, &rendered_clone)
                })
                .await;
                rendered
            } else {
                loaded
                    .map(|r| {
                        if r.summary.trim().is_empty() {
                            profiler.render_summary()
                        } else {
                            r.summary
                        }
                    })
                    .unwrap_or_else(|| profiler.render_summary())
            };
            // Lock the shared profiler so the `remember` tool and prompt stay in sync.
            if let Ok(mut p) = self.project.lock() {
                p.inner = profiler.inner;
            }
            self.summary_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(session_id.to_string(), summary.clone());
            Ok(summary)
        }
    }

    /// Builds the per-turn `<project-memory>` block (top curated facts,
    /// ranked by relevance to the current query). Injected as a prefix of the
    /// user message — NOT into the system prompt — so the prompt prefix
    /// stays cacheable.
    fn memory_block_for<'a>(
        &'a self,
        cwd: &'a std::path::Path,
        query: &str,
    ) -> impl std::future::Future<Output = Result<String>> + Send + 'a {
        let cwd = cwd.to_path_buf();
        let query = query.to_string();
        async move {
            let memory = self.project_memory.clone();
            let cwd2 = cwd.clone();
            let q2 = query.clone();
            let (facts, ranks) = tokio::task::spawn_blocking(move || {
                let facts = memory.active_facts(&cwd2).unwrap_or_default();
                let ranks: std::collections::HashMap<i64, f64> = if q2.trim().is_empty() {
                    std::collections::HashMap::new()
                } else {
                    memory
                        .search_facts(&cwd2, &q2)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(f, rank)| (f.id, rank))
                        .collect()
                };
                (facts, ranks)
            })
            .await
            .map_err(|e| anyhow::anyhow!("join error: {e}"))?;
            let rendered = crate::harness::project::memory::render_memory_ranked(
                &facts,
                &query,
                crate::harness::project::memory::MAX_MEMORY_CHARS,
                &ranks,
            );
            // Bump usage for the facts that were actually injected.
            if !rendered.is_empty() {
                let memory = self.project_memory.clone();
                let ids: Vec<i64> = facts
                    .iter()
                    .filter(|f| rendered.contains(&f.text))
                    .map(|f| f.id)
                    .collect();
                let _ = tokio::task::spawn_blocking(move || {
                    for id in ids {
                        let _ = memory.bump_usage(&cwd, id);
                    }
                })
                .await;
            }
            if rendered.trim().is_empty() {
                return Ok(String::new());
            }
            Ok(format!(
                "{}\n{}\n{}",
                crate::harness::project::memory::MEMORY_BLOCK_START,
                rendered.trim_end(),
                crate::harness::project::memory::MEMORY_BLOCK_END
            ))
        }
    }
}

/// Builds the default registry with all core harness coding tools.
pub fn build_default_registry() -> ToolRegistry {
    use crate::harness::tool::{
        ast_search::AstSearchTool,
        bash::BashTool,
        diagnostics::DiagnosticsTool,
        edit::EditTool,
        fetch_webpage::FetchWebpageTool,
        git::{GitDiffTool, GitLogTool, GitStatusTool},
        glob::GlobTool,
        grep::GrepTool,
        question::QuestionTool,
        read::ReadTool,
        remember::RememberTool,
        task::TaskTool,
        todo::{TodoReadTool, TodoWriteTool},
        web_search::WebSearchTool,
        write::WriteTool,
    };
    ToolRegistry::builder()
        .register(Arc::new(BashTool))
        .register(Arc::new(ReadTool))
        .register(Arc::new(WriteTool))
        .register(Arc::new(EditTool))
        .register(Arc::new(GlobTool))
        .register(Arc::new(GrepTool))
        .register(Arc::new(AstSearchTool))
        .register(Arc::new(DiagnosticsTool))
        .register(Arc::new(TodoReadTool))
        .register(Arc::new(TodoWriteTool))
        .register(Arc::new(QuestionTool))
        .register(Arc::new(TaskTool))
        .register(Arc::new(RememberTool))
        .register(Arc::new(FetchWebpageTool))
        .register(Arc::new(WebSearchTool))
        .register(Arc::new(GitStatusTool))
        .register(Arc::new(GitDiffTool))
        .register(Arc::new(GitLogTool))
        .build()
}

/// Runs a subagent in a fresh child session, returning its final summary.
/// Child events are forwarded to the caller's channel (tagged with the child's
/// session id and the parent session id) so UIs can render subagent activity.
pub struct TaskRunner {
    pub runtime: Arc<SessionRuntime>,
    /// Session that spawned the task (used to tag child events).
    pub parent_session_id: String,
    /// Whether the root agent may write files. When false, subagents are
    /// demoted to a non-writing agent so they can never edit files.
    pub allow_write: bool,
}

/// Resolves the effective subagent agent given whether the parent allows file
/// writes. When writes are not allowed, a requested `build` subagent is
/// demoted to `general` (same toolset minus write/edit) so subagents can
/// never edit files when the root agent isn't `build`.
fn resolve_subagent_agent(allow_write: bool, requested: &str) -> &str {
    if !allow_write && requested == crate::harness::agent::builtin::BUILD {
        crate::harness::agent::builtin::GENERAL
    } else {
        requested
    }
}

#[async_trait::async_trait]
impl SubagentRunner for TaskRunner {
    async fn run_task(
        &self,
        agent: String,
        prompt: String,
        events: crate::harness::event::EventSender,
    ) -> Result<TaskOutcome, String> {
        // Resolve agent to allow "explore" by default.
        let agent = if agent.is_empty() { "explore" } else { &agent };
        // Demote `build` subagents when the root agent can't write files.
        let agent = resolve_subagent_agent(self.allow_write, agent);
        let mut child = self
            .runtime
            .store
            .create_session(agent, &self.runtime_current_cwd())
            .map_err(|e| e.to_string())?;
        child.agent = agent.to_string();
        let child_cwd = child.cwd.clone();
        // Link the child to this session so UIs can group its events and the
        // store can garbage-collect orphans when the parent is deleted.
        self.runtime
            .store
            .set_session_parent(&child.id, &child_cwd, Some(&self.parent_session_id))
            .map_err(|e| e.to_string())?;

        let result = self
            .runtime
            .prompt(
                &mut child,
                &events,
                &prompt,
                crate::harness::tool::context::AbortSignal::new(),
                None,
            )
            .await
            .map_err(|e| e.to_string())?;

        Ok(TaskOutcome {
            final_text: result.final_text,
            session_id: child.id.clone(),
            iterations: result.iterations,
        })
    }
}

impl TaskRunner {
    fn runtime_current_cwd(&self) -> PathBuf {
        self.runtime.project_root.clone()
    }
}

#[cfg(test)]
mod smoke_tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    #[ignore = "requires a live token in the auth store (~/.local/share/rustclaw/auth.json)"]
    async fn smoke_native_tool_calling() {
        let config = crate::config::RuntimeConfig::load();
        assert!(
            config.is_configured(),
            "run the TUI once with /models + /auth to store a token before this smoke test"
        );
        let registry = crate::harness::runtime::build_default_registry();
        let db = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/smoke.db");
        let _ = std::fs::remove_file(&db);
        let permission = Arc::new(crate::harness::permission::PermissionEngine::default());
        let asker = Arc::new(crate::harness::ui::cli::CliAsker::new(permission.clone()));
        let runtime = SessionRuntime::from_config(
            &config,
            registry,
            &db,
            permission,
            asker,
            Arc::new(crate::harness::ui::cli::CliUserAsker),
        )
        .expect("runtime");

        let mut session = runtime.create_session("build").await.unwrap();
        let (tx, mut rx) = crate::harness::event::event_channel();
        let printer = tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if let crate::harness::event::HarnessEvent::ToolStart { name, .. } = ev {
                    println!("  [tool] {}", name);
                }
            }
        });

        let prompt = "Use the glob tool to list .rs files in the src/ directory. \
Then use the read tool to read src/main.rs. Report what tools you used.";
        let result = runtime
            .prompt(
                &mut session,
                &tx,
                prompt,
                crate::harness::tool::context::AbortSignal::new(),
                None,
            )
            .await
            .expect("prompt");
        let _ = tx.send(crate::harness::event::HarnessEvent::RunFinished {
            session_id: session.id.clone(),
        });
        printer.abort();
        let _ = std::io::stdout().flush();

        println!("\n===== FINAL TEXT =====");
        println!("{}", result.final_text);
        println!(
            "\niterations={} usage_in={} usage_out={}",
            result.iterations, result.usage.input_tokens, result.usage.output_tokens
        );

        let tool_used = session.messages.iter().any(|m| m.has_tool_calls());
        assert!(
            tool_used,
            "expected at least one tool call in the conversation; got: {:?}",
            session
                .messages
                .iter()
                .map(|m| m.role.as_str().to_string())
                .collect::<Vec<_>>()
        );
    }

    /// A fake SubagentRunner that records the event channel it received and
    /// emits a child event through it (used to verify event propagation).
    struct RecordingRunner {
        parent_session_id: String,
        seen: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl SubagentRunner for RecordingRunner {
        async fn run_task(
            &self,
            _agent: String,
            _prompt: String,
            events: crate::harness::event::EventSender,
        ) -> Result<TaskOutcome, String> {
            // Emit a child event tagged with a fake child session id.
            let child_id = "child-1".to_string();
            let _ = events.send(HarnessEvent::ToolStart {
                session_id: child_id.clone(),
                message_id: "m1".into(),
                tool_id: "t1".into(),
                name: "grep".into(),
                input: serde_json::json!({}),
                parent_session_id: Some(self.parent_session_id.clone()),
            });
            self.seen
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(child_id);
            Ok(TaskOutcome {
                final_text: "done".into(),
                session_id: "child-1".into(),
                iterations: 1,
            })
        }
    }

    #[tokio::test]
    async fn test_subagent_events_reach_parent_channel() {
        let (tx, mut rx) = crate::harness::event::event_channel();
        let runner = RecordingRunner {
            parent_session_id: "parent-1".into(),
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let outcome = runner
            .run_task("explore".into(), "p".into(), tx.clone())
            .await
            .unwrap();
        assert_eq!(outcome.session_id, "child-1");
        assert_eq!(outcome.final_text, "done");
        drop(tx);
        // The child event must arrive tagged with the parent session id.
        let mut found = false;
        while let Ok(ev) = rx.try_recv() {
            if let HarnessEvent::ToolStart { .. } = &ev {
                assert_eq!(ev.parent_session_id(), Some("parent-1"));
                assert_eq!(ev.session_id(), Some("child-1"));
                found = true;
            }
        }
        assert!(found, "child event not received on parent channel");
    }

    #[test]
    fn test_parent_session_id_helper() {
        let ev = HarnessEvent::Error {
            session_id: "s".into(),
            message: "m".into(),
            parent_session_id: Some("p".into()),
        };
        assert_eq!(ev.parent_session_id(), Some("p"));
        let ev = HarnessEvent::RunStarted {
            session_id: "s".into(),
        };
        assert_eq!(ev.parent_session_id(), None);
    }
}

#[cfg(test)]
mod model_switch_tests {
    use super::*;
    use crate::harness::tool::registry::ToolRegistry;
    use std::sync::Arc;

    struct AllowAsker;
    #[async_trait::async_trait]
    impl crate::harness::tool::context::PermissionAsker for AllowAsker {
        async fn ask(&self, _req: crate::harness::tool::context::PermissionAskInput) -> bool {
            true
        }
    }
    struct NoUserAsker;
    #[async_trait::async_trait]
    impl crate::harness::tool::context::UserAsker for NoUserAsker {
        async fn ask(&self, _q: String, _o: Vec<String>) -> Option<String> {
            None
        }
    }

    fn test_runtime(dir: &std::path::Path) -> Result<SessionRuntime> {
        let http = HttpConfig {
            client: crate::harness::provider::build_http_client(),
            base_url: "https://api.deepinfra.com/v1/openai".to_string(),
            api_key: "sk-initial-test-key-123456".to_string(),
        };
        let provider = build_provider_from("deepinfra", http, false)?;
        let db = dir.join("test.db");
        SessionRuntime::new_in(
            dir,
            provider,
            ToolRegistry::builder().build(),
            crate::config::RuntimeConfig {
                model: "deepseek-ai/DeepSeek-V4-Flash-0731".to_string(),
                provider: "deepinfra".to_string(),
                base_url: "https://api.deepinfra.com/v1/openai".to_string(),
                api_key: "sk-initial-test-key-123456".to_string(),
                ..Default::default()
            },
            &db,
            Arc::new(crate::harness::permission::PermissionEngine::default()),
            Arc::new(AllowAsker),
            Arc::new(NoUserAsker),
        )
    }

    #[tokio::test]
    async fn test_concurrent_permission_persists_no_lost_update() {
        let dir = tempfile::tempdir().unwrap();
        let rt = test_runtime(dir.path()).unwrap();
        let persist = rt.permission.persist_callback();
        let persist = persist.expect("persist callback should be set");

        // Two concurrent "always allow" decisions for different tools.
        let (p1, p2) = (persist.clone(), persist.clone());
        let (r1, r2) = tokio::join!(
            tokio::task::spawn_blocking(move || p1("tool_a")),
            tokio::task::spawn_blocking(move || p2("tool_b")),
        );
        r1.unwrap().unwrap();
        r2.unwrap().unwrap();

        let proj = crate::harness::project::config_file::ProjectConfig::load(dir.path());
        assert_eq!(
            proj.permission.tools.get("tool_a"),
            Some(&crate::harness::permission::Rule::Allow)
        );
        assert_eq!(
            proj.permission.tools.get("tool_b"),
            Some(&crate::harness::permission::Rule::Allow)
        );
    }

    #[tokio::test]
    async fn test_switch_model_updates_config_and_global_settings() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();

        rt.switch_model_at(dir.path(), "moonshot", "kimi-k2.5")
            .unwrap();
        assert_eq!(rt.config.provider, "moonshot");
        assert_eq!(rt.config.model, "kimi-k2.5");
        assert_eq!(rt.config.base_url, "https://api.moonshot.ai/v1");

        // Provider rebuilt: routing name follows the adapter.
        let name = rt.provider.name().to_string();
        assert!(!name.is_empty());

        // Selection persisted into the GLOBAL config.json, not the project file.
        let proj = crate::harness::project::config_file::ProjectConfig::load(dir.path());
        assert!(
            proj.is_empty(),
            "project file must not carry model/provider"
        );

        let s = crate::config::GlobalSettings::load_from(&crate::config::GlobalSettings::path())
            .unwrap();
        assert_eq!(s.provider, "moonshot");
        assert_eq!(s.model, "kimi-k2.5");
        assert_eq!(s.base_url, "https://api.moonshot.ai/v1");
    }

    #[tokio::test]
    async fn test_switch_model_clears_key_when_auth_store_empty() {
        // Switching to a provider with no stored token must NOT reuse the
        // previous provider's key (would leak the old credential to a new API).
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        rt.switch_model_at(dir.path(), "openrouter", "z-ai/glm-4.6")
            .unwrap();
        assert_eq!(rt.config.provider, "openrouter");
        assert_eq!(rt.config.model, "z-ai/glm-4.6");
        assert_eq!(rt.config.api_key, "");
        assert!(!rt.config.is_configured());
    }

    #[tokio::test]
    async fn test_switch_model_uses_token_from_auth_store() {
        // Switching to a provider that HAS a stored token must use that token.
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        let mut auth = crate::harness::auth::AuthStore::default();
        auth.set_key("moonshot", "sk-moonshot-1234567890");
        rt.switch_model_with_auth(dir.path(), &auth, "moonshot", "kimi-k2.5")
            .unwrap();
        assert_eq!(rt.config.provider, "moonshot");
        assert_eq!(rt.config.api_key, "sk-moonshot-1234567890");
        assert!(rt.config.is_configured());
    }

    #[tokio::test]
    async fn test_switch_model_unknown_provider_keeps_base_url() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        rt.switch_model_at(dir.path(), "my-custom-relay", "model-x")
            .unwrap();
        assert_eq!(rt.config.provider, "my-custom-relay");
        // Unknown provider keeps the previous base_url.
        assert_eq!(rt.config.base_url, "https://api.deepinfra.com/v1/openai");
    }

    #[tokio::test]
    async fn test_load_last_session_returns_most_recent() {
        let dir = tempfile::tempdir().unwrap();
        let rt = test_runtime(dir.path()).unwrap();

        // No sessions yet → None.
        assert!(rt
            .load_last_session_at(std::path::Path::new(dir.path()))
            .unwrap()
            .is_none());

        // Create two sessions directly in the store (scoped to the tempdir),
        // the second being the most recently updated.
        let mut s1 = rt.store.create_session("build", dir.path()).unwrap();
        let s2 = rt.store.create_session("plan", dir.path()).unwrap();

        let last = rt
            .load_last_session_at(std::path::Path::new(dir.path()))
            .unwrap()
            .unwrap();
        assert_eq!(last.id, s2.id);
        assert_eq!(last.agent, "plan");

        // Touching s1 (bumping its updated_at) makes it the most recent again.
        s1.updated_at = chrono::Utc::now() + chrono::Duration::seconds(1);
        rt.store.save_session(&s1).unwrap();
        let last = rt
            .load_last_session_at(std::path::Path::new(dir.path()))
            .unwrap()
            .unwrap();
        assert_eq!(last.id, s1.id);
    }

    #[tokio::test]
    async fn test_custom_agents_discovered_and_shadow_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let agents_dir = dir.path().join(".agents").join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(
            agents_dir.join("reviewer.md"),
            "---\ndescription: Code reviewer\ntools: [read, grep]\nmodel: gpt-5\n---\nYou review code carefully.",
        )
        .unwrap();
        // Custom agent shadowing a builtin name.
        std::fs::write(
            agents_dir.join("build.md"),
            "---\ndescription: custom build\n---\nCustom build prompt.",
        )
        .unwrap();

        let rt = test_runtime(dir.path()).unwrap();
        assert_eq!(rt.custom_agents.len(), 2);

        let review = rt.resolve_agent("reviewer");
        assert_eq!(review.name, "reviewer");
        assert_eq!(review.description, "Code reviewer");
        assert_eq!(review.tools, vec!["read", "grep"]);
        assert_eq!(review.model.as_deref(), Some("gpt-5"));
        assert_eq!(review.system_prompt, "You review code carefully.");

        // Custom wins over the builtin of the same name.
        let build = rt.resolve_agent("build");
        assert_eq!(build.system_prompt, "Custom build prompt.");

        // Builtins not shadowed remain available.
        assert_eq!(rt.resolve_agent("plan").name, "plan");
    }

    #[tokio::test]
    async fn test_session_ops_use_project_root_not_process_cwd() {
        // The runtime's session operations must be scoped to its `project_root`,
        // not to the process's current working directory. We build a runtime
        // rooted at a tempdir and verify sessions land there.
        let dir = tempfile::tempdir().unwrap();
        let rt = test_runtime(dir.path()).unwrap();
        assert_eq!(rt.project_root, dir.path());

        // create_session uses project_root.
        let s = rt.create_session("build").await.unwrap();
        assert_eq!(s.cwd, dir.path());

        // list_sessions / load_last_session see the session created above.
        let listed = rt.list_sessions().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, s.id);

        let last = rt.load_last_session().unwrap().unwrap();
        assert_eq!(last.id, s.id);

        // load_session finds it by id.
        let loaded = rt.load_session(&s.id).unwrap().unwrap();
        assert_eq!(loaded.id, s.id);

        // set_session_title + delete_session also operate on project_root.
        rt.set_session_title(&s.id, "título").unwrap();
        assert_eq!(
            rt.load_session(&s.id).unwrap().unwrap().title.as_deref(),
            Some("título")
        );
        rt.delete_session(&s.id).unwrap();
        assert!(rt.load_session(&s.id).unwrap().is_none());
    }

    #[tokio::test]
    async fn test_onboarding_becomes_configured_after_token() {
        // Simulate a fresh unconfigured runtime (no token).
        let dir = tempfile::tempdir().unwrap();
        let http = HttpConfig {
            client: crate::harness::provider::build_http_client(),
            base_url: "https://api.deepinfra.com/v1/openai".to_string(),
            api_key: String::new(),
        };
        let provider = build_provider_from("deepinfra", http, false).unwrap();
        let db = dir.path().join("test.db");
        let mut rt = SessionRuntime::new_in(
            dir.path(),
            provider,
            ToolRegistry::builder().build(),
            crate::config::RuntimeConfig {
                model: String::new(),
                provider: String::new(),
                base_url: "https://api.deepinfra.com/v1/openai".to_string(),
                api_key: String::new(),
                ..Default::default()
            },
            &db,
            Arc::new(crate::harness::permission::PermissionEngine::default()),
            Arc::new(AllowAsker),
            Arc::new(NoUserAsker),
        )
        .unwrap();
        assert!(!rt.config.is_configured());

        // /models: pick provider+model — no token yet → stays unconfigured.
        rt.switch_model_with_auth(
            dir.path(),
            &crate::harness::auth::AuthStore::default(),
            "deepinfra",
            "deepseek-ai/DeepSeek-V4-Flash-0731",
        )
        .unwrap();
        assert_eq!(rt.config.provider, "deepinfra");
        assert!(!rt.config.is_configured());

        // /auth: token saved (mirrors handle_auth_prompt_key on config).
        rt.config.api_key = "sk-live-token-1234567890".to_string();
        assert!(
            rt.config.is_configured(),
            "prompt must enable after token save"
        );
    }
}

#[cfg(test)]
mod write_guard_tests {
    use super::resolve_subagent_agent;

    #[test]
    fn test_build_subagent_allowed_when_write_enabled() {
        assert_eq!(resolve_subagent_agent(true, "build"), "build");
        assert_eq!(resolve_subagent_agent(true, "explore"), "explore");
    }

    #[test]
    fn test_build_subagent_demoted_when_write_disabled() {
        assert_eq!(resolve_subagent_agent(false, "build"), "general");
        // non-build agents are untouched
        assert_eq!(resolve_subagent_agent(false, "explore"), "explore");
        assert_eq!(resolve_subagent_agent(false, "plan"), "plan");
        assert_eq!(resolve_subagent_agent(false, "general"), "general");
    }
}
