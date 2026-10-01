//! Incremental indexer: walks a project tree and keeps the index in sync.
//!
//! A file is re-indexed only when its content hash changed since the last run;
//! files that disappeared are pruned. Embeddings are computed in batches when
//! an embedder is available, and skipped otherwise (BM25-only index).

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::embed::Embedder;
use super::store::{chunks_for, content_hash, IndexStats, SemanticIndex};

/// Max file size indexed (larger files are skipped to bound cost).
const MAX_FILE_BYTES: u64 = 512 * 1024;
/// Max files indexed in one run (safety bound for huge repos).
const MAX_FILES: usize = 5000;
/// Texts embedded per API call.
const EMBED_BATCH: usize = 64;

/// Extensions indexed as text. Binary/asset files are skipped.
const TEXT_EXTS: &[&str] = &[
    "rs", "toml", "json", "yaml", "yml", "md", "txt", "sh", "bash", "zsh", "py", "js", "ts", "tsx",
    "jsx", "go", "java", "c", "h", "cpp", "hpp", "cc", "rb", "php", "sql", "html", "css", "scss",
    "xml", "ini", "cfg", "conf", "env", "lock", "gradle", "kt", "swift", "lua", "vim",
];

/// Directories never walked (mirrors the `glob` tool's ignore list).
const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".idea",
    ".vscode",
];

/// Rebuilds/refreshes the index for `cwd`. Incremental: unchanged files are
/// skipped, deleted files are pruned.
pub async fn index_project(
    index: &SemanticIndex,
    embedder: &dyn Embedder,
    cwd: &Path,
) -> Result<IndexStats> {
    index.ensure_project(cwd)?;
    let mut stats = IndexStats::default();

    let files = collect_files(cwd);
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for path in files {
        let rel = match path.strip_prefix(cwd) {
            Ok(r) => r.to_string_lossy().to_string(),
            Err(_) => continue,
        };
        seen.insert(rel.clone());

        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let hash = content_hash(&content);
        if index.file_hash(cwd, &rel)?.as_deref() == Some(hash.as_str()) {
            stats.files_skipped += 1;
            continue;
        }

        let chunks = chunks_for(&path, &content);
        if chunks.is_empty() {
            index.remove_file(cwd, &rel)?;
            continue;
        }

        // Embed in batches when a backend is configured.
        let embeddings = if embedder.dim() > 0 {
            let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
            let mut all = Vec::with_capacity(texts.len());
            for batch in texts.chunks(EMBED_BATCH) {
                match embedder.embed(batch).await {
                    Ok(mut vecs) => all.append(&mut vecs),
                    Err(e) => {
                        tracing::warn!("embedding batch failed for {rel}: {e}");
                        all.clear();
                        break;
                    }
                }
            }
            if all.len() == chunks.len() {
                stats.embedded += all.len();
                all
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        index.replace_file(cwd, &rel, &hash, &chunks, &embeddings)?;
        stats.files_indexed += 1;
        stats.chunks += chunks.len();
    }

    // Prune files that no longer exist on disk.
    for rel in index.indexed_paths(cwd)? {
        if !seen.contains(&rel) {
            index.remove_file(cwd, &rel)?;
            stats.files_removed += 1;
        }
    }

    Ok(stats)
}

/// Collects indexable files under `cwd` (bounded, ignoring build dirs).
fn collect_files(cwd: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(cwd, cwd, &mut out);
    out
}

fn walk(base: &Path, cwd: &Path, out: &mut Vec<PathBuf>) {
    if out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !is_ignored(&path, cwd) {
                dirs.push(path);
            }
        } else if path.is_file() && is_indexable(&path) {
            if let Ok(meta) = path.metadata() {
                if meta.len() <= MAX_FILE_BYTES {
                    out.push(path);
                }
            }
        }
        if out.len() >= MAX_FILES {
            return;
        }
    }
    for dir in dirs {
        walk(&dir, cwd, out);
        if out.len() >= MAX_FILES {
            return;
        }
    }
}

/// Whether a path is under an ignored directory (relative to `cwd`).
fn is_ignored(path: &Path, cwd: &Path) -> bool {
    let rel = match path.strip_prefix(cwd) {
        Ok(r) => r,
        Err(_) => return true,
    };
    rel.components().any(|c| {
        IGNORED_DIRS
            .iter()
            .any(|d| c.as_os_str().to_string_lossy() == *d)
    })
}

/// Whether a file has an indexable text extension.
fn is_indexable(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => TEXT_EXTS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::index::embed::NullEmbedder;

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    #[tokio::test]
    async fn test_indexes_rust_files() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/a.rs", "pub fn foo() {}\n");
        write(dir.path(), "README.md", "# hi\n");
        let db = dir.path().join("index.db");
        let idx = SemanticIndex::open(&db).unwrap();
        let stats = index_project(&idx, &NullEmbedder, dir.path())
            .await
            .unwrap();
        assert_eq!(stats.files_indexed, 2);
        assert!(stats.chunks >= 2);
        assert_eq!(idx.file_count(dir.path()).unwrap(), 2);
    }

    #[tokio::test]
    async fn test_incremental_skips_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "pub fn foo() {}\n");
        let db = dir.path().join("index.db");
        let idx = SemanticIndex::open(&db).unwrap();
        let s1 = index_project(&idx, &NullEmbedder, dir.path())
            .await
            .unwrap();
        assert_eq!(s1.files_indexed, 1);
        let s2 = index_project(&idx, &NullEmbedder, dir.path())
            .await
            .unwrap();
        assert_eq!(s2.files_indexed, 0);
        assert_eq!(s2.files_skipped, 1);
    }

    #[tokio::test]
    async fn test_reindexes_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "pub fn foo() {}\n");
        let db = dir.path().join("index.db");
        let idx = SemanticIndex::open(&db).unwrap();
        index_project(&idx, &NullEmbedder, dir.path())
            .await
            .unwrap();
        write(dir.path(), "a.rs", "pub fn bar() {}\n");
        let s = index_project(&idx, &NullEmbedder, dir.path())
            .await
            .unwrap();
        assert_eq!(s.files_indexed, 1);
        let hits = idx.search_bm25(dir.path(), "bar", 5).unwrap();
        assert!(hits.iter().any(|h| h.text.contains("bar")));
    }

    #[tokio::test]
    async fn test_prunes_deleted_file() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "pub fn foo() {}\n");
        write(dir.path(), "b.rs", "pub fn baz() {}\n");
        let db = dir.path().join("index.db");
        let idx = SemanticIndex::open(&db).unwrap();
        index_project(&idx, &NullEmbedder, dir.path())
            .await
            .unwrap();
        std::fs::remove_file(dir.path().join("b.rs")).unwrap();
        let s = index_project(&idx, &NullEmbedder, dir.path())
            .await
            .unwrap();
        assert_eq!(s.files_removed, 1);
        assert_eq!(idx.file_count(dir.path()).unwrap(), 1);
    }

    #[tokio::test]
    async fn test_ignores_target_dir() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/a.rs", "pub fn foo() {}\n");
        write(dir.path(), "target/debug/junk.rs", "pub fn junk() {}\n");
        let db = dir.path().join("index.db");
        let idx = SemanticIndex::open(&db).unwrap();
        index_project(&idx, &NullEmbedder, dir.path())
            .await
            .unwrap();
        assert_eq!(idx.file_count(dir.path()).unwrap(), 1);
    }

    #[test]
    fn test_is_indexable() {
        assert!(is_indexable(Path::new("a.rs")));
        assert!(is_indexable(Path::new("a.RS")));
        assert!(!is_indexable(Path::new("a.png")));
        assert!(!is_indexable(Path::new("Makefile")));
    }
}
