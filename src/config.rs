//! Harness configuration — fully file-based, no `.env`.
//!
//! Resolution order (later wins, except the token which only lives in the
//! auth store):
//!
//! 1. Builtin provider catalog (defaults for provider/model/base_url)
//! 2. Global settings — `<base_dir>/config.json` (see `harness::paths`)
//! 3. Auth store — `<base_dir>/auth.json` (token per provider)
//!
//! Provider/model/base_url are global-only. The project's `rustclaw.json`
//! carries only persistent permission rules and does not influence the
//! resolved provider/model.
//!
//! An absent API key is tolerated so the TUI can onboarding the user
//! (`/auth`); the CLI surfaces an actionable error instead.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::harness::auth::AuthStore;

/// Global, cross-project settings stored as `config.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GlobalSettings {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_url: String,
    #[serde(default, skip_serializing_if = "usize_is_zero")]
    pub max_iterations: usize,
    /// Hard cap on total iterations across all continuations of a single turn
    /// (0 = conservative default of `max_iterations * 3`).
    #[serde(default, skip_serializing_if = "usize_is_zero")]
    pub max_total_iterations: usize,
    #[serde(default, skip_serializing_if = "usize_is_zero")]
    pub max_context_tokens: usize,
    /// Wall-clock limit per turn, in seconds (0 = keep default).
    #[serde(default, skip_serializing_if = "usize_is_zero")]
    pub turn_timeout_secs: usize,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub theme: String,
    /// Kill-switch for Anthropic-style prompt caching (`cache_control`
    /// breakpoints). Default `true`; set to `false` to disable everywhere.
    #[serde(default = "default_true", skip_serializing_if = "is_false")]
    pub prompt_caching: bool,
    /// Daily spend limit in USD (estimated cost). `0.0` = no limit.
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub daily_budget_usd: f64,
    /// Optional cheaper model used only for compaction summaries. Empty =
    /// use the main model. Summarization quality doesn't need the primary
    /// model, so a cheaper one cuts compaction cost.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary_model: String,
    /// Fraction of `max_context_tokens` at which proactive compaction
    /// triggers. `0.0` = keep the default (0.7). Lower values compact sooner,
    /// keeping the context leaner at the cost of more frequent summaries.
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub compact_trigger_ratio: f64,
    /// When `true`, the `build` mode is served by the `cursor` agent, which
    /// delegates the whole task to the Cursor CLI. Default `false` (native
    /// build). Takes effect on the next turn (registry rebuilt per turn).
    #[serde(default, skip_serializing_if = "is_false")]
    pub cursor_agent: bool,
}

fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

fn default_true() -> bool {
    true
}

fn is_false(v: &bool) -> bool {
    !*v
}

// Manual Default so the absent-file case honors `prompt_caching = true`
// (the derived Default would give `false`).
impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            provider: String::new(),
            model: String::new(),
            base_url: String::new(),
            max_iterations: 0,
            max_total_iterations: 0,
            max_context_tokens: 0,
            turn_timeout_secs: 0,
            theme: String::new(),
            daily_budget_usd: 0.0,
            prompt_caching: true,
            summary_model: String::new(),
            compact_trigger_ratio: 0.0,
            cursor_agent: false,
        }
    }
}

fn usize_is_zero(v: &usize) -> bool {
    *v == 0
}

impl GlobalSettings {
    /// Default path: `<base_dir>/config.json` (see `harness::paths`).
    pub fn path() -> std::path::PathBuf {
        crate::harness::paths::config_json()
    }

    /// Loads the global settings; missing file = empty (catalog defaults).
    pub fn load() -> Self {
        Self::load_from(&Self::path()).unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
        match serde_json::from_str(&raw) {
            Ok(settings) => Ok(settings),
            Err(e) => {
                // Preserve the corrupt file as a backup before any save()
                // overwrites it, and surface a clear warning.
                crate::harness::auth::backup_corrupt(path, "config");
                Err(anyhow::anyhow!(
                    "failed to parse {}: {e} (backed up to {}.bak)",
                    path.display(),
                    path.display()
                ))
            }
        }
    }

    /// Persists with `0600` permissions (best effort).
    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("failed to create dir {}: {e}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
            .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    // (no is_empty helper: the wizard keys off `Config::is_configured`)
}

/// Resolved runtime configuration (provider/model/limits/token).
///
/// This is the single resolved config used by the runtime (replaces the old
/// `Config` + `HarnessConfig` pair). The API key is a plain `String` because
/// the runtime requires a token; an absent key is represented as an empty
/// string and surfaced by `is_configured()`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub provider: String,
    pub max_iterations: usize,
    /// Hard cap on total iterations across all continuations of a single turn
    /// (0 = conservative default of `max_iterations * 3`).
    pub max_total_iterations: usize,
    pub max_context_tokens: usize,
    /// Wall-clock limit per turn, in seconds.
    pub turn_timeout_secs: usize,
    /// Agent used when starting a fresh session.
    pub default_agent: String,
    /// Daily spend limit in USD (estimated cost). `0.0` = no limit.
    pub daily_budget_usd: f64,
    /// Optional cheaper model for compaction summaries. Empty = main model.
    pub summary_model: String,
    /// Fraction of `max_context_tokens` at which proactive compaction
    /// triggers. `0.0` = default (0.7).
    pub compact_trigger_ratio: f64,
    /// When `true`, the `build` mode is served by the `cursor` agent (Cursor
    /// CLI delegation) instead of the native build agent.
    pub cursor_agent: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

impl RuntimeConfig {
    /// Fallback provider info for opencode-go, used if the catalog lookup
    /// somehow fails (defensive; the catalog always contains it).
    fn opencode_go_fallback() -> crate::harness::provider::catalog::ProviderInfo {
        crate::harness::provider::catalog::ProviderInfo {
            name: "opencode-go".to_string(),
            base_url: "https://opencode.ai/api".to_string(),
            default_model: "grok-4.5".to_string(),
            models: vec!["grok-4.5".to_string()],
            user_defined: false,
        }
    }

    /// Catalog-derived defaults.
    pub fn defaults() -> Self {
        let p = crate::harness::provider::catalog::find_provider("opencode-go")
            .unwrap_or_else(Self::opencode_go_fallback);
        Self {
            api_key: String::new(),
            base_url: p.base_url.to_string(),
            model: p.default_model.to_string(),
            provider: p.name.to_string(),
            max_iterations: 50,
            max_total_iterations: 0,
            max_context_tokens: 100_000,
            turn_timeout_secs: 1200,
            default_agent: "build".to_string(),
            daily_budget_usd: 0.0,
            summary_model: String::new(),
            compact_trigger_ratio: 0.0,
            cursor_agent: false,
        }
    }

    /// File-based resolution: catalog defaults → global `config.json` → auth
    /// store token. Never reads env vars; the project's `rustclaw.json` (only
    /// permission rules) does not influence provider/model/base_url.
    pub fn load() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let settings = GlobalSettings::load();
        let auth = AuthStore::load();
        Self::resolve(&cwd, &settings, &auth)
    }

    /// Testable resolution given explicit inputs.
    pub fn resolve(_project_root: &Path, settings: &GlobalSettings, auth: &AuthStore) -> Self {
        let fallback = crate::harness::provider::catalog::find_provider("opencode-go")
            .unwrap_or_else(Self::opencode_go_fallback);
        let mut cfg = RuntimeConfig::defaults();

        // 1. Provider/model: catalog defaults <- global settings.
        if !settings.provider.is_empty() {
            cfg.provider = settings.provider.clone();
        }
        if !settings.model.is_empty() {
            cfg.model = settings.model.clone();
        }

        // 2. base_url follows the *final* provider: global (when it matches
        //    the resolved provider) > catalog default.
        cfg.base_url = if !settings.base_url.is_empty() && cfg.provider == settings.provider {
            settings.base_url.clone()
        } else {
            crate::harness::provider::catalog::default_base_url(&cfg.provider)
                .unwrap_or_else(|| fallback.base_url.to_string())
        };

        // 3. Limits: 0 = keep catalog default.
        if settings.max_iterations != 0 {
            cfg.max_iterations = settings.max_iterations;
        }
        if settings.max_total_iterations != 0 {
            cfg.max_total_iterations = settings.max_total_iterations;
        }
        if settings.max_context_tokens != 0 {
            cfg.max_context_tokens = settings.max_context_tokens;
        }
        if settings.turn_timeout_secs != 0 {
            cfg.turn_timeout_secs = settings.turn_timeout_secs;
        }
        if settings.daily_budget_usd != 0.0 {
            cfg.daily_budget_usd = settings.daily_budget_usd;
        }
        if !settings.summary_model.is_empty() {
            cfg.summary_model = settings.summary_model.clone();
        }
        if settings.compact_trigger_ratio != 0.0 {
            cfg.compact_trigger_ratio = settings.compact_trigger_ratio;
        }
        cfg.cursor_agent = settings.cursor_agent;

        // 4. Token from the global auth store for the resolved provider.
        cfg.api_key = auth
            .get_key(&cfg.provider)
            .filter(|k| !k.trim().is_empty())
            .unwrap_or_default();
        cfg
    }

    /// True when the harness has enough to talk to the API.
    pub fn is_configured(&self) -> bool {
        self.api_key.trim().len() >= 10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn test_empty_settings_fall_back_to_catalog() {
        let d = dir();
        let cfg =
            RuntimeConfig::resolve(d.path(), &GlobalSettings::default(), &AuthStore::default());
        assert_eq!(cfg.provider, "opencode-go");
        assert_eq!(cfg.model, "deepseek-v4-flash");
        assert_eq!(cfg.base_url, "https://opencode.ai/zen/go/v1");
        assert_eq!(cfg.max_iterations, 50);
        assert_eq!(cfg.max_context_tokens, 100_000);
        assert_eq!(cfg.api_key, "");
        assert!(!cfg.is_configured());
    }

    #[test]
    fn test_global_settings_apply() {
        let d = dir();
        let s = GlobalSettings {
            provider: "deepinfra".into(),
            model: "zai-org/GLM-5.3".into(),
            max_iterations: 7,
            max_context_tokens: 42_000,
            ..Default::default()
        };
        let cfg = RuntimeConfig::resolve(d.path(), &s, &AuthStore::default());
        assert_eq!(cfg.provider, "deepinfra");
        assert_eq!(cfg.model, "zai-org/GLM-5.3");
        assert_eq!(cfg.base_url, "https://api.deepinfra.com/v1/openai");
        assert_eq!(cfg.max_iterations, 7);
        assert_eq!(cfg.max_context_tokens, 42_000);
    }

    #[test]
    fn test_project_file_does_not_override_model_provider() {
        // A legacy rustclaw.json carrying provider/model must be ignored: the
        // project no longer influences provider/model/base_url (global only).
        let d = dir();
        // Write a legacy rustclaw.json with the old fields as raw JSON (the
        // current ProjectConfig struct no longer parses them, so we write the
        // file directly to simulate a stale project config).
        std::fs::write(
            d.path().join("rustclaw.json"),
            r#"{"provider":"openrouter","model":"z-ai/glm-4.6","base_url":"https://openrouter.ai/api/v1"}"#,
        )
        .unwrap();

        let s = GlobalSettings {
            provider: "deepinfra".into(),
            model: "zai-org/GLM-5.3".into(),
            ..Default::default()
        };
        let cfg = RuntimeConfig::resolve(d.path(), &s, &AuthStore::default());
        assert_eq!(cfg.provider, "deepinfra");
        assert_eq!(cfg.model, "zai-org/GLM-5.3");
        assert_eq!(cfg.base_url, "https://api.deepinfra.com/v1/openai");
    }

    #[test]
    fn test_settings_roundtrip() {
        let d = dir();
        let p = d.path().join("config.json");
        let s = GlobalSettings {
            provider: "moonshot".into(),
            model: "kimi-k2.5".into(),
            ..Default::default()
        };
        s.save_to(&p).unwrap();
        let back = GlobalSettings::load_from(&p).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn test_corrupt_config_is_backed_up() {
        let d = dir();
        let p = d.path().join("config.json");
        std::fs::write(&p, "not json at all").unwrap();
        let err = GlobalSettings::load_from(&p).unwrap_err();
        assert!(err.to_string().contains("backed up"), "err: {err}");
        let bak = p.with_extension("json.bak");
        assert!(bak.exists(), ".bak should exist");
        assert!(!p.exists(), "original should be moved to .bak");
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "not json at all");
    }

    #[test]
    fn test_unified_defaults_are_single_source_of_truth() {
        // D6: the runtime and `/settings` must agree on the same limits. The
        // defaults live in exactly one place (`RuntimeConfig::defaults`) and
        // the runtime is always built from a resolved `RuntimeConfig`.
        let d = dir();
        let cfg =
            RuntimeConfig::resolve(d.path(), &GlobalSettings::default(), &AuthStore::default());
        assert_eq!(cfg.max_iterations, 50);
        assert_eq!(cfg.max_context_tokens, 100_000);
        assert_eq!(cfg.turn_timeout_secs, 1200);
        assert_eq!(cfg.default_agent, "build");
    }

    #[test]
    fn test_is_configured_uses_string_key() {
        let d = dir();
        let mut cfg =
            RuntimeConfig::resolve(d.path(), &GlobalSettings::default(), &AuthStore::default());
        assert!(!cfg.is_configured());
        cfg.api_key = "sk-short".to_string();
        assert!(!cfg.is_configured());
        cfg.api_key = "sk-1234567890".to_string();
        assert!(cfg.is_configured());
    }

    #[test]
    fn test_cursor_agent_defaults_false() {
        let d = dir();
        let cfg =
            RuntimeConfig::resolve(d.path(), &GlobalSettings::default(), &AuthStore::default());
        assert!(!cfg.cursor_agent);
        assert!(!GlobalSettings::default().cursor_agent);
    }

    #[test]
    fn test_cursor_agent_roundtrip() {
        let d = dir();
        let p = d.path().join("config.json");
        let s = GlobalSettings {
            cursor_agent: true,
            ..Default::default()
        };
        s.save_to(&p).unwrap();
        let back = GlobalSettings::load_from(&p).unwrap();
        assert!(back.cursor_agent);
        assert_eq!(back, s);
    }

    #[test]
    fn test_cursor_agent_omitted_when_false() {
        let d = dir();
        let p = d.path().join("config.json");
        GlobalSettings::default().save_to(&p).unwrap();
        let raw = std::fs::read_to_string(&p).unwrap();
        assert!(
            !raw.contains("cursor_agent"),
            "false must be skipped: {raw}"
        );
    }

    #[test]
    fn test_cursor_agent_propagates_to_runtime() {
        let d = dir();
        let s = GlobalSettings {
            cursor_agent: true,
            ..Default::default()
        };
        let cfg = RuntimeConfig::resolve(d.path(), &s, &AuthStore::default());
        assert!(cfg.cursor_agent);
    }
}
