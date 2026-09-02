//! Global auth store, keyed by provider (opencode-style).
//!
//! Tokens live in `<data_local_dir>/rustclaw/auth.json` with `0600`
//! permissions, shared across projects. Selection of provider/model is
//! project-scoped (`rustclaw.json`); credentials are global per provider.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A single provider credential entry (only api supported today).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthEntry {
    #[serde(rename = "type", default = "default_type")]
    pub kind: String,
    pub key: String,
}

fn default_type() -> String {
    "api".to_string()
}

/// Global provider credential store backed by `auth.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuthStore {
    /// provider name -> credentials.
    #[serde(flatten)]
    pub entries: HashMap<String, AuthEntry>,
}

impl AuthStore {
    /// Default file path: `<data_local_dir>/rustclaw/auth.json`.
    pub fn path() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rustclaw")
            .join("auth.json")
    }

    /// Loads the store from the default path; missing file = empty store.
    pub fn load() -> Self {
        Self::load_from(&Self::path()).unwrap_or_default()
    }

    pub fn load_from(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read auth store {}", path.display()))?;
        serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse auth store {}", path.display()))
    }

    /// Persists the store with `0600` permissions (best effort).
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create auth dir {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
            .with_context(|| format!("failed to write auth store {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
        Ok(())
    }

    /// Returns the stored API key for `provider`, if any.
    pub fn get_key(&self, provider: &str) -> Option<String> {
        self.entries.get(provider).map(|e| e.key.clone())
    }

    /// Inserts or updates the API key for `provider`.
    pub fn set_key(&mut self, provider: impl Into<String>, key: impl Into<String>) {
        self.entries.insert(
            provider.into(),
            AuthEntry {
                kind: "api".to_string(),
                key: key.into(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("auth.json");
        (d, p)
    }

    #[test]
    fn test_missing_file_is_empty_store() {
        let (_d, p) = tmp();
        let s = AuthStore::load_from(&p).unwrap();
        assert!(s.entries.is_empty());
    }

    #[test]
    fn test_set_get_roundtrip() {
        let (d, p) = tmp();
        let mut s = AuthStore::default();
        s.set_key("deepinfra", "sk-test-12345");
        assert_eq!(s.get_key("deepinfra"), Some("sk-test-12345".to_string()));
        s.save_to(&p).unwrap();

        let back = AuthStore::load_from(&p).unwrap();
        assert_eq!(back.get_key("deepinfra"), Some("sk-test-12345".to_string()));
        assert_eq!(back.get_key("moonshot"), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert!(d.path().exists());
    }

    #[test]
    fn test_serde_shape_matches_opencode() {
        let mut s = AuthStore::default();
        s.set_key("deepinfra", "sk-1");
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"deepinfra\""));
        assert!(json.contains("\"type\":\"api\"") || json.contains("\"type\": \"api\""));
        assert!(json.contains("\"key\": \"sk-1\"") || json.contains("\"key\":\"sk-1\""));
    }
}
