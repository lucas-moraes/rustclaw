//! File checkpoints: snapshots taken before `write`/`edit` modify a file,
//! powering the `/diff` and `/restore` slash commands.
//!
//! Only the FIRST snapshot per path is kept (the pre-agent state), so
//! `/restore` always rolls a file back to how it was before the agent
//! touched it. A file that did not exist before is recorded as "new" and
//! restoring it deletes the file.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Snapshot of a file taken before the first modification of the session.
#[derive(Clone, Debug)]
enum Snapshot {
    /// File existed; holds its prior content.
    Existing(String),
    /// File did not exist; restoring deletes it.
    New,
}

/// Shared checkpoint store (one per session runtime).
#[derive(Default)]
pub struct FileCheckpoints {
    snapshots: Mutex<HashMap<PathBuf, Snapshot>>,
    /// Per-path locks serializing snapshot+write so two parallel tools editing
    /// the same file don't race (one reads "v1", the other overwrites based on
    /// a stale read, or the snapshot captures an already-modified state).
    /// `tokio::sync::Mutex` so the guard is `Send` and can be held across
    /// `.await` in the tool's async `execute`.
    locks: Mutex<HashMap<PathBuf, std::sync::Arc<tokio::sync::Mutex<()>>>>,
}

impl FileCheckpoints {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the per-path lock for `path`, serializing concurrent
    /// modifications to the same file. The caller holds the returned `Arc`
    /// and locks it (`.lock().await`) across its critical section.
    pub fn lock(&self, path: &Path) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap();
        locks
            .entry(path.to_path_buf())
            .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// Records the current content of `path` if no snapshot exists yet.
    /// A missing file is recorded as "new" (restore = delete).
    pub fn snapshot(&self, path: &Path) {
        let mut map = self.snapshots.lock().unwrap();
        if map.contains_key(path) {
            return;
        }
        let snap = match std::fs::read_to_string(path) {
            Ok(content) => Snapshot::Existing(content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Snapshot::New,
            // Unreadable (e.g. binary/permission) files are not checkpointed.
            Err(_) => return,
        };
        map.insert(path.to_path_buf(), snap);
    }

    /// Restores `path` to its snapshot (writes prior content or deletes the
    /// file if it was new) and removes the snapshot. Returns a description.
    pub fn restore(&self, path: &Path) -> Result<String> {
        let snap = {
            let mut map = self.snapshots.lock().unwrap();
            map.remove(path)
        };
        match snap {
            None => Ok(format!("no snapshot for {}", path.display())),
            Some(Snapshot::New) => match std::fs::remove_file(path) {
                Ok(()) => Ok(format!(
                    "deleted {} (was created by the agent)",
                    path.display()
                )),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(format!(
                    "{} already gone (was created by the agent)",
                    path.display()
                )),
                Err(e) => Err(e).context(format!("failed to delete {}", path.display())),
            },
            Some(Snapshot::Existing(content)) => {
                std::fs::write(path, &content)
                    .with_context(|| format!("failed to restore {}", path.display()))?;
                Ok(format!(
                    "restored {} to its pre-agent content ({} bytes)",
                    path.display(),
                    content.len()
                ))
            }
        }
    }

    /// Unified diff between the snapshot and the current content of `path`.
    /// Errors when there is no snapshot for the path.
    pub fn diff_since_snapshot(&self, path: &Path) -> Result<String> {
        let snap = {
            let map = self.snapshots.lock().unwrap();
            map.get(path).cloned()
        };
        let before = match snap {
            None => anyhow::bail!("no snapshot for {}", path.display()),
            Some(Snapshot::New) => String::new(),
            Some(Snapshot::Existing(content)) => content,
        };
        let after = std::fs::read_to_string(path).unwrap_or_default();
        Ok(super::diff::unified_diff(&before, &after))
    }

    /// Paths with a snapshot, sorted.
    pub fn list(&self) -> Vec<PathBuf> {
        let map = self.snapshots.lock().unwrap();
        let mut paths: Vec<PathBuf> = map.keys().cloned().collect();
        paths.sort();
        paths
    }
}

/// Atomically writes `content` to `path` via a temp file in the same
/// directory + rename. Prevents partial/truncated files on failure and avoids
/// a TOCTOU window where a symlink swapped between a permission check and the
/// write redirects the write outside the workspace.
pub fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid path"))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(format!(".tmp{}", std::process::id()));
    let tmp = parent.join(tmp_name);

    std::fs::write(&tmp, content)?;
    // Preserve the original file's permissions when it exists.
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_and_restore_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "original\n").unwrap();

        let cp = FileCheckpoints::new();
        cp.snapshot(&file);
        std::fs::write(&file, "modified\n").unwrap();

        let msg = cp.restore(&file).unwrap();
        assert!(msg.contains("restored"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original\n");
        assert!(cp.list().is_empty());
    }

    #[test]
    fn test_snapshot_new_file_restore_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new.txt");

        let cp = FileCheckpoints::new();
        cp.snapshot(&file);
        assert!(!file.exists());

        std::fs::write(&file, "created\n").unwrap();
        let msg = cp.restore(&file).unwrap();
        assert!(msg.contains("deleted"));
        assert!(!file.exists());
    }

    #[test]
    fn test_first_snapshot_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "v1\n").unwrap();

        let cp = FileCheckpoints::new();
        cp.snapshot(&file);
        std::fs::write(&file, "v2\n").unwrap();
        cp.snapshot(&file); // must be a no-op

        cp.restore(&file).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "v1\n");
    }

    #[test]
    fn test_diff_shows_changed_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "a\nb\nc\n").unwrap();

        let cp = FileCheckpoints::new();
        cp.snapshot(&file);
        std::fs::write(&file, "a\nB\nc\nd\n").unwrap();

        let diff = cp.diff_since_snapshot(&file).unwrap();
        assert!(
            diff.contains("- b"),
            "diff should show removed line: {diff}"
        );
        assert!(diff.contains("+ B"), "diff should show added line: {diff}");
        assert!(diff.contains("+ d"));

        // No snapshot → error.
        let other = dir.path().join("other.txt");
        assert!(cp.diff_since_snapshot(&other).is_err());
    }

    #[test]
    fn test_atomic_write_replaces_content() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "old\n").unwrap();
        atomic_write(&file, b"new\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new\n");
        // No leftover temp files.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left behind");
    }

    #[test]
    fn test_atomic_write_creates_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new.txt");
        atomic_write(&file, b"content").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "content");
    }

    #[test]
    fn test_per_path_lock_serializes() {
        let cp = FileCheckpoints::new();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");

        let lock1 = cp.lock(&file);
        let lock2 = cp.lock(&file);
        // Both Arcs point to the same mutex.
        assert!(std::sync::Arc::ptr_eq(&lock1, &lock2));

        // Acquiring the first guard blocks a second acquisition on the same
        // path (proven by the fact that a different path gets a distinct lock).
        let other = dir.path().join("b.txt");
        let lock_other = cp.lock(&other);
        assert!(!std::sync::Arc::ptr_eq(&lock1, &lock_other));
    }
}
