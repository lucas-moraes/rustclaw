use crate::auth::get_api_key;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub max_tokens: usize,
    pub max_iterations: usize,
    pub plan_auto_threshold: usize,
    pub max_retries: usize,
    pub tavily_api_key: Option<String>,
    pub timezone: String,
    pub provider: String,
    pub fallback_models: Vec<FallbackModel>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FallbackModel {
    pub model: String,
    pub base_url: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let api_key = std::env::var("OPENCODE_API_KEY")
            .or_else(|_| std::env::var("TOKEN"))
            .map_err(|_| {
                anyhow::anyhow!("OPENCODE_API_KEY or TOKEN environment variable not set")
            })?;

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

        let max_retries = std::env::var("MAX_RETRIES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(5);

        let provider = std::env::var("PROVIDER").unwrap_or_else(|_| "opencode-go".to_string());

        let (base_url, model) = match provider.as_str() {
            "moonshot" => (
                "https://api.moonshot.ai/v1".to_string(),
                "kimi-k2.5".to_string(),
            ),
            "opencode-go" | "opencode" => (
                "https://opencode.ai/zen/go/v1".to_string(),
                "minimax-m2.5".to_string(),
            ),
            "openrouter" => (
                "https://openrouter.ai/api/v1".to_string(),
                "minimax/minimax-m2.5:free".to_string(),
            ),
            "villamarket" => (
                "https://api.minimax.villamarket.ai/v1".to_string(),
                "minimax-m2.5".to_string(),
            ),
            "huggingface" => (
                "https://router.huggingface.co/v1".to_string(),
                "Qwen/Qwen3-Coder-Next".to_string(),
            ),
            _ => (
                "https://opencode.ai/zen/go/v1".to_string(),
                "minimax-m2.5".to_string(),
            ),
        };

        let fallback_models = Self::load_fallback_models();

        Ok(Self {
            api_key,
            base_url,
            model,
            max_tokens,
            max_iterations,
            plan_auto_threshold,
            max_retries,
            tavily_api_key,
            timezone,
            provider,
            fallback_models,
        })
    }

    fn load_fallback_models() -> Vec<FallbackModel> {
        let mut fallbacks = Vec::new();

        if let Ok(fallback_config) = std::env::var("FALLBACK_MODELS") {
            for line in fallback_config.split(',') {
                let parts: Vec<&str> = line.trim().split('|').collect();
                if parts.len() >= 2 {
                    fallbacks.push(FallbackModel {
                        model: parts[0].trim().to_string(),
                        base_url: parts[1].trim().to_string(),
                    });
                }
            }
        } else {
            fallbacks.push(FallbackModel {
                model: "minimax/minimax-m2.5:free".to_string(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
            });
            fallbacks.push(FallbackModel {
                model: "Qwen/Qwen3-Coder-Next".to_string(),
                base_url: "https://router.huggingface.co/v1".to_string(),
            });
        }

        fallbacks
    }
}
