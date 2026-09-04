//! opencode-go provider: routes by model.
//! - MiniMax models use the Anthropic-style `/messages` endpoint.
//! - Others use the OpenAI-style `/chat/completions` endpoint.
//! Auth uses `X-API-Key` (with Bearer fallback header).

use super::{
    anthropic::AnthropicProvider, openai::OpenAiProvider, AuthStyle, HttpConfig, LlmRequest,
    LlmResponse, Provider, ProviderStream,
};

pub struct OpenCodeGoProvider {
    pub http: HttpConfig,
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
pub fn build_provider(
    provider: &str,
    http: HttpConfig,
) -> anyhow::Result<std::sync::Arc<dyn Provider>> {
    let provider = provider.to_lowercase();
    let provider = provider.as_str();
    match provider {
        "opencode-go" | "opencode" => Ok(std::sync::Arc::new(OpenCodeGoProvider { http })),
        "anthropic" => Ok(std::sync::Arc::new(AnthropicProvider {
            http,
            auth: AuthStyle::Bearer,
        })),
        // openrouter, moonshot, villamarket, huggingface, custom => OpenAI-compatible
        _ => Ok(std::sync::Arc::new(OpenAiProvider {
            http,
            auth: AuthStyle::Bearer,
        })),
    }
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
        let p = build_provider("opencode-go", http.clone()).unwrap();
        assert_eq!(p.name(), "opencode-go");
        let p = build_provider("openrouter", http.clone()).unwrap();
        assert_eq!(p.name(), "openai-compatible");
        let p = build_provider("anthropic", http).unwrap();
        assert_eq!(p.name(), "anthropic-compatible");
    }
}
