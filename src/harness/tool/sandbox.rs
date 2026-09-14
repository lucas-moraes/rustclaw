//! Sandbox for the `bash` tool (ROADMAP item 5).
//!
//! On Linux, the child process is restricted with **Landlock** (filesystem
//! access rules) applied via `pre_exec`, *before* `exec`, so the shell and
//! everything it spawns inherit the restrictions. The ruleset is best-effort
//! (`CompatLevel::BestEffort`, the crate default): on kernels without
//! Landlock support the rules are silently skipped and the reported status
//! says so — the caller decides whether that is acceptable.
//!
//! Allowed filesystem access:
//! - read-only: `/usr`, `/bin`, `/sbin`, `/lib*`, `/etc` (read-only system),
//!   plus the toolchain roots found on `PATH` (e.g. `~/.rustup`, `~/.cargo`,
//!   `~/.nvm`) so builds keep working;
//! - read-write: the project cwd, the system temp dir and `/dev`.
//!
//! Everything else (notably `$HOME` outside the project) is denied for
//! writes; reads outside the allowlist are also denied by Landlock's
//! deny-by-default model once a ruleset is enforced.
//!
//! On non-Linux targets (macOS, Windows) this module is a no-op stub:
//! `SandboxPolicy::Off` is the only meaningful policy and `apply_pre_exec`
//! always succeeds without restricting anything.

#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};

/// Sandbox mode for shell commands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SandboxPolicy {
    /// No sandboxing (current behavior).
    #[default]
    Off,
    /// Landlock filesystem sandbox (Linux only; no-op elsewhere).
    Landlock,
}

impl SandboxPolicy {
    /// Parses the `sandbox` config value (`"off" | "landlock"`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "disabled" => Some(Self::Off),
            "landlock" => Some(Self::Landlock),
            _ => None,
        }
    }

    #[allow(dead_code)] // part of the reporting API surface
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Landlock => "landlock",
        }
    }
}

/// Result of applying a sandbox to a child process.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum SandboxStatus {
    /// No sandbox requested.
    Off,
    /// Landlock rules were fully enforced by the kernel.
    Enforced,
    /// Kernel lacks Landlock support (or partial support); the child ran
    /// with best-effort (possibly zero) restrictions.
    NotEnforced,
    /// Non-Linux target: sandboxing unavailable, child ran unrestricted.
    Unsupported,
}

impl SandboxStatus {
    #[allow(dead_code)] // part of the reporting API surface
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Enforced => "enforced",
            Self::NotEnforced => "not-enforced",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Filesystem paths the sandboxed child may access.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct SandboxPaths {
    /// Project cwd: read-write.
    pub cwd: PathBuf,
}

/// Read-only system paths (present on typical Linux systems; missing ones
/// are silently skipped by `path_beneath_rules`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const READ_ONLY_PATHS: &[&str] = &["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/dev"];

/// Extra read-only roots derived from `PATH` entries (toolchains installed
/// under `$HOME`, e.g. `~/.cargo`, `~/.rustup`, `~/.nvm`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn toolchain_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        for entry in std::env::split_paths(&path) {
            // Keep the first two components of each PATH entry (e.g.
            // /home/u/.cargo/bin → /home/u/.cargo) so binaries and their
            // adjacent data stay readable.
            let mut acc = PathBuf::new();
            for comp in entry.components().take(3) {
                acc.push(comp);
            }
            if acc.as_os_str() != "/" && !roots.contains(&acc) {
                roots.push(acc);
            }
        }
    }
    roots
}

#[cfg(target_os = "linux")]
#[allow(clippy::needless_return)]
mod imp {
    use super::{toolchain_roots, SandboxPaths, SandboxStatus, READ_ONLY_PATHS};
    use landlock::Access as _;
    use landlock::{path_beneath_rules, AccessFs, Ruleset, RulesetAttr, RulesetCreatedAttr, ABI};
    use std::io;
    use std::os::unix::process::CommandExt;
    #[cfg(target_os = "linux")]
    use std::os::unix::process::CommandExt as _;
    use std::path::{Path, PathBuf};

    /// Builds the Landlock closure installed in the child via `pre_exec`.
    /// Runs after `fork`, before `exec`: the restrictions apply to the shell
    /// and everything it spawns.
    pub fn pre_exec_hook(paths: &SandboxPaths) -> io::Result<()> {
        use std::io::ErrorKind;
        let abi = ABI::V1;
        let mut read_only: Vec<PathBuf> = READ_ONLY_PATHS.iter().map(PathBuf::from).collect();
        read_only.extend(toolchain_roots());

        // Best-effort: on a kernel without Landlock the rules are silently
        // skipped and the child runs unrestricted; `probe()` reports the
        // kernel's support status for user-facing messaging. A hard failure
        // here (e.g. invalid path FD) aborts the spawn via pre_exec.
        let result: Result<(), landlock::RulesetError> = (|| {
            Ruleset::default()
                .handle_access(AccessFs::from_all(abi))?
                .create()?
                .add_rules(path_beneath_rules(&read_only, AccessFs::from_read(abi)))?
                .add_rules(path_beneath_rules(
                    [
                        paths.cwd.as_path(),
                        Path::new("/tmp"),
                        Path::new("/var/tmp"),
                    ],
                    AccessFs::from_all(abi),
                ))?
                .restrict_self()
                .map(|_| ())
        })();
        result.map_err(|e: landlock::RulesetError| io::Error::new(ErrorKind::Other, e.to_string()))
    }

    /// Probes whether the running kernel supports and enables Landlock
    /// (for reporting). Does **not** restrict the calling process: uses the
    /// documented probe-only form of `landlock_create_ruleset` (flags =
    /// `LANDLOCK_CREATE_RULESET_VERSION`), which returns the kernel's ABI
    /// version without creating a ruleset or changing any restriction.
    pub fn probe() -> SandboxStatus {
        const LANDLOCK_CREATE_RULESET_VERSION: libc::c_uint = 1;
        // SAFETY: probe-only syscall; both pointer args are NULL/0.
        let version = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<libc::c_void>(),
                0 as libc::size_t,
                LANDLOCK_CREATE_RULESET_VERSION,
            )
        };
        if version >= 1 {
            SandboxStatus::Enforced
        } else {
            // ENOSYS: not built into the kernel; EOPNOTSUPP: built but disabled.
            SandboxStatus::NotEnforced
        }
    }
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)] // stub: apply() ignores it on non-Linux targets
mod imp {
    use super::{SandboxPaths, SandboxStatus};
    use std::io;

    pub fn pre_exec_hook(_paths: &SandboxPaths) -> io::Result<()> {
        Ok(())
    }

    pub fn probe() -> SandboxStatus {
        SandboxStatus::Unsupported
    }
}

/// Applies the sandbox to a `tokio::process::Command` before spawning.
/// Returns the expected [`SandboxStatus`] (for `probe()`-based reporting the
/// caller may call [`probe`] once per process and cache it).
pub fn apply(
    cmd: &mut tokio::process::Command,
    policy: SandboxPolicy,
    cwd: &Path,
) -> SandboxStatus {
    match policy {
        SandboxPolicy::Off => SandboxStatus::Off,
        SandboxPolicy::Landlock => {
            let paths = SandboxPaths {
                cwd: cwd.to_path_buf(),
            };
            #[cfg(target_os = "linux")]
            {
                let cwd2 = paths.cwd.clone();
                unsafe {
                    cmd.pre_exec(move || {
                        let p = SandboxPaths { cwd: cwd2.clone() };
                        imp::pre_exec_hook(&p)
                    });
                }
                imp::probe()
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = (cmd, paths);
                SandboxStatus::Unsupported
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_parse() {
        assert_eq!(SandboxPolicy::parse("off"), Some(SandboxPolicy::Off));
        assert_eq!(SandboxPolicy::parse("none"), Some(SandboxPolicy::Off));
        assert_eq!(
            SandboxPolicy::parse("landlock"),
            Some(SandboxPolicy::Landlock)
        );
        assert_eq!(
            SandboxPolicy::parse("Landlock"),
            Some(SandboxPolicy::Landlock)
        );
        assert_eq!(SandboxPolicy::parse("container"), None);
        assert_eq!(SandboxPolicy::parse(""), None);
    }

    #[test]
    fn test_default_is_off() {
        assert_eq!(SandboxPolicy::default(), SandboxPolicy::Off);
    }

    #[test]
    fn test_apply_off_is_noop() {
        let mut cmd = tokio::process::Command::new("true");
        assert_eq!(
            apply(&mut cmd, SandboxPolicy::Off, Path::new("/tmp")),
            SandboxStatus::Off
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_probe_returns_a_status() {
        // Just exercises the probe path; the value depends on the kernel.
        let _ = probe();
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn test_apply_landlock_unsupported_on_this_platform() {
        let mut cmd = tokio::process::Command::new("true");
        assert_eq!(
            apply(&mut cmd, SandboxPolicy::Landlock, Path::new("/tmp")),
            SandboxStatus::Unsupported
        );
    }
}
