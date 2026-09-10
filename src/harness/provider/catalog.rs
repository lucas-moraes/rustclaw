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
/// with the same name as a builtin replaces it in place; a tombstone entry
/// (`removed`) hides the builtin from the list.
pub fn all_providers() -> Vec<ProviderInfo> {
    merge(&UserProviders::load().providers)
}

/// Merges builtins with user entries (tombstones hide builtins, user
/// providers replace same-name builtins, unknown names are appended).
pub(crate) fn merge(user: &[UserProvider]) -> Vec<ProviderInfo> {
    let hidden = |name: &str| {
        user.iter()
            .any(|u| u.name.eq_ignore_ascii_case(name) && u.removed)
    };
    let mut out: Vec<ProviderInfo> = BUILTINS
        .iter()
        .map(builtin_info)
        .filter(|p| !hidden(&p.name))
        .collect();
    for up in user.iter().filter(|up| !up.removed) {
        if let Some(slot) = out
            .iter_mut()
            .find(|p| p.name.eq_ignore_ascii_case(&up.name))
        {
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

// ─── Estimated pricing (USD per 1M tokens) ─────────────────────────────────

/// Estimated USD per 1M tokens for the given `provider`+`model`, as
/// `(input, output)`. Resolution order: exact model match → provider default
/// → generic default. Used only for rough cost estimates in the sidebar.
pub fn price_per_million(provider: &str, model: &str) -> (f64, f64) {
    // Generic fallback for providers/models we have no data for.
    const GENERIC: (f64, f64) = (1.0, 3.0);

    let provider = provider.to_lowercase();
    let model_l = model.to_lowercase();

    // (provider, model-substring, input_$/1M, output_$/1M)
    const OVERRIDES: &[(&str, &str, f64, f64)] = &[
        ("deepinfra", "deepseek-v4-flash", 0.25, 1.00),
        ("deepinfra", "deepseek-v4", 1.25, 2.50),
        ("deepinfra", "qwen3-coder", 0.30, 0.90),
        ("deepinfra", "glm-5", 0.50, 1.50),
        ("deepinfra", "llama-4", 0.25, 0.75),
        ("deepinfra", "devstral", 0.20, 0.80),
        ("xai", "grok-build", 0.30, 0.90),
        ("xai", "grok-4", 3.00, 15.00),
        ("opencode-go", "deepseek-v4-flash", 0.25, 1.00),
        ("opencode-go", "grok-code", 3.00, 15.00),
        ("opencode-go", "qwen3-coder", 0.30, 0.90),
        ("opencode-go", "claude-sonnet", 3.00, 15.00),
        ("opencode-go", "gpt-5-nano", 0.50, 2.00),
        ("openrouter", "claude-sonnet", 3.00, 15.00),
        ("openrouter", "gpt-5-codex", 2.50, 10.00),
        ("openrouter", "gemini-3-pro", 2.00, 12.00),
        ("openrouter", "deepseek-v4-flash", 0.25, 1.00),
        ("moonshot", "kimi-k2", 1.00, 8.00),
        ("moonshot", "moonshot-v1", 0.15, 0.30),
        ("huggingface", "deepseek-v4", 1.25, 2.50),
        ("huggingface", "llama-4", 0.25, 0.75),
        ("villamarket", "minimax-m2", 2.00, 8.00),
        ("anthropic", "claude-opus-4", 15.00, 75.00),
        ("anthropic", "claude-sonnet", 3.00, 15.00),
        ("anthropic", "claude-3-7", 3.00, 15.00),
    ];

    for &(p, sub, i, o) in OVERRIDES {
        if provider == p && model_l.contains(sub) {
            return (i, o);
        }
    }

    let pdef = match provider.as_str() {
        "deepinfra" => (0.25, 1.00),
        "xai" => (3.00, 15.00),
        "opencode-go" => (1.00, 3.00),
        "openrouter" => (1.00, 3.00),
        "moonshot" => (1.00, 8.00),
        "huggingface" => (1.00, 3.00),
        "villamarket" => (2.00, 8.00),
        "anthropic" => (3.00, 15.00),
        _ => GENERIC,
    };
    pdef
}

/// Estimated cost in USD for the given token usage on `provider`/`model`.
/// Kept for tests and simple callers; prefer [`estimate_cost_cached`].
#[allow(dead_code)]
pub fn estimate_cost(provider: &str, model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let (pi, po) = price_per_million(provider, model);
    (input_tokens as f64 / 1_000_000.0) * pi + (output_tokens as f64 / 1_000_000.0) * po
}

/// Cache-aware cost estimate in USD for the given `provider`/`model`.
///
/// - Anthropic: `input_tokens` EXCLUDES cache tokens, so billable input is
///   `input + 1.25×cache_write + 0.1×cache_read`.
/// - OpenAI-compatible: `prompt_tokens` INCLUDES cached tokens, so billable
///   input is `(input − cache_read) + 0.5×cache_read` (writes don't exist).
pub fn estimate_cost_cached(
    provider: &str,
    model: &str,
    usage: &crate::harness::provider::Usage,
) -> f64 {
    let (pi, po) = price_per_million(provider, model);
    let per_m = |t: f64| t / 1_000_000.0;
    let input_billable = if provider.eq_ignore_ascii_case("anthropic") {
        usage.input_tokens as f64
            + 1.25 * usage.cache_write_tokens as f64
            + 0.1 * usage.cache_read_tokens as f64
    } else {
        (usage.input_tokens.saturating_sub(usage.cache_read_tokens)) as f64
            + 0.5 * usage.cache_read_tokens as f64
    };
    per_m(input_billable) * pi + per_m(usage.output_tokens as f64) * po
}

/// Compact USD formatting for the sidebar / cost display.
pub fn format_cost(usd: f64) -> String {
    if usd <= 0.0 {
        return "$0".to_string();
    }
    if usd < 0.01 {
        format!("${:.4}", usd)
    } else {
        format!("${:.2}", usd)
    }
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
            removed: false,
            prompt_cache: None,
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

    #[test]
    fn test_price_per_million_known_model() {
        // exact model override wins
        let (i, o) = price_per_million("deepinfra", "deepseek-ai/DeepSeek-V4-Flash-0731");
        assert_eq!(i, 0.25);
        assert_eq!(o, 1.00);
        // provider default when model unknown
        let (i, o) = price_per_million("anthropic", "some-future-model");
        assert_eq!(i, 3.00);
        assert_eq!(o, 15.00);
        // case-insensitive
        let (i, _o) = price_per_million("XAI", "GROK-4.5");
        assert_eq!(i, 3.00);
    }

    #[test]
    fn test_estimate_cost_and_format() {
        // 1M input @ deepseek-v4-flash ($0.25) + 1M output ($1.00) = $1.25
        let c = estimate_cost(
            "deepinfra",
            "deepseek-ai/DeepSeek-V4-Flash-0731",
            1_000_000,
            1_000_000,
        );
        assert!((c - 1.25).abs() < 1e-9);
        assert_eq!(format_cost(0.0042), "$0.0042");
        assert_eq!(format_cost(1.25), "$1.25");
        assert_eq!(format_cost(0.0), "$0");
    }

    #[test]
    fn test_estimate_cost_cached_anthropic() {
        use crate::harness::provider::Usage;
        // Anthropic: input excludes cache. 1M input ($3) + 1M write (1.25×$3)
        // + 1M read (0.1×$3) + 1M output ($15).
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
            cache_write_tokens: 1_000_000,
        };
        let c = estimate_cost_cached("anthropic", "claude-sonnet-4-20250514", &usage);
        let expected = (1.0 + 1.25 + 0.1) * 3.0 + 15.0;
        assert!((c - expected).abs() < 1e-9);
    }

    #[test]
    fn test_estimate_cost_cached_openai_style() {
        use crate::harness::provider::Usage;
        // OpenAI: prompt_tokens includes cached. 1M input of which 400k cached
        // → billable = 600k×$0.25 + 400k×0.5×$0.25 + 1M output×$1.00.
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_read_tokens: 400_000,
            cache_write_tokens: 0,
        };
        let c = estimate_cost_cached("deepinfra", "deepseek-ai/DeepSeek-V4-Flash-0731", &usage);
        let expected = (0.6 + 0.5 * 0.4) * 0.25 + 1.0;
        assert!((c - expected).abs() < 1e-9);
        // No double-count: without cache it equals estimate_cost.
        let plain = Usage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        assert!(
            (estimate_cost_cached("deepinfra", "deepseek-ai/DeepSeek-V4-Flash-0731", &plain)
                - estimate_cost(
                    "deepinfra",
                    "deepseek-ai/DeepSeek-V4-Flash-0731",
                    1_000_000,
                    1_000_000
                ))
            .abs()
                < 1e-9
        );
    }
}
