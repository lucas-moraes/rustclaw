use anyhow::Result;
use serde_json::json;

pub struct EmbeddingService {
    api_key: String,
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl EmbeddingService {
    pub fn new() -> Result<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .or_else(|_| std::env::var("COHERE_API_KEY"))
            .unwrap_or_default();
        
        if api_key.is_empty() {
            tracing::warn!("No embedding API key found. Set OPENAI_API_KEY or COHERE_API_KEY");
        }
        
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        
        Ok(Self {
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
            model: "text-embedding-3-small".to_string(),
            client,
        })
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        if self.api_key.is_empty() {
            return Ok(self.fallback_embedding(text));
        }
        
        let url = format!("{}/embeddings", self.base_url);
        
        let body = json!({
            "model": self.model,
            "input": text,
            "encoding_format": "float"
        });
        
        let response = self.client
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
        
        for (i, word) in words.iter().enumerate() {
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

    pub fn normalize(embedding: &mut [f32]) {
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if magnitude > 0.0 {
            for x in embedding.iter_mut() {
                *x /= magnitude;
            }
        }
    }

    pub fn dimensions(&self) -> usize {
        384
    }
}

impl Default for EmbeddingService {
    fn default() -> Self {
        Self::new().expect("Failed to initialize embedding service")
    }
}
