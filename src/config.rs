use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub max_iterations: usize,
    pub tavily_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub conversation_history_limit: usize,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let api_key = std::env::var("HF_TOKEN")
            .map_err(|_| anyhow::anyhow!("HF_TOKEN environment variable not set"))?;

        let tavily_api_key = std::env::var("TAVILY_API_KEY").ok();
        let openai_api_key = std::env::var("OPENAI_API_KEY").ok();

        Ok(Self {
            api_key,
            base_url: "https://router.huggingface.co/v1".to_string(),
            model: "Qwen/Qwen2.5-72B-Instruct".to_string(),
            max_iterations: 5,
            tavily_api_key,
            openai_api_key,
            conversation_history_limit: 10,
        })
    }
}
