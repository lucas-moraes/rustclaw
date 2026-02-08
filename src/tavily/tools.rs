use crate::tavily::TavilyClient;
use crate::tools::Tool;
use serde_json::Value;
use tracing::{info, error};

pub struct TavilySearchTool {
    api_key: String,
}

impl TavilySearchTool {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait::async_trait]
impl Tool for TavilySearchTool {
    fn name(&self) -> &str {
        "tavily_search"
    }

    fn description(&self) -> &str {
        "Busca na internet usando IA (Tavily). Input: { \"query\": \"termo de busca\", \"max_results\": 5, \"search_depth\": \"basic\", \"include_answer\": true }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let query = args["query"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'query' é obrigatório".to_string())?;

        let max_results = args["max_results"].as_i64().unwrap_or(5) as i32;
        let search_depth = args["search_depth"]
            .as_str()
            .unwrap_or("basic");
        let include_answer = args["include_answer"].as_bool().unwrap_or(true);

        info!("Tavily search query: {}, depth: {}", query, search_depth);

        let client = TavilyClient::new(&self.api_key)
            .map_err(|e| format!("Erro ao criar cliente Tavily: {}", e))?;

        let results = client
            .search(query, max_results, search_depth, include_answer)
            .await
            .map_err(|e| {
                error!("Tavily search failed: {}", e);
                format!("Erro na busca Tavily: {}", e)
            })?;

        let formatted = results.format_results(3000);
        
        info!("Tavily search completed successfully");
        
        Ok(formatted)
    }
}

impl Default for TavilySearchTool {
    fn default() -> Self {
        // This will fail at runtime if TAVILY_API_KEY is not set
        // In practice, you should always use new() with a proper key
        let api_key = std::env::var("TAVILY_API_KEY").unwrap_or_default();
        Self::new(api_key)
    }
}

pub struct TavilyQuickSearchTool {
    api_key: String,
}

impl TavilyQuickSearchTool {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait::async_trait]
impl Tool for TavilyQuickSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Busca rápida na web. Input: { \"query\": \"termo de busca\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let query = args["query"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'query' é obrigatório".to_string())?;

        info!("Web search (Tavily) query: {}", query);

        let client = TavilyClient::new(&self.api_key)
            .map_err(|e| format!("Erro ao criar cliente Tavily: {}", e))?;

        let results = client
            .search_basic(query)
            .await
            .map_err(|e| {
                error!("Tavily search failed: {}", e);
                format!("Erro na busca: {}", e)
            })?;

        let formatted = results.format_results(2000);
        
        Ok(formatted)
    }
}
