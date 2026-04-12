# Phase 2: Config Options

**Date:** 2026-04-12
**Status:** ✅ Complete

## Summary

Added `EMBEDDING_MODEL` config option to switch between OpenAI, Cohere, and Local embeddings.

## Modified Files
- `src/config.rs` - Added `EmbeddingModel` enum and config field
- `src/memory/embeddings.rs` - Updated to use config
- `src/memory/mod.rs` - Added `embeddings_tfidf` module

## Changes Made

### EmbeddingModel Enum
```rust
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub enum EmbeddingModel {
    #[default]
    OpenAI,
    Cohere,
    Local,
}

impl EmbeddingModel {
    pub fn from_env() -> Self {
        let model_str = std::env::var("EMBEDDING_MODEL")
            .unwrap_or_else(|_| "openai".to_string())
            .to_lowercase();
        
        match model_str.as_str() {
            "cohere" => EmbeddingModel::Cohere,
            "local" => EmbeddingModel::Local,
            _ => EmbeddingModel::OpenAI,
        }
    }
}
```

### Config Changes
```rust
pub struct Config {
    // ... existing fields ...
    pub embedding_model: EmbeddingModel,
}
```

### Environment Variable
```
EMBEDDING_MODEL=openai   # Default, uses OpenAI API
EMBEDDING_MODEL=cohere   # Uses Cohere API
EMBEDDING_MODEL=local    # Uses local TF-IDF (no API key needed)
```

### EmbeddingService Changes
- Added `embedding_model: EmbeddingModel` field
- Added `tfidf_embedder: TfidfEmbedder` field
- Updated `embed()` to use the configured model
- Added `fetch_cohere_embedding()` method

## Verification
```bash
cargo build  # Compiles
cargo test  # 91 tests pass
```
