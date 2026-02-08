pub mod tools;

use playwright_rs::Playwright;
use std::path::Path;
use std::time::Duration;
use tracing::{info, warn};

pub struct BrowserSession {
    playwright: Playwright,
    page: playwright_rs::Page,
}

impl BrowserSession {
    pub async fn new() -> anyhow::Result<Self> {
        info!("Starting browser session with Playwright...");
        
        // Launch Playwright
        let playwright = Playwright::launch().await
            .map_err(|e| anyhow::anyhow!("Failed to launch Playwright: {}", e))?;
        
        info!("Playwright launched successfully");
        
        // Launch browser (Chromium)
        let browser = playwright.chromium()
            .launch()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to launch browser: {}", e))?;
        
        info!("Browser launched successfully");
        
        // Create new page directly
        let page = browser.new_page().await
            .map_err(|e| anyhow::anyhow!("Failed to create page: {}", e))?;
        
        info!("Browser session started successfully");
        
        Ok(Self { playwright, page })
    }

    pub async fn navigate(&self, url: &str) -> anyhow::Result<()> {
        info!("Navigating to: {}", url);
        self.page.goto(url, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to navigate: {}", e))?;
        
        // Wait for page to load
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        Ok(())
    }

    pub async fn search_brave(&self, query: &str) -> anyhow::Result<String> {
        let encoded_query = urlencoding::encode(query);
        let url = format!("https://search.brave.com/search?q={}", encoded_query);
        
        self.navigate(&url).await?;
        
        // Wait for results to load
        tokio::time::sleep(Duration::from_secs(3)).await;
        
        // Extract search results
        let results = self.extract_search_results().await?;
        
        Ok(results)
    }

    async fn extract_search_results(&self) -> anyhow::Result<String> {
        // Try different selectors for Brave search results
        let selectors = [
            ".snippet",
            ".result",
            "[data-testid='search-result']",
            ".web-result",
        ];
        
        for selector in &selectors {
            let js_code = format!(
                r#"() => {{
                    const snippets = document.querySelectorAll('{}');
                    let text = '';
                    snippets.forEach((s, i) => {{
                        if (i < 5) {{
                            const title = s.querySelector('.title, h3')?.textContent?.trim() || '';
                            const desc = s.querySelector('.description, .snippet-description, p')?.textContent?.trim() || '';
                            if (title) {{
                                text += `${{i+1}}. ${{title}}\n${{desc}}\n\n`;
                            }}
                        }}
                    }});
                    return text;
                }}"#,
                selector
            );
            
            // evaluate<T, U> where T is input args type, U is return type
            // Pass None for args since our JS function takes no arguments
            match self.page.evaluate::<String, String>(&js_code, None).await {
                Ok(text) => {
                    if !text.is_empty() {
                        info!("Found results using selector: {}", selector);
                        return Ok(text);
                    }
                }
                Err(e) => {
                    warn!("Selector {} failed: {:?}", selector, e);
                }
            }
        }
        
        // Fallback: get page title and basic content
        info!("No search results found, using fallback");
        let title = self.page.title().await
            .unwrap_or_else(|_| "Sem título".to_string());
        
        let content = self.page.content().await
            .map_err(|e| anyhow::anyhow!("Failed to get content: {}", e))?;
        
        let text = format!("Título: {}\n\nConteúdo:\n{}", title, &content[..content.len().min(2000)]);
        
        Ok(text)
    }

    pub async fn take_screenshot(&self, filename: &str) -> anyhow::Result<String> {
        // Ensure screenshots directory exists
        let screenshots_dir = Path::new("data/screenshots");
        std::fs::create_dir_all(screenshots_dir)?;
        
        let filepath = screenshots_dir.join(filename);

        // Take screenshot - playwright-rs returns bytes, we save to file
        let screenshot_bytes = self.page.screenshot(None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to take screenshot: {}", e))?;
        
        // Save bytes to file
        std::fs::write(&filepath, screenshot_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to save screenshot: {}", e))?;
        
        info!("Screenshot saved to: {:?}", filepath);
        
        Ok(filepath.to_string_lossy().to_string())
    }

    pub async fn extract_text(&self, selector: Option<&str>) -> anyhow::Result<String> {
        let text = if let Some(sel) = selector {
            let js_code = format!(
                r#"() => {{
                    const el = document.querySelector('{}');
                    return el ? el.innerText : 'Elemento não encontrado';
                }}"#,
                sel
            );
            
            self.page.evaluate::<String, String>(&js_code, None).await
                .map_err(|e| anyhow::anyhow!("Failed to extract text: {}", e))?
        } else {
            // Get full page text
            let js_code = r#"
                () => {
                    // Remove script and style elements
                    const scripts = document.querySelectorAll('script, style, nav, header, footer');
                    scripts.forEach(s => s.remove());
                    return document.body.innerText.substring(0, 5000);
                }
            "#;
            
            self.page.evaluate::<String, String>(js_code, None).await
                .map_err(|e| anyhow::anyhow!("Failed to extract text: {}", e))?
        };
        
        Ok(text)
    }

    pub async fn close(self) {
        info!("Closing browser session");
        // Playwright will automatically cleanup when dropped
    }
}

/// Test Playwright installation
pub async fn test_browser() -> anyhow::Result<String> {
    info!("Testing browser installation...");
    
    let session = BrowserSession::new().await?;
    
    // Navigate to a test page
    session.navigate("https://www.google.com").await?;
    
    // Get page title
    let title = session.page.title().await
        .unwrap_or_else(|_| "Unknown".to_string());
    
    session.close().await;
    
    Ok(format!("✅ Browser test passed!\nTest page title: {}", title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_playwright_initialization() {
        let result = test_browser().await;
        assert!(result.is_ok(), "Browser test failed: {:?}", result.err());
        println!("{}", result.unwrap());
    }
}
