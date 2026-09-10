//! Sanitized environment for spawned subprocesses (bash tool, MCP stdio).
//!
//! The parent process may carry API keys and other secrets in its environment
//! (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, cloud credentials, etc.). Rather
//! than propagating the whole environment to every child, we build an
//! allowlist of benign variables so secrets never leak into a shell command
//! the model can run (`env | curl -d @- …`).

use std::collections::BTreeMap;

/// Environment variables that are safe to propagate to child processes.
/// Everything else (notably `*_KEY`, `*_TOKEN`, `*_SECRET`, `*_PASSWORD`,
/// `AWS_*`, `AZURE_*`, `GCP_*`) is stripped.
const ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "LC_NUMERIC",
    "LC_TIME",
    "TERM",
    "TERM_PROGRAM",
    "COLORTERM",
    "NO_COLOR",
    "TMPDIR",
    "TMP",
    "TEMP",
    "EDITOR",
    "VISUAL",
    "PAGER",
    "SSH_AUTH_SOCK",
    "SSH_AGENT_PID",
    "GPG_TTY",
    "CI",
    "GITHUB_ACTIONS",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUST_LOG",
    "RUST_BACKTRACE",
    "GOPATH",
    "GOMODCACHE",
    "NVM_DIR",
    "NVM_BIN",
    "NODE_ENV",
    "PYTHONPATH",
    "VIRTUAL_ENV",
    "CONDA_PREFIX",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_SESSION_TYPE",
];

/// Builds a sanitized environment map from the current process environment,
/// keeping only allowlisted variables. `extra` (explicit user config, e.g. an
/// MCP server's `env`) is merged on top and always wins.
pub fn sanitized_env(extra: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in std::env::vars() {
        if ALLOWLIST.contains(&k.as_str()) {
            out.insert(k, v);
        }
    }
    for (k, v) in extra {
        out.insert(k.clone(), v.clone());
    }
    out
}

/// True if a variable name looks like it carries a secret (best-effort).
#[allow(dead_code)] // used in tests, clippy false positive
pub fn looks_like_secret(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("KEY")
        || upper.contains("TOKEN")
        || upper.contains("SECRET")
        || upper.contains("PASSWORD")
        || upper.contains("PASSWD")
        || upper.starts_with("AWS_")
        || upper.starts_with("AZURE_")
        || upper.starts_with("GCP_")
        || upper.starts_with("GOOGLE_")
        || upper.starts_with("OPENAI_")
        || upper.starts_with("ANTHROPIC_")
        || upper.starts_with("DEEPSEEK_")
        || upper.starts_with("GEMINI_")
        || upper.starts_with("MISTRAL_")
        || upper.starts_with("GROQ_")
        || upper.starts_with("XAI_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowlist_keeps_benign_vars() {
        let mut extra = BTreeMap::new();
        extra.insert("MCP_CUSTOM".to_string(), "1".to_string());
        let env = sanitized_env(&extra);
        // PATH is always present in the test process.
        assert!(env.contains_key("PATH"));
        // Explicit extra always wins.
        assert_eq!(env.get("MCP_CUSTOM").map(|s| s.as_str()), Some("1"));
    }

    #[test]
    fn test_secret_vars_are_stripped() {
        // Simulate a secret in the parent env by checking the classifier.
        for name in [
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "GITHUB_TOKEN",
            "DB_PASSWORD",
            "XAI_API_KEY",
        ] {
            assert!(looks_like_secret(name), "{}", name);
        }
        for name in ["PATH", "HOME", "LANG", "TERM", "CI"] {
            assert!(!looks_like_secret(name), "{}", name);
        }
    }
}
