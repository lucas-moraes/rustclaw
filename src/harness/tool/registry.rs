//! Registry of harness tools; exports specs for provider requests.

use crate::harness::tool::context::ToolContext;
use crate::harness::tool::{Tool, ToolResult, ToolSpec};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: Arc<BTreeMap<String, Arc<dyn Tool>>>,
}

impl ToolRegistry {
    /// Empty registry. Convenience over `Default`; kept for API completeness.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn builder() -> ToolRegistryBuilder {
        ToolRegistryBuilder::default()
    }

    /// Returns a copy of the registry with an extra tool registered
    /// (used to inject MCP tools after the initial build).
    pub fn with_tool(&self, tool: Arc<dyn Tool>) -> Self {
        let mut tools = (*self.tools).clone();
        tools.insert(tool.name().to_string(), tool);
        ToolRegistry {
            tools: Arc::new(tools),
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Whether a tool is registered. Only used by tests.
    #[cfg(test)]
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// All registered tool names. Public API; kept for completeness.
    #[allow(dead_code)]
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Specs filtered by an allowlist (empty = all).
    /// The special entry `mcp_readonly` admits MCP tools annotated with
    /// `readOnlyHint: true` (used by readonly agents like plan/explore).
    pub fn specs(&self, allow: &[String]) -> Vec<ToolSpec> {
        let mcp_readonly = allow.iter().any(|a| a == "mcp_readonly");
        self.tools
            .values()
            .filter(|t| {
                allow.is_empty()
                    || allow.iter().any(|a| a == t.name())
                    || (mcp_readonly
                        && t.name().starts_with("mcp_")
                        && is_readonly_tool(t.as_ref()))
            })
            .map(|t| ToolSpec::from_tool(t.as_ref()))
            .collect()
    }

    pub async fn execute(
        &self,
        name: &str,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult, String> {
        let tool = self
            .get(name)
            .ok_or_else(|| format!("unknown tool `{}`", name))?;
        tool.execute(args, ctx).await
    }
}

/// True when the tool is an MCP tool annotated `readOnlyHint: true`.
fn is_readonly_tool(tool: &dyn Tool) -> bool {
    tool.name().starts_with("mcp_") && tool.read_only()
}

#[derive(Default)]
pub struct ToolRegistryBuilder {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistryBuilder {
    pub fn register(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.insert(tool.name().to_string(), tool);
        self
    }

    pub fn build(self) -> ToolRegistry {
        ToolRegistry {
            tools: Arc::new(self.tools),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serde_json::Value;

    struct EchoTool;

    #[async_trait::async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echoes input"
        }
        fn parameters(&self) -> Value {
            json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            })
        }
        async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, String> {
            Ok(ToolResult::simple(
                "echo",
                args["text"].as_str().unwrap_or(""),
            ))
        }
    }

    #[test]
    fn test_registry_register_and_specs() {
        let registry = ToolRegistry::builder().register(Arc::new(EchoTool)).build();
        assert!(registry.contains("echo"));
        let specs = registry.specs(&[]);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "echo");
        assert_eq!(specs[0].parameters["type"], "object");

        let filtered = registry.specs(&["other".to_string()]);
        assert!(filtered.is_empty());
    }

    struct McpEchoTool;

    #[async_trait::async_trait]
    impl Tool for McpEchoTool {
        fn name(&self) -> &str {
            "mcp_fs_read"
        }
        fn description(&self) -> &str {
            "mcp read"
        }
        fn parameters(&self) -> Value {
            json!({"type": "object"})
        }
        fn read_only(&self) -> bool {
            true
        }
        async fn execute(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolResult, String> {
            Ok(ToolResult::simple("mcp", ""))
        }
    }

    struct McpWriteTool;

    #[async_trait::async_trait]
    impl Tool for McpWriteTool {
        fn name(&self) -> &str {
            "mcp_fs_write"
        }
        fn description(&self) -> &str {
            "mcp write"
        }
        fn parameters(&self) -> Value {
            json!({"type": "object"})
        }
        async fn execute(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolResult, String> {
            Ok(ToolResult::simple("mcp", ""))
        }
    }

    #[test]
    fn test_specs_mcp_readonly_marker() {
        let registry = ToolRegistry::builder()
            .register(Arc::new(McpEchoTool))
            .register(Arc::new(McpWriteTool))
            .build();
        let allow = ["mcp_readonly".to_string()];
        let specs = registry.specs(&allow);
        // Only the readonly MCP tool is admitted.
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "mcp_fs_read");
    }

    #[tokio::test]
    async fn test_registry_execute() {
        let registry = ToolRegistry::builder().register(Arc::new(EchoTool)).build();
        let ctx = test_context();
        let result = registry
            .execute("echo", json!({"text": "hi"}), &ctx)
            .await
            .unwrap();
        assert_eq!(result.output, "hi");
        let err = registry.execute("nope", json!({}), &ctx).await.unwrap_err();
        assert!(err.contains("unknown tool"));
    }

    fn test_context() -> ToolContext {
        use crate::harness::permission::PermissionEngine;
        ToolContext {
            session_id: "s".into(),
            agent: "build".into(),
            agent_tools: vec![],
            cwd: crate::harness::tool::context::PathBufGuard(std::path::PathBuf::from("/tmp")),
            abort: crate::harness::tool::context::AbortSignal::new(),
            permission: Arc::new(PermissionEngine::default()),
            asker: Arc::new(tests_helper::AllowAsker),
            user_asker: Arc::new(tests_helper::NoUserAsker),
            todos: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            task_runner: None,
            events: crate::harness::event::event_channel().0,
            project_memory: None,
            hooks: Default::default(),
            checkpoints: std::sync::Arc::new(
                crate::harness::tool::checkpoint::FileCheckpoints::new(),
            ),
            jobs: std::sync::Arc::new(crate::harness::tool::jobs::JobRegistry::new()),
        }
    }

    /// Only the build agent may run mutating tools — a hallucinated tool call
    /// is refused at execution time even if the LLM advertises it.
    #[tokio::test]
    async fn test_agent_allowlist_blocks_mutating_tools() {
        let mut ctx = test_context();
        ctx.agent = "general".into();
        ctx.agent_tools = vec!["read".to_string(), "glob".to_string(), "grep".to_string()];
        let err = ctx
            .check_permission("edit", &json!({"path": "/tmp/x"}))
            .await;
        assert!(err.is_err());
        assert!(err
            .unwrap_err()
            .contains("not available to the `general` agent"));

        // Allowed tools pass the gate (permission engine allows reads).
        let ok = ctx
            .check_permission("read", &serde_json::json!({"path": "/tmp/x"}))
            .await;
        assert!(ok.is_ok());

        // Build (empty allowlist) keeps full access.
        let ctx_build = test_context();
        assert!(ctx_build
            .check_permission("edit", &serde_json::json!({"path": "/tmp/x"}))
            .await
            .is_ok());
    }

    mod tests_helper {
        use crate::harness::tool::context::{PermissionAskInput, PermissionAsker, UserAsker};

        pub struct AllowAsker;
        pub struct NoUserAsker;

        #[async_trait::async_trait]
        impl PermissionAsker for AllowAsker {
            async fn ask(&self, _req: PermissionAskInput) -> bool {
                true
            }
        }

        #[async_trait::async_trait]
        impl UserAsker for NoUserAsker {
            async fn ask(&self, _q: String, _o: Vec<String>) -> Option<String> {
                None
            }
        }
    }
}
