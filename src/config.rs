use serde::{Deserialize, Serialize};
use std::result::Result;

use crate::error::{AgentError, ConfigError};

pub mod prompts;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub max_tokens: usize,
    pub max_context_tokens: usize,
    pub max_iterations: usize,
    pub plan_auto_threshold: usize,
    pub max_retries: usize,
    pub tavily_api_key: Option<String>,
    pub timezone: String,
    pub provider: String,
    pub fallback_models: Vec<FallbackModel>,
    pub agent_loop: AgentLoopConfig,
    pub self_review: SelfReviewConfig,
    pub embedding_model: EmbeddingModel,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentLoopConfig {
    pub auto_retry: bool,
    pub max_retries_per_step: usize,
    pub validation_required: bool,
    pub exit_on_error: ExitBehavior,
    pub force_tool_use: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ExitBehavior {
    Task,
    Session,
    Never,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            auto_retry: true,
            max_retries_per_step: 3,
            validation_required: true,
            exit_on_error: ExitBehavior::Task,
            force_tool_use: true,
        }
    }
}

impl std::fmt::Display for ExitBehavior {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExitBehavior::Task => write!(f, "task"),
            ExitBehavior::Session => write!(f, "session"),
            ExitBehavior::Never => write!(f, "never"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelfReviewConfig {
    pub enabled: bool,
    pub max_loops: usize,
    pub show_process: bool,
}

impl Default for SelfReviewConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_loops: 3,
            show_process: true,
        }
    }
}

impl From<&str> for ExitBehavior {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "session" => ExitBehavior::Session,
            "never" => ExitBehavior::Never,
            _ => ExitBehavior::Task,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FallbackModel {
    pub model: String,
    pub base_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub enum EmbeddingModel {
    #[default]
    OpenAI,
    Cohere,
    Local,
}

impl EmbeddingModel {
    pub fn from_env() -> Self {
        let model_str = std::env::var("EMBEDDING_MODEL")
            .unwrap_or_else(|_| "openai".to_string())
            .to_lowercase();

        match model_str.as_str() {
            "cohere" => EmbeddingModel::Cohere,
            "local" => EmbeddingModel::Local,
            _ => EmbeddingModel::OpenAI,
        }
    }
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

        let tavily_api_key = std::env::var("TAVILY_API_KEY").ok();
        let timezone = std::env::var("TZ").unwrap_or_else(|_| "America/Sao_Paulo".to_string());
        let max_tokens = std::env::var("MAX_TOKENS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(4000);

        let max_context_tokens = std::env::var("MAX_CONTEXT_TOKENS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(128_000);

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

        // Get default base_url and model for the provider
        let (default_base_url, default_model) = match provider.as_str() {
            "moonshot" => (
                "https://api.moonshot.ai/v1".to_string(),
                "kimi-k2.5".to_string(),
            ),
            "opencode-go" | "opencode" => (
                "https://opencode.ai/zen/go/v1".to_string(),
                "minimax-m2.7".to_string(),
            ),
            "openrouter" => (
                "https://openrouter.ai/api/v1".to_string(),
                "minimax-minimax-max".to_string(),
            ),
            "villamarket" => (
                "https://api.minimax.villamarket.ai/v1".to_string(),
                "minimax-m2.7".to_string(),
            ),
            "huggingface" => (
                "https://router.huggingface.co/v1".to_string(),
                "Qwen/Qwen3-Coder-Next".to_string(),
            ),
            _ => (
                "https://opencode.ai/zen/go/v1".to_string(),
                "minimax-m2.7".to_string(),
            ),
        };

        // Allow override from environment variables (only if non-empty)
        let base_url = std::env::var("BASE_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or(default_base_url);
        let model = std::env::var("MODEL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or(default_model);

        let fallback_models = Self::load_fallback_models();

        // Agent loop configuration
        let agent_loop = AgentLoopConfig {
            auto_retry: std::env::var("AGENT_AUTO_RETRY")
                .map(|v| v != "false")
                .unwrap_or(true),
            max_retries_per_step: std::env::var("AGENT_MAX_RETRIES_PER_STEP")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
            validation_required: std::env::var("AGENT_VALIDATION_REQUIRED")
                .map(|v| v != "false")
                .unwrap_or(true),
            exit_on_error: ExitBehavior::from(
                std::env::var("AGENT_EXIT_ON_ERROR")
                    .as_deref()
                    .unwrap_or("task"),
            ),
            force_tool_use: std::env::var("AGENT_FORCE_TOOL_USE")
                .map(|v| v != "false")
                .unwrap_or(true),
        };

        // Self-review configuration
        let self_review = SelfReviewConfig {
            enabled: std::env::var("SELF_REVIEW_ENABLED")
                .map(|v| v != "false")
                .unwrap_or(true),
            max_loops: std::env::var("SELF_REVIEW_MAX_LOOPS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
            show_process: std::env::var("SELF_REVIEW_SHOW_PROCESS")
                .map(|v| v != "false")
                .unwrap_or(true),
        };

        Ok(Self {
            api_key,
            base_url,
            model,
            max_tokens,
            max_context_tokens,
            max_iterations,
            plan_auto_threshold,
            max_retries,
            tavily_api_key,
            timezone,
            provider,
            fallback_models,
            agent_loop,
            self_review,
            embedding_model: EmbeddingModel::from_env(),
        })
    }

    pub fn validate(&self) -> Result<(), AgentError> {
        let mut errors: Vec<String> = Vec::new();

        if self.api_key.is_empty() {
            errors.push("API key is empty. Set TOKEN environment variable.".to_string());
        } else if self.api_key.len() < 10 {
            errors.push("API key seems too short. Please check your TOKEN.".to_string());
        }

        if !self.base_url.starts_with("http://") && !self.base_url.starts_with("https://") {
            errors.push(format!(
                "Invalid BASE_URL: '{}'. Must start with http:// or https://",
                self.base_url
            ));
        }

        if self.max_tokens == 0 {
            errors.push("MAX_TOKENS cannot be 0.".to_string());
        } else if self.max_tokens > 100_000 {
            errors.push("MAX_TOKENS seems too high (max 100000).".to_string());
        }

        if self.max_context_tokens == 0 {
            errors.push("MAX_CONTEXT_TOKENS cannot be 0.".to_string());
        }

        if self.max_iterations == 0 {
            errors.push("MAX_ITERATIONS cannot be 0.".to_string());
        }

        if self.timezone.is_empty() {
            errors.push("TZ (timezone) cannot be empty.".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            let msg = format!("Configuration errors:\n  - {}", errors.join("\n  - "));
            Err(ConfigError::InvalidUrl(msg).into())
        }
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
        }
        // Default: empty - no fallbacks unless user configures them
        // Fallbacks like OpenRouter/HuggingFace need their own API keys

        fallbacks
    }
}
