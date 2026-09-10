//! Project hooks (`hooks` section of `rustclaw.json`): shell commands run
//! around tool calls and at the end of a turn.
//!
//! ```json
//! { "hooks": {
//!     "pre_tool":  [{"match": "bash", "run": "echo bloqueado", "block": true}],
//!     "post_tool": [{"match": "edit", "glob": "*.rs", "run": "cargo fmt"}],
//!     "on_turn_end": [{"run": "make lint"}]
//! }}
//! ```
//!
//! Hooks never break the turn: spawn/timeout errors are logged and ignored,
//! except for a blocking `pre_tool` hook that exits non-zero (the tool call is
//! skipped and the hook's stderr becomes the model-facing error).

use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single hook rule.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookRule {
    /// Tool name to match: exact name or prefix ending in `*` (e.g. `mcp_*`).
    /// `None` matches every tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#match: Option<String>,
    /// Glob filter applied to the first path-looking string argument
    /// (pre/post only). `None` matches every path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,
    /// Shell command to run in the project cwd.
    pub run: String,
    /// pre_tool only: non-zero exit blocks the tool call.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub block: bool,
}

/// Hooks configured for the current project.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HooksConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_tool: Vec<HookRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_tool: Vec<HookRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_turn_end: Vec<HookRule>,
}

impl HooksConfig {
    /// True when no hook is configured (fast skip, zero overhead).
    pub fn is_empty(&self) -> bool {
        self.pre_tool.is_empty() && self.post_tool.is_empty() && self.on_turn_end.is_empty()
    }

    /// Loads the `hooks` section from `<cwd>/rustclaw.json`; missing file or
    /// section = empty config.
    pub fn load_for_cwd(cwd: &Path) -> Self {
        let path = cwd.join("rustclaw.json");
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str::<HooksFile>(&raw)
            .map(|f| f.hooks.unwrap_or_default())
            .unwrap_or_default()
    }

    /// Rules matching `tool` (and, when `check_glob`, the extracted path).
    pub fn matching<'a>(
        &'a self,
        rules: &'a [HookRule],
        tool: &str,
        args: &serde_json::Value,
        check_glob: bool,
    ) -> Vec<&'a HookRule> {
        rules
            .iter()
            .filter(|r| matches_rule(r, tool, args, check_glob))
            .collect()
    }
}

/// Wrapper so the loader only reads the `hooks` key of rustclaw.json.
#[derive(Deserialize)]
struct HooksFile {
    #[serde(default)]
    hooks: Option<HooksConfig>,
}

/// Does this rule match the tool call?
/// `check_glob` is false for `on_turn_end` (no tool/args there).
pub fn matches_rule(
    rule: &HookRule,
    tool: &str,
    args: &serde_json::Value,
    check_glob: bool,
) -> bool {
    if let Some(m) = &rule.r#match {
        if let Some(prefix) = m.strip_suffix('*') {
            if !tool.starts_with(prefix) {
                return false;
            }
        } else if m != tool {
            return false;
        }
    }
    if check_glob {
        if let Some(g) = &rule.glob {
            match first_path_arg(args) {
                Some(p) => {
                    if !glob_match(g, &p) {
                        return false;
                    }
                }
                None => return false,
            }
        }
    }
    true
}

/// First string argument that looks like a path (contains `/`, `.` or a known
/// path-ish key).
fn first_path_arg(args: &serde_json::Value) -> Option<String> {
    for key in ["path", "file_path", "working_dir", "pattern_path"] {
        if let Some(p) = args.get(key).and_then(|v| v.as_str()) {
            return Some(p.to_string());
        }
    }
    if let Some(obj) = args.as_object() {
        for v in obj.values() {
            if let Some(s) = v.as_str() {
                if s.contains('/') || s.ends_with(".rs") || s.contains('.') && !s.contains(' ') {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

/// Simple glob match supporting `*` (within a component), `**` and `?`.
/// A pattern without `/` also matches against the file basename, so `*.rs`
/// matches `src/main.rs`.
fn glob_match(pattern: &str, path: &str) -> bool {
    if !pattern.contains('/') {
        if let Some(base) = std::path::Path::new(path).file_name() {
            if glob_inner(
                &pattern.chars().collect::<Vec<_>>(),
                &base.to_string_lossy().chars().collect::<Vec<_>>(),
            ) {
                return true;
            }
        }
    }
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = path.chars().collect();
    glob_inner(&pat, &txt)
}

fn glob_inner(pat: &[char], txt: &[char]) -> bool {
    if pat.is_empty() {
        return txt.is_empty();
    }
    match pat[0] {
        '*' => {
            if pat.len() > 1 && pat[1] == '*' {
                // `**` matches anything (including `/`).
                let rest = &pat[2..];
                for i in 0..=txt.len() {
                    if glob_inner(rest, &txt[i..]) {
                        return true;
                    }
                }
                false
            } else {
                // `*` matches within a path component.
                for i in 0..=txt.len() {
                    if glob_inner(&pat[1..], &txt[i..]) {
                        return true;
                    }
                    if i < txt.len() && txt[i] == '/' {
                        break;
                    }
                }
                false
            }
        }
        '?' => !txt.is_empty() && txt[0] != '/' && glob_inner(&pat[1..], &txt[1..]),
        c => !txt.is_empty() && txt[0] == c && glob_inner(&pat[1..], &txt[1..]),
    }
}

/// Outcome of a hook command run.
#[derive(Debug, Clone)]
pub struct HookOutput {
    pub exit_code: i32,
    #[allow(dead_code)] // read by tests
    pub stdout: String,
    pub stderr: String,
}

/// Runs a hook command in `cwd` with a timeout. Never panics; errors are
/// surfaced as a non-zero exit with the message on stderr.
#[cfg(test)]
pub fn run_hook_command(cmd: &str, cwd: &Path) -> HookOutput {
    let mut command = std::process::Command::new("sh");
    command
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    match command.output() {
        Ok(out) => HookOutput {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        },
        Err(e) => HookOutput {
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("failed to spawn hook: {e}"),
        },
    }
}

/// Runs a hook with a timeout (blocking; call from `spawn_blocking`).
/// On timeout the child is killed and reported as a failed run.
pub fn run_hook_with_timeout(cmd: &str, cwd: &Path, timeout: std::time::Duration) -> HookOutput {
    let mut child = match std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return HookOutput {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("failed to spawn hook: {e}"),
            }
        }
    };
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return match child.wait_with_output() {
                    Ok(out) => HookOutput {
                        exit_code: status.code().unwrap_or(-1),
                        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                    },
                    Err(e) => HookOutput {
                        exit_code: status.code().unwrap_or(-1),
                        stdout: String::new(),
                        stderr: format!("failed to read hook output: {e}"),
                    },
                };
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return HookOutput {
                        exit_code: -1,
                        stdout: String::new(),
                        stderr: format!("hook timed out after {timeout:?}"),
                    };
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => {
                return HookOutput {
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: format!("failed to wait for hook: {e}"),
                }
            }
        }
    }
}

/// Runs the pre_tool hooks; returns `Err(msg)` when a blocking hook fails.
pub async fn run_pre_tool(
    hooks: &HooksConfig,
    tool: &str,
    args: &serde_json::Value,
    cwd: &Path,
) -> Result<(), String> {
    if hooks.pre_tool.is_empty() {
        return Ok(());
    }
    let matched = hooks.matching(&hooks.pre_tool, tool, args, true);
    for rule in matched {
        let cwd = cwd.to_path_buf();
        let cmd = rule.run.clone();
        let out = tokio::task::spawn_blocking(move || {
            run_hook_with_timeout(&cmd, &cwd, std::time::Duration::from_secs(30))
        })
        .await
        .unwrap_or(HookOutput {
            exit_code: -1,
            stdout: String::new(),
            stderr: "hook task panicked".to_string(),
        });
        if rule.block && out.exit_code != 0 {
            let stderr = out.stderr.trim();
            return Err(format!(
                "blocked by hook: {}",
                if stderr.is_empty() {
                    format!("`{}` exited with {}", rule.run, out.exit_code)
                } else {
                    stderr.to_string()
                }
            ));
        }
        if out.exit_code != 0 {
            tracing::warn!("pre_tool hook `{}` failed: {}", rule.run, out.stderr.trim());
        }
    }
    Ok(())
}

/// Fire-and-forget post_tool hooks (spawn_blocking; failures logged only).
pub fn spawn_post_tool(hooks: &HooksConfig, tool: &str, args: &serde_json::Value, cwd: &Path) {
    if hooks.post_tool.is_empty() {
        return;
    }
    let matched = hooks.matching(&hooks.post_tool, tool, args, true);
    if matched.is_empty() {
        return;
    }
    let cmds: Vec<String> = matched.iter().map(|r| r.run.clone()).collect();
    let cwd = cwd.to_path_buf();
    tokio::task::spawn_blocking(move || {
        for cmd in cmds {
            let out = run_hook_with_timeout(&cmd, &cwd, std::time::Duration::from_secs(30));
            if out.exit_code != 0 {
                tracing::warn!("post_tool hook `{}` failed: {}", cmd, out.stderr);
            }
        }
    });
}

/// Fire-and-forget on_turn_end hooks.
pub fn spawn_turn_end(hooks: &HooksConfig, cwd: &Path) {
    if hooks.on_turn_end.is_empty() {
        return;
    }
    let cmds: Vec<String> = hooks.on_turn_end.iter().map(|r| r.run.clone()).collect();
    let cwd = cwd.to_path_buf();
    tokio::task::spawn_blocking(move || {
        for cmd in cmds {
            let out = run_hook_with_timeout(&cmd, &cwd, std::time::Duration::from_secs(30));
            if out.exit_code != 0 {
                tracing::warn!("on_turn_end hook `{}` failed: {}", cmd, out.stderr);
            }
        }
    });
}

/// Convenience: load hooks for the session cwd (used by the processor).
#[cfg(test)]
#[allow(dead_code)]
pub fn load_for(cwd: &Path) -> HooksConfig {
    HooksConfig::load_for_cwd(cwd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_example_json() {
        let raw = r#"{
            "hooks": {
                "pre_tool":  [{"match": "bash", "run": "echo bloqueado", "block": true}],
                "post_tool": [{"match": "edit", "glob": "*.rs", "run": "cargo fmt"}],
                "on_turn_end": [{"run": "make lint"}]
            }
        }"#;
        let cfg: HooksFile = serde_json::from_str(raw).unwrap();
        let hooks = cfg.hooks.unwrap();
        assert_eq!(hooks.pre_tool.len(), 1);
        assert_eq!(hooks.pre_tool[0].r#match.as_deref(), Some("bash"));
        assert!(hooks.pre_tool[0].block);
        assert_eq!(hooks.post_tool[0].run, "cargo fmt");
        assert_eq!(hooks.post_tool[0].glob.as_deref(), Some("*.rs"));
        assert_eq!(hooks.on_turn_end[0].run, "make lint");
        assert!(!hooks.on_turn_end[0].block);
    }

    #[test]
    fn test_load_for_cwd_reads_rustclaw_json() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("rustclaw.json"),
            r#"{"hooks": {"on_turn_end": [{"run": "true"}]}}"#,
        )
        .unwrap();
        let hooks = HooksConfig::load_for_cwd(d.path());
        assert_eq!(hooks.on_turn_end.len(), 1);
        assert_eq!(hooks.on_turn_end[0].run, "true");
    }

    #[test]
    fn test_load_for_cwd_missing_file_is_empty() {
        let d = tempfile::tempdir().unwrap();
        assert!(HooksConfig::load_for_cwd(d.path()).is_empty());
    }

    #[test]
    fn test_match_by_tool_name() {
        let rule = HookRule {
            r#match: Some("bash".into()),
            glob: None,
            run: "x".into(),
            block: false,
        };
        assert!(matches_rule(&rule, "bash", &json!({}), true));
        assert!(!matches_rule(&rule, "edit", &json!({}), true));
    }

    #[test]
    fn test_match_prefix_wildcard() {
        let rule = HookRule {
            r#match: Some("mcp_*".into()),
            glob: None,
            run: "x".into(),
            block: false,
        };
        assert!(matches_rule(&rule, "mcp_fs_read", &json!({}), true));
        assert!(!matches_rule(&rule, "bash", &json!({}), true));
    }

    #[test]
    fn test_match_no_match_field_matches_all() {
        let rule = HookRule {
            r#match: None,
            glob: None,
            run: "x".into(),
            block: false,
        };
        assert!(matches_rule(&rule, "anything", &json!({}), true));
    }

    #[test]
    fn test_match_by_glob() {
        let rule = HookRule {
            r#match: Some("edit".into()),
            glob: Some("*.rs".into()),
            run: "cargo fmt".into(),
            block: false,
        };
        assert!(matches_rule(
            &rule,
            "edit",
            &json!({"path": "src/main.rs"}),
            true
        ));
        assert!(!matches_rule(
            &rule,
            "edit",
            &json!({"path": "README.md"}),
            true
        ));
        // No path arg at all -> no match when glob is set.
        assert!(!matches_rule(&rule, "edit", &json!({}), true));
    }

    #[test]
    fn test_glob_ignores_on_turn_end() {
        let rule = HookRule {
            r#match: None,
            glob: Some("*.rs".into()),
            run: "x".into(),
            block: false,
        };
        // check_glob=false skips the glob filter entirely.
        assert!(matches_rule(&rule, "any", &json!({}), false));
    }

    #[test]
    fn test_block_when_exit_nonzero() {
        let out = run_hook_command("sh -c 'exit 1'", Path::new("."));
        assert_eq!(out.exit_code, 1);
        let out = run_hook_command("false", Path::new("."));
        assert_eq!(out.exit_code, 1);
        let out = run_hook_command("true", Path::new("."));
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_hook_captures_stderr() {
        let out = run_hook_command("echo boom >&2", Path::new("."));
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stderr.trim(), "boom");
    }

    #[test]
    fn test_hook_runs_in_cwd() {
        let d = tempfile::tempdir().unwrap();
        let out = run_hook_command("pwd", d.path());
        // macOS reports tempdirs under /private/var/... (symlinked /var);
        // compare canonicalized paths.
        assert_eq!(
            std::fs::canonicalize(out.stdout.trim()).unwrap(),
            std::fs::canonicalize(d.path()).unwrap()
        );
    }

    #[test]
    fn test_timeout_kills_hook() {
        let out = run_hook_with_timeout(
            "sleep 10",
            Path::new("."),
            std::time::Duration::from_millis(100),
        );
        assert_ne!(out.exit_code, 0);
        assert!(out.stderr.contains("timed out"));
    }

    #[test]
    fn test_noop_when_empty() {
        let hooks = HooksConfig::default();
        assert!(hooks.is_empty());
        assert!(hooks
            .matching(&hooks.pre_tool, "bash", &json!({}), true)
            .is_empty());
    }

    #[tokio::test]
    async fn test_pre_tool_block_returns_err() {
        let hooks = HooksConfig {
            pre_tool: vec![HookRule {
                r#match: Some("bash".into()),
                glob: None,
                run: "sh -c 'echo nope >&2; exit 1'".into(),
                block: true,
            }],
            ..Default::default()
        };
        let err = run_pre_tool(&hooks, "bash", &json!({}), Path::new("."))
            .await
            .unwrap_err();
        assert!(err.contains("blocked by hook"), "got: {err}");
        assert!(err.contains("nope"));
    }

    #[tokio::test]
    async fn test_pre_tool_non_blocking_failure_is_ok() {
        let hooks = HooksConfig {
            pre_tool: vec![HookRule {
                r#match: None,
                glob: None,
                run: "false".into(),
                block: false,
            }],
            ..Default::default()
        };
        assert!(run_pre_tool(&hooks, "bash", &json!({}), Path::new("."))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_pre_tool_no_match_is_ok() {
        let hooks = HooksConfig {
            pre_tool: vec![HookRule {
                r#match: Some("edit".into()),
                glob: None,
                run: "false".into(),
                block: true,
            }],
            ..Default::default()
        };
        assert!(run_pre_tool(&hooks, "bash", &json!({}), Path::new("."))
            .await
            .is_ok());
    }

    #[test]
    fn test_first_path_arg_fallback() {
        assert_eq!(
            first_path_arg(&json!({"file_path": "a/b.rs"})).as_deref(),
            Some("a/b.rs")
        );
        assert_eq!(first_path_arg(&json!({"cmd": "ls -la"})), None);
        assert_eq!(
            first_path_arg(&json!({"path": "x.txt"})).as_deref(),
            Some("x.txt")
        );
    }

    #[test]
    fn test_load_for_cwd_corrupt_file_is_empty() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("rustclaw.json"), "{ not json").unwrap();
        assert!(HooksConfig::load_for_cwd(d.path()).is_empty());
    }
}
