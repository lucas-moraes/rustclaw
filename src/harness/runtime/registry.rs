//! Default tool registry construction.

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
        .register(Arc::new(CursorTool))
        .build()
}

/// Applies the `cursor_agent` kill-switch to a registry: when the toggle is
/// off, the `cursor` tool is removed so no agent can call it.
pub fn apply_cursor_toggle(registry: ToolRegistry, cursor_agent: bool) -> ToolRegistry {
    if cursor_agent {
        // Re-register the tool so it comes back after a previous "off": the
        // kill-switch (`without_tool`) permanently drops it, so simply keeping
        // the registry would leave the `cursor` agent with zero tool specs.
        registry.with_tool(Arc::new(crate::harness::tool::cursor::CursorTool))
    } else {
        registry.without_tool("cursor")
    }
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
        let reg = apply_cursor_toggle(reg, false);
        assert!(
            reg.specs(&["cursor".to_string()]).is_empty(),
            "toggle off must remove the cursor tool"
        );

        // Toggle ON again: the tool must come back.
        let reg = apply_cursor_toggle(reg, true);
        let specs = reg.specs(&["cursor".to_string()]);
        assert_eq!(
            specs.len(),
            1,
            "toggle on must restore exactly one cursor tool spec"
        );
    }

    /// The `cursor` agent resolves to exactly one tool spec when the toggle is
    /// on, and zero when off (kill-switch).
    #[test]
    fn test_cursor_agent_tool_specs_follow_toggle() {
        let agent = find_builtin("cursor").expect("cursor agent exists");

        let on = apply_cursor_toggle(build_default_registry(), true);
        assert_eq!(on.specs(&agent.tools).len(), 1);

        let off = apply_cursor_toggle(build_default_registry(), false);
        assert!(off.specs(&agent.tools).is_empty());
    }
}
