//! opencode-go provider: routes by model.
//! - MiniMax models use the Anthropic-style `/messages` endpoint.
//! - Others use the OpenAI-style `/chat/completions` endpoint.
//!
//! Auth uses `X-API-Key` (with Bearer fallback header).

use super::{
    anthropic::AnthropicProvider, openai::OpenAiProvider, AuthStyle, HttpConfig, LlmRequest,
    LlmResponse, Provider, ProviderStream,
};

pub struct OpenCodeGoProvider {
    pub http: HttpConfig,
    /// Passed through to the Anthropic-style route (MiniMax). Default false.
    pub prompt_cache: bool,
    /// Anthropic cache TTL passed through to the /messages route. Empty = "5m".
    pub cache_ttl: String,
}

impl OpenCodeGoProvider {
    fn is_minimax(model: &str) -> bool {
        model.contains("minimax")
    }

    fn openai(&self) -> OpenAiProvider {
        OpenAiProvider {
            http: self.http.clone(),
            auth: AuthStyle::ApiKey,
        }
    }

    fn anthropic(&self) -> AnthropicProvider {
        AnthropicProvider {
            http: self.http.clone(),
            auth: AuthStyle::ApiKey,
            prompt_cache: self.prompt_cache,
            cache_ttl: self.cache_ttl.clone(),
            native: false,
        }
    }
}

#[async_trait::async_trait]
impl Provider for OpenCodeGoProvider {
    fn name(&self) -> &str {
        "opencode-go"
    }

    async fn stream(&self, req: &LlmRequest) -> anyhow::Result<ProviderStream> {
        if Self::is_minimax(&req.model) {
            self.anthropic().stream(req).await
        } else {
            self.openai().stream(req).await
        }
    }

    async fn complete(&self, req: &LlmRequest) -> anyhow::Result<LlmResponse> {
        if Self::is_minimax(&req.model) {
            self.anthropic().complete(req).await
        } else {
            self.openai().complete(req).await
        }
    }
}

/// Factory: builds the provider for a configured provider name.
///
/// `prompt_cache` controls Anthropic-style `cache_control` breakpoints; the
/// per-provider default is `true` for `anthropic`, `false` otherwise (see
/// `default_prompt_cache`).
pub fn build_provider(
    provider: &str,
    http: HttpConfig,
    prompt_cache: bool,
    cache_ttl: &str,
) -> anyhow::Result<std::sync::Arc<dyn Provider>> {
    let provider = provider.to_lowercase();
    let provider = provider.as_str();
    match provider {
        "opencode-go" | "opencode" => Ok(std::sync::Arc::new(OpenCodeGoProvider {
            http,
            prompt_cache,
            cache_ttl: cache_ttl.to_string(),
        })),
        "anthropic" => Ok(std::sync::Arc::new(AnthropicProvider {
            http,
            auth: AuthStyle::ApiKey,
            prompt_cache,
            cache_ttl: cache_ttl.to_string(),
            native: true,
        })),
        // openrouter, moonshot, villamarket, huggingface, custom => OpenAI-compatible
        _ => Ok(std::sync::Arc::new(OpenAiProvider {
            http,
            auth: AuthStyle::Bearer,
        })),
    }
}

/// Per-provider default for prompt caching: Anthropic-style endpoints support
/// `cache_control`; MiniMax (via opencode-go) may reject it.
pub fn default_prompt_cache(provider: &str) -> bool {
    provider.eq_ignore_ascii_case("anthropic")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_minimax() {
        assert!(OpenCodeGoProvider::is_minimax("minimax-m2.7"));
        assert!(!OpenCodeGoProvider::is_minimax("qwen3-coder"));
    }

    #[test]
    fn test_build_provider_routing() {
        let http = HttpConfig {
            client: crate::harness::provider::build_http_client(),
            base_url: "https://example.com/v1".into(),
            api_key: "key".into(),
        };
        let p = build_provider("opencode-go", http.clone(), false, "").unwrap();
        assert_eq!(p.name(), "opencode-go");
        let p = build_provider("openrouter", http.clone(), false, "").unwrap();
        assert_eq!(p.name(), "openai-compatible");
        let p = build_provider("anthropic", http, true, "").unwrap();
        assert_eq!(p.name(), "anthropic-compatible");
    }

    #[test]
    fn test_default_prompt_cache() {
        assert!(default_prompt_cache("anthropic"));
        assert!(default_prompt_cache("Anthropic"));
        assert!(!default_prompt_cache("opencode-go"));
        assert!(!default_prompt_cache("deepinfra"));
    }
}
