//! Project context builders: frozen structural summary and the per-turn
//! `<project-memory>` block.

use super::SessionRuntime;
use crate::harness::project::ProjectProfiler;
use anyhow::Result;

/// Returns the frozen structural summary for a session, computing (and
/// persisting) it on first use. The summary is cached per session so the
/// system prompt stays byte-stable across turns — a prerequisite for
/// provider prompt caches (Anthropic `cache_control`, OpenAI automatic),
/// which require an identical prefix. Compaction invalidates the entry
/// (the context is rewritten anyway).
pub(super) fn frozen_summary_for<'a>(
    rt: &'a SessionRuntime,
    session_id: &'a str,
    cwd: &'a std::path::Path,
) -> impl std::future::Future<Output = Result<String>> + Send + 'a {
    let cached = rt
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
        let memory = rt.project_memory.clone();
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
            let memory = rt.project_memory.clone();
            let ctx = profiler.inner.clone();
            let rendered_clone = rendered.clone();
            let _ =
                tokio::task::spawn_blocking(move || memory.upsert_summary(&ctx, &rendered_clone))
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
        if let Ok(mut p) = rt.project.lock() {
            p.inner = profiler.inner;
        }
        rt.summary_cache
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
pub(super) fn memory_block_for<'a>(
    rt: &'a SessionRuntime,
    cwd: &'a std::path::Path,
    query: &str,
) -> impl std::future::Future<Output = Result<String>> + Send + 'a {
    let cwd = cwd.to_path_buf();
    let query = query.to_string();
    async move {
        let memory = rt.project_memory.clone();
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
            let memory = rt.project_memory.clone();
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
