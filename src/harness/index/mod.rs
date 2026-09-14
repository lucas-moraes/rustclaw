//! Semantic code search: symbol-aware chunking, a persistent SQLite index
//! (BM25 + optional embeddings) and hybrid retrieval.
//!
//! The index is opt-in and degrades gracefully: with no embedding backend
//! configured, search is pure BM25 (FTS5) over symbol chunks. When an
//! OpenAI-compatible embeddings endpoint is configured, vectors are stored
//! alongside the chunks and search becomes hybrid (BM25 + cosine).

pub mod chunk;
pub mod embed;
pub mod indexer;
pub mod search;
pub mod store;

pub use embed::{ApiEmbedder, EmbedConfig, Embedder, NullEmbedder};
pub use indexer::index_project;
pub use search::search;
pub use store::SemanticIndex;

use std::path::Path;
use std::sync::Arc;

/// Builds the embedder for a project from its config.
///
/// Resolution order:
/// 1. `rustclaw.json` `embeddings` section (per-project).
/// 2. `RUSTCLAW_EMBED_*` env vars (base_url/api_key/model).
/// 3. No backend → [`NullEmbedder`] (BM25-only search).
pub fn build_embedder(cwd: &Path) -> Arc<dyn Embedder> {
    if let Some(cfg) = embed_config_for(cwd) {
        return Arc::new(ApiEmbedder::new(cfg));
    }
    Arc::new(NullEmbedder)
}

/// Resolves an [`EmbedConfig`] from project config or env, if any.
fn embed_config_for(cwd: &Path) -> Option<EmbedConfig> {
    let proj = crate::harness::project::config_file::ProjectConfig::load(cwd);
    if let Some(e) = proj.embeddings {
        if !e.base_url.is_empty() && !e.model.is_empty() {
            return Some(e);
        }
    }
    let base_url = std::env::var("RUSTCLAW_EMBED_BASE_URL").ok()?;
    let model = std::env::var("RUSTCLAW_EMBED_MODEL").ok()?;
    if base_url.is_empty() || model.is_empty() {
        return None;
    }
    Some(EmbedConfig {
        base_url,
        api_key: std::env::var("RUSTCLAW_EMBED_API_KEY").unwrap_or_default(),
        model,
        dim: std::env::var("RUSTCLAW_EMBED_DIM")
            .ok()
            .and_then(|d| d.parse().ok())
            .unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_config_yields_null_embedder() {
        let dir = tempfile::tempdir().unwrap();
        // Ensure env vars don't leak into the test.
        std::env::remove_var("RUSTCLAW_EMBED_BASE_URL");
        std::env::remove_var("RUSTCLAW_EMBED_MODEL");
        let e = build_embedder(dir.path());
        assert_eq!(e.dim(), 0);
        assert_eq!(e.name(), "none");
    }
}
