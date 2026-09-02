//! Builtin provider catalog: known providers, their default base URLs and
//! models. Single source of truth for defaults in `config.rs` and for the
//! `/models` picker. Free-form model input is always allowed.

/// A known provider and its connection defaults.
#[derive(Clone, Copy, Debug)]
pub struct ProviderInfo {
    /// Value accepted by `PROVIDER` env / `build_provider`.
    pub name: &'static str,
    pub base_url: &'static str,
    pub default_model: &'static str,
    /// A few well-known models for the picker (non-exhaustive).
    pub models: &'static [&'static str],
}

/// Builtin provider registry (display order = picker order).
pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        name: "deepinfra",
        base_url: "https://api.deepinfra.com/v1/openai",
        default_model: "deepseek-ai/DeepSeek-V4-Flash-0731",
        models: &[
            "deepseek-ai/DeepSeek-V4-Flash-0731",
            "deepseek-ai/DeepSeek-V4-0324",
            "Qwen/Qwen3-Coder-480B-A35B-Instruct",
            "zai-org/GLM-5.3",
            "meta-llama/Llama-4-Maverick-17B-128E-Instruct-FP8",
            "mistralai/Devstral-Small-2507",
        ],
    },
    ProviderInfo {
        name: "opencode-go",
        base_url: "https://opencode.ai/zen/go/v1",
        default_model: "deepseek-v4-flash",
        models: &[
            "deepseek-v4-flash",
            "grok-code",
            "qwen3-coder",
            "claude-sonnet-4-6",
            "gpt-5-nano",
        ],
    },
    ProviderInfo {
        name: "openrouter",
        base_url: "https://openrouter.ai/api/v1",
        default_model: "deepseek-ai/DeepSeek-V4-Flash-0731",
        models: &[
            "deepseek-ai/DeepSeek-V4-Flash-0731",
            "anthropic/claude-sonnet-4.6",
            "openai/gpt-5-codex",
            "google/gemini-3-pro",
            "z-ai/glm-4.6",
            "qwen/qwen3-coder-plus",
        ],
    },
    ProviderInfo {
        name: "moonshot",
        base_url: "https://api.moonshot.ai/v1",
        default_model: "kimi-k2.5",
        models: &["kimi-k2.5", "kimi-k2-0905-preview", "moonshot-v1-128k"],
    },
    ProviderInfo {
        name: "huggingface",
        base_url: "https://router.huggingface.co/v1",
        default_model: "Qwen/Qwen3-Coder-Next",
        models: &[
            "Qwen/Qwen3-Coder-Next",
            "deepseek-ai/DeepSeek-V4-0324",
            "meta-llama/Llama-4-Maverick-17B-128E-Instruct",
        ],
    },
    ProviderInfo {
        name: "villamarket",
        base_url: "https://api.minimax.villamarket.ai/v1",
        default_model: "minimax-m2.7",
        models: &["minimax-m2.7", "minimax-m2.5"],
    },
    ProviderInfo {
        name: "anthropic",
        base_url: "https://api.anthropic.com/v1",
        default_model: "claude-sonnet-4-20250514",
        models: &[
            "claude-sonnet-4-20250514",
            "claude-opus-4-20250514",
            "claude-3-7-sonnet-latest",
        ],
    },
];

/// Looks up provider info by name (case-insensitive).
pub fn find_provider(name: &str) -> Option<&'static ProviderInfo> {
    let lower = name.to_lowercase();
    PROVIDERS.iter().find(|p| p.name == lower)
}

/// Well-known models for a provider (may be empty for unknown providers).
pub fn models_for(provider: &str) -> Vec<&'static str> {
    find_provider(provider)
        .map(|p| p.models.to_vec())
        .unwrap_or_default()
}

/// Default base URL for a provider (unknown → openai-compatible default).
pub fn default_base_url(name: &str) -> Option<&'static str> {
    find_provider(name).map(|p| p.base_url)
}

/// Default model for a provider (unknown → None).
pub fn default_model(name: &str) -> Option<&'static str> {
    find_provider(name).map(|p| p.default_model)
}

/// Provider names in picker order.
pub fn provider_names() -> Vec<&'static str> {
    PROVIDERS.iter().map(|p| p.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_provider_case_insensitive() {
        assert!(find_provider("DEEPINFRA").is_some());
        assert!(find_provider("deepinfra").is_some());
        assert!(find_provider("unknown-xyz").is_none());
    }

    #[test]
    fn test_all_providers_have_defaults_and_models() {
        for p in PROVIDERS {
            assert!(!p.base_url.is_empty(), "missing base_url: {}", p.name);
            assert!(!p.default_model.is_empty(), "missing model: {}", p.name);
            assert!(!p.models.is_empty(), "missing models: {}", p.name);
            assert!(p.models.contains(&p.default_model));
        }
        // config.rs defaults must exist in the catalog.
        for name in [
            "moonshot",
            "opencode-go",
            "openrouter",
            "deepinfra",
            "villamarket",
            "huggingface",
        ] {
            assert!(
                find_provider(name).is_some(),
                "config provider missing: {}",
                name
            );
        }
    }

    #[test]
    fn test_helpers() {
        assert_eq!(
            default_base_url("moonshot"),
            Some("https://api.moonshot.ai/v1")
        );
        assert_eq!(default_model("villamarket"), Some("minimax-m2.7"));
        assert!(default_base_url("ghost").is_none());
        assert!(provider_names().len() >= 6);
    }
}
