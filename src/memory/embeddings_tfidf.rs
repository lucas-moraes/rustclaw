use std::collections::HashMap;
use std::sync::RwLock;

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by",
    "from", "as", "is", "was", "are", "were", "been", "be", "have", "has", "had", "do", "does",
    "did", "will", "would", "could", "should", "may", "might", "must", "shall", "can", "need",
    "dare", "ought", "used", "it", "its", "this", "that", "these", "those", "i", "you", "he",
    "she", "we", "they", "what", "which", "who", "whom", "whose", "where", "when", "why", "how",
    "all", "each", "every", "both", "few", "more", "most", "other", "some", "such", "no", "nor",
    "not", "only", "own", "same", "so", "than", "too", "very", "just", "if", "then", "else",
    "while", "although", "because", "since", "until", "unless", "though", "before", "after",
];

const EMBEDDING_DIM: usize = 384;

pub struct TfidfEmbedder {
    idf_cache: RwLock<HashMap<String, f32>>,
}

impl TfidfEmbedder {
    pub fn new() -> Self {
        Self {
            idf_cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn embed(&self, text: &str) -> Vec<f32> {
        let tokens = self.tokenize(text);
        if tokens.is_empty() {
            return vec![0.0f32; EMBEDDING_DIM];
        }

        let term_freqs = self.compute_term_frequency(&tokens);
        let tfidf_weights = self.compute_tfidf(&term_freqs, tokens.len());

        let mut embedding = vec![0.0f32; EMBEDDING_DIM];
        for (token, weight) in tfidf_weights {
            let hash = Self::simple_hash(&token);
            let idx = (hash % EMBEDDING_DIM as u64) as usize;
            embedding[idx] += weight;
        }

        Self::normalize(&mut embedding);
        embedding
    }

    fn tokenize(&self, text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric() && c != '\'')
            .filter(|s| !s.is_empty())
            .filter(|s| s.len() > 1)
            .filter(|s| !STOPWORDS.contains(s))
            .map(|s| s.to_string())
            .collect()
    }

    fn compute_term_frequency(&self, tokens: &[String]) -> HashMap<String, f32> {
        let mut freq: HashMap<String, f32> = HashMap::new();
        let total = tokens.len() as f32;

        for token in tokens {
            *freq.entry(token.clone()).or_insert(0.0) += 1.0;
        }

        for (_, count) in freq.iter_mut() {
            *count /= total;
        }

        freq
    }

    fn compute_tfidf(
        &self,
        term_freqs: &HashMap<String, f32>,
        _doc_len: usize,
    ) -> HashMap<String, f32> {
        let mut tfidf: HashMap<String, f32> = HashMap::new();

        for (token, tf) in term_freqs {
            let idf = self.get_idf(token);
            tfidf.insert(token.clone(), tf * idf);
        }

        tfidf
    }

    fn get_idf(&self, term: &str) -> f32 {
        if let Ok(cache) = self.idf_cache.read() {
            if let Some(&idf) = cache.get(term) {
                return idf;
            }
        }

        let idf = 2.0;

        if let Ok(mut cache) = self.idf_cache.write() {
            cache.insert(term.to_string(), idf);
        }

        idf
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
}

impl Default for TfidfEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        let embedder = TfidfEmbedder::new();
        let tokens = embedder.tokenize("Hello world! This is a test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(!tokens.contains(&"this".to_string()));
        assert!(!tokens.contains(&"is".to_string()));
    }

    #[test]
    fn test_embed() {
        let embedder = TfidfEmbedder::new();
        let embedding = embedder.embed("Hello world");
        assert_eq!(embedding.len(), 384);

        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (magnitude - 1.0).abs() < 0.001,
            "Embedding should be normalized"
        );
    }

    #[test]
    fn test_empty_text() {
        let embedder = TfidfEmbedder::new();
        let embedding = embedder.embed("");
        assert_eq!(embedding.len(), 384);
        assert!(embedding.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn test_normalize() {
        let mut embedding = vec![3.0f32, 4.0f32];
        TfidfEmbedder::normalize(&mut embedding);
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((magnitude - 1.0).abs() < 0.001);
    }
}
