//! Hybrid search over the semantic index: BM25 (lexical) + cosine (vector).
//!
//! Results from both backends are merged by chunk id and scored with a
//! weighted combination. When no embeddings are available the vector half is
//! empty and the result is pure BM25, so search always works.

use std::path::Path;

use anyhow::Result;

use super::embed::{cosine, Embedder};
use super::store::{IndexedChunk, SemanticIndex};

/// Weight of the BM25 (lexical) signal in the hybrid score.
const W_BM25: f32 = 0.5;
/// Weight of the cosine (semantic) signal in the hybrid score.
const W_COSINE: f32 = 0.5;

/// A ranked search hit.
#[derive(Clone, Debug)]
pub struct Hit {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub symbol: Option<String>,
    pub kind: Option<String>,
    pub text: String,
    /// Combined hybrid score (higher = better).
    pub score: f32,
    /// Normalized BM25 component (0..1).
    pub bm25: f32,
    /// Cosine component (0..1).
    pub cosine: f32,
}

/// Runs a hybrid search. `k` is the number of hits to return.
pub async fn search(
    index: &SemanticIndex,
    embedder: &dyn Embedder,
    cwd: &Path,
    query: &str,
    k: usize,
) -> Result<Vec<Hit>> {
    // Lexical half: BM25 over the FTS index. Fetch more than `k` so the merge
    // has room to reorder.
    let fetch = (k * 4).max(20);
    let bm25_hits = index.search_bm25(cwd, query, fetch)?;

    // Semantic half: embed the query, then cosine against every embedded chunk.
    let mut vector_hits: Vec<(IndexedChunk, f32)> = Vec::new();
    if embedder.dim() > 0 {
        let qvec = embedder.embed(&[query.to_string()]).await?;
        if let Some(qv) = qvec.first() {
            for (chunk, vec) in index.embedded_chunks(cwd)? {
                let sim = cosine(qv, &vec);
                if sim > 0.0 {
                    vector_hits.push((chunk, sim));
                }
            }
        }
    }

    // Merge by chunk id, keeping the best of each signal.
    let mut merged: std::collections::HashMap<i64, Hit> = std::collections::HashMap::new();
    for c in bm25_hits {
        let norm = normalize_bm25(c.bm25.unwrap_or(0.0));
        merged.insert(
            c.id,
            Hit {
                path: c.path,
                start_line: c.start_line,
                end_line: c.end_line,
                symbol: c.symbol,
                kind: c.kind,
                text: c.text,
                score: 0.0,
                bm25: norm,
                cosine: 0.0,
            },
        );
    }
    for (c, sim) in vector_hits {
        let entry = merged.entry(c.id).or_insert_with(|| Hit {
            path: c.path.clone(),
            start_line: c.start_line,
            end_line: c.end_line,
            symbol: c.symbol.clone(),
            kind: c.kind.clone(),
            text: c.text.clone(),
            score: 0.0,
            bm25: 0.0,
            cosine: 0.0,
        });
        entry.cosine = sim.clamp(0.0, 1.0);
    }

    let mut hits: Vec<Hit> = merged.into_values().collect();
    for h in &mut hits {
        h.score = W_BM25 * h.bm25 + W_COSINE * h.cosine;
    }
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(k);
    Ok(hits)
}

/// Normalizes an FTS5 BM25 rank (lower = better, typically ≤ 0) to 0..1.
fn normalize_bm25(rank: f64) -> f32 {
    ((1.0 + rank / 10.0).clamp(0.0, 1.0)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::index::embed::NullEmbedder;
    use crate::harness::index::store::chunks_for;

    fn setup() -> (tempfile::TempDir, SemanticIndex) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        let idx = SemanticIndex::open(&db).unwrap();
        idx.ensure_project(dir.path()).unwrap();
        let src = "pub fn parse_config() {}\npub fn render_ui() {}\n";
        let chunks = chunks_for(Path::new("a.rs"), src);
        idx.replace_file(dir.path(), "a.rs", "h1", &chunks, &[])
            .unwrap();
        (dir, idx)
    }

    #[tokio::test]
    async fn test_bm25_only_search_works_without_embeddings() {
        let (dir, idx) = setup();
        let hits = search(&idx, &NullEmbedder, dir.path(), "parse_config", 5)
            .await
            .unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].text.contains("parse_config"));
        assert_eq!(hits[0].cosine, 0.0);
    }

    #[tokio::test]
    async fn test_empty_query_returns_nothing() {
        let (dir, idx) = setup();
        let hits = search(&idx, &NullEmbedder, dir.path(), "!!!", 5)
            .await
            .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn test_normalize_bm25_bounds() {
        assert_eq!(normalize_bm25(0.0), 1.0);
        assert_eq!(normalize_bm25(-10.0), 0.0);
        assert_eq!(normalize_bm25(-20.0), 0.0);
    }
}
