#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum TrustLevel {
    #[default]
    Untrusted,
    UntrustedReadOnly,
    Trusted,
    FullyTrusted,
}

impl TrustLevel {
    pub fn can_execute_shell(&self) -> bool {
        matches!(self, Self::Trusted | Self::FullyTrusted)
    }

    pub fn can_write_files(&self) -> bool {
        matches!(self, Self::Trusted | Self::FullyTrusted)
    }

    pub fn can_install_packages(&self) -> bool {
        matches!(self, Self::FullyTrusted)
    }

    pub fn can_access_network(&self) -> bool {
        !matches!(self, Self::Untrusted)
    }

    pub fn can_read_sensitive(&self) -> bool {
        matches!(self, Self::Trusted | Self::FullyTrusted)
    }

    pub fn to_u8(&self) -> u8 {
        match self {
            Self::Untrusted => 0,
            Self::UntrustedReadOnly => 1,
            Self::Trusted => 2,
            Self::FullyTrusted => 3,
        }
    }

    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::Untrusted,
            1 => Self::UntrustedReadOnly,
            2 => Self::Trusted,
            _ => Self::FullyTrusted,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTrustConfig {
    pub path: PathBuf,
    pub trust_level: TrustLevel,
    pub verified_at: i64,
    pub verified_by: Option<String>,
    pub restrictions: WorkspaceRestrictions,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceRestrictions {
    pub allowed_paths: Vec<PathBuf>,
    pub blocked_paths: Vec<PathBuf>,
    pub allowed_commands: Vec<String>,
    pub blocked_commands: Vec<String>,
    pub max_file_size_kb: Option<u64>,
    pub max_network_requests: Option<u32>,
}

impl WorkspaceRestrictions {
    pub fn is_path_allowed(&self, path: &Path) -> bool {
        if self.blocked_paths.iter().any(|b| path.starts_with(b)) {
            return false;
        }

        if self.allowed_paths.is_empty() {
            return true;
        }

        self.allowed_paths.iter().any(|a| path.starts_with(a))
    }

    pub fn is_command_allowed(&self, cmd: &str) -> bool {
        let cmd_lower = cmd.to_lowercase();

        if self
            .blocked_commands
            .iter()
            .any(|b| cmd_lower.contains(&b.to_lowercase()))
        {
            return false;
        }

        if self.allowed_commands.is_empty() {
            return true;
        }

        self.allowed_commands
            .iter()
            .any(|a| cmd_lower.contains(&a.to_lowercase()))
    }
}

pub struct WorkspaceTrustStore {
    workspaces: HashMap<PathBuf, WorkspaceTrustConfig>,
    global_default: TrustLevel,
    canonical_cache: RwLock<HashMap<PathBuf, PathBuf>>,
}

impl WorkspaceTrustStore {
    pub fn new() -> Self {
        Self {
            workspaces: HashMap::new(),
            global_default: TrustLevel::Untrusted,
            canonical_cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn set_global_default(&mut self, level: TrustLevel) {
        self.global_default = level;
    }

    fn canonicalize_cached(&self, path: &Path) -> PathBuf {
        if let Ok(cache) = self.canonical_cache.read() {
            if let Some(cached) = cache.get(path) {
                return cached.clone();
            }
        }

        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        if let Ok(mut cache) = self.canonical_cache.write() {
            cache.insert(path.to_path_buf(), canonical.clone());
        }

        canonical
    }

    pub fn get_trust(&self, path: &Path) -> TrustLevel {
        let canonical = self.canonicalize_cached(path);

        for (ws_path, config) in &self.workspaces {
            if canonical.starts_with(ws_path) {
                return config.trust_level;
            }
        }

        self.global_default
    }

    pub fn set_trust(&mut self, path: &Path, level: TrustLevel, verified_by: Option<String>) {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        self.workspaces.insert(
            canonical.clone(),
            WorkspaceTrustConfig {
                path: path.to_path_buf(),
                trust_level: level,
                verified_at: chrono::Utc::now().timestamp(),
                verified_by,
                restrictions: WorkspaceRestrictions::default(),
            },
        );

        if let Ok(mut cache) = self.canonical_cache.write() {
            cache.insert(path.to_path_buf(), canonical);
        }
    }

    pub fn add_restriction(&mut self, path: &Path, restriction: WorkspaceRestrictions) {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        if let Some(config) = self.workspaces.get_mut(&canonical) {
            config.restrictions = restriction;
        }
    }

    pub fn is_shell_allowed(&self, path: &Path) -> bool {
        self.get_trust(path).can_execute_shell()
    }

    pub fn is_file_write_allowed(&self, path: &Path) -> bool {
        self.get_trust(path).can_write_files()
    }

    pub fn is_network_allowed(&self, path: &Path) -> bool {
        self.get_trust(path).can_access_network()
    }

    pub fn is_command_allowed(&self, path: &Path, command: &str) -> bool {
        let trust = self.get_trust(path);
        if !trust.can_execute_shell() {
            return false;
        }

        let canonical = self.canonicalize_cached(path);
        if let Some(config) = self.workspaces.get(&canonical) {
            return config.restrictions.is_command_allowed(command);
        }

        true
    }

    pub fn remove(&mut self, path: &Path) {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.workspaces.remove(&canonical);
    }

    pub fn list_workspaces(&self) -> Vec<(&PathBuf, TrustLevel)> {
        self.workspaces
            .iter()
            .map(|(p, c)| (p, c.trust_level))
            .collect()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.workspaces)
            .map_err(|e| format!("Serialization error: {}", e))?;

        std::fs::write(path, json).map_err(|e| format!("IO error: {}", e))
    }

    pub fn load(&mut self, path: &Path) -> Result<(), String> {
        if !path.exists() {
            return Ok(());
        }

        let json = std::fs::read_to_string(path).map_err(|e| format!("IO error: {}", e))?;

        self.workspaces =
            serde_json::from_str(&json).map_err(|e| format!("Deserialization error: {}", e))?;

        Ok(())
    }

    pub fn load_from(path: &Path) -> Result<Self, String> {
        let mut store = Self::new();
        store.load(path)?;
        Ok(store)
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.workspaces)
            .map_err(|e| format!("Serialization error: {}", e))?;

        std::fs::write(path, json).map_err(|e| format!("IO error: {}", e))
    }

    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.canonical_cache.write() {
            cache.clear();
        }
    }
}

impl Default for WorkspaceTrustStore {
    fn default() -> Self {
        Self::new()
    }
}

pub struct TrustEvaluator {
    store: WorkspaceTrustStore,
}

impl TrustEvaluator {
    pub fn new() -> Self {
        Self {
            store: WorkspaceTrustStore::new(),
        }
    }

    pub fn with_store(store: WorkspaceTrustStore) -> Self {
        Self { store }
    }

    pub fn evaluate(&self, path: &Path, operation: &Operation) -> TrustDecision {
        let trust = self.store.get_trust(path);

        let allowed = match operation {
            Operation::ReadFile => true,
            Operation::WriteFile => trust.can_write_files(),
            Operation::ExecuteShell => trust.can_execute_shell(),
            Operation::InstallPackage => trust.can_install_packages(),
            Operation::NetworkRequest => trust.can_access_network(),
            Operation::ReadSensitive => trust.can_read_sensitive(),
        };

        TrustDecision {
            allowed,
            trust_level: trust,
            reason: if allowed {
                None
            } else {
                Some(format!(
                    "Operation '{}' not allowed for trust level {:?}",
                    operation, trust
                ))
            },
        }
    }

    pub fn prompt_user(&self, path: &Path, operation: &Operation) -> String {
        let trust = self.store.get_trust(path);

        match operation {
            Operation::ExecuteShell => format!(
                "Allow executing shell commands in {}? Current trust: {:?}",
                path.display(),
                trust
            ),
            Operation::WriteFile => format!(
                "Allow writing files to {}? Current trust: {:?}",
                path.display(),
                trust
            ),
            _ => format!(
                "Allow operation '{}' in {}? Current trust: {:?}",
                operation,
                path.display(),
                trust
            ),
        }
    }

    pub fn can_write_file(&self, path: &Path) -> bool {
        let trust = self.store.get_trust(path);
        trust.can_write_files()
    }

    pub fn can_execute_shell(&self, path: &Path) -> bool {
        let trust = self.store.get_trust(path);
        trust.can_execute_shell()
    }

    pub fn set_trust(&mut self, path: &Path, level: TrustLevel) {
        self.store.set_trust(path, level, Some("user".to_string()));
    }

    pub fn get_store_mut(&mut self) -> &mut WorkspaceTrustStore {
        &mut self.store
    }

    pub fn get_store(&self) -> &WorkspaceTrustStore {
        &self.store
    }
}

#[derive(Debug, Clone)]
pub enum Operation {
    ReadFile,
    WriteFile,
    ExecuteShell,
    InstallPackage,
    NetworkRequest,
    ReadSensitive,
}

impl std::fmt::Display for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Operation::ReadFile => write!(f, "ReadFile"),
            Operation::WriteFile => write!(f, "WriteFile"),
            Operation::ExecuteShell => write!(f, "ExecuteShell"),
            Operation::InstallPackage => write!(f, "InstallPackage"),
            Operation::NetworkRequest => write!(f, "NetworkRequest"),
            Operation::ReadSensitive => write!(f, "ReadSensitive"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrustDecision {
    pub allowed: bool,
    pub trust_level: TrustLevel,
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trust_levels() {
        assert!(!TrustLevel::Untrusted.can_execute_shell());
        assert!(!TrustLevel::UntrustedReadOnly.can_execute_shell());
        assert!(TrustLevel::Trusted.can_execute_shell());
        assert!(TrustLevel::FullyTrusted.can_execute_shell());
    }

    #[test]
    fn test_restrictions() {
        let mut restrictions = WorkspaceRestrictions::default();
        restrictions.blocked_commands.push("rm -rf".to_string());

        assert!(!restrictions.is_command_allowed("rm -rf /"));
        assert!(restrictions.is_command_allowed("ls"));
    }
}
