//! SQLite-backed project memory store.
//!
//! Caches the structural project summary keyed by project root. Uses the same
//! DB file as sessions (`harness.db`) with a dedicated table.
//!
//! Curated facts (from the `remember` tool) live in a per-project `_facts`
//! table, one row per fact, each carrying metadata (`kind`, `confidence`,
//! `hit_count`, `last_used`, `archived`) to enable ranking, promotion and GC.

use super::profiler::{ProjectContext, StackKind};
use crate::harness::project::table::table_name;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Known fact kinds (free-form strings are allowed, these are the common ones).
#[allow(dead_code)] // documented reference values; `remember` accepts them.
pub const KIND_FACT: &str = "fact";
#[allow(dead_code)]
pub const KIND_COMMAND: &str = "command";
#[allow(dead_code)]
pub const KIND_CONVENTION: &str = "convention";
#[allow(dead_code)]
pub const KIND_PATTERN: &str = "pattern";
#[allow(dead_code)]
pub const KIND_DECISION: &str = "decision";
#[allow(dead_code)]
pub const KIND_TRAP: &str = "trap";

/// Confidence levels for a fact.
pub const CONFIDENCE_INFERRED: &str = "inferred";
pub const CONFIDENCE_CONFIRMED: &str = "confirmed";

/// A single curated fact with its metadata.
#[derive(Clone, Debug)]
pub struct MemoryFact {
    pub id: i64,
    pub text: String,
    pub kind: String,
    pub confidence: String,
    pub hit_count: i64,
    pub last_used: Option<String>,
    pub archived: bool,
}

/// A persisted row of project memory.
#[allow(dead_code)] // `remember` tool (next phase) reads/updates these.
#[derive(Clone, Debug)]
pub struct ProjectMemoryRow {
    pub project_path: String,
    pub stack: String,
    pub summary: String,
    pub archive: String,
    pub manifest_mtimes: HashMap<String, u64>,
}

/// Default budget (bytes) for curated memory injected into the system prompt.
pub const MAX_MEMORY_CHARS: usize = 2048;

/// Max bytes for the active structural `summary` before compaction rolls
/// lower-priority facts into the `archive` column.
pub const MAX_SUMMARY_CHARS: usize = 4096;

/// Weights for the relevance score.
const W_RECENCY: f64 = 0.4;
const W_HITS: f64 = 0.3;
const W_LEXICAL: f64 = 0.3;
/// Half-life (days) for recency decay.
const RECENCY_HALF_LIFE_DAYS: f64 = 30.0;

/// Computes a relevance score for a fact given the current turn text.
/// Higher is more relevant. Combines recency (decay since `last_used`),
/// usage (`hit_count`) and lexical overlap with the query.
pub fn score_fact(fact: &MemoryFact, query: &str, now: &chrono::DateTime<chrono::Utc>) -> f64 {
    // Recency: 1.0 when used now, decaying toward 0 with half-life.
    let recency = match &fact.last_used {
        Some(ts) => match chrono::DateTime::parse_from_rfc3339(ts) {
            Ok(t) => {
                let age_days = (now.signed_duration_since(t.with_timezone(&chrono::Utc)))
                    .num_seconds() as f64
                    / 86_400.0;
                if age_days <= 0.0 {
                    1.0
                } else {
                    0.5_f64.powf(age_days / RECENCY_HALF_LIFE_DAYS)
                }
            }
            Err(_) => 0.0,
        },
        None => 0.0,
    };

    // Usage: normalized hit count (capped at 10).
    let hits = (fact.hit_count as f64).min(10.0) / 10.0;

    // Lexical overlap: fraction of query tokens present in the fact text.
    let lexical = lexical_overlap(query, &fact.text);

    W_RECENCY * recency + W_HITS * hits + W_LEXICAL * lexical
}

/// Fraction of query tokens (lowercased, alphanumeric) present in `text`.
fn lexical_overlap(query: &str, text: &str) -> f64 {
    let text_lower = text.to_lowercase();
    let tokens: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty() && t.len() > 2)
        .map(|t| t.to_string())
        .collect();
    if tokens.is_empty() {
        return 0.0;
    }
    let matched = tokens
        .iter()
        .filter(|t| text_lower.contains(t.as_str()))
        .count();
    matched as f64 / tokens.len() as f64
}

/// Renders the top active facts for the system prompt, ordered by relevance
/// and bounded by `max_chars`. Only non-archived facts are considered.
pub fn render_memory(facts: &[MemoryFact], query: &str, max_chars: usize) -> String {
    let now = chrono::Utc::now();
    let mut active: Vec<&MemoryFact> = facts.iter().filter(|f| !f.archived).collect();
    active.sort_by(|a, b| {
        score_fact(b, query, &now)
            .partial_cmp(&score_fact(a, query, &now))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut out = String::new();
    let mut used = 0usize;
    for fact in active {
        let line = format!("- {}\n", fact.text);
        if used + line.len() > max_chars && !out.is_empty() {
            break;
        }
        out.push_str(&line);
        used += line.len();
    }
    out
}

/// Normalizes a fact's text for dedup: strips the `- [timestamp] ` prefix,
/// lowercases and collapses whitespace.
fn normalize_fact(text: &str) -> String {
    let stripped = strip_timestamp_prefix(text);
    stripped
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Strips a leading `- [timestamp] ` prefix from a fact line, if present.
fn strip_timestamp_prefix(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("- [") {
        if let Some(end) = rest.find("] ") {
            return rest[end + 2..].to_string();
        }
    }
    trimmed.to_string()
}

impl ProjectMemoryRow {
    #[allow(dead_code)]
    pub fn from_context(ctx: &ProjectContext) -> Self {
        Self {
            project_path: ctx.cwd.to_string_lossy().to_string(),
            stack: stack_name(ctx.stack).to_string(),
            summary: String::new(),
            archive: String::new(),
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
                archive TEXT NOT NULL DEFAULT '',
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
        // Idempotent migration: add the `archive` column to pre-existing tables.
        let has_archive: bool = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name='archive'"),
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_archive {
            conn.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN archive TEXT NOT NULL DEFAULT '';"
            ))
            .with_context(|| format!("failed to add archive column to {table}"))?;
        }
        Ok(())
    }

    /// Creates the per-project facts table (one row per curated fact) and
    /// migrates any legacy facts embedded in the `summary` column into it.
    /// Idempotent.
    pub fn ensure_facts(&self, cwd: &Path) -> Result<()> {
        self.ensure_project(cwd)?;
        let table = table_name(cwd, "facts");
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {table} (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT '{KIND_FACT}',
                confidence TEXT NOT NULL DEFAULT '{CONFIDENCE_INFERRED}',
                hit_count INTEGER NOT NULL DEFAULT 0,
                last_used TEXT,
                archived INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_{table}_archived ON {table}(archived);"
        ))
        .with_context(|| format!("failed to ensure facts table for {}", cwd.display()))?;
        drop(conn);

        // Migrate legacy facts embedded in the summary (lines starting with
        // `- [timestamp]`) into the facts table, then strip them from summary.
        self.migrate_summary_facts(cwd)
    }

    /// Moves legacy `- [timestamp] ...` lines from the `summary` column into
    /// the facts table (idempotent; no-op when there are none).
    fn migrate_summary_facts(&self, cwd: &Path) -> Result<()> {
        let Some(row) = self.load(cwd)? else {
            return Ok(());
        };
        let mut legacy: Vec<String> = Vec::new();
        let mut kept: Vec<String> = Vec::new();
        for line in row.summary.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("- [") {
                legacy.push(trimmed.to_string());
            } else if !trimmed.is_empty() {
                kept.push(trimmed.to_string());
            }
        }
        if legacy.is_empty() {
            return Ok(());
        }
        let table = table_name(cwd, "facts");
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        for fact in &legacy {
            conn.execute(
                &format!(
                    "INSERT INTO {table}
                        (text, kind, confidence, hit_count, last_used, archived, created_at, updated_at)
                     VALUES (?1, '{KIND_FACT}', '{CONFIDENCE_INFERRED}', 0, NULL, 0, ?2, ?2)"
                ),
                params![fact, now],
            )
            .context("failed to migrate legacy summary fact")?;
        }
        drop(conn);
        self.set_summary(cwd, &kept.join("\n"))
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
                    "SELECT project_path, stack, summary, archive, manifest_mtimes
                     FROM {table} WHERE project_path = ?1"
                ),
                params![key],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .context("failed to query project memory")?;

        let Some((path, stack, summary, archive, mtimes_json)) = row else {
            return Ok(None);
        };
        let manifest_mtimes: HashMap<String, u64> =
            serde_json::from_str(&mtimes_json).unwrap_or_default();
        Ok(Some(ProjectMemoryRow {
            project_path: path,
            stack,
            summary,
            archive,
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

    /// Appends a curated fact to the project's memory (used by the `remember`
    /// tool). Inserts a row into the facts table with the given metadata.
    pub fn append_fact(&self, cwd: &Path, text: &str, kind: &str, confidence: &str) -> Result<()> {
        self.ensure_facts(cwd)?;
        let table = table_name(cwd, "facts");
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO {table}
                    (text, kind, confidence, hit_count, last_used, archived, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 0, NULL, 0, ?4, ?4)"
            ),
            params![text, kind, confidence, now],
        )
        .context("failed to append project memory fact")?;
        Ok(())
    }

    /// Appends curated content to the summary (legacy path, kept for
    /// compatibility). New facts should use `append_fact`.
    #[allow(dead_code)]
    pub fn append(&self, cwd: &Path, content: &str) -> Result<()> {
        self.append_fact(cwd, content, KIND_FACT, CONFIDENCE_INFERRED)
    }

    /// Returns the non-empty fact lines of the project's memory, 1-indexed.
    /// Facts are the newline-separated lines persisted by the `remember` tool.
    #[allow(dead_code)] // used by tests and kept for API compatibility.
    pub fn list_facts(&self, cwd: &Path) -> Result<Vec<(usize, String)>> {
        Ok(self
            .list_fact_rows(cwd)?
            .into_iter()
            .enumerate()
            .map(|(i, f)| (i + 1, f.text))
            .collect())
    }

    /// Returns all curated facts (active + archived) with their metadata,
    /// ordered by id. 1-indexed position is the caller's concern.
    pub fn list_fact_rows(&self, cwd: &Path) -> Result<Vec<MemoryFact>> {
        self.ensure_facts(cwd)?;
        let table = table_name(cwd, "facts");
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT id, text, kind, confidence, hit_count, last_used, archived
                 FROM {table} ORDER BY id"
            ))
            .context("failed to prepare facts read")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(MemoryFact {
                    id: r.get(0)?,
                    text: r.get(1)?,
                    kind: r.get(2)?,
                    confidence: r.get(3)?,
                    hit_count: r.get(4)?,
                    last_used: r.get(5)?,
                    archived: r.get::<_, i64>(6)? != 0,
                })
            })
            .context("failed to query facts")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to collect facts")?;
        Ok(rows)
    }

    /// Returns the active (non-archived) facts, ordered by id.
    pub fn active_facts(&self, cwd: &Path) -> Result<Vec<MemoryFact>> {
        Ok(self
            .list_fact_rows(cwd)?
            .into_iter()
            .filter(|f| !f.archived)
            .collect())
    }

    /// Removes the fact at `index` (1-based) from the facts table.
    /// Returns `Ok(true)` if a fact was removed, `Ok(false)` when the index is
    /// out of range or the project has no persisted memory.
    pub fn delete_fact_by_index(&self, cwd: &Path, index: usize) -> Result<bool> {
        let facts = self.list_fact_rows(cwd)?;
        if index == 0 || index > facts.len() {
            return Ok(false);
        }
        let id = facts[index - 1].id;
        let table = table_name(cwd, "facts");
        let conn = self.conn.lock().unwrap();
        conn.execute(&format!("DELETE FROM {table} WHERE id = ?1"), params![id])
            .context("failed to delete project memory fact")?;
        Ok(true)
    }

    /// Empties all persisted facts for the project (idempotent).
    pub fn clear_memory(&self, cwd: &Path) -> Result<()> {
        self.ensure_facts(cwd)?;
        let table = table_name(cwd, "facts");
        let conn = self.conn.lock().unwrap();
        conn.execute(&format!("DELETE FROM {table}"), [])
            .context("failed to clear project memory facts")?;
        Ok(())
    }

    /// Increments `hit_count` and updates `last_used` for a fact (by id).
    pub fn bump_usage(&self, cwd: &Path, id: i64) -> Result<()> {
        self.ensure_facts(cwd)?;
        let table = table_name(cwd, "facts");
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "UPDATE {table} SET hit_count = hit_count + 1, last_used = ?1, updated_at = ?1
                 WHERE id = ?2"
            ),
            params![now, id],
        )
        .context("failed to bump fact usage")?;
        Ok(())
    }

    /// Sets the `archived` flag for a fact (by id).
    pub fn set_archived(&self, cwd: &Path, id: i64, archived: bool) -> Result<()> {
        self.ensure_facts(cwd)?;
        let table = table_name(cwd, "facts");
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!("UPDATE {table} SET archived = ?1, updated_at = ?2 WHERE id = ?3"),
            params![if archived { 1 } else { 0 }, now, id],
        )
        .context("failed to set fact archived")?;
        Ok(())
    }

    /// Merges duplicate facts (same normalized text): keeps the most recent
    /// active one, sums `hit_count`, archives the rest. Returns the number of
    /// facts merged (archived as duplicates).
    pub fn dedup(&self, cwd: &Path) -> Result<usize> {
        let facts = self.list_fact_rows(cwd)?;
        let mut by_norm: std::collections::HashMap<String, Vec<MemoryFact>> =
            std::collections::HashMap::new();
        for f in &facts {
            by_norm
                .entry(normalize_fact(&f.text))
                .or_default()
                .push(f.clone());
        }
        let mut merged = 0usize;
        for group in by_norm.values() {
            if group.len() < 2 {
                continue;
            }
            // Keep the most recent active fact; archive the rest.
            let mut sorted = group.clone();
            sorted.sort_by(|a, b| b.id.cmp(&a.id));
            let keep = &sorted[0];
            let total_hits: i64 = group.iter().map(|f| f.hit_count).sum();
            for dup in &sorted[1..] {
                if !dup.archived {
                    self.set_archived(cwd, dup.id, true)?;
                    merged += 1;
                }
            }
            // Sum hit counts into the kept fact.
            if total_hits > keep.hit_count {
                let table = table_name(cwd, "facts");
                let conn = self.conn.lock().unwrap();
                conn.execute(
                    &format!("UPDATE {table} SET hit_count = ?1 WHERE id = ?2"),
                    params![total_hits, keep.id],
                )
                .context("failed to sum fact hit counts")?;
            }
        }
        Ok(merged)
    }

    /// Archives facts that have never been used (`hit_count == 0`) and whose
    /// `last_used` is older than `max_age_days`. Facts with
    /// `confidence == "confirmed"` are never archived. Returns the number
    /// archived.
    pub fn archive_stale(&self, cwd: &Path, max_age_days: i64) -> Result<usize> {
        let facts = self.list_fact_rows(cwd)?;
        let now = chrono::Utc::now();
        let mut archived = 0usize;
        for f in &facts {
            if f.archived || f.confidence == CONFIDENCE_CONFIRMED || f.hit_count > 0 {
                continue;
            }
            let stale = match &f.last_used {
                Some(ts) => match chrono::DateTime::parse_from_rfc3339(ts) {
                    Ok(t) => {
                        let age_days = now
                            .signed_duration_since(t.with_timezone(&chrono::Utc))
                            .num_days();
                        age_days > max_age_days
                    }
                    Err(_) => false,
                },
                None => false,
            };
            if stale {
                self.set_archived(cwd, f.id, true)?;
                archived += 1;
            }
        }
        Ok(archived)
    }

    /// Archives lower-priority active facts when the total size of active
    /// facts exceeds `MAX_SUMMARY_CHARS`, keeping the context lean. Facts with
    /// `confidence == "confirmed"` are never archived. Returns the number of
    /// facts moved to the archive.
    pub fn compact(&self, cwd: &Path) -> Result<usize> {
        let facts = self.active_facts(cwd)?;
        let total_bytes: usize = facts.iter().map(|f| f.text.len()).sum();
        if total_bytes <= MAX_SUMMARY_CHARS {
            return Ok(0);
        }
        let now = chrono::Utc::now();
        // Sort by score ascending so we archive the least relevant first.
        let mut sorted = facts.clone();
        sorted.sort_by(|a, b| {
            score_fact(a, "", &now)
                .partial_cmp(&score_fact(b, "", &now))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut moved = 0usize;
        let mut used = total_bytes;
        for fact in &sorted {
            if used <= MAX_SUMMARY_CHARS {
                break;
            }
            if fact.confidence == CONFIDENCE_CONFIRMED {
                continue;
            }
            self.set_archived(cwd, fact.id, true)?;
            used -= fact.text.len();
            moved += 1;
        }
        Ok(moved)
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
        // Appended facts go to the facts table, not the summary.
        let facts = s.list_fact_rows(d.path()).unwrap();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].text.contains("extra fact"));
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

    #[test]
    fn test_append_fact_metadata() {
        let (d, s) = store();
        s.append_fact(d.path(), "fact one", "command", "confirmed")
            .unwrap();
        let facts = s.list_fact_rows(d.path()).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].kind, "command");
        assert_eq!(facts[0].confidence, "confirmed");
        assert_eq!(facts[0].hit_count, 0);
        assert!(!facts[0].archived);
    }

    #[test]
    fn test_bump_usage() {
        let (d, s) = store();
        s.append_fact(d.path(), "fact one", "fact", "inferred")
            .unwrap();
        let id = s.list_fact_rows(d.path()).unwrap()[0].id;
        s.bump_usage(d.path(), id).unwrap();
        s.bump_usage(d.path(), id).unwrap();
        let facts = s.list_fact_rows(d.path()).unwrap();
        assert_eq!(facts[0].hit_count, 2);
        assert!(facts[0].last_used.is_some());
    }

    #[test]
    fn test_set_archived() {
        let (d, s) = store();
        s.append_fact(d.path(), "fact one", "fact", "inferred")
            .unwrap();
        let id = s.list_fact_rows(d.path()).unwrap()[0].id;
        s.set_archived(d.path(), id, true).unwrap();
        let facts = s.list_fact_rows(d.path()).unwrap();
        assert!(facts[0].archived);
        assert!(s.active_facts(d.path()).unwrap().is_empty());
    }

    #[test]
    fn test_dedup_merges_duplicates() {
        let (d, s) = store();
        s.append_fact(
            d.path(),
            "- [2024-01-01 10:00] use cargo test",
            "fact",
            "inferred",
        )
        .unwrap();
        s.append_fact(
            d.path(),
            "- [2024-01-02 11:00] use cargo test",
            "fact",
            "inferred",
        )
        .unwrap();
        let id = s.list_fact_rows(d.path()).unwrap()[0].id;
        s.bump_usage(d.path(), id).unwrap();

        let merged = s.dedup(d.path()).unwrap();
        assert_eq!(merged, 1);
        let facts = s.list_fact_rows(d.path()).unwrap();
        // One active with summed hit_count, one archived.
        let active: Vec<_> = facts.iter().filter(|f| !f.archived).collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].hit_count, 1);
        assert_eq!(facts.iter().filter(|f| f.archived).count(), 1);
    }

    #[test]
    fn test_archive_stale_skips_confirmed() {
        let (d, s) = store();
        // Old, unused, inferred → stale.
        s.append_fact(d.path(), "old fact", "fact", "inferred")
            .unwrap();
        let old_id = s.list_fact_rows(d.path()).unwrap()[0].id;
        // Force an old last_used.
        let table = table_name(d.path(), "facts");
        {
            let conn = s.conn.lock().unwrap();
            conn.execute(
                &format!("UPDATE {table} SET last_used = ?1 WHERE id = ?2"),
                params!["2020-01-01T00:00:00+00:00", old_id],
            )
            .unwrap();
        }
        // Confirmed fact, also old → must NOT be archived.
        s.append_fact(d.path(), "confirmed fact", "fact", "confirmed")
            .unwrap();
        let conf_id = s.list_fact_rows(d.path()).unwrap()[1].id;
        {
            let conn = s.conn.lock().unwrap();
            conn.execute(
                &format!("UPDATE {table} SET last_used = ?1 WHERE id = ?2"),
                params!["2020-01-01T00:00:00+00:00", conf_id],
            )
            .unwrap();
        }

        let archived = s.archive_stale(d.path(), 60).unwrap();
        assert_eq!(archived, 1);
        let facts = s.list_fact_rows(d.path()).unwrap();
        assert!(facts[0].archived);
        assert!(!facts[1].archived);
    }

    #[test]
    fn test_compact_archives_low_priority_facts() {
        let (d, s) = store();
        // Add enough active facts to exceed the byte budget.
        for i in 0..200 {
            s.append_fact(
                d.path(),
                &format!(
                    "- [2024-01-01 10:00] fact number {} with some padding text",
                    i
                ),
                "fact",
                "inferred",
            )
            .unwrap();
        }
        let total: usize = s
            .active_facts(d.path())
            .unwrap()
            .iter()
            .map(|f| f.text.len())
            .sum();
        assert!(total > MAX_SUMMARY_CHARS);

        let moved = s.compact(d.path()).unwrap();
        assert!(moved > 0);
        let active: Vec<_> = s.active_facts(d.path()).unwrap();
        let active_bytes: usize = active.iter().map(|f| f.text.len()).sum();
        assert!(active_bytes <= MAX_SUMMARY_CHARS);
    }

    #[test]
    fn test_compact_keeps_confirmed() {
        let (d, s) = store();
        // One confirmed fact must never be archived.
        s.append_fact(
            d.path(),
            "- [2024-01-01 10:00] confirmed fact with some padding text",
            "fact",
            "confirmed",
        )
        .unwrap();
        for i in 0..200 {
            s.append_fact(
                d.path(),
                &format!(
                    "- [2024-01-01 10:00] fact number {} with some padding text",
                    i
                ),
                "fact",
                "inferred",
            )
            .unwrap();
        }

        s.compact(d.path()).unwrap();
        let active = s.active_facts(d.path()).unwrap();
        assert!(active.iter().any(|f| f.confidence == "confirmed"));
    }

    #[test]
    fn test_migrates_summary_facts_into_facts_table() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let proj = dir.path().join("proj");

        // Seed a legacy summary with `- [timestamp]` fact lines plus a plain line.
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(&format!(
                "CREATE TABLE {} (
                    project_path TEXT PRIMARY KEY, stack TEXT NOT NULL,
                    summary TEXT NOT NULL DEFAULT '', commands TEXT NOT NULL DEFAULT '{{}}',
                    manifest_mtimes TEXT NOT NULL DEFAULT '{{}}',
                    created_at TEXT NOT NULL, updated_at TEXT NOT NULL
                );",
                crate::harness::project::table::table_name(&proj, "memory")
            ))
            .unwrap();
            conn.execute(
                &format!(
                    "INSERT INTO {} (project_path, stack, summary, created_at, updated_at)
                     VALUES (?1, 'rust', ?2, '2024-01-01T00:00:00+00:00', '2024-01-01T00:00:00+00:00')",
                    crate::harness::project::table::table_name(&proj, "memory")
                ),
                params![
                    proj.to_string_lossy().to_string(),
                    "- [2024-01-01 10:00] legacy fact\n- [2024-01-02 11:00] another fact\nplain structural line"
                ],
            )
            .unwrap();
        }

        let store = ProjectMemoryStore::open(&db).unwrap();
        let facts = store.list_fact_rows(&proj).unwrap();
        assert_eq!(facts.len(), 2);
        assert!(facts[0].text.contains("legacy fact"));
        assert!(facts[1].text.contains("another fact"));
        // Plain structural line stays in the summary.
        let row = store.load(&proj).unwrap().unwrap();
        assert!(row.summary.contains("plain structural line"));
        assert!(!row.summary.contains("legacy fact"));
    }

    fn fact(id: i64, text: &str, hits: i64, last_used: Option<&str>, archived: bool) -> MemoryFact {
        MemoryFact {
            id,
            text: text.to_string(),
            kind: "fact".to_string(),
            confidence: "inferred".to_string(),
            hit_count: hits,
            last_used: last_used.map(|s| s.to_string()),
            archived,
        }
    }

    #[test]
    fn test_score_prefers_lexical_match() {
        let now = chrono::Utc::now();
        let matching = fact(1, "use cargo test for the build", 0, None, false);
        let unrelated = fact(2, "log to stderr not stdout", 0, None, false);
        let s_match = score_fact(&matching, "how do I run cargo test?", &now);
        let s_unrelated = score_fact(&unrelated, "how do I run cargo test?", &now);
        assert!(s_match > s_unrelated);
    }

    #[test]
    fn test_score_prefers_recent_and_used() {
        let now = chrono::Utc::now();
        let recent = fact(1, "use cargo test", 0, Some(&now.to_rfc3339()), false);
        let stale = fact(2, "use cargo test", 0, None, false);
        assert!(score_fact(&recent, "cargo", &now) > score_fact(&stale, "cargo", &now));

        let used = fact(3, "use cargo test", 10, None, false);
        let unused = fact(4, "use cargo test", 0, None, false);
        assert!(score_fact(&used, "cargo", &now) > score_fact(&unused, "cargo", &now));
    }

    #[test]
    fn test_render_memory_bounds_by_budget() {
        let facts = vec![
            fact(1, "fact one", 0, None, false),
            fact(2, "fact two", 0, None, false),
            fact(3, "fact three", 0, None, false),
        ];
        // Tiny budget fits only the first fact.
        let out = render_memory(&facts, "", 12);
        assert!(out.contains("fact one"));
        assert!(!out.contains("fact two"));
        assert!(!out.contains("fact three"));
    }

    #[test]
    fn test_render_memory_excludes_archived() {
        let facts = vec![
            fact(1, "fact one", 0, None, false),
            fact(2, "archived fact", 0, None, true),
        ];
        let out = render_memory(&facts, "", 1000);
        assert!(out.contains("fact one"));
        assert!(!out.contains("archived fact"));
    }

    #[test]
    fn test_render_memory_orders_by_relevance() {
        let facts = vec![
            fact(1, "unrelated thing", 0, None, false),
            fact(2, "use cargo test for build", 0, None, false),
        ];
        let out = render_memory(&facts, "how to run cargo test", 1000);
        // The matching fact should appear first.
        let pos_match = out.find("cargo test").unwrap();
        let pos_unrelated = out.find("unrelated").unwrap();
        assert!(pos_match < pos_unrelated);
    }
}
