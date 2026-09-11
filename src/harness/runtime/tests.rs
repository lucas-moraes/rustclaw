#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::super::*;
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

    use super::super::task_runner::resolve_subagent_agent;

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
