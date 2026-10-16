//! MCP trust gate: fingerprints de configuração de servidores MCP.
//!
//! Um servidor só é conectado no boot se a fingerprint da sua configuração
//! (comando, args, env, url, headers) estiver no trust store. Qualquer
//! alteração na config muda a fingerprint e o servidor volta a exigir
//! aprovação via `/mcp trust <name>` (fail-closed).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::harness::mcp::config::McpServerConfig;

/// File name of the trust store inside the base dir.
const TRUST_FILE: &str = "mcp_trust.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustFile {
    /// Trusted configuration fingerprints (sha256 hex).
    #[serde(default)]
    trusted: Vec<String>,
}

/// Persistent store of trusted MCP server configuration fingerprints.
#[derive(Debug, Clone)]
pub struct McpTrustStore {
    path: PathBuf,
}

impl McpTrustStore {
    /// Trust store at `<base>/mcp_trust.json`.
    pub fn global() -> Self {
        Self::at(&crate::harness::paths::base_dir())
    }

    /// Trust store at `<base_dir>/mcp_trust.json` (tests pass a temp dir).
    pub fn at(base_dir: &Path) -> Self {
        Self {
            path: base_dir.join(TRUST_FILE),
        }
    }

    /// Path of the backing file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self) -> TrustFile {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// True if `fingerprint` was explicitly trusted.
    pub fn is_trusted(&self, fingerprint: &str) -> bool {
        self.load().trusted.iter().any(|t| t == fingerprint)
    }

    /// True if the server config is trusted.
    pub fn is_config_trusted(&self, name: &str, cfg: &McpServerConfig) -> bool {
        self.is_trusted(&fingerprint(name, cfg))
    }

    /// Adds `fingerprint` to the store (idempotent) and persists atomically.
    pub fn trust(&self, fingerprint: &str) -> Result<()> {
        let mut file = self.load();
        if file.trusted.iter().any(|t| t == fingerprint) {
            return Ok(());
        }
        file.trusted.push(fingerprint.to_string());
        self.persist(&file)
    }

    /// Removes `fingerprint` from the store (idempotent) and persists.
    pub fn untrust(&self, fingerprint: &str) -> Result<()> {
        let mut file = self.load();
        let before = file.trusted.len();
        file.trusted.retain(|t| t != fingerprint);
        if file.trusted.len() == before {
            return Ok(());
        }
        self.persist(&file)
    }

    /// All trusted fingerprints (sorted for stable output).
    pub fn trusted(&self) -> Vec<String> {
        let mut list = self.load().trusted;
        list.sort();
        list
    }

    fn persist(&self, file: &TrustFile) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(file).context("failed to serialize trust store")?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).with_context(|| format!("failed to write {}", tmp.display()))?;
        set_owner_only(&tmp)?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("failed to replace {}", self.path.display()))?;
        set_owner_only(&self.path)?;
        Ok(())
    }
}

/// Deterministic sha256 fingerprint of a server's identity-bearing config.
///
/// `enabled` and `timeout_secs` are excluded: they do not change what code
/// runs or where it connects. Env/headers are sorted (HashMap order is
/// non-deterministic) so the fingerprint is stable across runs.
pub fn fingerprint(name: &str, cfg: &McpServerConfig) -> String {
    let sorted = |m: &std::collections::HashMap<String, String>| -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        v.sort();
        v
    };
    let canonical = format!(
        "name={name}\ncommand={}\nargs={:?}\nenv={:?}\nurl={}\nheaders={:?}",
        cfg.command,
        cfg.args,
        sorted(&cfg.env),
        cfg.url,
        sorted(&cfg.headers),
    );
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Short (12-char) prefix of a fingerprint for display.
pub fn short(fingerprint: &str) -> String {
    let end = fingerprint.len().min(12);
    fingerprint[..end].to_string()
}

/// Restricts a file to owner-only permissions (0600 on unix).
#[cfg(unix)]
fn set_owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to set permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(command: &str, args: &[&str]) -> McpServerConfig {
        McpServerConfig {
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn test_fingerprint_changes_with_identity_fields() {
        let base = cfg("server", &["--stdio"]);
        let fp = fingerprint("a", &base);
        assert_eq!(fp, fingerprint("a", &base), "must be deterministic");

        // Different name / command / args → different fingerprint.
        assert_ne!(fp, fingerprint("b", &base));
        assert_ne!(fp, fingerprint("a", &cfg("other", &["--stdio"])));
        assert_ne!(fp, fingerprint("a", &cfg("server", &["--http"])));

        // env participates.
        let mut with_env = base.clone();
        with_env.env.insert("KEY".into(), "v".into());
        assert_ne!(fp, fingerprint("a", &with_env));

        // url participates.
        let mut with_url = base.clone();
        with_url.url = "https://example.com/mcp".into();
        assert_ne!(fp, fingerprint("a", &with_url));
    }

    #[test]
    fn test_fingerprint_ignores_non_identity_fields() {
        let base = cfg("server", &["--stdio"]);
        let fp = fingerprint("a", &base);

        let mut disabled = base.clone();
        disabled.enabled = false;
        assert_eq!(fp, fingerprint("a", &disabled), "enabled is not identity");

        let mut slower = base.clone();
        slower.timeout_secs = 999;
        assert_eq!(fp, fingerprint("a", &slower), "timeout is not identity");
    }

    #[test]
    fn test_fingerprint_env_order_stable() {
        // HashMap iteration order must not leak into the fingerprint.
        let mut a = cfg("server", &[]);
        a.env.insert("A".into(), "1".into());
        a.env.insert("B".into(), "2".into());
        a.env.insert("C".into(), "3".into());
        let mut b = cfg("server", &[]);
        b.env.insert("C".into(), "3".into());
        b.env.insert("B".into(), "2".into());
        b.env.insert("A".into(), "1".into());
        assert_eq!(fingerprint("a", &a), fingerprint("a", &b));
    }

    #[test]
    fn test_store_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = McpTrustStore::at(dir.path());
        let fp = fingerprint("srv", &cfg("server", &[]));

        assert!(!store.is_trusted(&fp), "starts untrusted");
        store.trust(&fp).expect("trust");
        assert!(store.is_trusted(&fp));
        assert!(store.is_config_trusted("srv", &cfg("server", &[])));

        // Idempotent.
        store.trust(&fp).expect("trust again");
        assert_eq!(store.trusted().len(), 1);

        // Reload from disk (new handle, same path).
        let store2 = McpTrustStore::at(dir.path());
        assert!(store2.is_trusted(&fp));

        store2.untrust(&fp).expect("untrust");
        assert!(!store.is_trusted(&fp));
        assert!(store.trusted().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_store_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let store = McpTrustStore::at(dir.path());
        store
            .trust(&fingerprint("srv", &cfg("server", &[])))
            .expect("trust");
        let mode = std::fs::metadata(store.path())
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "trust store must be 0600");
    }
}
