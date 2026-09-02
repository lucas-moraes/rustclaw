//! Per-project model/provider selection, persisted as `rustclaw.json` in the
//! project root (opencode-style). Token resolution is global (auth store);
//! provider/model/base_url selection is project-scoped.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Project-scoped model/provider selection (`rustclaw.json`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_url: String,
}

impl ProjectConfig {
    /// Path to the project config file: `<root>/rustclaw.json`.
    pub fn path(root: &Path) -> PathBuf {
        root.join("rustclaw.json")
    }

    /// Loads the project config; missing file = empty (all env defaults).
    pub fn load(root: &Path) -> Self {
        Self::load_from(&Self::path(root)).unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
    }

    /// Persists the project config (pretty JSON).
    pub fn save(&self, root: &Path) -> Result<()> {
        self.save_to(&Self::path(root))
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create dir {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    /// True when the file carries any explicit selection.
    pub fn is_empty(&self) -> bool {
        self.provider.is_empty() && self.model.is_empty() && self.base_url.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_file_is_empty() {
        let d = tempfile::tempdir().unwrap();
        let c = ProjectConfig::load(d.path());
        assert!(c.is_empty());
    }

    #[test]
    fn test_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let mut c = ProjectConfig::default();
        c.provider = "deepinfra".into();
        c.model = "deepseek-ai/DeepSeek-V4-Flash-0731".into();
        c.save(d.path()).unwrap();

        let back = ProjectConfig::load_from(&ProjectConfig::path(d.path())).unwrap();
        assert_eq!(back.provider, "deepinfra");
        assert_eq!(back.model, "deepseek-ai/DeepSeek-V4-Flash-0731");
        assert!(back.base_url.is_empty());
    }

    #[test]
    fn test_corrupt_file_errors_on_strict_load() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("rustclaw.json"), "{ not json").unwrap();
        // Strict load surfaces the parse error...
        let err = ProjectConfig::load_from(&d.path().join("rustclaw.json")).unwrap_err();
        assert!(err.to_string().contains("failed to parse"));
        // ...while the lenient project loader falls back to empty.
        let c = ProjectConfig::load(d.path());
        assert!(c.is_empty());
    }

    #[test]
    fn test_is_empty_and_field_access() {
        let mut c = ProjectConfig {
            provider: "deepinfra".into(),
            model: String::new(),
            base_url: String::new(),
        };
        assert!(!c.is_empty());
        c.provider = String::new();
        assert!(c.is_empty());
    }
}
