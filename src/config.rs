//! Harness configuration — fully file-based, no `.env`.
//!
//! Resolution order (later wins, except the token which only lives in the
//! auth store):
//!
//! 1. Builtin provider catalog (defaults for provider/model/base_url)
//! 2. Global settings — `~/.local/share/rustclaw/config.json`
//! 3. Project settings — `rustclaw.json` in the project root
//! 4. Auth store — `~/.local/share/rustclaw/auth.json` (token per provider)
//!
//! An absent API key is tolerated so the TUI can onboarding the user
//! (`/auth`); the CLI surfaces an actionable error instead.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::harness::auth::AuthStore;

/// Global, cross-project settings stored as `config.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GlobalSettings {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_url: String,
    #[serde(default, skip_serializing_if = "usize_is_zero")]
    pub max_iterations: usize,
    #[serde(default, skip_serializing_if = "usize_is_zero")]
    pub max_context_tokens: usize,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub theme: String,
}

fn usize_is_zero(v: &usize) -> bool {
    *v == 0
}

impl GlobalSettings {
    /// Default path: `<data_local_dir>/rustclaw/config.json`.
    pub fn path() -> std::path::PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("rustclaw")
            .join("config.json")
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
        serde_json::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", path.display()))
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
    pub provider: String,
    pub max_iterations: usize,
    pub max_context_tokens: usize,
}

impl Config {
    /// Catalog-derived defaults.
    pub fn defaults() -> Self {
        let p = crate::harness::provider::catalog::find_provider("opencode-go")
            .expect("opencode-go must exist in the catalog");
        Self {
            api_key: None,
            base_url: p.base_url.to_string(),
            model: p.default_model.to_string(),
            provider: p.name.to_string(),
            max_iterations: 50,
            max_context_tokens: 100_000,
        }
    }

    /// File-based resolution: catalog defaults → global `config.json` →
    /// project `rustclaw.json` → auth store token. Never reads env vars.
    pub fn load() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let settings = GlobalSettings::load();
        let auth = AuthStore::load();
        Self::resolve(&cwd, &settings, &auth)
    }

    /// Testable resolution given explicit inputs.
    pub fn resolve(project_root: &Path, settings: &GlobalSettings, auth: &AuthStore) -> Self {
        let fallback = crate::harness::provider::catalog::find_provider("opencode-go")
            .expect("opencode-go must exist in the catalog");
        let mut cfg = Config::defaults();

        // 1. Provider/model: catalog <- global settings <- project config.
        if !settings.provider.is_empty() {
            cfg.provider = settings.provider.clone();
        }
        if !settings.model.is_empty() {
            cfg.model = settings.model.clone();
        }
        let proj = crate::harness::project::config_file::ProjectConfig::load_from(
            &crate::harness::project::config_file::ProjectConfig::path(project_root),
        )
        .unwrap_or_default();
        if !proj.is_empty() {
            if !proj.provider.is_empty() {
                cfg.provider = proj.provider;
            }
            if !proj.model.is_empty() {
                cfg.model = proj.model;
            }
        }
        let proj_base_url = proj.base_url.clone();

        // 2. base_url follows the *final* provider: project > global (only
        //    when it matches the resolved provider) > catalog default.
        cfg.base_url = if !proj_base_url.is_empty() {
            proj_base_url
        } else if !settings.base_url.is_empty() && cfg.provider == settings.provider {
            settings.base_url.clone()
        } else {
            crate::harness::provider::catalog::default_base_url(&cfg.provider)
                .map(str::to_string)
                .unwrap_or_else(|| fallback.base_url.to_string())
        };

        // 3. Limits: 0 = keep catalog default.
        if settings.max_iterations != 0 {
            cfg.max_iterations = settings.max_iterations;
        }
        if settings.max_context_tokens != 0 {
            cfg.max_context_tokens = settings.max_context_tokens;
        }

        // 4. Token from the global auth store for the resolved provider.
        cfg.api_key = auth.get_key(&cfg.provider).filter(|k| !k.trim().is_empty());
        cfg
    }

    /// True when the harness has enough to talk to the API.
    pub fn is_configured(&self) -> bool {
        self.api_key
            .as_ref()
            .map(|k| k.trim().len() >= 10)
            .unwrap_or(false)
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
        let cfg = Config::resolve(d.path(), &GlobalSettings::default(), &AuthStore::default());
        assert_eq!(cfg.provider, "opencode-go");
        assert_eq!(cfg.model, "deepseek-v4-flash");
        assert_eq!(cfg.base_url, "https://opencode.ai/zen/go/v1");
        assert_eq!(cfg.max_iterations, 50);
        assert_eq!(cfg.max_context_tokens, 100_000);
        assert_eq!(cfg.api_key, None);
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
        let cfg = Config::resolve(d.path(), &s, &AuthStore::default());
        assert_eq!(cfg.provider, "deepinfra");
        assert_eq!(cfg.model, "zai-org/GLM-5.3");
        assert_eq!(cfg.base_url, "https://api.deepinfra.com/v1/openai");
        assert_eq!(cfg.max_iterations, 7);
        assert_eq!(cfg.max_context_tokens, 42_000);
    }

    #[test]
    fn test_project_config_wins_over_global() {
        let d = dir();
        let mut proj = crate::harness::project::config_file::ProjectConfig::default();
        proj.provider = "openrouter".into();
        proj.model = "z-ai/glm-4.6".into();
        proj.save(d.path()).unwrap();

        let s = GlobalSettings {
            provider: "deepinfra".into(),
            model: "zai-org/GLM-5.3".into(),
            ..Default::default()
        };
        let cfg = Config::resolve(d.path(), &s, &AuthStore::default());
        assert_eq!(cfg.provider, "openrouter");
        assert_eq!(cfg.model, "z-ai/glm-4.6");
        assert_eq!(cfg.base_url, "https://openrouter.ai/api/v1");
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
}
