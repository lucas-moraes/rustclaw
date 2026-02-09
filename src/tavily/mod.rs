pub mod tools;

use std::time::Duration;
use tavily::{Tavily, SearchRequest};
use tracing::{info, warn};

pub struct TavilyClient {
    client: Tavily,
    api_key: String,
}

impl TavilyClient {
    pub fn new(api_key: &str) -> anyhow::Result<Self> {
        let client = Tavily::builder(api_key)
            .timeout(Duration::from_secs(60))
            .max_retries(3)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build Tavily client: {}", e))?;

        info!("Tavily client initialized successfully");
        
        Ok(Self { 
            client,
            api_key: api_key.to_string(),
        })
    }

    pub async fn search(
        &self,
        query: &str,
        max_results: i32,
        search_depth: &str,
        include_answer: bool,
    ) -> anyhow::Result<SearchResponse> {
        info!("Searching Tavily for: {}", query);

        let request = SearchRequest::new(&self.api_key, query)
            .search_depth(search_depth)
            .max_results(max_results)
            .include_answer(include_answer)
            .include_raw_content(false);

        let results = self.client.call(&request).await
            .map_err(|e| anyhow::anyhow!("Tavily search failed: {}", e))?;

        let response = SearchResponse {
            query: results.query,
            answer: results.answer,
            results: results.results.into_iter().map(|r| SearchResultItem {
                title: r.title,
                url: r.url,
                content: r.content,
                score: r.score as f64,
            }).collect(),
        };

        info!("Tavily search completed with {} results", response.results.len());
        
        Ok(response)
    }

    pub async fn search_basic(&self, query: &str) -> anyhow::Result<SearchResponse> {
        self.search(query, 5, "basic", true).await
    }

    pub async fn search_advanced(&self, query: &str) -> anyhow::Result<SearchResponse> {
        self.search(query, 10, "advanced", true).await
    }
}

#[derive(Debug, Clone)]
pub struct SearchResponse {
    pub query: String,
    pub answer: Option<String>,
    pub results: Vec<SearchResultItem>,
}

#[derive(Debug, Clone)]
pub struct SearchResultItem {
    pub title: String,
    pub url: String,
    pub content: String,
    pub score: f64,
}

impl SearchResponse {
    pub fn format_results(&self, max_chars: usize) -> String {
        let mut output = format!("🔍 Resultados da busca: '{}'\n\n", self.query);

        
        if let Some(answer) = &self.answer {
            output.push_str(&format!("🤖 Resumo IA:\n{}\n\n", answer));
        }

        output.push_str("📚 Resultados:\n\n");

        for (i, result) in self.results.iter().enumerate() {
            let entry = format!(
                "{}. **{}**\n   🔗 {}\n   📝 {}\n\n",
                i + 1,
                result.title,
                result.url,
                result.content.chars().take(200).collect::<String>()
            );

            if output.len() + entry.len() > max_chars {
                output.push_str(&format!("\n... e mais {} resultados", 
                    self.results.len() - i));
                break;
            }

            output.push_str(&entry);
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_response_format() {
        let response = SearchResponse {
            query: "test query".to_string(),
            answer: Some("This is a test answer".to_string()),
            results: vec![
                SearchResultItem {
                    title: "Test Result".to_string(),
                    url: "https://example.com".to_string(),
                    content: "Test content here".to_string(),
                    score: 0.95,
                },
            ],
        };

        let formatted = response.format_results(1000);
        assert!(formatted.contains("test query"));
        assert!(formatted.contains("Resumo IA"));
        assert!(formatted.contains("Test Result"));
    }
}
