//! Provider catalog: builtin providers plus user-defined ones merged at
//! runtime. Single source of truth for defaults in `config.rs` and for the
//! `/models` picker. Free-form model input is always allowed.
//!
//! Builtins are static; user providers come from `providers.json` (see
//! `user_store`). A user provider with the same name as a builtin overrides
//! it (name → base_url/models).

use crate::harness::provider::user_store::{UserProvider, UserProviders};

/// A known provider and its connection defaults (owned strings).
#[derive(Clone, Debug)]
pub struct ProviderInfo {
    /// Value accepted by `PROVIDER` env / `build_provider`.
    pub name: String,
    pub base_url: String,
    pub default_model: String,
    /// A few well-known models for the picker (non-exhaustive).
    pub models: Vec<String>,
    /// True when this entry came from the user store (not builtin).
    pub user_defined: bool,
}

/// Builtin provider registry (display order = picker order).
pub const BUILTINS: &[(&str, &str, &str, &[&str])] = &[
    (
        "deepinfra",
        "https://api.deepinfra.com/v1/openai",
        "deepseek-ai/DeepSeek-V4-Flash-0731",
        &[
            "deepseek-ai/DeepSeek-V4-Flash-0731",
            "deepseek-ai/DeepSeek-V4-0324",
            "Qwen/Qwen3-Coder-480B-A35B-Instruct",
            "zai-org/GLM-5.3",
            "meta-llama/Llama-4-Maverick-17B-128E-Instruct-FP8",
            "mistralai/Devstral-Small-2507",
        ],
    ),
    (
        "xai",
        "https://api.x.ai/v1",
        "grok-4.5",
        &["grok-4.5", "grok-4.6", "grok-4.3", "grok-build-0.1"],
    ),
    (
        "opencode-go",
        "https://opencode.ai/zen/go/v1",
        "deepseek-v4-flash",
        &[
            "deepseek-v4-flash",
            "grok-code",
            "qwen3-coder",
            "claude-sonnet-4-6",
            "gpt-5-nano",
        ],
    ),
    (
        "openrouter",
        "https://openrouter.ai/api/v1",
        "deepseek-ai/DeepSeek-V4-Flash-0731",
        &[
            "deepseek-ai/DeepSeek-V4-Flash-0731",
            "anthropic/claude-sonnet-4.6",
            "openai/gpt-5-codex",
            "google/gemini-3-pro",
            "z-ai/glm-4.6",
            "qwen/qwen3-coder-plus",
        ],
    ),
    (
        "moonshot",
        "https://api.moonshot.ai/v1",
        "kimi-k2.5",
        &["kimi-k2.5", "kimi-k2-0905-preview", "moonshot-v1-128k"],
    ),
    (
        "huggingface",
        "https://router.huggingface.co/v1",
        "Qwen/Qwen3-Coder-Next",
        &[
            "Qwen/Qwen3-Coder-Next",
            "deepseek-ai/DeepSeek-V4-0324",
            "meta-llama/Llama-4-Maverick-17B-128E-Instruct",
        ],
    ),
    (
        "villamarket",
        "https://api.minimax.villamarket.ai/v1",
        "minimax-m2.7",
        &["minimax-m2.7", "minimax-m2.5"],
    ),
    (
        "anthropic",
        "https://api.anthropic.com/v1",
        "claude-sonnet-4-20250514",
        &[
            "claude-sonnet-4-20250514",
            "claude-opus-4-20250514",
            "claude-3-7-sonnet-latest",
        ],
    ),
];

fn builtin_info(
    (name, base_url, default_model, models): &(&str, &str, &str, &[&str]),
) -> ProviderInfo {
    ProviderInfo {
        name: (*name).to_string(),
        base_url: (*base_url).to_string(),
        default_model: (*default_model).to_string(),
        models: models.iter().map(|m| (*m).to_string()).collect(),
        user_defined: false,
    }
}

fn user_info(p: &UserProvider) -> ProviderInfo {
    let models = if p.models.is_empty() {
        if p.default_model.is_empty() {
            Vec::new()
        } else {
            vec![p.default_model.clone()]
        }
    } else {
        p.models.clone()
    };
    ProviderInfo {
        name: p.name.clone(),
        base_url: p.base_url.clone(),
        default_model: p.default_model.clone(),
        models,
        user_defined: true,
    }
}

/// All providers (builtins + user-defined), in picker order. A user provider
/// with the same name as a builtin replaces it in place.
pub fn all_providers() -> Vec<ProviderInfo> {
    let user = UserProviders::load();
    let mut out: Vec<ProviderInfo> = BUILTINS.iter().map(builtin_info).collect();
    for up in &user.providers {
        if let Some(slot) = out.iter_mut().find(|p| p.name == up.name) {
            *slot = user_info(up);
        } else {
            out.push(user_info(up));
        }
    }
    out
}

/// Looks up provider info by name (case-insensitive).
pub fn find_provider(name: &str) -> Option<ProviderInfo> {
    let lower = name.to_lowercase();
    all_providers()
        .into_iter()
        .find(|p| p.name.to_lowercase() == lower)
}

/// Well-known models for a provider (may be empty for unknown providers).
pub fn models_for(provider: &str) -> Vec<String> {
    find_provider(provider)
        .map(|p| p.models)
        .unwrap_or_default()
}

/// Default base URL for a provider (unknown → None).
pub fn default_base_url(name: &str) -> Option<String> {
    find_provider(name).map(|p| p.base_url)
}

/// Default model for a provider (unknown → None).
pub fn default_model(name: &str) -> Option<String> {
    find_provider(name).map(|p| p.default_model)
}

/// Provider names in picker order.
pub fn provider_names() -> Vec<String> {
    all_providers().into_iter().map(|p| p.name).collect()
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
    fn test_all_builtins_have_defaults_and_models() {
        for p in all_providers() {
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
            "xai",
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
            Some("https://api.moonshot.ai/v1".to_string())
        );
        assert_eq!(
            default_model("villamarket"),
            Some("minimax-m2.7".to_string())
        );
        assert!(default_base_url("ghost").is_none());
        assert!(provider_names().len() >= 6);
    }

    #[test]
    fn test_user_provider_overrides_builtin() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("providers.json");
        let mut store = UserProviders::default();
        store.upsert(UserProvider {
            name: "xai".into(),
            base_url: "https://custom.x.ai/v1".into(),
            default_model: "custom-model".into(),
            models: vec!["custom-model".into()],
        });
        store.save_to(&path).unwrap();

        // Point the store at the temp file for the duration of the test.
        let orig = UserProviders::path();
        // (path() is not overridable; instead verify merge logic directly)
        let _ = orig;
        let _ = path;

        // Direct merge check without touching the real store:
        let mut out: Vec<ProviderInfo> = BUILTINS.iter().map(builtin_info).collect();
        for up in &store.providers {
            if let Some(slot) = out.iter_mut().find(|p| p.name == up.name) {
                *slot = user_info(up);
            } else {
                out.push(user_info(up));
            }
        }
        let xai = out.iter().find(|p| p.name == "xai").unwrap();
        assert!(xai.user_defined);
        assert_eq!(xai.base_url, "https://custom.x.ai/v1");
        assert_eq!(xai.default_model, "custom-model");
    }
}
