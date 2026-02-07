use anyhow::Result;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct EmbeddingService {
    model: Arc<Mutex<TextEmbedding>>,
}

impl EmbeddingService {
    pub fn new() -> Result<Self> {
        tracing::info!("Initializing embedding model (BAAI/bge-small-en-v1.5)...");
        
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGESmallENV15)
                .with_show_download_progress(true),
        )?;

        tracing::info!("Embedding model loaded successfully");
        
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
        })
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let model = self.model.clone();
        let text = text.to_string();
        
        // Run embedding in a blocking task since fastembed can be CPU intensive
        let embeddings = tokio::task::spawn_blocking(move || {
            let model = model.blocking_lock();
            model.embed(vec![text], None)
        })
        .await??;

        // Return first (and only) embedding
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No embedding generated"))
    }

    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model = self.model.clone();
        
        let embeddings = tokio::task::spawn_blocking(move || {
            let model = model.blocking_lock();
            model.embed(texts, None)
        })
        .await??;

        Ok(embeddings)
    }

    /// Normalize embedding to unit vector for cosine similarity
    pub fn normalize(embedding: &mut [f32]) {
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if magnitude > 0.0 {
            for x in embedding.iter_mut() {
                *x /= magnitude;
            }
        }
    }

    /// Get the dimension size for this model (384 for BGE-Small)
    pub fn dimensions(&self) -> usize {
        384
    }
}

impl Default for EmbeddingService {
    fn default() -> Self {
        Self::new().expect("Failed to initialize embedding service")
    }
}
