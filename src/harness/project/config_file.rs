//! Per-project persistent permission rules, persisted as `rustclaw.json` in
//! the project root (opencode-style). Token resolution is global (auth store)
//! and provider/model/base_url selection is global (config.json); only
//! permission rules are project-scoped.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::harness::hooks::HooksConfig;
use crate::harness::permission::PermissionConfig;

/// Project-scoped config (`rustclaw.json`): persistent per-tool permission
/// rules. Provider/model/base_url are global (config.json), not project-scoped.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectConfig {
    /// Persistent per-tool permission rules (e.g. `{ "bash": "allow" }`).
    #[serde(default, skip_serializing_if = "PermissionConfig::is_empty")]
    pub permission: PermissionConfig,
    /// Project hooks (pre_tool/post_tool/on_turn_end).
    #[serde(default, skip_serializing_if = "HooksConfig::is_empty")]
    pub hooks: HooksConfig,
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

    /// True when the file carries no explicit permission rules.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.permission.is_empty()
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
        use crate::harness::permission::Rule;
        let d = tempfile::tempdir().unwrap();
        let mut c = ProjectConfig::default();
        c.permission.tools.insert("bash".to_string(), Rule::Allow);
        c.save(d.path()).unwrap();

        let back = ProjectConfig::load_from(&ProjectConfig::path(d.path())).unwrap();
        assert_eq!(back.permission.tools.get("bash"), Some(&Rule::Allow));
        assert!(!back.is_empty());
    }

    #[test]
    fn test_no_model_or_provider_serialized() {
        let d = tempfile::tempdir().unwrap();
        let c = ProjectConfig::default();
        c.save(d.path()).unwrap();

        let raw = std::fs::read_to_string(d.path().join("rustclaw.json")).unwrap();
        assert!(!raw.contains("provider"), "got: {}", raw);
        assert!(!raw.contains("model"), "got: {}", raw);
        assert!(!raw.contains("base_url"), "got: {}", raw);
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
    fn test_is_empty_only_checks_permission() {
        use crate::harness::permission::Rule;
        let mut c = ProjectConfig::default();
        assert!(c.is_empty());
        c.permission.tools.insert("bash".to_string(), Rule::Allow);
        assert!(!c.is_empty());
    }

    #[test]
    fn test_permission_roundtrip() {
        use crate::harness::permission::Rule;
        let d = tempfile::tempdir().unwrap();
        let mut c = ProjectConfig::default();
        c.permission.tools.insert("bash".to_string(), Rule::Allow);
        c.save(d.path()).unwrap();

        let back = ProjectConfig::load(d.path());
        assert_eq!(
            back.permission.tools.get("bash"),
            Some(&Rule::Allow),
            "permission rule should roundtrip through rustclaw.json"
        );
        // Empty permission is skipped in the JSON output.
        let empty = ProjectConfig::default();
        let json = serde_json::to_string(&empty).unwrap();
        assert!(!json.contains("permission"), "got: {}", json);
    }
}
