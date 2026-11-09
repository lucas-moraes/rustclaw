//! History-independent evidence for the runtime delivery gate.
//!
//! Workspace capture is synchronous filesystem/process work. Async callers should
//! run it on a blocking worker, and serialize potentially mutating operations.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VerificationEvidence {
    pub label: String,
    pub success: bool,
    pub current: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeliveryReport {
    pub changed_files: Vec<String>,
    pub operations: Vec<String>,
    pub checks: Vec<VerificationEvidence>,
    pub verified: bool,
}

impl DeliveryReport {
    pub fn has_changes(&self) -> bool {
        !self.changed_files.is_empty() || !self.operations.is_empty()
    }

    pub fn pending(&self) -> bool {
        self.has_changes() && !self.verified
    }
}

#[derive(Debug)]
struct RecordedVerification {
    label: String,
    started_revision: u64,
    success: bool,
}

#[derive(Debug, Default)]
struct DeliveryState {
    revision: u64,
    changed_files: BTreeSet<String>,
    operations: Vec<String>,
    checks: Vec<RecordedVerification>,
}

/// Thread-safe evidence shared independently of conversation history.
#[derive(Debug, Default)]
pub struct DeliveryTracker {
    state: Mutex<DeliveryState>,
}

impl DeliveryTracker {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, DeliveryState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn snapshot(&self) -> DeliveryReport {
        let state = self.lock();
        let mut report = DeliveryReport {
            changed_files: state.changed_files.iter().cloned().collect(),
            operations: state.operations.clone(),
            checks: state
                .checks
                .iter()
                .map(|check| VerificationEvidence {
                    label: check.label.clone(),
                    success: check.success,
                    current: check.started_revision == state.revision,
                })
                .collect(),
            verified: false,
        };
        report.verified = report.has_changes()
            && report
                .checks
                .last()
                .is_some_and(|check| check.success && check.current);
        report
    }

    /// Record one observed mutation batch. Repeated paths still advance revision;
    /// an empty batch does not represent a mutation and leaves revision unchanged.
    pub fn record_changes(&self, paths: impl IntoIterator<Item = String>) {
        let paths: Vec<_> = paths.into_iter().collect();
        if paths.is_empty() {
            return;
        }
        let mut state = self.lock();
        state.changed_files.extend(paths);
        state.revision += 1;
    }

    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// Completion order determines the latest check, not its starting revision.
    pub fn record_verification(&self, label: String, started_revision: u64, success: bool) {
        self.lock().checks.push(RecordedVerification {
            label,
            started_revision,
            success,
        });
    }

    /// Record a potentially mutating operation not fully observable in a snapshot
    /// (for example, a background process), invalidating prior verification.
    pub fn mark_uncertain(&self, label: String) {
        let mut state = self.lock();
        state.operations.push(label);
        state.revision += 1;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
enum FileState {
    Missing,
    File([u8; 32]),
    Symlink { path: PathBuf, target: PathBuf },
    Other,
}

/// Content fingerprints of tracked and non-ignored untracked Git paths.
#[derive(Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct WorkspaceSnapshot {
    files: BTreeMap<String, FileState>,
}

impl WorkspaceSnapshot {
    /// Return `None` outside Git, or if a complete snapshot cannot be obtained.
    /// Deleted tracked paths remain present with a missing-file fingerprint.
    /// Symlinks (including directory ancestors) are inspected, never followed in
    /// a stable workspace. This is not an atomic filesystem snapshot: callers
    /// must account conservatively for concurrent/background mutations.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn capture(cwd: &Path) -> Option<Self> {
        let root_output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .ok()?;
        if !root_output.status.success() {
            return None;
        }
        let root_bytes = root_output.stdout.strip_suffix(b"\n")?;
        let root = Path::new(std::str::from_utf8(root_bytes).ok()?);
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let mut files = BTreeMap::new();
        for path in output.stdout.split(|byte| *byte == 0) {
            if path.is_empty() {
                continue;
            }
            let path = std::str::from_utf8(path).ok()?;
            let state = file_state(root, Path::new(path)).ok()?;
            files.insert(path.to_owned(), state);
        }
        Some(Self { files })
    }

    /// Sorted, deduplicated repository-relative paths with changed fingerprints.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn changed_paths(&self, after: &Self) -> Vec<String> {
        self.files
            .keys()
            .chain(after.files.keys())
            .filter(|path| self.files.get(*path) != after.files.get(*path))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn file_state(root: &Path, relative: &Path) -> io::Result<FileState> {
    let mut path = root.to_path_buf();
    for component in relative.components() {
        path.push(component);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(FileState::Missing);
            }
            Err(error) => return Err(error),
        };
        if metadata.file_type().is_symlink() {
            return Ok(FileState::Symlink {
                path: path.strip_prefix(root).unwrap_or(&path).to_path_buf(),
                target: fs::read_link(&path)?,
            });
        }
        if path == root.join(relative) {
            return if metadata.is_file() {
                Ok(FileState::File(Sha256::digest(fs::read(path)?).into()))
            } else {
                Ok(FileState::Other)
            };
        }
    }
    Ok(FileState::Other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn repeated_changes_invalidate_verification() {
        let tracker = DeliveryTracker::new();
        assert!(!tracker.snapshot().has_changes());
        assert!(!tracker.snapshot().pending());
        tracker.record_changes(Vec::new());
        assert_eq!(tracker.revision(), 0);

        tracker.record_changes(["b.rs".into(), "a.rs".into()]);
        assert_eq!(tracker.revision(), 1);
        tracker.record_verification("cargo test".into(), tracker.revision(), true);
        assert!(tracker.snapshot().verified);

        tracker.record_changes(["a.rs".into()]);
        assert_eq!(tracker.revision(), 2);
        let report = tracker.snapshot();
        assert_eq!(report.changed_files, ["a.rs", "b.rs"]);
        assert!(report.pending());
        assert!(!report.checks[0].current);
        assert!(!report.verified);
    }

    #[test]
    fn failure_after_success_invalidates_verification() {
        let tracker = DeliveryTracker::default();
        tracker.record_changes(["a.rs".into()]);
        let revision = tracker.revision();
        tracker.record_verification("first".into(), revision, true);
        assert!(tracker.snapshot().verified);
        tracker.record_verification("second".into(), revision, false);
        let report = tracker.snapshot();
        assert!(report.checks.iter().all(|check| check.current));
        assert!(report.pending());
        assert!(!report.verified);
    }

    #[test]
    fn stale_completion_cannot_verify_newer_changes() {
        let tracker = DeliveryTracker::default();
        tracker.record_changes(["a.rs".into()]);
        let started = tracker.revision();
        tracker.record_changes(["a.rs".into()]);
        tracker.record_verification("newer".into(), tracker.revision(), true);
        assert!(tracker.snapshot().verified);
        tracker.record_verification("older finished last".into(), started, true);
        assert!(tracker.snapshot().pending());
    }

    #[test]
    fn uncertainty_counts_as_a_mutation_without_known_files() {
        let tracker = DeliveryTracker::default();
        tracker.record_verification("clean".into(), 0, true);
        assert!(!tracker.snapshot().verified);
        tracker.mark_uncertain("background command".into());
        let report = tracker.snapshot();
        assert_eq!(tracker.revision(), 1);
        assert!(report.changed_files.is_empty());
        assert_eq!(report.operations, ["background command"]);
        assert!(report.pending());
        tracker.record_verification("checked".into(), tracker.revision(), true);
        assert!(tracker.snapshot().verified);
        tracker.mark_uncertain("another command".into());
        assert!(tracker.snapshot().pending());
    }

    #[test]
    fn mutations_are_thread_safe() {
        let tracker = Arc::new(DeliveryTracker::default());
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let tracker = Arc::clone(&tracker);
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        tracker.record_changes(["same.rs".into()]);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(tracker.revision(), 800);
        assert_eq!(tracker.snapshot().changed_files, ["same.rs"]);
    }

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn git_snapshots_detect_untracked_staged_and_deleted_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "--quiet"]);
        fs::write(root.join("tracked.rs"), "before").unwrap();
        fs::write(root.join("deleted.rs"), "delete me").unwrap();
        fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        git(root, &["add", "."]);
        let before = WorkspaceSnapshot::capture(root).unwrap();

        fs::write(root.join("tracked.rs"), "after!").unwrap();
        fs::remove_file(root.join("deleted.rs")).unwrap();
        fs::write(root.join("untracked.rs"), "new").unwrap();
        fs::write(root.join("staged.rs"), "staged").unwrap();
        fs::write(root.join("ignored.txt"), "not included").unwrap();
        git(root, &["add", "staged.rs"]);
        let after = WorkspaceSnapshot::capture(root).unwrap();
        assert_eq!(
            before.changed_paths(&after),
            ["deleted.rs", "staged.rs", "tracked.rs", "untracked.rs"]
        );
        assert!(after.changed_paths(&after).is_empty());
        assert_eq!(after.files.get("deleted.rs"), Some(&FileState::Missing));
        assert!(!after.files.contains_key("ignored.txt"));
        assert!(!after.files.keys().any(|path| path.starts_with(".git/")));

        fs::create_dir(root.join("nested")).unwrap();
        let from_nested = WorkspaceSnapshot::capture(&root.join("nested")).unwrap();
        assert!(after.changed_paths(&from_nested).is_empty());
        fs::remove_file(root.join("untracked.rs")).unwrap();
        git(root, &["rm", "--cached", "deleted.rs"]);
        let removed = WorkspaceSnapshot::capture(root).unwrap();
        assert_eq!(
            after.changed_paths(&removed),
            ["deleted.rs", "untracked.rs"]
        );
    }

    #[test]
    fn replacing_tracked_directory_with_a_file_keeps_snapshot_available() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "--quiet"]);
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/tracked.rs"), "before").unwrap();
        git(root, &["add", "."]);
        let before = WorkspaceSnapshot::capture(root).unwrap();
        fs::remove_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested"), "replacement").unwrap();
        let after = WorkspaceSnapshot::capture(root).unwrap();
        assert_eq!(
            before.changed_paths(&after),
            ["nested", "nested/tracked.rs"]
        );
        assert_eq!(
            after.files.get("nested/tracked.rs"),
            Some(&FileState::Missing)
        );
    }

    #[test]
    fn non_repository_has_no_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        assert!(WorkspaceSnapshot::capture(dir.path()).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_targets_and_ancestors_are_not_followed() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "--quiet"]);
        fs::write(external.path().join("outside.rs"), "before").unwrap();
        symlink(external.path().join("outside.rs"), root.join("link.rs")).unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/outside.rs"), "inside").unwrap();
        git(root, &["add", "."]);
        fs::remove_dir_all(root.join("nested")).unwrap();
        symlink(external.path(), root.join("nested")).unwrap();
        let before = WorkspaceSnapshot::capture(root).unwrap();
        fs::write(external.path().join("outside.rs"), "after!").unwrap();
        let after = WorkspaceSnapshot::capture(root).unwrap();
        assert!(before.changed_paths(&after).is_empty());

        fs::remove_file(root.join("link.rs")).unwrap();
        symlink("missing.rs", root.join("link.rs")).unwrap();
        let changed = WorkspaceSnapshot::capture(root).unwrap();
        assert_eq!(after.changed_paths(&changed), ["link.rs"]);
    }
}
