use anyhow::Result;
use serde_json::json;
use std::collections::HashMap;
use std::sync::RwLock;

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
}

impl EmbeddingService {
    pub fn new() -> Result<Self> {
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
        })
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let cache_key = self.cache_key(text);
        
        if let Some(embedding) = self.get_cached(&cache_key) {
            tracing::debug!("Embedding cache hit for: {}...", &text[..text.len().min(50)]);
            return Ok(embedding);
        }

        let embedding = if self.api_key.is_empty() {
            self.fallback_embedding(text)
        } else {
            self.fetch_embedding(text).await?
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
            cache.insert(key.to_string(), CacheEntry {
                embedding,
                cached_at: std::time::Instant::now(),
            });
        }
    }

    async fn fetch_embedding(&self, text: &str) -> Result<Vec<f32>> {
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
            .ok_or_else(|| anyhow::anyhow!("Invalid embedding response"))?
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect();

        Ok(embedding)
    }

    #[allow(dead_code)]
    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let mut embeddings = Vec::new();
        for text in texts {
            embeddings.push(self.embed(&text).await?);
        }
        Ok(embeddings)
    }

    fn fallback_embedding(&self, text: &str) -> Vec<f32> {
        let mut embedding = vec![0.0f32; 384];
        let words: Vec<&str> = text.split_whitespace().collect();

        for word in words.iter() {
            let hash = Self::simple_hash(word);
            let idx = (hash % 384) as usize;
            embedding[idx] += 1.0;
        }

        Self::normalize(&mut embedding);
        embedding
    }

    fn simple_hash(s: &str) -> u64 {
        let mut hash: u64 = 5381;
        for c in s.chars() {
            hash = ((hash << 5).wrapping_add(hash)).wrapping_add(c as u64);
        }
        hash
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
}

impl Default for EmbeddingService {
    fn default() -> Self {
        Self::new().unwrap_or_else(|e| {
            tracing::warn!("Failed to initialize embedding service: {}. Using fallback mode.", e);
            Self {
                api_key: String::new(),
                base_url: String::new(),
                model: String::new(),
                client: reqwest::Client::new(),
                cache: RwLock::new(HashMap::new()),
                cache_ttl_secs: 3600,
            }
        })
    }
}
