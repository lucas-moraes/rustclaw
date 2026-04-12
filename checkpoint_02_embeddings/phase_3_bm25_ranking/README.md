# Phase 3: BM25 Secondary Ranking

**Date:** 2026-04-12
**Status:** ✅ Complete

## Summary

Created BM25 ranking infrastructure for text-based similarity scoring as a secondary signal.

## Created Files
- `src/memory/bm25.rs` - BM25 scorer implementation

## Implementation Details

### BM25 vs TF-IDF
- **TF-IDF (fallback embeddings)**: Good for semantic similarity based on word co-occurrence
- **BM25**: Better for exact term matching and relevance ranking

### BM25 Score
```rust
pub struct Bm25Score {
    idf_cache: HashMap<String, f32>,
    avg_doc_len: f32,
    k1: f32,  // Term frequency saturation parameter
    b: f32,    # Document length normalization parameter
}
```

### Key Formula
```
score = IDF(t) * (tf * (k1 + 1)) / (tf + k1 * (1 - b + b * |d|/avgdl))
```

### Tokenization
Uses same stopword list as TF-IDF embedder for consistency.

## Usage
```rust
let mut scorer = Bm25Score::new();
let query_terms = tokenize_for_bm25("hello world");
let doc_terms = tokenize_for_bm25("hello there world");
let score = scorer.score(&query_terms, &doc_terms, 3);
```

## Tests
- `test_tokenize` - Tokenization
- `test_bm25_score` - Scoring with matches
- `test_bm25_empty_query` - Empty query returns 0
- `test_bm25_no_match` - No matching terms returns 0

## Integration
BM25 infrastructure is in place but not yet wired into the memory search flow. For full integration, the `MemoryStore::search()` method would need to combine BM25 text scores with embedding similarity scores.

## Verification
```bash
cargo test bm25  # 4 tests pass
```
