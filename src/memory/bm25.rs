#![allow(dead_code)]

use std::collections::HashMap;

const STOPWORDS_BM25: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by",
    "from", "as", "is", "was", "are", "were", "been", "be", "have", "has", "had", "do", "does",
    "did", "will", "would", "could", "should", "may", "might", "must", "shall", "can", "need",
    "it", "its", "this", "that", "these", "those", "i", "you", "he", "she", "we", "they",
];

pub struct Bm25Score {
    idf_cache: HashMap<String, f32>,
    avg_doc_len: f32,
    k1: f32,
    b: f32,
}

impl Bm25Score {
    pub fn new() -> Self {
        Self {
            idf_cache: HashMap::new(),
            avg_doc_len: 100.0,
            k1: 1.5,
            b: 0.75,
        }
    }

    pub fn score(&mut self, query_terms: &[String], doc_terms: &[String], doc_len: usize) -> f32 {
        if query_terms.is_empty() || doc_len == 0 {
            return 0.0;
        }

        let doc_term_set: HashMap<&str, usize> =
            doc_terms.iter().fold(HashMap::new(), |mut acc, term| {
                *acc.entry(term.as_str()).or_insert(0) += 1;
                acc
            });

        let mut score = 0.0f32;
        for term in query_terms {
            if let Some(&tf) = doc_term_set.get(term.as_str()) {
                let idf = self.get_idf(term);
                let tf_norm = (tf as f32 * (self.k1 + 1.0))
                    / (tf as f32
                        + self.k1 * (1.0 - self.b + self.b * (doc_len as f32 / self.avg_doc_len)));
                score += idf * tf_norm;
            }
        }

        score
    }

    pub fn set_avg_doc_len(&mut self, avg: f32) {
        self.avg_doc_len = avg;
    }

    fn get_idf(&mut self, term: &str) -> f32 {
        if let Some(&idf) = self.idf_cache.get(term) {
            return idf;
        }

        let idf = 2.0;
        self.idf_cache.insert(term.to_string(), idf);
        idf
    }
}

impl Default for Bm25Score {
    fn default() -> Self {
        Self::new()
    }
}

pub fn tokenize_for_bm25(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|s| !s.is_empty())
        .filter(|s| s.len() > 1)
        .filter(|s| !STOPWORDS_BM25.contains(s))
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        let tokens = tokenize_for_bm25("Hello world! This is a test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(!tokens.contains(&"this".to_string()));
    }

    #[test]
    fn test_bm25_score() {
        let mut scorer = Bm25Score::new();
        let query = vec!["hello".to_string(), "world".to_string()];
        let doc = vec!["hello".to_string(), "world".to_string(), "test".to_string()];

        let score = scorer.score(&query, &doc, 3);
        assert!(score > 0.0);
    }

    #[test]
    fn test_bm25_empty_query() {
        let mut scorer = Bm25Score::new();
        let query: Vec<String> = vec![];
        let doc = vec!["hello".to_string(), "world".to_string()];

        let score = scorer.score(&query, &doc, 2);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_bm25_no_match() {
        let mut scorer = Bm25Score::new();
        let query = vec!["foo".to_string(), "bar".to_string()];
        let doc = vec!["hello".to_string(), "world".to_string()];

        let score = scorer.score(&query, &doc, 2);
        assert_eq!(score, 0.0);
    }
}
