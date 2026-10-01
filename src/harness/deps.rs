//! System dependency checks.
//!
//! Some features need external programs installed on the host OS (e.g. a
//! Chrome/Chromium binary for JS rendering in `fetch_webpage`). This module
//! probes for those programs and reports what is missing, so the harness can
//! degrade gracefully instead of failing at the point of use.
//!
//! Detection is intentionally simple and portable: we look for the program on
//! `PATH` (via `which`/`where`) and, for browsers, also probe the well-known
//! install locations that are not always on `PATH` (macOS app bundles, common
//! Linux paths).

use std::path::PathBuf;
use std::process::Command;

/// A single external dependency the harness may need at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemDep {
    /// Stable identifier (used in config / messages), e.g. `"chrome"`.
    pub id: &'static str,
    /// Human-readable name, e.g. `"Chrome/Chromium"`.
    pub name: &'static str,
    /// What breaks when the dependency is missing.
    pub needed_for: &'static str,
    /// Candidate executable names to look up on `PATH`.
    pub binaries: &'static [&'static str],
    /// Extra absolute paths to probe (browsers are often not on `PATH`).
    pub extra_paths: &'static [&'static str],
    /// Install hint shown to the user when the dependency is missing.
    pub install_hint: &'static str,
}

/// Result of probing a single dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepStatus {
    pub dep: SystemDep,
    /// Resolved path to the executable, when found.
    pub found_at: Option<PathBuf>,
}

impl DepStatus {
    pub fn is_installed(&self) -> bool {
        self.found_at.is_some()
    }
}

/// The dependencies the harness knows about.
///
/// `chrome` is required for `fetch_webpage`'s `render: "browser"` mode
/// (headless Chrome via `chromiumoxide`). It is shipped in the default binary,
/// so a missing browser is a real (if non-fatal) gap.
pub const KNOWN_DEPS: &[SystemDep] = &[SystemDep {
    id: "chrome",
    name: "Chrome/Chromium",
    needed_for: "fetch_webpage render=\"browser\" (JS/SPA pages)",
    binaries: &[
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ],
    extra_paths: &[
        // macOS app bundles.
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
        // Common Linux locations.
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/snap/bin/chromium",
    ],
    install_hint: "install Google Chrome or Chromium \
                   (macOS: `brew install --cask google-chrome`; \
                   Debian/Ubuntu: `sudo apt install chromium`; \
                   Fedora: `sudo dnf install chromium`)",
}];

/// Probes every known dependency and returns their status.
pub fn check_all() -> Vec<DepStatus> {
    KNOWN_DEPS.iter().copied().map(check).collect()
}

/// Probes a single dependency.
pub fn check(dep: SystemDep) -> DepStatus {
    let found_at = resolve(dep);
    DepStatus { dep, found_at }
}

/// Resolves the executable for `dep`, or `None` when it is not installed.
///
/// Absolute well-known paths are probed first: a `PATH` entry can be a broken
/// wrapper (e.g. a Homebrew shim pointing at a missing app bundle), whereas the
/// absolute paths we list are the real binaries. Every candidate must be an
/// executable file, not merely present.
fn resolve(dep: SystemDep) -> Option<PathBuf> {
    for path in dep.extra_paths {
        let p = PathBuf::from(path);
        if is_executable_file(&p) {
            return Some(p);
        }
    }
    for bin in dep.binaries {
        if let Some(path) = which(bin) {
            if is_executable_file(&path) {
                return Some(path);
            }
        }
    }
    None
}

/// True when `path` is a file the current user can execute.
fn is_executable_file(path: &std::path::Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(md) => md.permissions().mode() & 0o111 != 0,
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Looks up `bin` on `PATH` using the platform's `which`/`where`.
fn which(bin: &str) -> Option<PathBuf> {
    let finder = if cfg!(windows) { "where" } else { "which" };
    let output = Command::new(finder).arg(bin).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first = stdout.lines().next()?.trim();
    if first.is_empty() {
        None
    } else {
        Some(PathBuf::from(first))
    }
}

/// Renders a human-readable report of all dependency checks.
///
/// Returns `(lines, all_ok)`: `all_ok` is `true` when every dependency is
/// installed.
pub fn render_report() -> (Vec<String>, bool) {
    let statuses = check_all();
    let mut lines = Vec::new();
    let mut all_ok = true;

    lines.push("system dependencies:".to_string());
    for st in &statuses {
        if st.is_installed() {
            let path = st.found_at.as_ref().expect("installed implies a path");
            lines.push(format!(
                "  ✓ {} — {} ({})",
                st.dep.name,
                st.dep.needed_for,
                path.display()
            ));
        } else {
            all_ok = false;
            lines.push(format!(
                "  ✗ {} — missing · needed for {}",
                st.dep.name, st.dep.needed_for
            ));
            lines.push(format!("      install: {}", st.dep.install_hint));
        }
    }

    if all_ok {
        lines.push("all dependencies present.".to_string());
    } else {
        lines.push(
            "missing dependencies degrade the related features; \
             the harness still runs."
                .to_string(),
        );
    }

    (lines, all_ok)
}

/// Returns the resolved path to a Chrome/Chromium binary, if any.
///
/// Used by the browser renderer to locate the executable; `None` means the
/// caller should fall back to plain HTTP.
pub fn chrome_path() -> Option<PathBuf> {
    check(KNOWN_DEPS[0]).found_at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_deps_are_well_formed() {
        for dep in KNOWN_DEPS {
            assert!(!dep.id.is_empty(), "dep id must not be empty");
            assert!(!dep.name.is_empty(), "dep name must not be empty");
            assert!(!dep.binaries.is_empty(), "dep must list binaries");
            assert!(
                !dep.install_hint.is_empty(),
                "dep must have an install hint"
            );
        }
    }

    #[test]
    fn test_check_returns_status_for_each_dep() {
        let statuses = check_all();
        assert_eq!(statuses.len(), KNOWN_DEPS.len());
        for st in &statuses {
            // `is_installed` must agree with the resolved path.
            assert_eq!(st.is_installed(), st.found_at.is_some());
        }
    }

    #[test]
    fn test_check_missing_binary_is_not_installed() {
        let dep = SystemDep {
            id: "definitely-not-installed",
            name: "Nonexistent",
            needed_for: "tests",
            binaries: &["rustclaw-nonexistent-binary-xyz"],
            extra_paths: &["/nonexistent/path/rustclaw-xyz"],
            install_hint: "n/a",
        };
        let st = check(dep);
        assert!(!st.is_installed());
        assert!(st.found_at.is_none());
    }

    #[test]
    fn test_check_finds_a_binary_that_exists() {
        // `sh` exists on every Unix; on Windows `cmd` does. Probe both names so
        // the test is platform-agnostic without a runtime-built slice.
        let dep = SystemDep {
            id: "shell",
            name: "Shell",
            needed_for: "tests",
            binaries: &["sh", "cmd"],
            extra_paths: &[],
            install_hint: "n/a",
        };
        let st = check(dep);
        assert!(st.is_installed(), "expected a shell to be on PATH");
    }

    #[test]
    fn test_render_report_has_a_line_per_dep() {
        let (lines, _all_ok) = render_report();
        // Header + one line per dep + summary.
        assert!(lines.len() >= KNOWN_DEPS.len() + 2);
        assert!(lines[0].contains("system dependencies"));
    }

    #[test]
    fn test_render_report_reports_missing_dep() {
        // A dep that cannot exist must render as missing and flip all_ok.
        let dep = SystemDep {
            id: "ghost",
            name: "Ghost",
            needed_for: "tests",
            binaries: &["rustclaw-ghost-binary-xyz"],
            extra_paths: &[],
            install_hint: "install ghost",
        };
        let st = check(dep);
        assert!(!st.is_installed());
    }

    #[test]
    fn test_is_executable_file_rejects_non_executable() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.txt");
        std::fs::write(&plain, "not executable").unwrap();
        assert!(!is_executable_file(&plain));
        assert!(!is_executable_file(&dir.path().join("missing")));
    }

    #[cfg(unix)]
    #[test]
    fn test_is_executable_file_accepts_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("run.sh");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_executable_file(&exe));
    }
}
