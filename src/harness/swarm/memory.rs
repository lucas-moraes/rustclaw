//! Shared key/value memory between agents of the same project (SQLite-backed).
//!
//! Unlike the message bus (point-to-point, drained), shared memory is a
//! namespace: any agent can `set` a key and any other can `get` it later. It is
//! meant for findings that several subagents need — a file map, a decision, a
//! command that worked — without copying them through the parent's context.

use crate::harness::project::table::table_name;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

/// A single shared-memory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryEntry {
    pub key: String,
    pub value: String,
    /// Session that wrote the entry (last writer wins on upsert).
    pub author: String,
    pub updated_at: String,
}

/// SQLite-backed shared memory, partitioned per project root.
pub struct SharedMemory {
    conn: Mutex<Connection>,
    table: String,
}

impl SharedMemory {
    /// Opens (or creates) the shared memory for `cwd` inside `db_path`.
    pub fn open(db_path: &Path, cwd: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create data dir {}", parent.display()))?;
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("failed to open shared memory db {}", db_path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .context("failed to set journal mode")?;
        conn.execute_batch("PRAGMA busy_timeout=5000;")
            .context("failed to set busy timeout")?;
        let mem = Self {
            conn: Mutex::new(conn),
            table: table_name(cwd, "swarm_memory"),
        };
        mem.ensure_schema()?;
        Ok(mem)
    }

    fn ensure_schema(&self) -> Result<()> {
        let conn = self.conn.lock().expect("shared memory mutex poisoned");
        // `table` is hex-only (see project::table), so interpolation is safe.
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {} (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                author TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
            self.table
        ))
        .with_context(|| format!("failed to create table {}", self.table))?;
        Ok(())
    }

    /// Writes `key` (insert or overwrite). Returns `true` when a previous
    /// value was replaced.
    pub fn set(&self, key: &str, value: &str, author: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("shared memory mutex poisoned");
        let existed: bool = conn
            .query_row(
                &format!("SELECT 1 FROM {} WHERE key = ?1", self.table),
                params![key],
                |_| Ok(true),
            )
            .unwrap_or(false);
        conn.execute(
            &format!(
                "INSERT INTO {} (key, value, author, updated_at)
                 VALUES (?1, ?2, ?3, datetime('now'))
                 ON CONFLICT(key) DO UPDATE SET
                     value = excluded.value,
                     author = excluded.author,
                     updated_at = excluded.updated_at",
                self.table
            ),
            params![key, value, author],
        )
        .context("failed to write shared memory entry")?;
        Ok(existed)
    }

    /// Reads a single entry.
    pub fn get(&self, key: &str) -> Result<Option<MemoryEntry>> {
        let conn = self.conn.lock().expect("shared memory mutex poisoned");
        let mut stmt = conn
            .prepare(&format!(
                "SELECT key, value, author, updated_at FROM {} WHERE key = ?1",
                self.table
            ))
            .context("failed to prepare get query")?;
        let mut rows = stmt
            .query_map(params![key], |row| {
                Ok(MemoryEntry {
                    key: row.get(0)?,
                    value: row.get(1)?,
                    author: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })
            .context("failed to read shared memory entry")?;
        match rows.next() {
            Some(row) => Ok(Some(row.context("failed to decode entry")?)),
            None => Ok(None),
        }
    }

    /// Lists entries, most recently updated first.
    pub fn list(&self) -> Result<Vec<MemoryEntry>> {
        let conn = self.conn.lock().expect("shared memory mutex poisoned");
        let mut stmt = conn
            .prepare(&format!(
                "SELECT key, value, author, updated_at FROM {} ORDER BY updated_at DESC, key ASC",
                self.table
            ))
            .context("failed to prepare list query")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(MemoryEntry {
                    key: row.get(0)?,
                    value: row.get(1)?,
                    author: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })
            .context("failed to list shared memory")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("failed to decode entry")?);
        }
        Ok(out)
    }

    /// Deletes `key`. Returns `true` when something was removed.
    pub fn delete(&self, key: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("shared memory mutex poisoned");
        let removed = conn
            .execute(
                &format!("DELETE FROM {} WHERE key = ?1", self.table),
                params![key],
            )
            .context("failed to delete shared memory entry")?;
        Ok(removed > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(dir: &Path) -> SharedMemory {
        SharedMemory::open(&dir.join("test.db"), dir).unwrap()
    }

    #[test]
    fn set_then_get_roundtrips() {
        let d = tempfile::tempdir().unwrap();
        let m = mem(d.path());
        assert!(!m.set("map", "src/a.rs -> A", "s1").unwrap());

        let e = m.get("map").unwrap().expect("entry");
        assert_eq!(e.value, "src/a.rs -> A");
        assert_eq!(e.author, "s1");
    }

    #[test]
    fn set_overwrites_and_reports_replacement() {
        let d = tempfile::tempdir().unwrap();
        let m = mem(d.path());
        m.set("k", "v1", "s1").unwrap();
        assert!(m.set("k", "v2", "s2").unwrap());

        let e = m.get("k").unwrap().expect("entry");
        assert_eq!(e.value, "v2");
        assert_eq!(e.author, "s2");
        assert_eq!(m.list().unwrap().len(), 1, "upsert must not duplicate");
    }

    #[test]
    fn get_missing_key_is_none() {
        let d = tempfile::tempdir().unwrap();
        assert!(mem(d.path()).get("nope").unwrap().is_none());
    }

    #[test]
    fn delete_reports_whether_it_removed_anything() {
        let d = tempfile::tempdir().unwrap();
        let m = mem(d.path());
        m.set("k", "v", "s").unwrap();
        assert!(m.delete("k").unwrap());
        assert!(!m.delete("k").unwrap());
        assert!(m.get("k").unwrap().is_none());
    }

    #[test]
    fn list_returns_all_entries() {
        let d = tempfile::tempdir().unwrap();
        let m = mem(d.path());
        m.set("a", "1", "s").unwrap();
        m.set("b", "2", "s").unwrap();
        let keys: Vec<String> = m.list().unwrap().into_iter().map(|e| e.key).collect();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"a".to_string()));
        assert!(keys.contains(&"b".to_string()));
    }

    #[test]
    fn two_instances_share_the_namespace() {
        let d = tempfile::tempdir().unwrap();
        mem(d.path()).set("k", "from-a", "a").unwrap();
        let e = mem(d.path()).get("k").unwrap().expect("entry");
        assert_eq!(e.value, "from-a");
    }
}
