#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SettingsJson {
    #[serde(default)]
    pub hooks: HashMap<String, HookConfig>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub skills_dir: Option<String>,
}

pub struct HookManager {
    settings_path: PathBuf,
    settings: SettingsJson,
}

impl HookManager {
    pub fn new(config_dir: PathBuf) -> Self {
        let settings_path = config_dir.join("settings.json");
        let settings = Self::load_settings(&settings_path).unwrap_or_default();

        Self {
            settings_path,
            settings,
        }
    }

    fn load_settings(path: &Path) -> Option<SettingsJson> {
        if !path.exists() {
            return None;
        }

        let content = fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    pub fn reload(&mut self) {
        if let Some(settings) = Self::load_settings(&self.settings_path) {
            self.settings = settings;
            info!("Reloaded settings.json");
        }
    }

    pub fn get_hook(&self, hook_name: &str) -> Option<&HookConfig> {
        self.settings.hooks.get(hook_name)
    }

    pub fn list_hooks(&self) -> Vec<String> {
        self.settings.hooks.keys().cloned().collect()
    }

    pub fn execute_hook(&self, hook_name: &str) -> Result<String, String> {
        let hook = self
            .get_hook(hook_name)
            .ok_or_else(|| format!("Hook '{}' not found", hook_name))?;

        let command = hook
            .command
            .as_ref()
            .ok_or_else(|| format!("Hook '{}' has no command", hook_name))?;

        let mut cmd = Command::new(command);

        if let Some(ref args) = hook.args {
            cmd.args(args);
        }

        if let Some(ref env) = hook.env {
            for (key, value) in env {
                cmd.env(key, value);
            }
        }

        let output = cmd
            .output()
            .map_err(|e| format!("Failed to execute hook '{}': {}", hook_name, e))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            debug!("Hook '{}' output: {}", hook_name, stdout);
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            error!("Hook '{}' error: {}", hook_name, stderr);
            Err(stderr)
        }
    }

    pub fn get_skills_dir(&self) -> Option<String> {
        self.settings.skills_dir.clone()
    }

    pub fn get_allowed_tools_override(&self) -> Option<Vec<String>> {
        self.settings.allowed_tools.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = SettingsJson::default();
        assert!(settings.hooks.is_empty());
        assert!(settings.allowed_tools.is_none());
    }
}
