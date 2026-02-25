use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use rand::Rng;

use chaser_oxide::{Browser, BrowserConfig, Page};
use chaser_oxide::page::ScreenshotParams;

pub struct BrowserManager {
    browser: Option<Browser>,
    page: Arc<RwLock<Option<Page>>>,
    data_dir: PathBuf,
}

impl BrowserManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            browser: None,
            page: Arc::new(RwLock::new(None)),
            data_dir,
        }
    }

    pub async fn initialize(&mut self) -> Result<(), String> {
        let config = BrowserConfig::builder()
            .build()
            .map_err(|e| format!("Failed to create browser config: {}", e))?;

        let (browser, _handler) = Browser::launch(config)
            .await
            .map_err(|e| format!("Failed to launch browser: {}", e))?;

        self.browser = Some(browser);
        Ok(())
    }

    pub async fn ensure_page(&self) -> Result<Page, String> {
        let mut page_lock = self.page.write().await;
        
        if page_lock.is_none() {
            let browser = self.browser.as_ref().ok_or("Browser not initialized")?;
            let page = browser.new_page("about:blank")
                .await
                .map_err(|e| format!("Failed to create page: {}", e))?;
            *page_lock = Some(page);
        }
        
        Ok(page_lock.as_ref().unwrap().clone())
    }

    fn random_delay_sync() -> u64 {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::from_entropy();
        rng.gen_range(100..1500)
    }

    async fn random_delay() {
        let delay = Self::random_delay_sync();
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }

    pub async fn navigate(&self, url: &str) -> Result<String, String> {
        Self::random_delay().await;
        
        let page = self.ensure_page().await?;
        
        page.goto(url)
            .await
            .map_err(|e| format!("Navigation failed: {}", e))?;
        
        Self::random_delay().await;
        
        let title = page.get_title().await.unwrap_or_else(|_| Some("Unknown".to_string())).unwrap_or_else(|| "Unknown".to_string());
        Ok(format!("Navigated to: {} ({})", url, title))
    }

    pub async fn click(&self, selector: &str) -> Result<String, String> {
        Self::random_delay().await;
        
        let page = self.ensure_page().await?;
        
        page.find_element(selector)
            .await
            .map_err(|e| format!("Element not found: {}", e))?
            .click()
            .await
            .map_err(|e| format!("Click failed: {}", e))?;

        Self::random_delay().await;
        Ok(format!("Clicked: {}", selector))
    }

    pub async fn fill(&self, selector: &str, value: &str) -> Result<String, String> {
        Self::random_delay().await;
        
        let page = self.ensure_page().await?;
        
        let element = page.find_element(selector)
            .await
            .map_err(|e| format!("Element not found: {}", e))?;
        
        for char in value.chars() {
            element.type_str(&char.to_string()).await.map_err(|e| format!("Type failed: {}", e))?;
            let delay = Self::random_delay_sync();
            tokio::time::sleep(Duration::from_millis(delay / 10)).await;
        }

        Self::random_delay().await;
        Ok(format!("Filled '{}' in {}", value, selector))
    }

    pub async fn screenshot(&self, path: &str) -> Result<String, String> {
        let page = self.ensure_page().await?;
        
        let params = ScreenshotParams::default();
        let data = page.screenshot(params).await.map_err(|e| format!("Screenshot failed: {}", e))?;

        std::fs::write(path, &data).map_err(|e| e.to_string())?;

        // Notify TMUX about screenshot
        crate::agent::output_write_browser(path, "Screenshot captured");
        
        Ok(format!("Screenshot saved to: {} ({} bytes)", path, data.len()))
    }

    pub async fn screenshot_base64(&self) -> Result<String, String> {
        let page = self.ensure_page().await?;
        
        let params = ScreenshotParams::default();
        let data = page.screenshot(params).await.map_err(|e| format!("Screenshot failed: {}", e))?;

        use base64::Engine;
        Ok(base64::engine::general_purpose::STANDARD.encode(&data))
    }

    pub async fn eval(&self, script: &str) -> Result<String, String> {
        let page = self.ensure_page().await?;
        
        let result = page.evaluate(script)
            .await
            .map_err(|e| format!("Eval failed: {}", e))?;

        Ok(format!("{:?}", result))
    }

    pub async fn get_text(&self, selector: &str) -> Result<String, String> {
        Self::random_delay().await;
        
        let page = self.ensure_page().await?;
        
        let element = page.find_element(selector)
            .await
            .map_err(|e| format!("Element not found: {}", e))?;
        
        let result = element.call_js_fn("textContent", false)
            .await
            .map_err(|e| format!("Get text failed: {}", e))?;

        Ok(format!("{:?}", result))
    }

    pub async fn get_html(&self) -> Result<String, String> {
        let page = self.ensure_page().await?;
        
        let html = page.content()
            .await
            .map_err(|e| format!("Get HTML failed: {}", e))?;

        Ok(html)
    }

    pub async fn close(&mut self) -> Result<(), String> {
        if let Some(page) = self.page.write().await.take() {
            let _ = page.close().await;
        }

        if let Some(mut browser) = self.browser.take() {
            browser.close().await.map_err(|e| format!("Failed to close browser: {}", e))?;
        }
        
        Ok(())
    }
}

impl Default for BrowserManager {
    fn default() -> Self {
        Self::new(PathBuf::from("data"))
    }
}
