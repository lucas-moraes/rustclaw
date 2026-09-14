//! `semantic_search` tool: hybrid (BM25 + optional embeddings) retrieval over
//! the project's symbol-aware code index. Read-only.
//!
//! The index is built by the `/index` command (or lazily on first use). When no
//! embedding backend is configured, search degrades to pure BM25 over symbol
//! chunks — still far better than a raw grep for "where is X implemented?".

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};

/// Default number of results when `k` is omitted.
const DEFAULT_K: usize = 8;
/// Hard cap on `k` (protects the model context).
const MAX_K: usize = 50;
/// Max chars of a chunk's source shown per hit.
const MAX_SNIPPET_CHARS: usize = 1200;

pub struct SemanticSearchTool;

#[async_trait::async_trait]
impl Tool for SemanticSearchTool {
    fn name(&self) -> &str {
        "semantic_search"
    }

    fn description(&self) -> &str {
        "Busca semântica no código do projeto (índice híbrido BM25 + embeddings). \
Retorna trechos de código relevantes com arquivo, linhas e símbolo. Use para \
perguntas do tipo \"onde X é implementado?\" ou \"como funciona Y?\" quando não \
souber o nome exato do símbolo. Requer que o índice esteja construído (/index)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Consulta em linguagem natural ou termos de código (ex: \"onde o retry do provider é configurado?\")"
                },
                "k": {
                    "type": "integer",
                    "description": "Número máximo de resultados (padrão 8, máx 50)"
                }
            },
            "required": ["query"]
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let query = args["query"]
            .as_str()
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .ok_or_else(|| "missing required argument: query".to_string())?;
        let k = args["k"]
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_K)
            .clamp(1, MAX_K);

        let index = ctx
            .semantic_index
            .as_ref()
            .ok_or_else(|| "semantic index is unavailable in this session".to_string())?;
        let embedder = ctx
            .embedder
            .as_ref()
            .ok_or_else(|| "no embeddings backend configured".to_string())?;

        let hits = crate::harness::index::search(index, embedder.as_ref(), &ctx.cwd.0, query, k)
            .await
            .map_err(|e| format!("semantic search failed: {e}"))?;

        if hits.is_empty() {
            return Ok(ToolResult::simple(
                format!("semantic_search {}", preview(query, 40)),
                format!(
                    "No indexed chunks matched `{query}`. The index may be empty or stale — \
                     run `/index` to (re)build it."
                ),
            ));
        }

        let mut out = String::new();
        for (i, hit) in hits.iter().enumerate() {
            let symbol = hit.symbol.as_deref().unwrap_or("<chunk>");
            let kind = hit.kind.as_deref().unwrap_or("");
            out.push_str(&format!(
                "{}. {}:{}-{}  [{} {}]  score={:.3} (bm25={:.2} cos={:.2})\n",
                i + 1,
                hit.path,
                hit.start_line,
                hit.end_line,
                kind,
                symbol,
                hit.score,
                hit.bm25,
                hit.cosine,
            ));
            out.push_str(&preview(&hit.text, MAX_SNIPPET_CHARS));
            out.push_str("\n\n");
        }

        Ok(ToolResult::simple(
            format!(
                "semantic_search {} ({} hits)",
                preview(query, 40),
                hits.len()
            ),
            out.trim_end().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::permission::PermissionEngine;
    use crate::harness::tool::context::ToolContext;
    use std::sync::Arc;

    struct AllowAsker;
    #[async_trait::async_trait]
    impl crate::harness::tool::context::PermissionAsker for AllowAsker {
        async fn ask(&self, _: crate::harness::tool::context::PermissionAskInput) -> bool {
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

    fn ctx_with_cwd(cwd: std::path::PathBuf) -> ToolContext {
        ToolContext {
            session_id: "s".into(),
            agent: "build".into(),
            agent_tools: vec![],
            cwd: crate::harness::tool::context::PathBufGuard(cwd),
            abort: crate::harness::tool::context::AbortSignal::new(),
            permission: Arc::new(PermissionEngine::default()),
            asker: Arc::new(AllowAsker),
            user_asker: Arc::new(NoUserAsker),
            todos: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            task_runner: None,
            depth: 0,
            events: crate::harness::event::event_channel().0,
            project_memory: None,
            hooks: Default::default(),
            checkpoints: std::sync::Arc::new(
                crate::harness::tool::checkpoint::FileCheckpoints::new(),
            ),
            jobs: std::sync::Arc::new(crate::harness::tool::jobs::JobRegistry::new()),
            semantic_index: None,
            embedder: None,
        }
    }

    #[test]
    fn test_schema_requires_query() {
        let t = SemanticSearchTool;
        let p = t.parameters();
        assert_eq!(p["required"][0], "query");
        assert!(t.read_only());
        assert_eq!(t.name(), "semantic_search");
    }

    #[tokio::test]
    async fn test_missing_query_errors() {
        let ctx = ctx_with_cwd(std::path::PathBuf::from("/tmp"));
        let err = SemanticSearchTool
            .execute(json!({}), &ctx)
            .await
            .unwrap_err();
        assert!(err.contains("query"), "got: {err}");
    }

    #[tokio::test]
    async fn test_no_index_reports_unavailable() {
        let ctx = ctx_with_cwd(std::path::PathBuf::from("/tmp"));
        let err = SemanticSearchTool
            .execute(json!({ "query": "retry" }), &ctx)
            .await
            .unwrap_err();
        assert!(err.contains("unavailable"), "got: {err}");
    }

    /// End-to-end: build a real index over a temp project and search it.
    #[tokio::test]
    async fn test_search_finds_indexed_symbol() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("retry.rs"),
            "pub struct RetryPolicy {\n    pub max_attempts: usize,\n}\n\n\
             pub fn backoff_delay(attempt: usize) -> u64 {\n    attempt as u64 * 100\n}\n",
        )
        .unwrap();

        let db = dir.path().join("index.db");
        let index = crate::harness::index::SemanticIndex::open(&db).unwrap();
        let embedder = crate::harness::index::NullEmbedder;
        crate::harness::index::index_project(&index, &embedder, dir.path())
            .await
            .unwrap();

        let mut ctx = ctx_with_cwd(dir.path().to_path_buf());
        ctx.semantic_index = Some(Arc::new(index));
        ctx.embedder = Some(Arc::new(embedder));

        let res = SemanticSearchTool
            .execute(json!({ "query": "backoff delay retry" }), &ctx)
            .await
            .unwrap();
        assert!(
            res.output.contains("retry.rs"),
            "expected retry.rs in output, got: {}",
            res.output
        );
        assert!(
            res.output.contains("backoff_delay"),
            "expected symbol in output, got: {}",
            res.output
        );
    }
}
