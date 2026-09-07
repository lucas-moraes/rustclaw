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
    /// Tombstone for hidden builtin providers: the builtin is filtered out
    /// of the catalog while this flag is set (serde default keeps old files).
    #[serde(default, skip_serializing_if = "is_true")]
    pub removed: bool,
}

/// `skip_serializing_if` helper: omit the field when false.
fn is_true(v: &bool) -> bool {
    *v
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
    pub fn upsert(&mut self, mut provider: UserProvider) -> bool {
        if let Some(existing) = self.providers.iter_mut().find(|p| p.name == provider.name) {
            // Re-adding a provider un-hides a hidden builtin tombstone.
            if !provider.removed {
                provider.removed = false;
            }
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
        let lower = name.to_lowercase();
        self.providers.retain(|p| !p.name.to_lowercase().eq(&lower));
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

    /// Adds a model to any provider, including builtins: when the provider is
    /// not in the user store, a user entry is created (overriding the builtin
    /// in the catalog) seeded with the builtin's base_url/default_model.
    /// Returns `true` when the model was added (or already present).
    pub fn add_model_anywhere(&mut self, name: &str, model: &str) -> bool {
        if self.add_model(name, model) {
            return true;
        }
        // Already present in an existing user entry?
        if self
            .providers
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(name) && p.models.iter().any(|m| m == model))
        {
            return true;
        }
        // Seed from the builtin catalog (if any) so the override keeps the
        // builtin's connection defaults.
        let (base_url, default_model, mut models) =
            match crate::harness::provider::catalog::find_provider(name) {
                Some(info) if !info.user_defined => {
                    (info.base_url, info.default_model, info.models)
                }
                _ => (String::new(), String::new(), Vec::new()),
            };
        if base_url.is_empty() {
            return false; // unknown provider entirely
        }
        if !models.iter().any(|m| m == model) {
            models.push(model.to_string());
        }
        self.upsert(UserProvider {
            name: name.to_string(),
            base_url,
            default_model,
            models,
            removed: false,
        });
        true
    }

    /// Removes a model from a provider's user entry. When the removal leaves
    /// the entry without models, the entry itself is dropped:
    /// - a builtin override keeps serving the builtin defaults again;
    /// - a fully user-defined provider with no models left is deleted.
    ///
    /// Returns `true` when the model (or the emptied entry) was removed.
    pub fn remove_model(&mut self, name: &str, model: &str) -> bool {
        let Some(idx) = self
            .providers
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name))
        else {
            return false;
        };
        let entry = &mut self.providers[idx];
        if !entry.models.iter().any(|m| m == model) {
            return false;
        }
        entry.models.retain(|m| m != model);
        if entry.default_model == model {
            entry.default_model = entry.models.last().cloned().unwrap_or_default();
        }
        if entry.models.is_empty() && !entry.removed {
            self.providers.remove(idx);
        }
        true
    }

    /// Looks up a provider by name (case-insensitive).
    pub fn find(&self, name: &str) -> Option<&UserProvider> {
        let lower = name.to_lowercase();
        self.providers
            .iter()
            .find(|p| p.name.to_lowercase() == lower)
    }

    /// Hides a builtin provider from the catalog via a tombstone entry
    /// (seeded with the builtin's base_url so re-adding is simple).
    /// Returns `true` when a change was made (`false` = already hidden).
    pub fn hide_builtin(&mut self, name: &str) -> bool {
        if let Some(p) = self.find(name) {
            if p.removed {
                return false;
            }
        }
        let seed = crate::harness::provider::catalog::find_provider(name);
        let (base_url, default_model) = match &seed {
            Some(info) if !info.user_defined => (info.base_url.clone(), info.default_model.clone()),
            _ => (String::new(), String::new()),
        };
        self.upsert(UserProvider {
            name: name.to_string(),
            base_url,
            default_model,
            models: Vec::new(),
            removed: true,
        });
        true
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
            removed: false,
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
            removed: false,
        });
        let replaced = store.upsert(UserProvider {
            name: "x".into(),
            base_url: "b".into(),
            default_model: String::new(),
            models: vec![],
            removed: false,
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
            removed: false,
        });
        assert!(store.add_model("x", "m2"));
        assert!(!store.add_model("x", "m2")); // duplicate no-op
        assert_eq!(store.find("x").unwrap().models.len(), 2);
        assert!(store.remove("x"));
        assert!(!store.remove("x"));
        assert!(store.find("x").is_none());
    }

    #[test]
    fn test_remove_model_drops_entry_when_empty() {
        let mut store = UserProviders::default();
        store.upsert(UserProvider {
            name: "x".into(),
            base_url: "a".into(),
            default_model: "m1".into(),
            models: vec!["m1".into(), "m2".into()],
            removed: false,
        });
        assert!(store.remove_model("X", "m1")); // case-insensitive
        assert_eq!(store.find("x").unwrap().default_model, "m2");
        assert!(store.remove_model("x", "m2"));
        assert!(store.find("x").is_none()); // leftover entry dropped
        assert!(!store.remove_model("x", "m1"));
    }

    #[test]
    fn test_hide_builtin_tombstone() {
        let mut store = UserProviders::default();
        // Uses the real builtin catalog (a name that surely exists).
        assert!(store.hide_builtin("moonshot"));
        assert_eq!(store.find("moonshot").unwrap().removed, true);
        assert!(!store.hide_builtin("MOONSHOT")); // already hidden → no-op
                                                  // remove_model must not resurrect a tombstone.
        assert!(!store.remove_model("moonshot", "whatever"));
        assert_eq!(store.find("moonshot").unwrap().removed, true);
        // Catalog no longer lists the hidden builtin (direct merge check).
        assert!(!crate::harness::provider::catalog::merge(&store.providers)
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case("moonshot")));
        // Re-adding via upsert clears the tombstone.
        store.upsert(UserProvider {
            name: "moonshot".into(),
            base_url: "https://api.moonshot.ai/v1".into(),
            default_model: String::new(),
            models: vec!["m".into()],
            removed: false,
        });
        assert_eq!(store.find("moonshot").unwrap().removed, false);
        crate::harness::provider::catalog::merge(&store.providers)
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case("moonshot"))
            .expect("re-added provider must be visible again");
    }

    #[test]
    fn test_remove_missing_file_is_empty() {
        let d = dir();
        let store = UserProviders::load_from(&d.path().join("nope.json")).unwrap();
        assert!(store.providers.is_empty());
    }

    #[test]
    fn test_add_model_anywhere_seeds_builtin_override() {
        let mut store = UserProviders::default();
        // Builtin provider not yet in the user store.
        assert!(store.add_model_anywhere("moonshot", "kimi-new-model"));
        let p = store.find("moonshot").unwrap();
        assert_eq!(p.base_url, "https://api.moonshot.ai/v1");
        assert_eq!(p.default_model, "kimi-k2.5");
        assert!(p.models.contains(&"kimi-new-model".to_string()));
        assert!(p.models.contains(&"kimi-k2.5".to_string()));
        // Idempotent.
        assert!(store.add_model_anywhere("moonshot", "kimi-new-model"));
        assert_eq!(store.find("moonshot").unwrap().models.len(), 4);
        // Unknown provider → false.
        assert!(!store.add_model_anywhere("ghost", "m"));
    }
}
