use serde::{Deserialize, Serialize};

use crate::error::{AgentError, ConfigError};

/// Harness runtime configuration, loaded from environment.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub provider: String,
    pub max_iterations: usize,
    pub max_context_tokens: usize,
    pub temperature: f32,
}

impl Config {
    pub fn from_env() -> Result<Self, AgentError> {
        let api_key = std::env::var("TOKEN")
            .or_else(|_| std::env::var("OPENCODE_API_KEY"))
            .unwrap_or_default();

        if api_key.is_empty() {
            return Err(ConfigError::MissingToken.into());
        }
        if api_key.len() < 10 {
            return Err(ConfigError::InvalidModel(
                "API key seems too short. Please check your TOKEN environment variable."
                    .to_string(),
            )
            .into());
        }

        let provider = std::env::var("PROVIDER").unwrap_or_else(|_| "opencode-go".to_string());

        // Defaults come from the builtin provider catalog (single source of truth).
        let (default_base_url, default_model) =
            crate::harness::provider::catalog::find_provider(&provider)
                .map(|p| (p.base_url.to_string(), p.default_model.to_string()))
                .unwrap_or_else(|| {
                    // Unknown/custom providers default to the opencode-go endpoint.
                    let p = crate::harness::provider::catalog::find_provider("opencode-go")
                        .expect("opencode-go must be in catalog");
                    (p.base_url.to_string(), p.default_model.to_string())
                });

        let base_url = std::env::var("BASE_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or(default_base_url);
        let model = std::env::var("MODEL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or(default_model);

        let max_iterations = std::env::var("MAX_ITERATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50);
        let max_context_tokens = std::env::var("MAX_CONTEXT_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100_000);
        let temperature = std::env::var("TEMPERATURE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.7);

        Ok(Self {
            api_key,
            base_url,
            model,
            provider,
            max_iterations,
            max_context_tokens,
            temperature,
        })
    }

    #[allow(dead_code)]
    pub fn validate(&self) -> Result<(), AgentError> {
        if self.api_key.is_empty() {
            return Err(ConfigError::MissingToken.into());
        }
        if !self.base_url.starts_with("http://") && !self.base_url.starts_with("https://") {
            return Err(ConfigError::InvalidUrl(self.base_url.clone()).into());
        }
        Ok(())
    }
}
