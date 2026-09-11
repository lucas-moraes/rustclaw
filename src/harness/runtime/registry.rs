//! Default tool registry construction.

use crate::harness::tool::registry::ToolRegistry;
use std::sync::Arc;

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
