//! Embedding backends for the semantic index.
//!
//! The index is designed to work with or without embeddings:
//!
//! - [`NullEmbedder`] returns no vectors, so search degrades to BM25 (FTS5)
//!   only. This is the default when no embedding provider is configured, and
//!   keeps the binary free of heavy local-model dependencies.
//! - [`ApiEmbedder`] calls an OpenAI-compatible `/embeddings` endpoint. It is
//!   configured per project (or globally) and used when available.
//!
//! Embeddings are stored as little-endian `f32` blobs; similarity is cosine.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// A backend that turns text into embedding vectors.
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// Human-readable backend name (for `/index` status).
    fn name(&self) -> &str;
    /// Vector dimension, or `0` when the backend produces no vectors.
    fn dim(&self) -> usize;
    /// Embeds a batch of texts. Returns one vector per input, in order.
    /// A backend that produces no vectors returns an empty `Vec`.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

/// An embedder that produces no vectors (BM25-only search).
pub struct NullEmbedder;

#[async_trait::async_trait]
impl Embedder for NullEmbedder {
    fn name(&self) -> &str {
        "none"
    }
    fn dim(&self) -> usize {
        0
    }
    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(Vec::new())
    }
}

/// Configuration for an OpenAI-compatible embeddings endpoint.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbedConfig {
    /// Base URL (e.g. `https://api.openai.com/v1`).
    pub base_url: String,
    /// API key (bearer token).
    pub api_key: String,
    /// Model name (e.g. `text-embedding-3-small`).
    pub model: String,
    /// Vector dimension expected from the model (0 = infer from first response).
    #[serde(default)]
    pub dim: usize,
}

/// Calls an OpenAI-compatible `/embeddings` endpoint.
pub struct ApiEmbedder {
    cfg: EmbedConfig,
    client: reqwest::Client,
    /// Dimension observed from the first successful response (when `cfg.dim` is 0).
    observed_dim: std::sync::atomic::AtomicUsize,
}

impl ApiEmbedder {
    pub fn new(cfg: EmbedConfig) -> Self {
        Self {
            cfg,
            client: reqwest::Client::new(),
            observed_dim: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/embeddings", self.cfg.base_url.trim_end_matches('/'))
    }
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedDatum>,
}

#[derive(Deserialize)]
struct EmbedDatum {
    embedding: Vec<f32>,
}

#[async_trait::async_trait]
impl Embedder for ApiEmbedder {
    fn name(&self) -> &str {
        &self.cfg.model
    }

    fn dim(&self) -> usize {
        if self.cfg.dim > 0 {
            self.cfg.dim
        } else {
            self.observed_dim.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let body = serde_json::json!({
            "model": self.cfg.model,
            "input": texts,
        });
        let resp = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.cfg.api_key)
            .json(&body)
            .send()
            .await
            .context("embeddings request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("embeddings endpoint returned {}: {}", status, text);
        }
        let parsed: EmbedResponse = resp
            .json()
            .await
            .context("failed to parse embeddings response")?;
        let vectors: Vec<Vec<f32>> = parsed.data.into_iter().map(|d| d.embedding).collect();
        if let Some(first) = vectors.first() {
            self.observed_dim
                .store(first.len(), std::sync::atomic::Ordering::Relaxed);
        }
        Ok(vectors)
    }
}

/// Encodes a vector as a little-endian `f32` blob for SQLite storage.
pub fn encode_vector(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Decodes a little-endian `f32` blob back into a vector.
pub fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Cosine similarity between two vectors. Returns `0.0` for mismatched or
/// empty vectors (so a stale index never panics).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_null_embedder_produces_nothing() {
        let e = NullEmbedder;
        assert_eq!(e.dim(), 0);
        let out = e.embed(&["a".into()]).await.unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn test_vector_roundtrip() {
        let v = vec![1.0f32, -2.5, 3.25];
        let bytes = encode_vector(&v);
        assert_eq!(bytes.len(), 12);
        assert_eq!(decode_vector(&bytes), v);
    }

    #[test]
    fn test_cosine_identical_is_one() {
        let v = vec![1.0f32, 2.0, 3.0];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_orthogonal_is_zero() {
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_mismatched_len_is_zero() {
        assert_eq!(cosine(&[1.0, 2.0], &[1.0]), 0.0);
        assert_eq!(cosine(&[], &[]), 0.0);
    }
}
