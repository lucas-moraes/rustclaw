use crate::browser::{test_browser, BrowserSession};
use crate::tools::Tool;
use serde_json::Value;
use std::time::Duration;

pub struct BrowserNavigateTool;
pub struct BrowserSearchTool;
pub struct BrowserExtractTool;
pub struct BrowserScreenshotTool;
pub struct BrowserTestTool;

impl BrowserNavigateTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for BrowserNavigateTool {
    fn name(&self) -> &str {
        "browser_navigate"
    }

    fn description(&self) -> &str {
        "Navega para uma URL. Input: { \"url\": \"https://example.com\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'url' é obrigatório".to_string())?;

        let session = BrowserSession::new().await
            .map_err(|e| format!("Erro ao iniciar browser: {}", e))?;

        let result = async {
            session.navigate(url).await
                .map_err(|e| format!("Erro ao navegar: {}", e))?;
            
            // Get page title
            let title = session.page.title().await
                .unwrap_or_else(|_| "Sem título".to_string());

            Ok::<_, String>(format!("✅ Navegado para: {}\n📝 Título: {}", url, title))
        }.await;

        session.close().await;
        result
    }
}

impl BrowserSearchTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for BrowserSearchTool {
    fn name(&self) -> &str {
        "browser_search"
    }

    fn description(&self) -> &str {
        "Busca na internet. Input: { \"query\": \"termo de busca\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let query = args["query"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'query' é obrigatório".to_string())?;

        let session = BrowserSession::new().await
            .map_err(|e| format!("Erro ao iniciar browser: {}", e))?;

        let result = async {
            let results = session.search_brave(query).await
                .map_err(|e| format!("Erro na busca: {}", e))?;
            
            // Tirar screenshot da página de resultados
            let screenshot_filename = format!("search_{}.png", 
                query.replace(" ", "_").replace(|c: char| !c.is_ascii_alphanumeric(), "_"));
            let screenshot_path = session.take_screenshot(&screenshot_filename).await
                .map_err(|e| format!("Erro ao tirar screenshot: {}", e))?;
            
            let truncated = if results.len() > 2000 {
                format!("{}...\n\n[Texto truncado]", &results[..2000])
            } else {
                results
            };

            Ok::<_, String>(format!("🔍 Resultados da busca '{}':\n\n{}\n\n📸 Screenshot: {}", 
                query, truncated, screenshot_path))
        }.await;

        session.close().await;
        result
    }
}

impl BrowserExtractTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for BrowserExtractTool {
    fn name(&self) -> &str {
        "browser_extract"
    }

    fn description(&self) -> &str {
        "Extrai texto da página atual. Input: { \"selector\": \"body\", \"max_chars\": 3000 }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let selector = args["selector"].as_str();
        let max_chars = args["max_chars"].as_u64().unwrap_or(3000) as usize;

        let session = BrowserSession::new().await
            .map_err(|e| format!("Erro ao iniciar browser: {}", e))?;

        let result = async {
            // Navigate to about:blank first to have a page
            session.navigate("about:blank").await
                .map_err(|e| format!("Erro: {}", e))?;
            
            let text = session.extract_text(selector).await
                .map_err(|e| format!("Erro ao extrair texto: {}", e))?;
            
            let truncated = if text.len() > max_chars {
                format!("{}...\n\n[Texto truncado em {} caracteres]", 
                    &text[..max_chars], 
                    max_chars
                )
            } else {
                text
            };

            Ok::<_, String>(format!("📄 Texto extraído:\n\n{}", truncated))
        }.await;

        session.close().await;
        result
    }
}

impl BrowserScreenshotTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for BrowserScreenshotTool {
    fn name(&self) -> &str {
        "browser_screenshot"
    }

    fn description(&self) -> &str {
        "Tira screenshot da página. Input: { \"url\": \"https://...\", \"filename\": \"screenshot.png\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let url = args["url"].as_str();
        let filename = args["filename"]
            .as_str()
            .unwrap_or("screenshot.png");

        let session = BrowserSession::new().await
            .map_err(|e| format!("Erro ao iniciar browser: {}", e))?;

        let result = async {
            if let Some(u) = url {
                session.navigate(u).await
                    .map_err(|e| format!("Erro ao navegar: {}", e))?;
            } else {
                // Use blank page
                session.navigate("about:blank").await
                    .map_err(|e| format!("Erro: {}", e))?;
            };

            // Wait a bit for page to fully render
            tokio::time::sleep(Duration::from_secs(1)).await;

            let filepath = session.take_screenshot(filename).await
                .map_err(|e| format!("Erro ao tirar screenshot: {}", e))?;

            Ok::<_, String>(format!("📸 Screenshot salvo em: {}", filepath))
        }.await;

        session.close().await;
        result
    }
}

impl Default for BrowserNavigateTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for BrowserSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for BrowserExtractTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for BrowserScreenshotTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserTestTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for BrowserTestTool {
    fn name(&self) -> &str {
        "browser_test"
    }

    fn description(&self) -> &str {
        "Testa a instalação do browser. Input: {}"
    }

    async fn call(&self, _args: Value) -> Result<String, String> {
        match test_browser().await {
            Ok(result) => Ok(result),
            Err(e) => Err(format!("❌ Browser test failed: {}", e)),
        }
    }
}

impl Default for BrowserTestTool {
    fn default() -> Self {
        Self::new()
    }
}
