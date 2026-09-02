//! Stable, injection-safe table naming for per-project SQLite tables.
//!
//! Each project root maps to a deterministic, hex-only suffix derived from a
//! SHA-256 of its canonical path. Table names are interpolated into SQL (SQLite
//! cannot parameterize identifiers), so the hex-only form guarantees safety.

use sha2::{Digest, Sha256};
use std::path::Path;

/// Hex-only suffix (16 chars) for a project root. Stable across runs.
pub fn project_suffix(cwd: &Path) -> String {
    let canon = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canon.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest
        .iter()
        .take(8)
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Fully-qualified per-project table name, e.g. `project_<h>_sessions`.
pub fn table_name(cwd: &Path, suffix: &str) -> String {
    format!("project_{}_{}", project_suffix(cwd), suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suffix_is_hex_and_stable() {
        let d = tempfile::tempdir().unwrap();
        let a = project_suffix(d.path());
        let b = project_suffix(d.path());
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_distinct_paths_distinct_suffixes() {
        let d1 = tempfile::tempdir().unwrap();
        let d2 = tempfile::tempdir().unwrap();
        assert_ne!(project_suffix(d1.path()), project_suffix(d2.path()));
    }

    #[test]
    fn test_table_name_shape() {
        let d = tempfile::tempdir().unwrap();
        let t = table_name(d.path(), "sessions");
        assert!(t.starts_with("project_"));
        assert!(t.ends_with("_sessions"));
    }
}
