//! Centralized system-data paths for RustClaw.
//!
//! Everything the harness persists at the "system" level (config, auth,
//! providers, MCP config, usage reports, attachments, logs and the main
//! SQLite database) lives under a single base directory, resolved once by
//! [`base_dir`] in this order:
//!
//! 1. `RUSTCLAW_HOME` environment variable (explicit override; also used
//!    by tests);
//! 2. The directory containing the running executable — portable /
//!    "carry your data with the binary" layout. The data lives in a
//!    sibling `rustclaw-data/` folder so binary updates never touch it;
//! 3. Fallback: the OS data-local dir (`~/.local/share/rustclaw` on
//!    Linux, `~/Library/Application Support/rustclaw` on macOS) — the
//!    historical location, used when the exe path is unavailable.
//!
//! There is **no migration**: existing installations keep their old files
//! untouched; a fresh base dir simply starts empty.

use std::path::PathBuf;

/// Base directory for all system-level data (see module docs).
pub fn base_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("RUSTCLAW_HOME") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            return parent.join("rustclaw-data");
        }
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rustclaw")
}

fn join(file: &str) -> PathBuf {
    base_dir().join(file)
}

/// Directory for pasted screenshots / ephemeral image attachments.
pub fn attachments_dir() -> PathBuf {
    join("attachments")
}

/// Auth token store (`auth.json`, 0600).
pub fn auth_json() -> PathBuf {
    join("auth.json")
}

/// Global settings (`config.json`).
pub fn config_json() -> PathBuf {
    join("config.json")
}

/// User-defined providers (`providers.json`).
pub fn providers_json() -> PathBuf {
    join("providers.json")
}

/// Global MCP servers (`mcp.json`).
pub fn mcp_json() -> PathBuf {
    join("mcp.json")
}

/// Main SQLite database (sessions + project memory).
pub fn harness_db() -> PathBuf {
    join("harness.db")
}

/// Monthly token/cost usage reports (`usage-YYYY-MM.json`).
pub fn usage_dir() -> PathBuf {
    base_dir()
}

/// Application log file.
pub fn log_txt() -> PathBuf {
    join("log.txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_paths_share_base_dir() {
        let dir = base_dir();
        assert_eq!(auth_json(), dir.join("auth.json"));
        assert_eq!(config_json(), dir.join("config.json"));
        assert_eq!(providers_json(), dir.join("providers.json"));
        assert_eq!(mcp_json(), dir.join("mcp.json"));
        assert_eq!(harness_db(), dir.join("harness.db"));
        assert_eq!(log_txt(), dir.join("log.txt"));
        assert_eq!(attachments_dir(), dir.join("attachments"));
        assert_eq!(usage_dir(), dir);
    }

    #[test]
    fn test_rustclaw_home_env_overrides_base() {
        // SAFETY: single-threaded test body; guard against concurrent test
        // runners by restoring the previous value afterwards.
        let key = "RUSTCLAW_HOME";
        let prev = std::env::var_os(key);
        std::env::set_var(key, "/tmp/rustclaw-paths-test");
        assert_eq!(base_dir(), PathBuf::from("/tmp/rustclaw-paths-test"));
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        // Sanity: without the env var the base is never the override.
        assert_ne!(base_dir(), PathBuf::from("/tmp/rustclaw-paths-test"));
    }
}
