//! `/index` slash command: build/refresh the semantic code index and run
//! ad-hoc hybrid searches against it.
//!
//! Usage:
//!   /index            — (re)build the index for the current project
//!   /index status     — show index statistics
//!   /index search <q> — run a hybrid search and print the top hits

use crate::harness::runtime::SessionRuntime;
use anyhow::Result;
use std::path::Path;

/// Handles `/index [status|search <q>]`, returning a single feedback line.
pub async fn handle_index_command(runtime: &SessionRuntime, args: &[&str]) -> Result<String> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    render_index_command(runtime, &cwd, args).await
}

/// Core implementation, split out for testability with an explicit project root.
async fn render_index_command(
    runtime: &SessionRuntime,
    cwd: &Path,
    args: &[&str],
) -> Result<String> {
    let Some(index) = runtime.semantic_index.as_ref() else {
        return Ok("semantic index is unavailable in this session".to_string());
    };
    let Some(embedder) = runtime.embedder.as_ref() else {
        return Ok("no embeddings backend configured".to_string());
    };

    match args {
        [] | ["build"] | ["rebuild"] => {
            let stats = crate::harness::index::index_project(index, embedder.as_ref(), cwd).await?;
            let mode = if embedder.dim() > 0 {
                format!("hybrid (embeddings: {})", embedder.name())
            } else {
                "BM25-only (no embeddings backend)".to_string()
            };
            Ok(format!(
                "index: {} file(s) indexed, {} skipped, {} removed, {} chunk(s), {} embedded · {mode}",
                stats.files_indexed,
                stats.files_skipped,
                stats.files_removed,
                stats.chunks,
                stats.embedded,
            ))
        }
        ["status"] => {
            let files = index.file_count(cwd)?;
            let chunks = index.chunk_count(cwd)?;
            let embedded = index.embedded_count(cwd)?;
            let mode = if embedder.dim() > 0 {
                format!("hybrid (embeddings: {})", embedder.name())
            } else {
                "BM25-only (no embeddings backend)".to_string()
            };
            Ok(format!(
                "index status: {files} file(s), {chunks} chunk(s), {embedded} embedded · {mode}"
            ))
        }
        ["search", rest @ ..] => {
            let q = rest.join(" ");
            if q.trim().is_empty() {
                return Ok("usage: /index search <query>".to_string());
            }
            let hits = crate::harness::index::search(index, embedder.as_ref(), cwd, &q, 8).await?;
            if hits.is_empty() {
                return Ok(format!(
                    "no index matches for \"{q}\" (run /index to build)"
                ));
            }
            let mut out = format!("index matches for \"{q}\" ({}):", hits.len());
            for (i, h) in hits.iter().enumerate() {
                let symbol = h.symbol.as_deref().unwrap_or("<chunk>");
                out.push_str(&format!(
                    "\n  {}. {}:{}-{} [{}] score={:.3}",
                    i + 1,
                    h.path,
                    h.start_line,
                    h.end_line,
                    symbol,
                    h.score,
                ));
            }
            Ok(out)
        }
        _ => Ok("usage: /index [status] [search <query>]".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct TestRuntime {
        runtime: SessionRuntime,
        _dir: tempfile::TempDir,
    }

    fn test_runtime() -> TestRuntime {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let runtime = SessionRuntime::new_in(
            dir.path(),
            crate::harness::provider::opencode_go::build_provider(
                "opencode-go",
                crate::harness::provider::HttpConfig {
                    client: crate::harness::provider::build_http_client(),
                    base_url: "http://localhost:9".to_string(),
                    api_key: "x".to_string(),
                },
                false,
            )
            .unwrap(),
            crate::harness::tool::registry::ToolRegistry::builder().build(),
            crate::config::RuntimeConfig::default(),
            &db,
            Arc::new(crate::harness::permission::PermissionEngine::default()),
            Arc::new(AllowAsker),
            Arc::new(NoUserAsker),
        )
        .unwrap();
        TestRuntime { runtime, _dir: dir }
    }

    struct AllowAsker;
    struct NoUserAsker;

    #[async_trait::async_trait]
    impl crate::harness::tool::context::PermissionAsker for AllowAsker {
        async fn ask(&self, _req: crate::harness::tool::context::PermissionAskInput) -> bool {
            true
        }
    }

    #[async_trait::async_trait]
    impl crate::harness::tool::context::UserAsker for NoUserAsker {
        async fn ask(&self, _q: String, _o: Vec<String>) -> Option<String> {
            None
        }
    }

    #[tokio::test]
    async fn test_index_builds_and_reports() {
        let tr = test_runtime();
        let cwd = tr._dir.path();
        std::fs::write(cwd.join("a.rs"), "pub fn foo() {}\n").unwrap();
        let out = render_index_command(&tr.runtime, cwd, &[]).await.unwrap();
        assert!(out.contains("1 file(s) indexed"), "got: {out}");
        assert!(out.contains("BM25-only"), "got: {out}");
    }

    #[tokio::test]
    async fn test_index_status_reports_counts() {
        let tr = test_runtime();
        let cwd = tr._dir.path();
        std::fs::write(cwd.join("a.rs"), "pub fn foo() {}\n").unwrap();
        render_index_command(&tr.runtime, cwd, &[]).await.unwrap();
        let out = render_index_command(&tr.runtime, cwd, &["status"])
            .await
            .unwrap();
        assert!(out.contains("1 file(s)"), "got: {out}");
        assert!(out.contains("chunk(s)"), "got: {out}");
    }

    #[tokio::test]
    async fn test_index_search_finds_symbol() {
        let tr = test_runtime();
        let cwd = tr._dir.path();
        std::fs::write(cwd.join("a.rs"), "pub fn backoff_delay() {}\n").unwrap();
        render_index_command(&tr.runtime, cwd, &[]).await.unwrap();
        let out = render_index_command(&tr.runtime, cwd, &["search", "backoff"])
            .await
            .unwrap();
        assert!(out.contains("a.rs"), "got: {out}");
        assert!(out.contains("backoff_delay"), "got: {out}");
    }

    #[tokio::test]
    async fn test_index_search_empty_query_usage() {
        let tr = test_runtime();
        let out = render_index_command(&tr.runtime, tr._dir.path(), &["search"])
            .await
            .unwrap();
        assert!(out.contains("usage: /index search"), "got: {out}");
    }

    #[tokio::test]
    async fn test_index_unknown_usage() {
        let tr = test_runtime();
        let out = render_index_command(&tr.runtime, tr._dir.path(), &["bogus"])
            .await
            .unwrap();
        assert!(out.contains("usage: /index"), "got: {out}");
    }

    /// Smoke test on the real repository: index it and search for a known
    /// symbol. Ignored by default (slow, depends on the checkout); run with
    /// `cargo test --bin rustclaw test_index_real_project -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn test_index_real_project() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        let runtime = SessionRuntime::new_in(
            root,
            crate::harness::provider::opencode_go::build_provider(
                "opencode-go",
                crate::harness::provider::HttpConfig {
                    client: crate::harness::provider::build_http_client(),
                    base_url: "http://localhost:9".to_string(),
                    api_key: "x".to_string(),
                },
                false,
            )
            .unwrap(),
            crate::harness::tool::registry::ToolRegistry::builder().build(),
            crate::config::RuntimeConfig::default(),
            &db,
            Arc::new(crate::harness::permission::PermissionEngine::default()),
            Arc::new(AllowAsker),
            Arc::new(NoUserAsker),
        )
        .unwrap();

        let out = render_index_command(&runtime, root, &[]).await.unwrap();
        assert!(out.contains("file(s) indexed"), "got: {out}");
        // The repo has hundreds of .rs files; the index must not be empty.
        let status = render_index_command(&runtime, root, &["status"])
            .await
            .unwrap();
        assert!(!status.contains("0 file(s)"), "got: {status}");

        let hits = render_index_command(&runtime, root, &["search", "doom loop detector"])
            .await
            .unwrap();
        assert!(hits.contains("doom_loop"), "got: {hits}");
    }
}
