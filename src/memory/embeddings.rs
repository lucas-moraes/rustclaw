use std::collections::HashMap;
use std::result::Result;
use std::sync::RwLock;

use serde_json::json;
use crate::config::EmbeddingModel;
use crate::error::{AgentError, MemoryError};
use crate::memory::embeddings_tfidf::TfidfEmbedder;

#[derive(Debug, Clone)]
struct CacheEntry {
    embedding: Vec<f32>,
    cached_at: std::time::Instant,
}

pub struct EmbeddingService {
    api_key: String,
    base_url: String,
    model: String,
    client: reqwest::Client,
    cache: RwLock<HashMap<String, CacheEntry>>,
    cache_ttl_secs: u64,
    embedding_model: EmbeddingModel,
    tfidf_embedder: TfidfEmbedder,
}

impl EmbeddingService {
    pub fn new() -> Result<Self, AgentError> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .or_else(|_| std::env::var("COHERE_API_KEY"))
            .unwrap_or_default();

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        Ok(Self {
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
            model: "text-embedding-3-small".to_string(),
            client,
            cache: RwLock::new(HashMap::new()),
            cache_ttl_secs: 3600,
            embedding_model: EmbeddingModel::from_env(),
            tfidf_embedder: TfidfEmbedder::new(),
        })
    }

    #[cfg(test)]
    pub fn new_mock() -> Self {
        Self {
            api_key: String::new(),
            base_url: String::new(),
            model: String::new(),
            client: reqwest::Client::new(),
            cache: RwLock::new(HashMap::new()),
            cache_ttl_secs: 3600,
            embedding_model: EmbeddingModel::OpenAI,
            tfidf_embedder: TfidfEmbedder::new(),
        }
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>, AgentError> {
        let cache_key = self.cache_key(text);

        if let Some(embedding) = self.get_cached(&cache_key) {
            tracing::debug!(
                "Embedding cache hit for: {}...",
                &text[..text.len().min(50)]
            );
            return Ok(embedding);
        }

        let embedding = match self.embedding_model {
            EmbeddingModel::Local => self.tfidf_embedder.embed(text),
            EmbeddingModel::Cohere if !self.api_key.is_empty() => {
                self.fetch_cohere_embedding(text).await?
            }
            _ => {
                if self.api_key.is_empty() {
                    tracing::debug!("No API key found, using local TF-IDF embeddings");
                    self.tfidf_embedder.embed(text)
                } else {
                    self.fetch_embedding(text).await?
                }
            }
        };

        self.put_cached(&cache_key, embedding.clone());
        Ok(embedding)
    }

    fn cache_key(&self, text: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    fn get_cached(&self, key: &str) -> Option<Vec<f32>> {
        let cache = self.cache.read().ok()?;
        cache.get(key).and_then(|entry| {
            if entry.cached_at.elapsed().as_secs() < self.cache_ttl_secs {
                Some(entry.embedding.clone())
            } else {
                None
            }
        })
    }

    fn put_cached(&self, key: &str, embedding: Vec<f32>) {
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(
                key.to_string(),
                CacheEntry {
                    embedding,
                    cached_at: std::time::Instant::now(),
                },
            );
        }
    }

    async fn fetch_embedding(&self, text: &str) -> Result<Vec<f32>, AgentError> {
        let url = format!("{}/embeddings", self.base_url);

        let body = json!({
            "model": self.model,
            "input": text,
            "encoding_format": "float"
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::warn!("Embedding API error: {}. Using fallback.", error_text);
            return Ok(self.fallback_embedding(text));
        }

        let json_response: serde_json::Value = response.json().await?;
        let embedding = json_response["data"][0]["embedding"]
            .as_array()
            .ok_or_else(|| MemoryError::EmbeddingFailed("Invalid embedding response".to_string()))?
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect();

        Ok(embedding)
    }

    async fn fetch_cohere_embedding(&self, text: &str) -> Result<Vec<f32>, AgentError> {
        let url = "https://api.cohere.ai/v1/embed".to_string();

        let body = json!({
            "model": "embed-english-v3.0",
            "texts": [text],
            "input_type": "search_document"
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            tracing::warn!("Cohere API error: {}. Using fallback.", error_text);
            return Ok(self.fallback_embedding(text));
        }

        let json_response: serde_json::Value = response.json().await?;
        let embedding = json_response["embeddings"][0]
            .as_array()
            .ok_or_else(|| MemoryError::EmbeddingFailed("Invalid Cohere embedding response".to_string()))?
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect();

        Ok(embedding)
    }

    #[allow(dead_code)]
    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, AgentError> {
        let mut embeddings = Vec::new();
        for text in texts {
            embeddings.push(self.embed(&text).await?);
        }
        Ok(embeddings)
    }

    fn fallback_embedding(&self, text: &str) -> Vec<f32> {
        self.tfidf_embedder.embed(text)
    }

    #[allow(dead_code)]
    pub fn normalize(embedding: &mut [f32]) {
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if magnitude > 0.0 {
            for x in embedding.iter_mut() {
                *x /= magnitude;
            }
        }
    }

    #[allow(dead_code)]
    pub fn dimensions(&self) -> usize {
        384
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub fn is_using_fallback(&self) -> bool {
        self.api_key.is_empty() || self.embedding_model == EmbeddingModel::Local
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum EmbeddingQuality {
    High,
    Medium,
    Low,
}

impl Default for EmbeddingService {
    fn default() -> Self {
        Self::new().unwrap_or_else(|e| {
            tracing::warn!(
                "Failed to initialize embedding service: {}. Using fallback mode.",
                e
            );
            Self {
                api_key: String::new(),
                base_url: String::new(),
                model: String::new(),
                client: reqwest::Client::new(),
                cache: RwLock::new(HashMap::new()),
                cache_ttl_secs: 3600,
                embedding_model: EmbeddingModel::Local,
                tfidf_embedder: TfidfEmbedder::new(),
            }
        })
    }
}
