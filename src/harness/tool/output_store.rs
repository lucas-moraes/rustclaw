//! Persistent store for original (uncompressed) tool outputs.
//!
//! When [`crate::harness::tool::compress`] replaces a large tool output
//! with a summary, the original bytes are saved here so the model (or the
//! user) can recover them byte-exact via the `read` tool. Files are
//! content-addressed by SHA-256, so identical outputs dedupe naturally
//! and nothing is ever lost.
//!
//! Layout: `<base_dir>/tool-outputs/<first2>/<sha256>.txt`

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// Subdirectory of the base dir holding saved outputs.
pub const OUTPUTS_DIR: &str = "tool-outputs";

/// Computes the SHA-256 hex digest of `text`.
pub fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in digest {
        hex.push_str(&format!("{:02x}", b));
    }
    hex
}

/// Returns the on-disk path for `text` under `base_dir`, without writing.
pub fn path_for(base_dir: &Path, text: &str) -> PathBuf {
    let hash = hash_text(text);
    base_dir
        .join(OUTPUTS_DIR)
        .join(&hash[..2])
        .join(format!("{}.txt", hash))
}

/// Saves `text` under `base_dir` and returns its path.
///
/// Content-addressed: if the file already exists, it is left untouched
/// (identical content, no write needed). Returns the path either way.
pub fn save(base_dir: &Path, text: &str) -> Result<PathBuf> {
    let path = path_for(base_dir, text);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create tool-outputs dir {}", parent.display()))?;
    }
    if path.exists() {
        return Ok(path); // dedupe: same content already saved
    }
    let tmp = path.with_extension("txt.tmp");
    fs::write(&tmp, text)
        .with_context(|| format!("failed to write tool output {}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .with_context(|| format!("failed to finalize tool output {}", path.display()))?;
    Ok(path)
}

/// Loads a previously saved output by its hash.
pub fn load(base_dir: &Path, hash: &str) -> io::Result<String> {
    let path = base_dir
        .join(OUTPUTS_DIR)
        .join(&hash[..2.min(hash.len())])
        .join(format!("{}.txt", hash));
    fs::read_to_string(&path)
}

/// Removes saved outputs that are no longer referenced by any session.
/// Returns the number of files removed.
///
/// Conservative: only files older than `min_age_secs` are considered, and
/// any file whose hash appears in `keep_hashes` is always retained.
pub fn gc(base_dir: &Path, keep_hashes: &[String], min_age_secs: u64) -> Result<usize> {
    let root = base_dir.join(OUTPUTS_DIR);
    if !root.exists() {
        return Ok(0);
    }
    let mut removed = 0usize;
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(min_age_secs))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    for entry in fs::read_dir(&root).context("failed to read tool-outputs dir")? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        for file in fs::read_dir(entry.path())? {
            let file = file?;
            let name = file.file_name().to_string_lossy().into_owned();
            let Some(hash) = name.strip_suffix(".txt") else {
                continue;
            };
            if keep_hashes.iter().any(|k| k == hash) {
                continue;
            }
            let meta = file.metadata()?;
            let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if modified < cutoff {
                fs::remove_file(file.path())?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_text_stable() {
        let a = hash_text("hello");
        let b = hash_text("hello");
        let c = hash_text("world");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let text = "original output\nline two\n";
        let path = save(base, text).expect("save");
        assert!(path.exists());
        assert!(path.to_string_lossy().contains("tool-outputs"));
        // Roundtrip.
        let loaded = load(base, &hash_text(text)).expect("load");
        assert_eq!(loaded, text);
        // Dedupe: second save of same content is a no-op returning same path.
        let path2 = save(base, text).expect("save again");
        assert_eq!(path, path2);
    }

    #[test]
    fn test_gc_removes_old_unreferenced() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let old = "old content";
        let keep = "keep content";
        save(base, old).expect("save old");
        let keep_path = save(base, keep).expect("save keep");
        // GC with min_age 0: everything unreferenced goes away.
        let removed = gc(base, &[hash_text(keep)], 0).expect("gc");
        assert_eq!(removed, 1);
        assert!(!path_for(base, old).exists());
        assert!(keep_path.exists());
    }

    #[test]
    fn test_gc_keeps_recent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        save(base, "recent").expect("save");
        // min_age huge: nothing is old enough to remove.
        let removed = gc(base, &[], u64::MAX / 2).expect("gc");
        assert_eq!(removed, 0);
        assert!(path_for(base, "recent").exists());
    }
}
