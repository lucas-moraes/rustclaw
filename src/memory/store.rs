use super::{MemoryEntry, MemoryType};
use anyhow::Result;
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

        let conn = Connection::open(db_path)?;
        let store = Self { conn };
        store.init_schema()?;

        tracing::info!("MemoryStore initialized at {:?}", db_path);
        Ok(store)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                embedding BLOB NOT NULL,
                timestamp TEXT NOT NULL,
                importance REAL NOT NULL DEFAULT 0.5,
                memory_type TEXT NOT NULL CHECK(memory_type IN ('fact', 'episode', 'tool_result')),
                metadata TEXT NOT NULL DEFAULT '{}',
                search_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_timestamp ON memories(timestamp);
            CREATE INDEX IF NOT EXISTS idx_importance ON memories(importance);
            CREATE INDEX IF NOT EXISTS idx_memory_type ON memories(memory_type);

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
            "#,
        )?;

        Ok(())
    }

    pub fn save(&self, entry: &MemoryEntry) -> Result<()> {
        let embedding_bytes = Self::vec_f32_to_bytes(&entry.embedding);
        let timestamp = entry.timestamp.to_rfc3339();
        let metadata = serde_json::to_string(&entry.metadata)?;

        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO memories 
            (id, content, embedding, timestamp, importance, memory_type, metadata, search_count)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                entry.id,
                entry.content,
                embedding_bytes,
                timestamp,
                entry.importance,
                entry.memory_type.to_string(),
                metadata,
                entry.search_count
            ],
        )?;

        Ok(())
    }

    pub fn get_all(&self) -> Result<Vec<MemoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content, embedding, timestamp, importance, memory_type, metadata, search_count 
             FROM memories ORDER BY timestamp DESC"
        )?;

        let entries = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let content: String = row.get(1)?;
            let embedding_bytes: Vec<u8> = row.get(2)?;
            let timestamp_str: String = row.get(3)?;
            let importance: f32 = row.get(4)?;
            let memory_type_str: String = row.get(5)?;
            let metadata_str: String = row.get(6)?;
            let search_count: i32 = row.get(7)?;

            let embedding = Self::bytes_to_vec_f32(&embedding_bytes);
            let timestamp = timestamp_str.parse().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
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

            let metadata: serde_json::Value = serde_json::from_str(&metadata_str).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                )
            })?;

            Ok(MemoryEntry {
                id,
                content,
                embedding,
                timestamp,
                importance,
                memory_type,
                metadata,
                search_count,
            })
        })?;

        entries.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    pub fn get_by_id(&self, id: &str) -> Result<Option<MemoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content, embedding, timestamp, importance, memory_type, metadata, search_count 
             FROM memories WHERE id = ?1"
        )?;

        let entry = stmt
            .query_row([id], |row| {
                let id: String = row.get(0)?;
                let content: String = row.get(1)?;
                let embedding_bytes: Vec<u8> = row.get(2)?;
                let timestamp_str: String = row.get(3)?;
                let importance: f32 = row.get(4)?;
                let memory_type_str: String = row.get(5)?;
                let metadata_str: String = row.get(6)?;
                let search_count: i32 = row.get(7)?;

                let embedding = Self::bytes_to_vec_f32(&embedding_bytes);
                let timestamp = timestamp_str.parse().map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
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

                let metadata: serde_json::Value =
                    serde_json::from_str(&metadata_str).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Text,
                            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                        )
                    })?;

                Ok(MemoryEntry {
                    id,
                    content,
                    embedding,
                    timestamp,
                    importance,
                    memory_type,
                    metadata,
                    search_count,
                })
            })
            .optional()?;

        Ok(entry)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM memories WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn increment_search_count(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE memories SET search_count = search_count + 1 WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

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

    pub fn delete_task(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM scheduled_tasks WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn toggle_task(&self, id: &str, is_active: bool) -> Result<()> {
        let active_int: i32 = if is_active { 1 } else { 0 };
        self.conn.execute(
            "UPDATE scheduled_tasks SET is_active = ?1 WHERE id = ?2",
            rusqlite::params![active_int, id],
        )?;
        Ok(())
    }

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
