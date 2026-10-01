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

/// Masks secret-looking values in command output (defense in depth).
///
/// Two passes:
/// 1. Lines matching `NAME=value`, `export NAME=value`, `"NAME": "value"` or
///    `NAME: value` where `looks_like_secret(NAME)` → value replaced by `***`.
/// 2. Known secret values from the *parent* process environment (vars where
///    `looks_like_secret` is true and value ≥ 8 chars) are replaced by `***`
///    wherever they appear verbatim (e.g. `cat ~/.aws/credentials`,
///    `echo $OPENAI_API_KEY` when the var leaked through some other path).
pub fn mask_secrets(text: &str) -> String {
    let masked: Vec<String> = text.lines().map(mask_assignment_line).collect();
    let mut out = masked.join("\n");
    // Preserve trailing newline if the input had one.
    if text.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }

    let mut secrets: Vec<String> = std::env::vars()
        .filter(|(k, v)| looks_like_secret(k) && v.len() >= 8)
        .map(|(_, v)| v)
        .collect();
    // Longest first: avoids partial masking when one secret is a substring
    // of another.
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    secrets.dedup();
    for secret in secrets {
        out = out.replace(&secret, "***");
    }
    out
}

/// Masks the value of a single `NAME=value` / `NAME: value` assignment line
/// when `NAME` looks like a secret. Handles `export ` prefixes and quotes.
fn mask_assignment_line(line: &str) -> String {
    let leading = &line[..line.len() - line.trim_start().len()];
    let trimmed = line.trim_start();
    let (prefix, body) = match trimmed.strip_prefix("export ") {
        Some(rest) => ("export ", rest),
        None => ("", trimmed),
    };

    // First `=` or `:` outside quotes.
    let mut sep_idx = None;
    let mut in_single = false;
    let mut in_double = false;
    for (i, c) in body.char_indices() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '=' | ':' if !in_single && !in_double => {
                sep_idx = Some(i);
                break;
            }
            _ => {}
        }
    }
    let Some(idx) = sep_idx else {
        return line.to_string();
    };

    let name = body[..idx].trim().trim_matches('"').trim_matches('\'');
    if name.is_empty() || !looks_like_secret(name) {
        return line.to_string();
    }

    let sep = body.as_bytes()[idx] as char;
    let after = &body[idx + 1..];
    // For `:` require whitespace after it (avoids mangling `PATH=/a:/b`, URLs).
    if sep == ':' && !after.starts_with(char::is_whitespace) {
        return line.to_string();
    }

    let spacing = &after[..after.len() - after.trim_start().len()];
    let value_full = after.trim_start();
    let value = value_full.trim_end();
    let trailing = &value_full[value.len()..];

    let masked = if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        let q = &value[..1];
        format!("{}***{}", q, q)
    } else {
        "***".to_string()
    };

    format!(
        "{}{}{}{}{}{}{}",
        leading,
        prefix,
        &body[..idx],
        sep,
        spacing,
        masked,
        trailing
    )
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

    #[test]
    fn test_mask_assignment_lines() {
        let input = "\
OPENAI_API_KEY=sk-abc123
export AWS_SECRET_ACCESS_KEY=\"foo\"
\"github_token\": \"ghp_xyz\"
DB_PASSWORD: hunter2
PATH=/usr/bin
";
        let out = mask_secrets(input);
        assert!(out.contains("OPENAI_API_KEY=***"), "{}", out);
        assert!(
            out.contains("export AWS_SECRET_ACCESS_KEY=\"***\""),
            "{}",
            out
        );
        assert!(out.contains("\"github_token\": \"***\""), "{}", out);
        assert!(out.contains("DB_PASSWORD: ***"), "{}", out);
        assert!(out.contains("PATH=/usr/bin"), "{}", out);
        assert!(!out.contains("sk-abc123"), "{}", out);
        assert!(!out.contains("ghp_xyz"), "{}", out);
        assert!(!out.contains("hunter2"), "{}", out);
    }

    #[test]
    fn test_mask_known_env_values() {
        // Unique name/value to avoid interference with parallel tests.
        std::env::set_var("RUSTCLAW_TEST_SECRET_TOKEN", "hunter2hunter2");
        let text = "leaked: hunter2hunter2\nother line";
        let out = mask_secrets(text);
        std::env::remove_var("RUSTCLAW_TEST_SECRET_TOKEN");
        assert!(out.contains("leaked: ***"), "{}", out);
        assert!(!out.contains("hunter2hunter2"), "{}", out);
    }

    #[test]
    fn test_mask_preserves_benign() {
        let input = "PATH=/usr/bin\nHOME=/Users/x\nLANG=en_US.UTF-8\n";
        assert_eq!(mask_secrets(input), input);
    }

    #[test]
    fn test_mask_short_values_ignored() {
        // Values < 8 chars are not masked by pass 2 (verbatim env replace).
        std::env::set_var("RUSTCLAW_TEST_SHORT_KEY", "abc");
        let text = "value abc here";
        let out = mask_secrets(text);
        std::env::remove_var("RUSTCLAW_TEST_SHORT_KEY");
        assert_eq!(out, text);
    }
}
