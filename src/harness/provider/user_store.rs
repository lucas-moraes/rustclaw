//! User-defined provider store: providers/models added at runtime and
//! persisted in `~/.local/share/rustclaw/providers.json`.
//!
//! These are merged with the builtin catalog at runtime (see `catalog.rs`),
//! so a user can add a provider/model without recompiling. A user provider
//! with the same name as a builtin overrides it.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// A user-defined provider (owned strings, unlike the static builtin catalog).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UserProvider {
    pub name: String,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_model: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

/// The persisted collection of user-defined providers.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UserProviders {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<UserProvider>,
}

impl UserProviders {
    /// Default path: `<data_local_dir>/rustclaw/providers.json`.
    pub fn path() -> std::path::PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("rustclaw")
            .join("providers.json")
    }

    /// Loads the user providers; missing file = empty.
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

    /// Adds or replaces a provider by name. Returns `true` when it replaced
    /// an existing entry.
    pub fn upsert(&mut self, provider: UserProvider) -> bool {
        if let Some(existing) = self.providers.iter_mut().find(|p| p.name == provider.name) {
            *existing = provider;
            true
        } else {
            self.providers.push(provider);
            false
        }
    }

    /// Removes a provider by name. Returns `true` when it was present.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.providers.len();
        self.providers.retain(|p| p.name != name);
        self.providers.len() != before
    }

    /// Adds a model to a provider's list (no-op if already present).
    pub fn add_model(&mut self, name: &str, model: &str) -> bool {
        if let Some(p) = self.providers.iter_mut().find(|p| p.name == name) {
            if !p.models.iter().any(|m| m == model) {
                p.models.push(model.to_string());
                return true;
            }
        }
        false
    }

    /// Looks up a provider by name (case-insensitive).
    #[allow(dead_code)] // public store API, exercised in tests
    pub fn find(&self, name: &str) -> Option<&UserProvider> {
        let lower = name.to_lowercase();
        self.providers
            .iter()
            .find(|p| p.name.to_lowercase() == lower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn test_roundtrip() {
        let d = dir();
        let p = d.path().join("providers.json");
        let mut store = UserProviders::default();
        store.upsert(UserProvider {
            name: "my-llm".into(),
            base_url: "https://api.my.com/v1".into(),
            default_model: "model-a".into(),
            models: vec!["model-a".into(), "model-b".into()],
        });
        store.save_to(&p).unwrap();
        let back = UserProviders::load_from(&p).unwrap();
        assert_eq!(back, store);
        assert_eq!(
            back.find("MY-LLM").unwrap().base_url,
            "https://api.my.com/v1"
        );
    }

    #[test]
    fn test_upsert_replaces() {
        let mut store = UserProviders::default();
        store.upsert(UserProvider {
            name: "x".into(),
            base_url: "a".into(),
            default_model: String::new(),
            models: vec![],
        });
        let replaced = store.upsert(UserProvider {
            name: "x".into(),
            base_url: "b".into(),
            default_model: String::new(),
            models: vec![],
        });
        assert!(replaced);
        assert_eq!(store.providers.len(), 1);
        assert_eq!(store.find("x").unwrap().base_url, "b");
    }

    #[test]
    fn test_remove_and_add_model() {
        let mut store = UserProviders::default();
        store.upsert(UserProvider {
            name: "x".into(),
            base_url: "a".into(),
            default_model: String::new(),
            models: vec!["m1".into()],
        });
        assert!(store.add_model("x", "m2"));
        assert!(!store.add_model("x", "m2")); // duplicate no-op
        assert_eq!(store.find("x").unwrap().models.len(), 2);
        assert!(store.remove("x"));
        assert!(!store.remove("x"));
        assert!(store.find("x").is_none());
    }

    #[test]
    fn test_missing_file_is_empty() {
        let d = dir();
        let store = UserProviders::load_from(&d.path().join("nope.json")).unwrap();
        assert!(store.providers.is_empty());
    }
}
