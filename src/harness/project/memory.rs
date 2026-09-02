//! SQLite-backed project memory store.
//!
//! Caches the structural project summary keyed by project root. Uses the same
//! DB file as sessions (`harness.db`) with a dedicated table.

use super::profiler::{ProjectContext, StackKind};
use crate::harness::project::table::table_name;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A persisted row of project memory.
#[allow(dead_code)] // `remember` tool (next phase) reads/updates these.
#[derive(Clone, Debug)]
pub struct ProjectMemoryRow {
    pub project_path: String,
    pub stack: String,
    pub summary: String,
    pub manifest_mtimes: HashMap<String, u64>,
}

impl ProjectMemoryRow {
    #[allow(dead_code)]
    pub fn from_context(ctx: &ProjectContext) -> Self {
        Self {
            project_path: ctx.cwd.to_string_lossy().to_string(),
            stack: stack_name(ctx.stack).to_string(),
            summary: String::new(),
            manifest_mtimes: ctx
                .source_mtimes
                .iter()
                .map(|(p, m)| (p.to_string_lossy().to_string(), *m))
                .collect(),
        }
    }
}

fn stack_name(s: StackKind) -> &'static str {
    match s {
        StackKind::Rust => "rust",
        StackKind::Node => "node",
        StackKind::Python => "python",
        StackKind::Go => "go",
        StackKind::Unknown => "unknown",
    }
}

pub struct ProjectMemoryStore {
    conn: Mutex<Connection>,
}

impl ProjectMemoryStore {
    /// Opens (and migrates) the store on the same DB file as sessions.
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create data dir {}", parent.display()))?;
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("failed to open project memory db {}", db_path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .context("failed to set journal mode")?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate_legacy()?;
        Ok(store)
    }

    /// Creates the per-project memory table for `cwd` (idempotent).
    pub fn ensure_project(&self, cwd: &Path) -> Result<()> {
        let table = table_name(cwd, "memory");
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {table} (
                project_path TEXT PRIMARY KEY,
                stack TEXT NOT NULL,
                summary TEXT NOT NULL DEFAULT '',
                commands TEXT NOT NULL DEFAULT '{{}}',
                manifest_mtimes TEXT NOT NULL DEFAULT '{{}}',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_{table}_path ON {table}(project_path);"
        ))
        .with_context(|| {
            format!(
                "failed to ensure project memory table for {}",
                cwd.display()
            )
        })?;
        Ok(())
    }

    /// Migrates rows from the legacy flat `project_memory` table into per-project
    /// tables (idempotent; no-op if the legacy table is absent).
    fn migrate_legacy(&self) -> Result<()> {
        let has_legacy: bool = {
            let conn = self.conn.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='project_memory'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
                > 0
        };
        if !has_legacy {
            return Ok(());
        }

        // Snapshot legacy rows while holding the lock, then release it.
        let rows: Vec<(String, String, String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT project_path, stack, summary, manifest_mtimes
                     FROM project_memory",
                )
                .context("failed to prepare legacy project memory read")?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .context("failed to read legacy project memory")?
                .collect::<std::result::Result<_, _>>()
                .context("failed to collect legacy project memory")?;
            rows
        };

        // Ensure per-project memory tables exist before inserting (no lock held).
        let mut projects = std::collections::BTreeSet::new();
        for (project_path, _, _, _) in &rows {
            projects.insert(PathBuf::from(project_path));
        }
        for p in &projects {
            self.ensure_project(p)?;
        }

        // Insert phase (re-lock).
        let conn = self.conn.lock().unwrap();
        for (project_path, stack, summary, mtimes_json) in rows {
            let cwd = Path::new(&project_path);
            let table = table_name(cwd, "memory");
            let now = chrono::Utc::now().to_rfc3339();
            conn.execute(
                &format!(
                    "INSERT OR IGNORE INTO {table}
                        (project_path, stack, summary, commands, manifest_mtimes, created_at, updated_at)
                     VALUES (?1, ?2, ?3, '{{}}', ?4, ?5, ?5)"
                ),
                params![project_path, stack, summary, mtimes_json, now],
            )
            .context("failed to migrate legacy project memory row")?;
        }
        conn.execute_batch("DROP TABLE IF EXISTS project_memory;")
            .context("failed to drop legacy project memory table")?;
        Ok(())
    }

    pub fn load(&self, cwd: &Path) -> Result<Option<ProjectMemoryRow>> {
        self.ensure_project(cwd)?;
        let table = table_name(cwd, "memory");
        let key = cwd.to_string_lossy().to_string();
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                &format!(
                    "SELECT project_path, stack, summary, manifest_mtimes
                     FROM {table} WHERE project_path = ?1"
                ),
                params![key],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .context("failed to query project memory")?;

        let Some((path, stack, summary, mtimes_json)) = row else {
            return Ok(None);
        };
        let manifest_mtimes: HashMap<String, u64> =
            serde_json::from_str(&mtimes_json).unwrap_or_default();
        Ok(Some(ProjectMemoryRow {
            project_path: path,
            stack,
            summary,
            manifest_mtimes,
        }))
    }

    /// Upserts a structural summary row (recomputed without LLM).
    pub fn upsert_summary(&self, ctx: &ProjectContext, summary: &str) -> Result<()> {
        self.ensure_project(&ctx.cwd)?;
        let table = table_name(&ctx.cwd, "memory");
        let key = ctx.cwd.to_string_lossy().to_string();
        let mtimes_json = serde_json::to_string(&ctx.source_mtimes)?;
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO {table}
                    (project_path, stack, summary, commands, manifest_mtimes, created_at, updated_at)
                VALUES (?1, ?2, ?3, '{{}}', ?4, ?5, ?5)
                ON CONFLICT(project_path) DO UPDATE SET
                    stack = excluded.stack,
                    summary = excluded.summary,
                    manifest_mtimes = excluded.manifest_mtimes,
                    updated_at = excluded.updated_at"
            ),
            params![key, stack_name(ctx.stack), summary, mtimes_json, now],
        )
        .context("failed to upsert project memory")?;
        Ok(())
    }

    /// Appends curated content to the summary (used by the `remember` tool).
    #[allow(dead_code)]
    pub fn append(&self, cwd: &Path, content: &str) -> Result<()> {
        self.ensure_project(cwd)?;
        let table = table_name(cwd, "memory");
        let key = cwd.to_string_lossy().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO {table}
                    (project_path, stack, summary, commands, manifest_mtimes, created_at, updated_at)
                VALUES (?1, 'unknown', ?2, '{{}}', '{{}}', ?3, ?3)
                ON CONFLICT(project_path) DO UPDATE SET
                    summary = summary || char(10) || excluded.summary,
                    updated_at = excluded.updated_at"
            ),
            params![key, content, now],
        )
        .context("failed to append project memory")?;
        Ok(())
    }

    /// Returns the non-empty fact lines of the project's memory, 1-indexed.
    /// Facts are the newline-separated lines persisted by the `remember` tool.
    pub fn list_facts(&self, cwd: &Path) -> Result<Vec<(usize, String)>> {
        let Some(row) = self.load(cwd)? else {
            return Ok(Vec::new());
        };
        Ok(row
            .summary
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .enumerate()
            .map(|(i, l)| (i + 1, l.to_string()))
            .collect())
    }

    /// Removes the fact at `index` (1-based), preserving the remaining lines.
    /// Returns `Ok(true)` if a fact was removed, `Ok(false)` when the index is
    /// out of range or the project has no persisted memory.
    pub fn delete_fact_by_index(&self, cwd: &Path, index: usize) -> Result<bool> {
        let Some(row) = self.load(cwd)? else {
            return Ok(false);
        };
        let facts: Vec<&str> = row
            .summary
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if index == 0 || index > facts.len() {
            return Ok(false);
        }
        let remaining: Vec<&str> = facts
            .iter()
            .enumerate()
            .filter(|(i, _)| *i + 1 != index)
            .map(|(_, l)| *l)
            .collect();
        self.set_summary(cwd, &remaining.join("\n"))?;
        Ok(true)
    }

    /// Empties all persisted facts for the project (idempotent).
    pub fn clear_memory(&self, cwd: &Path) -> Result<()> {
        self.set_summary(cwd, "")
    }

    /// Overwrites the project's `summary` column (used by memory management).
    fn set_summary(&self, cwd: &Path, summary: &str) -> Result<()> {
        self.ensure_project(cwd)?;
        let table = table_name(cwd, "memory");
        let key = cwd.to_string_lossy().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!("UPDATE {table} SET summary = ?1, updated_at = ?2 WHERE project_path = ?3"),
            params![summary, now, key],
        )
        .context("failed to update project memory")?;
        Ok(())
    }

    /// True if any current manifest mtime differs from the persisted snapshot.
    pub fn needs_regen(&self, cwd: &Path, current: &ProjectContext) -> Result<bool> {
        let Some(row) = self.load(cwd)? else {
            return Ok(true);
        };
        for (p, mtime) in &current.source_mtimes {
            let key = p.to_string_lossy().to_string();
            if row.manifest_mtimes.get(&key) != Some(mtime) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, ProjectMemoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = ProjectMemoryStore::open(&dir.path().join("test.db")).unwrap();
        (dir, s)
    }

    fn ctx(cwd: &Path) -> ProjectContext {
        let mut c = ProjectContext {
            cwd: cwd.to_path_buf(),
            stack: StackKind::Rust,
            ..Default::default()
        };
        c.source_mtimes.insert(cwd.join("Cargo.toml"), 1000);
        c
    }

    #[test]
    fn test_upsert_and_load() {
        let (d, s) = store();
        let c = ctx(d.path());
        s.upsert_summary(&c, "# Project context\n- Stack: rust")
            .unwrap();
        let row = s.load(d.path()).unwrap().unwrap();
        assert_eq!(row.stack, "rust");
        assert!(row.summary.contains("Stack: rust"));
    }

    #[test]
    fn test_load_missing_returns_none() {
        let (d, s) = store();
        assert!(s.load(d.path()).unwrap().is_none());
    }

    #[test]
    fn test_append() {
        let (d, s) = store();
        let c = ctx(d.path());
        s.upsert_summary(&c, "base").unwrap();
        s.append(d.path(), "extra fact").unwrap();
        let row = s.load(d.path()).unwrap().unwrap();
        assert!(row.summary.contains("base"));
        assert!(row.summary.contains("extra fact"));
    }

    #[test]
    fn test_needs_regen() {
        let (d, s) = store();
        let c = ctx(d.path());
        s.upsert_summary(&c, "s").unwrap();
        assert!(!s.needs_regen(d.path(), &c).unwrap());

        let mut changed = ctx(d.path());
        changed
            .source_mtimes
            .insert(d.path().join("Cargo.toml"), 9999);
        assert!(s.needs_regen(d.path(), &changed).unwrap());
    }

    #[test]
    fn test_migrates_legacy_flat_table() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let proj = dir.path().join("proj");

        // Legacy flat project_memory table.
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE project_memory (
                    project_path TEXT PRIMARY KEY, stack TEXT NOT NULL,
                    summary TEXT NOT NULL DEFAULT '', commands TEXT NOT NULL DEFAULT '{}',
                    manifest_mtimes TEXT NOT NULL DEFAULT '{}',
                    created_at TEXT NOT NULL, updated_at TEXT NOT NULL
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO project_memory
                    (project_path, stack, summary, created_at, updated_at)
                 VALUES (?1, 'rust', 'legacy memory line', '2024-01-01T00:00:00+00:00', '2024-01-01T00:00:00+00:00')",
                params![proj.to_string_lossy().to_string()],
            )
            .unwrap();
        }

        let store = ProjectMemoryStore::open(&db).unwrap();
        let row = store.load(&proj).unwrap().unwrap();
        assert_eq!(row.stack, "rust");
        assert!(row.summary.contains("legacy memory line"));

        let conn = rusqlite::Connection::open(&db).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='project_memory'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_list_facts_empty() {
        let (d, s) = store();
        assert!(s.list_facts(d.path()).unwrap().is_empty());
    }

    #[test]
    fn test_list_facts_numbered() {
        let (d, s) = store();
        s.append(d.path(), "fact one").unwrap();
        s.append(d.path(), "fact two").unwrap();
        let facts = s.list_facts(d.path()).unwrap();
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].0, 1);
        assert!(facts[0].1.contains("fact one"));
        assert_eq!(facts[1].0, 2);
        assert!(facts[1].1.contains("fact two"));
    }

    #[test]
    fn test_delete_fact_by_index() {
        let (d, s) = store();
        s.append(d.path(), "fact one").unwrap();
        s.append(d.path(), "fact two").unwrap();
        s.append(d.path(), "fact three").unwrap();

        assert!(s.delete_fact_by_index(d.path(), 2).unwrap());
        let facts = s.list_facts(d.path()).unwrap();
        assert_eq!(facts.len(), 2);
        assert!(facts[0].1.contains("fact one"));
        assert!(facts[1].1.contains("fact three"));
    }

    #[test]
    fn test_delete_fact_out_of_range_returns_false() {
        let (d, s) = store();
        s.append(d.path(), "fact one").unwrap();
        assert!(!s.delete_fact_by_index(d.path(), 0).unwrap());
        assert!(!s.delete_fact_by_index(d.path(), 5).unwrap());
        assert_eq!(s.list_facts(d.path()).unwrap().len(), 1);
    }

    #[test]
    fn test_clear_memory() {
        let (d, s) = store();
        s.append(d.path(), "fact one").unwrap();
        s.append(d.path(), "fact two").unwrap();
        s.clear_memory(d.path()).unwrap();
        assert!(s.list_facts(d.path()).unwrap().is_empty());
        // Idempotent: clearing again succeeds.
        s.clear_memory(d.path()).unwrap();
    }
}
