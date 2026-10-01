//! Default tool registry construction.

use crate::config::RuntimeConfig;
use crate::harness::tool::registry::ToolRegistry;
use std::sync::Arc;

/// Builds the default registry with all core harness coding tools.
///
/// The `cursor` tool is always registered here; the `cursor_agent` kill-switch
/// is applied by [`apply_cursor_toggle`] (removes it when the toggle is off).
pub fn build_default_registry() -> ToolRegistry {
    use crate::harness::tool::{
        ast_search::AstSearchTool,
        bash::BashTool,
        cursor::CursorTool,
        diagnostics::DiagnosticsTool,
        edit::EditTool,
        fetch_webpage::FetchWebpageTool,
        git::{GitDiffTool, GitLogTool, GitStatusTool},
        glob::GlobTool,
        grep::GrepTool,
        question::QuestionTool,
        read::ReadTool,
        remember::RememberTool,
        semantic_search::SemanticSearchTool,
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
        .register(Arc::new(SemanticSearchTool))
        .register(Arc::new(FetchWebpageTool))
        .register(Arc::new(WebSearchTool::new()))
        .register(Arc::new(GitStatusTool))
        .register(Arc::new(GitDiffTool))
        .register(Arc::new(GitLogTool))
        .register(Arc::new(CursorTool::default()))
        .build()
}

/// Applies the Cursor kill-switches to a registry.
///
/// Two independent toggles:
/// - `cursor_agent` → the `cursor` tool (build mode, `--force`);
/// - `cursor_plan`  → the `cursor_plan` tool (read-only, `--mode plan`).
///
/// When a toggle is off its tool is removed so no agent can call it. The
/// models (`cursor_model` / `cursor_plan_model`) are forwarded as
/// `--model <id>`; empty means the Cursor CLI's own default ("auto").
pub fn apply_cursor_toggle(registry: ToolRegistry, config: &RuntimeConfig) -> ToolRegistry {
    use crate::harness::tool::cursor::{CursorMode, CursorTool};

    let mut registry = registry;
    if config.cursor_agent {
        // Re-register the tool so it comes back after a previous "off": the
        // kill-switch (`without_tool`) permanently drops it, so simply keeping
        // the registry would leave the `cursor` agent with zero tool specs.
        registry = registry.with_tool(Arc::new(CursorTool::new(&config.cursor_model)));
    } else {
        registry = registry.without_tool("cursor");
    }
    if config.cursor_plan {
        registry = registry.with_tool(Arc::new(CursorTool::with_mode(
            CursorMode::Plan,
            &config.cursor_plan_model,
        )));
    } else {
        registry = registry.without_tool("cursor_plan");
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::agent::find_builtin;

    /// Regression: toggling `cursor_agent` off then on at runtime must restore
    /// the `cursor` tool, so the `cursor` agent never ends up with zero tool
    /// specs (the `agent=cursor ... tools=0` symptom).
    #[test]
    fn test_cursor_toggle_off_then_on_restores_tool() {
        let reg = build_default_registry();
        assert!(
            !reg.specs(&["cursor".to_string()]).is_empty(),
            "default registry must include the cursor tool"
        );

        // Toggle OFF: the tool is removed.
        let reg = apply_cursor_toggle(reg, &cfg(false, "", false, ""));
        assert!(
            reg.specs(&["cursor".to_string()]).is_empty(),
            "toggle off must remove the cursor tool"
        );

        // Toggle ON again: the tool must come back.
        let reg = apply_cursor_toggle(reg, &cfg(true, "", false, ""));
        let specs = reg.specs(&["cursor".to_string()]);
        assert_eq!(
            specs.len(),
            1,
            "toggle on must restore exactly one cursor tool spec"
        );
    }

    /// Builds a `RuntimeConfig` with the four cursor knobs set.
    fn cfg(agent: bool, model: &str, plan: bool, plan_model: &str) -> RuntimeConfig {
        RuntimeConfig {
            cursor_agent: agent,
            cursor_model: model.to_string(),
            cursor_plan: plan,
            cursor_plan_model: plan_model.to_string(),
            ..Default::default()
        }
    }

    /// The plan toggle is independent: it adds `cursor_plan` and leaves
    /// `cursor` absent when the build toggle is off.
    #[test]
    fn test_cursor_plan_toggle_independent() {
        let plan_only = apply_cursor_toggle(build_default_registry(), &cfg(false, "", true, ""));
        assert!(plan_only.specs(&["cursor".to_string()]).is_empty());
        assert_eq!(plan_only.specs(&["cursor_plan".to_string()]).len(), 1);

        let both = apply_cursor_toggle(build_default_registry(), &cfg(true, "", true, ""));
        assert_eq!(both.specs(&["cursor".to_string()]).len(), 1);
        assert_eq!(both.specs(&["cursor_plan".to_string()]).len(), 1);

        let neither = apply_cursor_toggle(build_default_registry(), &cfg(false, "", false, ""));
        assert!(neither.specs(&["cursor".to_string()]).is_empty());
        assert!(neither.specs(&["cursor_plan".to_string()]).is_empty());
    }

    /// The `cursor` agent resolves to exactly one tool spec when the toggle is
    /// on, and zero when off (kill-switch).
    #[test]
    fn test_cursor_agent_tool_specs_follow_toggle() {
        let agent = find_builtin("cursor").expect("cursor agent exists");

        let on = apply_cursor_toggle(build_default_registry(), &cfg(true, "", false, ""));
        assert_eq!(on.specs(&agent.tools).len(), 1);

        let off = apply_cursor_toggle(build_default_registry(), &cfg(false, "", false, ""));
        assert!(off.specs(&agent.tools).is_empty());
    }
}
