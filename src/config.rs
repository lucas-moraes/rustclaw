use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub max_iterations: usize,
    pub tavily_api_key: Option<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let api_key = std::env::var("HF_TOKEN")
            .map_err(|_| anyhow::anyhow!("HF_TOKEN environment variable not set"))?;

        let tavily_api_key = std::env::var("TAVILY_API_KEY").ok();

        Ok(Self {
            api_key,
            base_url: "https://router.huggingface.co/v1".to_string(),
            model: "Qwen/Qwen2.5-72B-Instruct".to_string(),
            max_iterations: 5,
            tavily_api_key,
        })
    }
}
