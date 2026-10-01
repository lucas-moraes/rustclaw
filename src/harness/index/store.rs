//! Persistent semantic index (SQLite).
//!
//! One set of per-project tables holds the indexed chunks:
//!
//! - `project_<h>_chunks` — one row per chunk (path, lines, symbol, text,
//!   content hash, embedding blob).
//! - `project_<h>_chunks_fts` — FTS5 external-content index over the chunk
//!   text, kept in sync by triggers, powering BM25 lexical search.
//!
//! Indexing is incremental: a file is re-indexed only when its content hash
//! changes, and chunks of deleted files are pruned. The store shares the same
//! SQLite file as sessions and project memory.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use super::chunk::{chunk_source, Chunk};
use super::embed::{decode_vector, encode_vector};
use crate::harness::project::table::table_name;

/// A chunk row as stored in the index.
#[derive(Clone, Debug)]
pub struct IndexedChunk {
    pub id: i64,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub symbol: Option<String>,
    pub kind: Option<String>,
    pub text: String,
    /// BM25 rank from FTS5 (lower = better); `None` for vector-only hits.
    pub bm25: Option<f64>,
}

/// Summary of an index run, for `/index` status.
#[derive(Clone, Debug, Default)]
pub struct IndexStats {
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub files_removed: usize,
    pub chunks: usize,
    pub embedded: usize,
}

/// The persistent semantic index for one project.
pub struct SemanticIndex {
    conn: Mutex<Connection>,
}

impl SemanticIndex {
    /// Opens (and migrates) the index on the shared DB file.
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create data dir {}", parent.display()))?;
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("failed to open semantic index db {}", db_path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .context("failed to set journal mode")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Creates the per-project chunk tables (idempotent).
    pub fn ensure_project(&self, cwd: &Path) -> Result<()> {
        let table = table_name(cwd, "chunks");
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {table} (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                symbol TEXT,
                kind TEXT,
                text TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                embedding BLOB,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_{table}_path ON {table}(path);"
        ))
        .with_context(|| format!("failed to ensure chunks table for {}", cwd.display()))?;
        self.ensure_fts(&conn, &table)?;
        Ok(())
    }

    /// Creates the FTS5 index over the chunk text (idempotent).
    fn ensure_fts(&self, conn: &Connection, table: &str) -> Result<()> {
        let fts = format!("{table}_fts");
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![fts],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !exists {
            conn.execute_batch(&format!(
                "CREATE VIRTUAL TABLE {fts} USING fts5(text, content='{table}', content_rowid='id');
                 CREATE TRIGGER {table}_fts_ai AFTER INSERT ON {table} BEGIN
                     INSERT INTO {fts}(rowid, text) VALUES (new.id, new.text);
                 END;
                 CREATE TRIGGER {table}_fts_ad AFTER DELETE ON {table} BEGIN
                     INSERT INTO {fts}({fts}, rowid, text) VALUES('delete', old.id, old.text);
                 END;
                 CREATE TRIGGER {table}_fts_au AFTER UPDATE ON {table} BEGIN
                     INSERT INTO {fts}({fts}, rowid, text) VALUES('delete', old.id, old.text);
                     INSERT INTO {fts}(rowid, text) VALUES (new.id, new.text);
                 END;"
            ))
            .with_context(|| format!("failed to create FTS index for {table}"))?;
        }
        Ok(())
    }

    /// Number of indexed chunks for `cwd`.
    pub fn chunk_count(&self, cwd: &Path) -> Result<usize> {
        let table = table_name(cwd, "chunks");
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap_or(0);
        Ok(n as usize)
    }

    /// Number of indexed files for `cwd`.
    pub fn file_count(&self, cwd: &Path) -> Result<usize> {
        let table = table_name(cwd, "chunks");
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n: i64 = conn
            .query_row(
                &format!("SELECT COUNT(DISTINCT path) FROM {table}"),
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok(n as usize)
    }

    /// Number of chunks that carry an embedding (vector-search candidates).
    pub fn embedded_count(&self, cwd: &Path) -> Result<usize> {
        let table = table_name(cwd, "chunks");
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE embedding IS NOT NULL"),
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok(n as usize)
    }

    /// Content hash of an indexed file, or `None` when not indexed.
    pub fn file_hash(&self, cwd: &Path, path: &str) -> Result<Option<String>> {
        let table = table_name(cwd, "chunks");
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let hash: Option<String> = conn
            .query_row(
                &format!("SELECT content_hash FROM {table} WHERE path=?1 LIMIT 1"),
                params![path],
                |r| r.get(0),
            )
            .ok();
        Ok(hash)
    }

    /// Replaces all chunks of `path` with `chunks`. `embeddings` must be either
    /// empty (no vectors) or one vector per chunk.
    pub fn replace_file(
        &self,
        cwd: &Path,
        path: &str,
        content_hash: &str,
        chunks: &[Chunk],
        embeddings: &[Vec<f32>],
    ) -> Result<()> {
        let table = table_name(cwd, "chunks");
        let now = chrono::Utc::now().to_rfc3339();
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction()?;
        tx.execute(&format!("DELETE FROM {table} WHERE path=?1"), params![path])?;
        {
            let mut stmt = tx.prepare(&format!(
                "INSERT INTO {table}
                 (path, start_line, end_line, symbol, kind, text, content_hash, embedding, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            ))?;
            for (i, c) in chunks.iter().enumerate() {
                let blob = embeddings.get(i).map(|v| encode_vector(v));
                stmt.execute(params![
                    path,
                    c.start_line as i64,
                    c.end_line as i64,
                    c.symbol,
                    c.kind,
                    c.text,
                    content_hash,
                    blob,
                    now,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Removes all chunks of `path` (used when a file is deleted).
    pub fn remove_file(&self, cwd: &Path, path: &str) -> Result<()> {
        let table = table_name(cwd, "chunks");
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(&format!("DELETE FROM {table} WHERE path=?1"), params![path])?;
        Ok(())
    }

    /// All indexed file paths for `cwd`.
    pub fn indexed_paths(&self, cwd: &Path) -> Result<Vec<String>> {
        let table = table_name(cwd, "chunks");
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!("SELECT DISTINCT path FROM {table}"))?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// BM25 lexical search over the chunk text. Returns chunks ordered by rank.
    pub fn search_bm25(&self, cwd: &Path, query: &str, limit: usize) -> Result<Vec<IndexedChunk>> {
        let table = table_name(cwd, "chunks");
        let fts = format!("{table}_fts");
        let match_expr = fts_escape(query);
        if match_expr.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let sql = format!(
            "SELECT c.id, c.path, c.start_line, c.end_line, c.symbol, c.kind, c.text,
                    bm25({fts}) AS rank
             FROM {fts} f
             JOIN {table} c ON c.id = f.rowid
             WHERE {fts} MATCH ?1
             ORDER BY rank
             LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![match_expr, limit as i64], |r| {
            Ok(IndexedChunk {
                id: r.get(0)?,
                path: r.get(1)?,
                start_line: r.get::<_, i64>(2)? as usize,
                end_line: r.get::<_, i64>(3)? as usize,
                symbol: r.get(4)?,
                kind: r.get(5)?,
                text: r.get(6)?,
                bm25: Some(r.get(7)?),
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Loads every chunk that has an embedding, for vector search.
    pub fn embedded_chunks(&self, cwd: &Path) -> Result<Vec<(IndexedChunk, Vec<f32>)>> {
        let table = table_name(cwd, "chunks");
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!(
            "SELECT id, path, start_line, end_line, symbol, kind, text, embedding
             FROM {table} WHERE embedding IS NOT NULL"
        ))?;
        let rows = stmt.query_map([], |r| {
            let blob: Vec<u8> = r.get(7)?;
            Ok((
                IndexedChunk {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    start_line: r.get::<_, i64>(2)? as usize,
                    end_line: r.get::<_, i64>(3)? as usize,
                    symbol: r.get(4)?,
                    kind: r.get(5)?,
                    text: r.get(6)?,
                    bm25: None,
                },
                decode_vector(&blob),
            ))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

/// SHA-256 hex of a file's content (the incremental-index key).
pub fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Escapes a query for FTS5 MATCH: alphanumeric tokens, each quoted.
/// Escapes a free-text query into an FTS5 MATCH expression.
///
/// Unlike the project-memory variant (which joins tokens with an implicit AND,
/// fine for short fact lookups), code search receives natural-language queries
/// like "where is the retry backoff configured?" where requiring *every* token
/// would return nothing. Tokens are therefore OR-joined: BM25 still ranks
/// chunks matching more (and rarer) terms higher, but a single strong term is
/// enough to surface a hit.
pub(crate) fn fts_escape(query: &str) -> String {
    let tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    tokens.join(" OR ")
}

/// Chunks a file's content (thin wrapper so callers don't import `chunk`).
pub fn chunks_for(path: &Path, content: &str) -> Vec<Chunk> {
    chunk_source(path, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SemanticIndex) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        let idx = SemanticIndex::open(&db).unwrap();
        idx.ensure_project(dir.path()).unwrap();
        (dir, idx)
    }

    #[test]
    fn test_replace_and_count() {
        let (dir, idx) = store();
        let chunks = chunks_for(Path::new("a.rs"), "pub fn foo() {}\npub fn bar() {}\n");
        idx.replace_file(dir.path(), "a.rs", "h1", &chunks, &[])
            .unwrap();
        assert_eq!(idx.chunk_count(dir.path()).unwrap(), chunks.len());
        assert_eq!(idx.file_count(dir.path()).unwrap(), 1);
        assert_eq!(
            idx.file_hash(dir.path(), "a.rs").unwrap().as_deref(),
            Some("h1")
        );
    }

    #[test]
    fn test_replace_is_idempotent() {
        let (dir, idx) = store();
        let chunks = chunks_for(Path::new("a.rs"), "pub fn foo() {}\n");
        idx.replace_file(dir.path(), "a.rs", "h1", &chunks, &[])
            .unwrap();
        idx.replace_file(dir.path(), "a.rs", "h2", &chunks, &[])
            .unwrap();
        assert_eq!(idx.chunk_count(dir.path()).unwrap(), chunks.len());
        assert_eq!(
            idx.file_hash(dir.path(), "a.rs").unwrap().as_deref(),
            Some("h2")
        );
    }

    #[test]
    fn test_remove_file() {
        let (dir, idx) = store();
        let chunks = chunks_for(Path::new("a.rs"), "pub fn foo() {}\n");
        idx.replace_file(dir.path(), "a.rs", "h1", &chunks, &[])
            .unwrap();
        idx.remove_file(dir.path(), "a.rs").unwrap();
        assert_eq!(idx.chunk_count(dir.path()).unwrap(), 0);
    }

    #[test]
    fn test_bm25_search_finds_symbol() {
        let (dir, idx) = store();
        let src = "pub fn parse_config() {}\npub fn render_ui() {}\n";
        let chunks = chunks_for(Path::new("a.rs"), src);
        idx.replace_file(dir.path(), "a.rs", "h1", &chunks, &[])
            .unwrap();
        let hits = idx.search_bm25(dir.path(), "parse_config", 10).unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].text.contains("parse_config"));
    }

    #[test]
    fn test_bm25_empty_query_no_hits() {
        let (dir, idx) = store();
        let chunks = chunks_for(Path::new("a.rs"), "pub fn foo() {}\n");
        idx.replace_file(dir.path(), "a.rs", "h1", &chunks, &[])
            .unwrap();
        assert!(idx.search_bm25(dir.path(), "!!!", 10).unwrap().is_empty());
    }

    #[test]
    fn test_embedded_chunks_roundtrip() {
        let (dir, idx) = store();
        let chunks = chunks_for(Path::new("a.rs"), "pub fn foo() {}\n");
        let emb = vec![vec![1.0f32, 2.0, 3.0]; chunks.len()];
        idx.replace_file(dir.path(), "a.rs", "h1", &chunks, &emb)
            .unwrap();
        let loaded = idx.embedded_chunks(dir.path()).unwrap();
        assert_eq!(loaded.len(), chunks.len());
        assert_eq!(loaded[0].1, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_content_hash_stable_and_distinct() {
        assert_eq!(content_hash("abc"), content_hash("abc"));
        assert_ne!(content_hash("abc"), content_hash("abd"));
    }

    #[test]
    fn test_fts_escape_quotes_tokens() {
        // Tokens are OR-joined so a natural-language query matches on any term.
        assert_eq!(fts_escape("foo bar"), "\"foo\" OR \"bar\"");
        assert_eq!(fts_escape("!!!"), "");
    }
}
