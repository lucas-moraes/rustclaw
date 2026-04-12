# Phase 4: Quality Metrics

**Date:** 2026-04-12
**Status:** ✅ Complete

## Summary

Added methods to detect embedding quality and warn when fallback is being used.

## Modified Files
- `src/memory/embeddings.rs` - Added quality detection methods

## Implementation

### EmbeddingQuality Enum
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingQuality {
    High,    // OpenAI/Cohere with API key
    Medium,
    Low,     // Local TF-IDF fallback
}
```

### Methods Added
```rust
impl EmbeddingService {
    pub fn embedding_quality(&self) -> EmbeddingQuality {
        match self.embedding_model {
            EmbeddingModel::Local => EmbeddingQuality::Low,
            EmbeddingModel::Cohere | EmbeddingModel::OpenAI => {
                if self.api_key.is_empty() {
                    EmbeddingQuality::Low
                } else {
                    EmbeddingQuality::High
                }
            }
        }
    }

    pub fn is_using_fallback(&self) -> bool {
        self.api_key.is_empty() || self.embedding_model == EmbeddingModel::Local
    }
}
```

## Usage
```rust
let embedding_service = EmbeddingService::new().await?;

if embedding_service.is_using_fallback() {
    tracing::warn!("Using local TF-IDF embeddings - search quality may be reduced");
}

let quality = embedding_service.embedding_quality();
match quality {
    EmbeddingQuality::High => println!("Using high-quality API embeddings"),
    EmbeddingQuality::Low => println!("Using local fallback embeddings"),
    _ => {}
}
```

## Future Integration
The quality metrics can be used to:
- Log warnings when fallback is active
- Show quality indicator in `/stats` command
- Adjust search parameters based on quality

## Verification
```bash
cargo clippy  # 0 warnings
cargo test    # 91 tests pass
```
