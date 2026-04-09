use super::{MemoryEntry, MemoryScope, MemoryType};
use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct MemoryStore {
    conn: Connection,
}

impl MemoryStore {
    pub fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        if !db_path.exists() {
            std::fs::File::create(db_path)?;
        }

        let conn = Connection::open(db_path)?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        let store = Self { conn };
        store.init_schema()?;

        tracing::info!("MemoryStore initialized at {:?}", db_path);
        Ok(store)
    }

    pub fn clear_all(&self) -> Result<()> {
        let _ = self.rebuild_fts();

        let queries = [
            "DELETE FROM memories",
            "DELETE FROM scheduled_tasks",
            "DELETE FROM reminders",
            "DELETE FROM checkpoints",
            "DELETE FROM session_summaries",
            "DELETE FROM session_events",
            "DELETE FROM active_skills",
        ];

        for query in queries {
            if let Err(e) = self.conn.execute_batch(query) {
                tracing::warn!("clear_all: could not execute '{}': {}", query, e);
            }
        }

        Ok(())
    }

    fn rebuild_fts(&self) -> Result<()> {
        let _ = self.conn.execute_batch("DROP TABLE IF EXISTS memories_fts");
        let _ = self.conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                content,
                content='memories',
                content_rowid='rowid'
            )",
        );
        let _ = self.conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
            END",
        );
        let _ = self.conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.rowid, old.content);
            END",
        );
        Ok(())
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                embedding BLOB NOT NULL,
                timestamp TEXT NOT NULL,
                importance REAL NOT NULL DEFAULT 0.5,
                memory_type TEXT NOT NULL CHECK(memory_type IN ('fact', 'episode', 'tool_result')),
                metadata TEXT NOT NULL DEFAULT '{}',
                search_count INTEGER NOT NULL DEFAULT 0
            )",
        )?;

        // Create FTS5 virtual table for scalable text search
        let _ = self.conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                content,
                content='memories',
                content_rowid='rowid'
            )",
            [],
        );

        // Create triggers to keep FTS in sync
        let _ = self.conn.execute(
            "CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
            END",
            [],
        );

        let _ = self.conn.execute(
            "CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.rowid, old.content);
            END",
            [],
        );

        let _ = self.conn.execute(
            "CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.rowid, old.content);
                INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
            END",
            [],
        );

        let _ = self
            .conn
            .execute("ALTER TABLE memories ADD COLUMN session_id TEXT", []);

        let _ = self.conn.execute(
            "ALTER TABLE memories ADD COLUMN scope TEXT DEFAULT 'session'",
            [],
        );

        let _ = self.conn.execute(
            "ALTER TABLE memories ADD COLUMN access_count INTEGER DEFAULT 0",
            [],
        );

        let _ = self.conn.execute(
            "ALTER TABLE memories ADD COLUMN last_accessed TEXT DEFAULT CURRENT_TIMESTAMP",
            [],
        );

        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_timestamp ON memories(timestamp);
            CREATE INDEX IF NOT EXISTS idx_importance ON memories(importance);
            CREATE INDEX IF NOT EXISTS idx_memory_type ON memories(memory_type);
            CREATE INDEX IF NOT EXISTS idx_session_id ON memories(session_id);
            CREATE INDEX IF NOT EXISTS idx_scope ON memories(scope);

            CREATE TABLE IF NOT EXISTS scheduled_tasks (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                cron_expression TEXT NOT NULL,
                task_type TEXT NOT NULL,
                is_active INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                last_run TEXT,
                next_run TEXT,
                metadata TEXT NOT NULL DEFAULT '{}'
            );

            CREATE INDEX IF NOT EXISTS idx_task_active ON scheduled_tasks(is_active);

            CREATE TABLE IF NOT EXISTS reminders (
                id TEXT PRIMARY KEY,
                message TEXT NOT NULL,
                remind_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                is_recurring INTEGER NOT NULL DEFAULT 0,
                cron_expression TEXT,
                chat_id INTEGER NOT NULL,
                is_sent INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_reminder_time ON reminders(remind_at);
            CREATE INDEX IF NOT EXISTS idx_reminder_chat ON reminders(chat_id);
            CREATE INDEX IF NOT EXISTS idx_reminder_sent ON reminders(is_sent);
            ",
        )?;

        Ok(())
    }

    pub fn save(&self, entry: &MemoryEntry) -> Result<()> {
        let embedding_bytes = Self::vec_f32_to_bytes(&entry.embedding);
        let timestamp = entry.timestamp.to_rfc3339();
        let last_accessed = entry.last_accessed.to_rfc3339();
        let metadata = serde_json::to_string(&entry.metadata)?;

        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO memories 
            (id, session_id, content, embedding, timestamp, importance, memory_type, metadata, search_count, scope, access_count, last_accessed)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                entry.id,
                entry.session_id,
                entry.content,
                embedding_bytes,
                timestamp,
                entry.importance,
                entry.memory_type.to_string(),
                metadata,
                entry.search_count,
                entry.scope.to_string(),
                entry.access_count,
                last_accessed,
            ],
        )?;

        Ok(())
    }

    fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<MemoryEntry> {
        let id: String = row.get(0)?;
        let session_id: Option<String> = row.get(1).ok();
        let content: String = row.get(2)?;
        let embedding_bytes: Vec<u8> = row.get(3)?;
        let timestamp_str: String = row.get(4)?;
        let importance: f32 = row.get(5)?;
        let memory_type_str: String = row.get(6)?;
        let metadata_str: String = row.get(7)?;
        let search_count: i32 = row.get(8)?;
        let scope_str: String = row
            .get::<_, Option<String>>(9)?
            .unwrap_or_else(|| "session".to_string());
        let access_count: i32 = row.get::<_, Option<i32>>(10)?.unwrap_or(0);
        let last_accessed_str: String = row
            .get::<_, Option<String>>(11)?
            .unwrap_or_else(|| Utc::now().to_rfc3339());

        let embedding = Self::bytes_to_vec_f32(&embedding_bytes);
        let timestamp = timestamp_str.parse().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?;

        let last_accessed = last_accessed_str.parse().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                11,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?;

        let memory_type = match memory_type_str.as_str() {
            "fact" => MemoryType::Fact,
            "episode" => MemoryType::Episode,
            "tool_result" => MemoryType::ToolResult,
            _ => MemoryType::Episode,
        };

        let scope = MemoryScope::from(scope_str.as_str());

        let metadata: serde_json::Value = serde_json::from_str(&metadata_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?;

        Ok(MemoryEntry {
            id,
            session_id,
            content,
            embedding,
            timestamp,
            importance,
            memory_type,
            metadata,
            search_count,
            scope,
            access_count,
            last_accessed,
        })
    }

    pub fn get_all(&self) -> Result<Vec<MemoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, content, embedding, timestamp, importance, memory_type, metadata, search_count, scope, access_count, last_accessed
             FROM memories ORDER BY timestamp DESC"
        )?;

        let entries = stmt.query_map([], Self::row_to_entry)?;

        entries.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    #[allow(dead_code)]
    pub fn get_by_id(&self, id: &str) -> Result<Option<MemoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, content, embedding, timestamp, importance, memory_type, metadata, search_count, scope, access_count, last_accessed
             FROM memories WHERE id = ?1"
        )?;

        let entry = stmt.query_row([id], Self::row_to_entry).optional()?;

        Ok(entry)
    }

    #[allow(dead_code)]
    pub fn delete(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM memories WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Get all memories for a specific session
    #[allow(dead_code)]
    pub fn get_by_session_id(&self, session_id: &str) -> Result<Vec<MemoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, content, embedding, timestamp, importance, memory_type, metadata, search_count, scope, access_count, last_accessed
             FROM memories WHERE session_id = ?1 ORDER BY timestamp DESC"
        )?;

        let entries = stmt.query_map([session_id], Self::row_to_entry)?;

        entries.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    /// Delete all memories without a session_id (cleanup per user request)
    pub fn delete_all_without_session(&self) -> Result<usize> {
        let deleted = self
            .conn
            .execute("DELETE FROM memories WHERE session_id IS NULL", [])?;
        Ok(deleted)
    }

    #[allow(dead_code)]
    pub fn increment_search_count(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE memories SET search_count = search_count + 1 WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn cleanup_old_memories(&self, days: i64) -> Result<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM memories WHERE timestamp < datetime('now', ?1)",
            [format!("-{} days", days)],
        )?;
        Ok(deleted)
    }

    pub fn count(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
        Ok(count)
    }

    #[allow(dead_code)]
    pub fn get_global_memories(&self) -> Result<Vec<MemoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, content, embedding, timestamp, importance, memory_type, metadata, search_count, scope, access_count, last_accessed
             FROM memories WHERE scope = 'global' ORDER BY importance DESC, last_accessed DESC"
        )?;

        let entries = stmt.query_map([], Self::row_to_entry)?;
        entries.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    #[allow(dead_code)]
    pub fn get_project_memories(&self, project_path: &str) -> Result<Vec<MemoryEntry>> {
        let pattern = format!("%{}%", project_path);
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, content, embedding, timestamp, importance, memory_type, metadata, search_count, scope, access_count, last_accessed
             FROM memories WHERE scope = 'project' AND metadata LIKE ?1 ORDER BY importance DESC, last_accessed DESC"
        )?;

        let entries = stmt.query_map([pattern], Self::row_to_entry)?;
        entries.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    pub fn get_cross_session_memories(
        &self,
        exclude_session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let mut stmt = match exclude_session_id {
            Some(_) => self.conn.prepare(
                "SELECT id, session_id, content, embedding, timestamp, importance, memory_type, metadata, search_count, scope, access_count, last_accessed
                 FROM memories WHERE scope != 'session' OR session_id != ?1 ORDER BY importance DESC, last_accessed DESC LIMIT 50"
            )?,
            None => self.conn.prepare(
                "SELECT id, session_id, content, embedding, timestamp, importance, memory_type, metadata, search_count, scope, access_count, last_accessed
                 FROM memories WHERE scope != 'session' ORDER BY importance DESC, last_accessed DESC LIMIT 50"
            )?,
        };

        let entries = match exclude_session_id {
            Some(sid) => stmt.query_map([sid], Self::row_to_entry)?,
            None => stmt.query_map([], Self::row_to_entry)?,
        };

        entries.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    #[allow(dead_code)]
    pub fn update_memory_access(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE memories SET access_count = access_count + 1, last_accessed = ?1, importance = importance * 0.95 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn promote_to_project(&self, id: &str, project_path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE memories SET scope = 'project', metadata = json_set(COALESCE(metadata, '{}'), '$.project_path', ?1) WHERE id = ?2",
            params![project_path, id],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn promote_to_global(&self, id: &str) -> Result<()> {
        self.conn
            .execute("UPDATE memories SET scope = 'global' WHERE id = ?1", [id])?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn downgrade_low_importance(&self, threshold: f32) -> Result<usize> {
        let updated = self.conn.execute(
            "UPDATE memories SET importance = importance * 0.8 WHERE importance < ?1 AND access_count < 2",
            [threshold],
        )?;
        Ok(updated)
    }

    pub fn touch_memory(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE memories SET access_count = access_count + 1, last_accessed = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    fn vec_f32_to_bytes(vec: &[f32]) -> Vec<u8> {
        vec.iter().flat_map(|&f| f.to_le_bytes()).collect()
    }

    fn bytes_to_vec_f32(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(4)
            .map(|chunk| {
                let mut arr = [0u8; 4];
                arr.copy_from_slice(chunk);
                f32::from_le_bytes(arr)
            })
            .collect()
    }

    // Scheduler module deleted - these functions are disabled
    #[allow(dead_code)]
    pub fn get_all_tasks(&self) -> Result<Vec<()>> {
        // Returns empty vec since scheduler module was deleted
        Ok(vec![])
    }

    #[allow(dead_code)]
    pub fn delete_task(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM scheduled_tasks WHERE id = ?1", [id])?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn toggle_task(&self, id: &str, is_active: bool) -> Result<()> {
        let active_int: i32 = if is_active { 1 } else { 0 };
        self.conn.execute(
            "UPDATE scheduled_tasks SET is_active = ?1 WHERE id = ?2",
            params![active_int, id],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn count_tasks(&self) -> Result<i64> {
        let count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM scheduled_tasks", [], |row| row.get(0))?;
        Ok(count)
    }

    // Reminder methods
    pub fn save_reminder(&self, reminder: &crate::memory::reminder::Reminder) -> Result<()> {
        let remind_at = reminder.remind_at.to_rfc3339();
        let created_at = reminder.created_at.to_rfc3339();
        let is_recurring: i32 = if reminder.is_recurring { 1 } else { 0 };
        let is_sent: i32 = if reminder.is_sent { 1 } else { 0 };

        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO reminders 
            (id, message, remind_at, created_at, is_recurring, cron_expression, chat_id, is_sent)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                reminder.id,
                reminder.message,
                remind_at,
                created_at,
                is_recurring,
                reminder.cron_expression,
                reminder.chat_id,
                is_sent
            ],
        )?;

        Ok(())
    }

    pub fn get_pending_reminders(
        &self,
        chat_id: i64,
    ) -> Result<Vec<crate::memory::reminder::Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, message, remind_at, created_at, is_recurring, cron_expression, chat_id, is_sent 
             FROM reminders 
             WHERE chat_id = ?1 AND is_sent = 0
             ORDER BY remind_at ASC"
        )?;

        let reminders = stmt.query_map([chat_id], |row| {
            let id: String = row.get(0)?;
            let message: String = row.get(1)?;
            let remind_at_str: String = row.get(2)?;
            let created_at_str: String = row.get(3)?;
            let is_recurring: i32 = row.get(4)?;
            let cron_expression: Option<String> = row.get(5)?;
            let chat_id: i64 = row.get(6)?;
            let is_sent: i32 = row.get(7)?;

            let remind_at = remind_at_str.parse().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                )
            })?;

            let created_at = created_at_str.parse().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                )
            })?;

            Ok(crate::memory::reminder::Reminder {
                id,
                message,
                remind_at,
                created_at,
                is_recurring: is_recurring != 0,
                cron_expression,
                chat_id,
                is_sent: is_sent != 0,
            })
        })?;

        reminders
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.into())
    }

    pub fn get_due_reminders(
        &self,
        before: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<crate::memory::reminder::Reminder>> {
        let before_str = before.to_rfc3339();

        let mut stmt = self.conn.prepare(
            "SELECT id, message, remind_at, created_at, is_recurring, cron_expression, chat_id, is_sent 
             FROM reminders 
             WHERE remind_at <= ?1 AND is_sent = 0
             ORDER BY remind_at ASC"
        )?;

        let reminders = stmt.query_map([&before_str], |row| {
            let id: String = row.get(0)?;
            let message: String = row.get(1)?;
            let remind_at_str: String = row.get(2)?;
            let created_at_str: String = row.get(3)?;
            let is_recurring: i32 = row.get(4)?;
            let cron_expression: Option<String> = row.get(5)?;
            let chat_id: i64 = row.get(6)?;
            let is_sent: i32 = row.get(7)?;

            let remind_at = remind_at_str.parse().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                )
            })?;

            let created_at = created_at_str.parse().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                )
            })?;

            Ok(crate::memory::reminder::Reminder {
                id,
                message,
                remind_at,
                created_at,
                is_recurring: is_recurring != 0,
                cron_expression,
                chat_id,
                is_sent: is_sent != 0,
            })
        })?;

        reminders
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.into())
    }

    pub fn mark_reminder_sent(&self, id: &str) -> Result<()> {
        self.conn
            .execute("UPDATE reminders SET is_sent = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn delete_reminder(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM reminders WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn update_reminder_time(
        &self,
        id: &str,
        new_time: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let time_str = new_time.to_rfc3339();
        self.conn.execute(
            "UPDATE reminders SET remind_at = ?1, is_sent = 0 WHERE id = ?2",
            [&time_str, id],
        )?;
        Ok(())
    }

    pub fn cleanup_sent_reminders(&self) -> Result<usize> {
        let count = self
            .conn
            .execute("DELETE FROM reminders WHERE is_sent = 1", [])?;
        Ok(count)
    }
}

#[cfg(test)]
mod benchmark_tests {
    use super::*;
    use std::time::Instant;
    use tempfile::tempdir;

    fn create_test_memories(store: &MemoryStore, count: usize) -> anyhow::Result<()> {
        for i in 0..count {
            let entry = MemoryEntry {
                id: format!("bench-{}", i),
                session_id: None,
                content: format!("Test memory {} with content about testing", i),
                embedding: vec![0.0; 384],
                timestamp: chrono::Utc::now(),
                importance: 0.5,
                memory_type: MemoryType::Episode,
                metadata: serde_json::json!({}),
                search_count: 0,
                scope: MemoryScope::Session,
                access_count: 0,
                last_accessed: chrono::Utc::now(),
            };
            store.save(&entry)?;
        }
        Ok(())
    }

    #[test]
    fn test_memory_store_100_entries() {
        let dir = tempdir().unwrap();
        let store = MemoryStore::new(&dir.path().join("bench.db")).unwrap();
        create_test_memories(&store, 100).unwrap();

        let start = Instant::now();
        let all = store.get_all().unwrap();
        let elapsed = start.elapsed();

        println!("Get 100 entries: {:?}", elapsed);
        assert_eq!(all.len(), 100);
    }

    #[test]
    fn test_memory_store_1000_entries() {
        let dir = tempdir().unwrap();
        let store = MemoryStore::new(&dir.path().join("bench.db")).unwrap();
        create_test_memories(&store, 1000).unwrap();

        let start = Instant::now();
        let all = store.get_all().unwrap();
        let elapsed = start.elapsed();

        println!("Get 1000 entries: {:?}", elapsed);
        assert_eq!(all.len(), 1000);
    }

    #[test]
    fn test_linear_search_100_entries() {
        let dir = tempdir().unwrap();
        let store = MemoryStore::new(&dir.path().join("bench.db")).unwrap();
        create_test_memories(&store, 100).unwrap();

        let start = Instant::now();
        let all = store.get_all().unwrap();
        let results: Vec<_> = all
            .into_iter()
            .filter(|m| m.content.contains("testing"))
            .collect();
        let elapsed = start.elapsed();

        println!("Linear search 100 entries: {:?}", elapsed);
        println!("Found {} matches", results.len());
    }
}
