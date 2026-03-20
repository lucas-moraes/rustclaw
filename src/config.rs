use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub max_tokens: usize,
    pub max_iterations: usize,
    pub plan_auto_threshold: usize,
    pub tavily_api_key: Option<String>,
    pub timezone: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let api_key = std::env::var("HF_TOKEN")
            .map_err(|_| anyhow::anyhow!("HF_TOKEN environment variable not set"))?;

        let tavily_api_key = std::env::var("TAVILY_API_KEY").ok();
        let timezone = std::env::var("TZ").unwrap_or_else(|_| "America/Sao_Paulo".to_string());
        let max_tokens = std::env::var("MAX_TOKENS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(4000);

        let max_iterations = std::env::var("MAX_ITERATIONS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(20);

        let plan_auto_threshold = std::env::var("PLAN_AUTO_THRESHOLD")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(4);

        Ok(Self {
            api_key,
            base_url: "https://router.huggingface.co/v1".to_string(),
            model: "moonshotai/Kimi-K2-Thinking".to_string(),
            max_tokens,
            max_iterations,
            plan_auto_threshold,
            tavily_api_key,
            timezone,
        })
    }
}
