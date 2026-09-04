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
    PathBufGuard, PermissionAsker, SubagentRunner, ToolContext, UserAsker,
};
use crate::harness::tool::registry::ToolRegistry;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;

/// Harness runtime configuration (derived from the legacy config or defaults).
#[derive(Clone, Debug)]
pub struct HarnessConfig {
    pub model: String,
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub max_iterations: usize,
    pub max_context_tokens: usize,
    pub default_agent: String,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            provider: String::new(),
            base_url: String::new(),
            api_key: String::new(),
            max_iterations: 50,
            max_context_tokens: 100_000,
            default_agent: "build".to_string(),
        }
    }
}

impl HarnessConfig {
    /// True when the harness has a usable provider token.
    pub fn is_configured(&self) -> bool {
        self.api_key.trim().len() >= 10
    }

    /// Builds from the legacy config (provider/model/tokens already resolved by
    /// `config::Config::load`).
    pub fn from_legacy(cfg: &crate::config::Config) -> Self {
        Self {
            model: cfg.model.clone(),
            provider: cfg.provider.clone(),
            base_url: cfg.base_url.clone(),
            api_key: cfg.api_key.clone().unwrap_or_default(),
            max_iterations: cfg.max_iterations,
            max_context_tokens: cfg.max_context_tokens,
            default_agent: "build".to_string(),
        }
    }
}

/// Result of a prompt call.
pub struct PromptResult {
    pub final_text: String,
    pub iterations: usize,
    pub usage: crate::harness::provider::Usage,
}

pub struct SessionRuntime {
    pub store: Arc<SessionStore>,
    pub provider: Arc<dyn Provider>,
    pub registry: ToolRegistry,
    pub permission: Arc<PermissionEngine>,
    pub asker: Arc<dyn PermissionAsker>,
    pub user_asker: Arc<dyn UserAsker>,
    pub config: HarnessConfig,
    /// Discovered skills catalog (this session's available "memory").
    pub skills: Arc<SkillCatalog>,
    /// Auto-discovered project profiler (stack/commands).
    pub project: Arc<std::sync::Mutex<ProjectProfiler>>,
    /// SQLite-backed project memory cache.
    pub project_memory: Arc<ProjectMemoryStore>,
    /// Custom agents injected by the CLI (overrides builtins).
    pub custom_agents: std::collections::HashMap<String, AgentSpec>,
}

impl SessionRuntime {
    /// Builds a runtime.
    pub fn new(
        provider: Arc<dyn Provider>,
        registry: ToolRegistry,
        config: HarnessConfig,
        db_path: &std::path::Path,
        asker: Arc<dyn PermissionAsker>,
        user_asker: Arc<dyn UserAsker>,
    ) -> Result<Self> {
        let store = Arc::new(SessionStore::open(db_path).context("failed to open session store")?);
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let skills = Arc::new(crate::harness::skill::loader::load_catalog(&cwd));
        let project_memory = Arc::new(
            ProjectMemoryStore::open(db_path).context("failed to open project memory store")?,
        );

        Ok(Self {
            store,
            provider,
            registry,
            permission: Arc::new(PermissionEngine::default()),
            asker,
            user_asker,
            config,
            skills,
            project: Arc::new(std::sync::Mutex::new(ProjectProfiler::new(&cwd))),
            project_memory,
            custom_agents: std::collections::HashMap::new(),
        })
    }

    /// Builds a runtime directly from the legacy config + registry.
    pub fn from_legacy(
        cfg: &crate::config::Config,
        registry: ToolRegistry,
        db_path: &std::path::Path,
        asker: Arc<dyn PermissionAsker>,
        user_asker: Arc<dyn UserAsker>,
    ) -> Result<Self> {
        // `Config::load` already resolved provider/model/base_url and picked
        // the token from the auth store; `from_legacy` just adapts it.
        let http = HttpConfig {
            client: reqwest::Client::new(),
            base_url: cfg.base_url.clone(),
            api_key: cfg.api_key.clone().unwrap_or_default(),
        };
        let provider = build_provider_from(&cfg.provider, http)?;
        Self::new(
            provider,
            registry,
            HarnessConfig::from_legacy(cfg),
            db_path,
            asker,
            user_asker,
        )
    }

    /// Switches provider/model at runtime (opencode-style `/models`).
    ///
    /// Rebuilds the provider with the API key from the auth store (falling
    /// back to the current key) and persists the selection in the project's
    /// `rustclaw.json`. Applies from the next turn on.
    pub fn switch_model(&mut self, provider: &str, model: &str) -> Result<()> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let auth = crate::harness::auth::AuthStore::load();
        self.switch_model_with_auth(&cwd, &auth, provider, model)
    }

    /// Like [`switch_model`] but persists to an explicit project root (testable).
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
    pub fn switch_model_with_auth(
        &mut self,
        project_root: &std::path::Path,
        auth: &crate::harness::auth::AuthStore,
        provider: &str,
        model: &str,
    ) -> Result<()> {
        let base_url = crate::harness::provider::catalog::default_base_url(provider)
            .map(str::to_string)
            .unwrap_or_else(|| self.config.base_url.clone());
        let api_key = auth
            .get_key(provider)
            .filter(|k| !k.trim().is_empty())
            .unwrap_or_else(|| self.config.api_key.clone());

        let http = HttpConfig {
            client: reqwest::Client::new(),
            base_url: base_url.clone(),
            api_key: api_key.clone(),
        };
        if !api_key.is_empty() {
            self.config.api_key = api_key.clone();
        }
        self.provider = build_provider_from(provider, http)?;
        self.config.provider = provider.to_string();
        self.config.model = model.to_string();
        self.config.base_url = base_url.clone();

        // Persist the project-scoped selection.
        let mut proj = crate::harness::project::config_file::ProjectConfig::load(project_root);
        proj.provider = provider.to_string();
        proj.model = model.to_string();
        proj.base_url = base_url;
        proj.save(project_root)
            .context("failed to persist rustclaw.json")?;
        Ok(())
    }

    pub fn resolve_agent(&self, name: &str) -> AgentSpec {
        if let Some(spec) = self.custom_agents.get(name) {
            return spec.clone();
        }
        crate::harness::agent::find_builtin(name)
            .unwrap_or_else(crate::harness::agent::builtin::build)
    }

    /// Effective sampling temperature for an agent (spec override wins,
    /// else the calibrated default for the mode).
    pub fn turn_temperature(&self, agent_name: &str) -> f32 {
        self.resolve_agent(agent_name).turn_temperature()
    }

    /// Updates global runtime limits (persisted in `config.json`) and
    /// applies them to the live config. `None` = leave unchanged.
    pub fn update_settings(
        &mut self,
        max_iterations: Option<usize>,
        max_context_tokens: Option<usize>,
    ) -> Result<()> {
        if let Some(n) = max_iterations {
            anyhow::ensure!(n > 0, "max_iterations must be > 0");
            self.config.max_iterations = n;
        }
        if let Some(n) = max_context_tokens {
            anyhow::ensure!(n >= 1000, "max_context_tokens must be at least 1000");
            self.config.max_context_tokens = n;
        }
        let mut s = crate::config::GlobalSettings::load();
        s.max_iterations = self.config.max_iterations;
        s.max_context_tokens = self.config.max_context_tokens;
        s.provider = self.config.provider.clone();
        s.model = self.config.model.clone();
        s.save().context("failed to persist config.json")?;
        Ok(())
    }

    pub async fn create_session(&self, agent_name: &str) -> Result<Session> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        self.store.create_session(agent_name, &cwd)
    }

    pub fn load_session(&self, id: &str) -> Result<Option<Session>> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        self.store.load_session(id, &cwd)
    }

    pub fn list_sessions(&self) -> Result<Vec<crate::harness::session::store::SessionSummary>> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        self.store.list_sessions(&cwd)
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        self.store.delete_session(id, &cwd)
    }

    /// Sets a user-defined title for a session.
    pub fn set_session_title(&self, id: &str, title: &str) -> Result<()> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        self.store.set_session_title(id, &cwd, title)
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
        let _ = events.send(HarnessEvent::RunStarted {
            session_id: session.id.clone(),
        });

        // 1. Append + persist user message.
        let user_msg = crate::harness::session::Message::user(user_text.to_string());
        let _ = events.send(HarnessEvent::UserMessage {
            session_id: session.id.clone(),
            message_id: user_msg.id.clone(),
        });
        session.push_message(user_msg.clone());
        self.store
            .save_message(&session.id, &session.cwd, &user_msg)?;

        // 2. Resolve agent + build system prompt (with enabled skills).
        let agent = self.resolve_agent(&session.agent);
        let enabled: Vec<String> = match enabled_skills {
            Some(ids) => ids.to_vec(),
            None => inject::enabled_for_turn(&session.skills, None),
        };
        let skills_block = inject::render_enabled(&self.skills, &session.skills, &enabled);
        let project_context = self.project_context_for(&session.cwd)?;
        let system_prompt =
            build_system_prompt(&agent, &session.cwd, &skills_block, None, &project_context);

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
            })),
            project_memory: Some(self.project_memory.clone()),
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
            },
        };

        let TurnOutcome {
            final_text,
            iterations,
            usage,
            aborted: _,
        } = processor
            .run_turn(session, &agent, &system_prompt, &ctx)
            .await
            .context("agent turn failed")?;

        // 5. Sync todos back and persist session.
        session.todos = ctx.todos.read().await.clone();
        self.store.save_session(session)?;

        let _ = events.send(HarnessEvent::RunFinished {
            session_id: session.id.clone(),
        });

        Ok(PromptResult {
            final_text,
            iterations,
            usage,
        })
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
            custom_agents: self.custom_agents.clone(),
        }
    }

    /// Builds the `# Project context` block for a session's working directory,
    /// recomputing (and persisting) the structural summary when stale.
    fn project_context_for(&self, cwd: &std::path::Path) -> Result<String> {
        let profiler = ProjectProfiler {
            inner: ProjectProfiler::analyze(cwd),
        };
        let needs_regen = self.project_memory.needs_regen(cwd, &profiler.inner)?;
        let summary = if needs_regen {
            let rendered = profiler.render_summary();
            self.project_memory
                .upsert_summary(&profiler.inner, &rendered)
                .ok();
            rendered
        } else {
            self.project_memory
                .load(cwd)?
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
        Ok(summary)
    }
}

/// Builds the default registry with all core harness coding tools.
pub fn build_default_registry() -> ToolRegistry {
    use crate::harness::tool::{
        bash::BashTool,
        edit::EditTool,
        fetch_webpage::FetchWebpageTool,
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
        .register(Arc::new(TodoReadTool))
        .register(Arc::new(TodoWriteTool))
        .register(Arc::new(QuestionTool))
        .register(Arc::new(TaskTool))
        .register(Arc::new(RememberTool))
        .register(Arc::new(FetchWebpageTool))
        .register(Arc::new(WebSearchTool))
        .build()
}

/// Runs a subagent in a fresh child session, returning its final summary.
pub struct TaskRunner {
    pub runtime: Arc<SessionRuntime>,
}

#[async_trait::async_trait]
impl SubagentRunner for TaskRunner {
    async fn run_task(&self, agent: String, prompt: String) -> Result<String, String> {
        // Resolve agent to allow "explore" by default.
        let agent = if agent.is_empty() { "explore" } else { &agent };
        let mut child = self
            .runtime
            .store
            .create_session(agent, &self.runtime_current_cwd())
            .map_err(|e| e.to_string())?;
        child.agent = agent.to_string();
        let child_cwd = child.cwd.clone();

        let (tx, _rx) = crate::harness::event::event_channel();
        let result = self
            .runtime
            .prompt(
                &mut child,
                &tx,
                &prompt,
                crate::harness::tool::context::AbortSignal::new(),
                None,
            )
            .await
            .map_err(|e| e.to_string())?;

        let _ = self.runtime.store.delete_session(&child.id, &child_cwd);
        Ok(result.final_text)
    }
}

impl TaskRunner {
    fn runtime_current_cwd(&self) -> PathBuf {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }
}

#[cfg(test)]
mod smoke_tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    #[ignore = "requires a live token in the auth store (~/.local/share/rustclaw/auth.json)"]
    async fn smoke_native_tool_calling() {
        let config = crate::config::Config::load();
        assert!(
            config.is_configured(),
            "run the TUI once with /models + /auth to store a token before this smoke test"
        );
        let registry = crate::harness::runtime::build_default_registry();
        let db = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/smoke.db");
        let _ = std::fs::remove_file(&db);
        let asker = Arc::new(crate::harness::ui::cli::CliAsker::new(Arc::new(
            crate::harness::permission::PermissionEngine::default(),
        )));
        let runtime = SessionRuntime::from_legacy(
            &config,
            registry,
            &db,
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
            client: reqwest::Client::new(),
            base_url: "https://api.deepinfra.com/v1/openai".to_string(),
            api_key: "sk-initial-test-key-123456".to_string(),
        };
        let provider = build_provider_from("deepinfra", http)?;
        let db = dir.join("test.db");
        SessionRuntime::new(
            provider,
            ToolRegistry::builder().build(),
            HarnessConfig {
                model: "deepseek-ai/DeepSeek-V4-Flash-0731".to_string(),
                provider: "deepinfra".to_string(),
                base_url: "https://api.deepinfra.com/v1/openai".to_string(),
                api_key: "sk-initial-test-key-123456".to_string(),
                ..Default::default()
            },
            &db,
            Arc::new(AllowAsker),
            Arc::new(NoUserAsker),
        )
    }

    #[tokio::test]
    async fn test_switch_model_updates_config_and_project_file() {
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

        // Selection persisted into the project file.
        let proj = crate::harness::project::config_file::ProjectConfig::load(dir.path());
        assert_eq!(proj.provider, "moonshot");
        assert_eq!(proj.model, "kimi-k2.5");
        assert_eq!(proj.base_url, "https://api.moonshot.ai/v1");
    }

    #[tokio::test]
    async fn test_switch_model_keeps_key_when_auth_store_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = test_runtime(dir.path()).unwrap();
        rt.switch_model_at(dir.path(), "openrouter", "z-ai/glm-4.6")
            .unwrap();
        assert_eq!(rt.config.provider, "openrouter");
        assert_eq!(rt.config.model, "z-ai/glm-4.6");
        assert_eq!(rt.config.api_key, "sk-initial-test-key-123456");
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
    async fn test_onboarding_becomes_configured_after_token() {
        // Simulate a fresh unconfigured runtime (no token).
        let dir = tempfile::tempdir().unwrap();
        let http = HttpConfig {
            client: reqwest::Client::new(),
            base_url: "https://api.deepinfra.com/v1/openai".to_string(),
            api_key: String::new(),
        };
        let provider = build_provider_from("deepinfra", http).unwrap();
        let db = dir.path().join("test.db");
        let mut rt = SessionRuntime::new(
            provider,
            ToolRegistry::builder().build(),
            HarnessConfig {
                model: String::new(),
                provider: String::new(),
                base_url: "https://api.deepinfra.com/v1/openai".to_string(),
                api_key: String::new(),
                ..Default::default()
            },
            &db,
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
