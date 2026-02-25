use crate::browser::BrowserManager;
use crate::tools::Tool;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct BrowserTool {
    manager: Arc<RwLock<Option<BrowserManager>>>,
    data_dir: PathBuf,
}

impl BrowserTool {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            manager: Arc::new(RwLock::new(None)),
            data_dir,
        }
    }

    async fn get_manager(&self) -> Result<Arc<RwLock<Option<BrowserManager>>>, String> {
        let mut manager = self.manager.write().await;
        
        if manager.is_none() {
            let mut bm = BrowserManager::new(self.data_dir.clone());
            bm.initialize().await?;
            *manager = Some(bm);
        }
        
        Ok(self.manager.clone())
    }
}

#[async_trait::async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Automate browser actions using Chromiumoxide (Chrome DevTools Protocol). Actions: navigate, click, fill, screenshot, screenshot_base64, eval, get_text, html"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let action = args["action"]
            .as_str()
            .ok_or("Parameter 'action' is required")?;

        let manager_lock = self.get_manager().await?;
        let manager = manager_lock.read().await;
        let manager = manager.as_ref().ok_or("Manager not initialized")?;

        match action {
            "navigate" => {
                let url = args["url"]
                    .as_str()
                    .ok_or("Parameter 'url' is required for navigate")?;
                manager.navigate(url).await
            }

            "click" => {
                let selector = args["selector"]
                    .as_str()
                    .ok_or("Parameter 'selector' is required for click")?;
                manager.click(selector).await
            }

            "fill" => {
                let selector = args["selector"]
                    .as_str()
                    .ok_or("Parameter 'selector' is required for fill")?;
                let value = args["value"]
                    .as_str()
                    .ok_or("Parameter 'value' is required for fill")?;
                manager.fill(selector, value).await
            }

            "screenshot" => {
                let custom_path = args["path"].as_str();
                
                let path = if let Some(path) = custom_path {
                    path.to_string()
                } else if let Some(tmux) = crate::agent::get_tmux_manager() {
                    let browser_dir = tmux.browser_dir();
                    std::fs::create_dir_all(&browser_dir).ok();
                    let count = std::fs::read_dir(&browser_dir)
                        .map(|d| d.count())
                        .unwrap_or(0);
                    format!("{}/{}.png", browser_dir.display(), count)
                } else {
                    "screenshot.png".to_string()
                };
                
                manager.screenshot(&path).await
            }

            "screenshot_base64" => {
                manager.screenshot_base64().await
            }

            "eval" => {
                let script = args["script"]
                    .as_str()
                    .ok_or("Parameter 'script' is required for eval")?;
                manager.eval(script).await
            }

            "get_text" => {
                let selector = args["selector"]
                    .as_str()
                    .ok_or("Parameter 'selector' is required for get_text")?;
                manager.get_text(selector).await
            }

            "html" => {
                manager.get_html().await
            }

            _ => Err(format!("Unknown action: {}. Available: navigate, click, fill, screenshot, screenshot_base64, eval, get_text, html", action)),
        }
    }
}
