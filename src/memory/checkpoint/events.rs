#![allow(dead_code)]

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

use crate::memory::checkpoint::types::{DevelopmentState, PlanPhase};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEventType {
    ToolExecuted,
    PhaseChanged,
    StateChanged,
    FileModified,
    MessageAdded,
    BranchCreated,
    CheckpointCreated,
    ErrorOccurred,
    RetryAttempt,
}

impl std::fmt::Display for SessionEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionEventType::ToolExecuted => write!(f, "tool_executed"),
            SessionEventType::PhaseChanged => write!(f, "phase_changed"),
            SessionEventType::StateChanged => write!(f, "state_changed"),
            SessionEventType::FileModified => write!(f, "file_modified"),
            SessionEventType::MessageAdded => write!(f, "message_added"),
            SessionEventType::BranchCreated => write!(f, "branch_created"),
            SessionEventType::CheckpointCreated => write!(f, "checkpoint_created"),
            SessionEventType::ErrorOccurred => write!(f, "error_occurred"),
            SessionEventType::RetryAttempt => write!(f, "retry_attempt"),
        }
    }
}

impl From<&str> for SessionEventType {
    fn from(s: &str) -> Self {
        match s {
            "tool_executed" => SessionEventType::ToolExecuted,
            "phase_changed" => SessionEventType::PhaseChanged,
            "state_changed" => SessionEventType::StateChanged,
            "file_modified" => SessionEventType::FileModified,
            "message_added" => SessionEventType::MessageAdded,
            "branch_created" => SessionEventType::BranchCreated,
            "checkpoint_created" => SessionEventType::CheckpointCreated,
            "error_occurred" => SessionEventType::ErrorOccurred,
            "retry_attempt" => SessionEventType::RetryAttempt,
            _ => SessionEventType::MessageAdded,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub id: String,
    pub session_id: String,
    pub event_type: SessionEventType,
    pub event_data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl SessionEvent {
    pub fn new(
        session_id: String,
        event_type: SessionEventType,
        event_data: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            session_id,
            event_type,
            event_data,
            created_at: Utc::now(),
        }
    }

    pub fn tool_executed(
        session_id: String,
        tool_name: String,
        input: String,
        output: String,
        iteration: usize,
    ) -> Self {
        Self::new(
            session_id,
            SessionEventType::ToolExecuted,
            serde_json::json!({
                "tool_name": tool_name,
                "input": input,
                "output": output,
                "iteration": iteration,
            }),
        )
    }

    pub fn phase_changed(
        session_id: String,
        from: PlanPhase,
        to: PlanPhase,
        reason: String,
    ) -> Self {
        Self::new(
            session_id,
            SessionEventType::PhaseChanged,
            serde_json::json!({
                "from": from.to_string(),
                "to": to.to_string(),
                "reason": reason,
            }),
        )
    }

    pub fn state_changed(session_id: String, from: DevelopmentState, to: DevelopmentState) -> Self {
        Self::new(
            session_id,
            SessionEventType::StateChanged,
            serde_json::json!({
                "from": from.to_string(),
                "to": to.to_string(),
            }),
        )
    }

    pub fn file_modified(session_id: String, path: String, change_type: String) -> Self {
        Self::new(
            session_id,
            SessionEventType::FileModified,
            serde_json::json!({
                "path": path,
                "change_type": change_type,
            }),
        )
    }

    pub fn message_added(session_id: String, role: String, content_length: usize) -> Self {
        Self::new(
            session_id,
            SessionEventType::MessageAdded,
            serde_json::json!({
                "role": role,
                "content_length": content_length,
            }),
        )
    }

    pub fn error_occurred(session_id: String, error: String, context: String) -> Self {
        Self::new(
            session_id,
            SessionEventType::ErrorOccurred,
            serde_json::json!({
                "error": error,
                "context": context,
            }),
        )
    }

    pub fn retry_attempt(session_id: String, attempt_number: usize, error: String) -> Self {
        Self::new(
            session_id,
            SessionEventType::RetryAttempt,
            serde_json::json!({
                "attempt_number": attempt_number,
                "error": error,
            }),
        )
    }

    pub fn checkpoint_created(session_id: String, checkpoint_id: String) -> Self {
        Self::new(
            session_id,
            SessionEventType::CheckpointCreated,
            serde_json::json!({
                "checkpoint_id": checkpoint_id,
            }),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSummary {
    pub event_type: SessionEventType,
    pub count: usize,
    pub last_occurrence: DateTime<Utc>,
}

pub struct SessionEventStore {
    conn: Connection,
}

impl SessionEventStore {
    pub fn new(db_path: &Path) -> SqliteResult<Self> {
        let conn = Connection::open(db_path)?;
        let store = Self { conn };
        store.init_table()?;
        Ok(store)
    }

    fn init_table(&self) -> SqliteResult<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS session_events (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                event_data BLOB NOT NULL,
                created_at TEXT NOT NULL,
                compressed INTEGER DEFAULT 0
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_session_id ON session_events(session_id)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_type ON session_events(event_type)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_created ON session_events(created_at)",
            [],
        )?;

        Ok(())
    }

    pub fn add_event(&self, event: &SessionEvent) -> SqliteResult<()> {
        let compressed = self.compress_event_data(&event.event_data);
        self.conn.execute(
            "INSERT INTO session_events (id, session_id, event_type, event_data, created_at, compressed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.id,
                event.session_id,
                event.event_type.to_string(),
                compressed,
                event.created_at.to_rfc3339(),
                0,
            ],
        )?;
        Ok(())
    }

    pub fn get_session_events(&self, session_id: &str) -> SqliteResult<Vec<SessionEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, event_type, event_data, created_at FROM session_events
             WHERE session_id = ?1 ORDER BY created_at ASC",
        )?;

        let events = stmt
            .query_map([session_id], |row| {
                let event_data_str: String = row.get(3)?;
                let event_type_str: String = row.get(2)?;
                let created_at_str: String = row.get(4)?;

                Ok(SessionEvent {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    event_type: SessionEventType::from(event_type_str.as_str()),
                    event_data: serde_json::from_str(&event_data_str)
                        .unwrap_or(serde_json::Value::Null),
                    created_at: DateTime::parse_from_rfc3339(&created_at_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .collect::<SqliteResult<Vec<_>>>()?;

        Ok(events)
    }

    pub fn get_events_by_type(
        &self,
        session_id: &str,
        event_type: SessionEventType,
    ) -> SqliteResult<Vec<SessionEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, event_type, event_data, created_at FROM session_events
             WHERE session_id = ?1 AND event_type = ?2 ORDER BY created_at ASC",
        )?;

        let events = stmt
            .query_map(params![session_id, event_type.to_string()], |row| {
                let event_data_str: String = row.get(3)?;
                let event_type_str: String = row.get(2)?;
                let created_at_str: String = row.get(4)?;

                Ok(SessionEvent {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    event_type: SessionEventType::from(event_type_str.as_str()),
                    event_data: serde_json::from_str(&event_data_str)
                        .unwrap_or(serde_json::Value::Null),
                    created_at: DateTime::parse_from_rfc3339(&created_at_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .collect::<SqliteResult<Vec<_>>>()?;

        Ok(events)
    }

    pub fn get_event_summaries(&self, session_id: &str) -> SqliteResult<Vec<EventSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_type, COUNT(*) as count, MAX(created_at) as last_occurrence
             FROM session_events WHERE session_id = ?1 GROUP BY event_type",
        )?;

        let summaries = stmt
            .query_map([session_id], |row| {
                let event_type_str: String = row.get(0)?;
                let count: usize = row.get(1)?;
                let last_occurrence_str: String = row.get(2)?;

                Ok(EventSummary {
                    event_type: SessionEventType::from(event_type_str.as_str()),
                    count,
                    last_occurrence: DateTime::parse_from_rfc3339(&last_occurrence_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .collect::<SqliteResult<Vec<_>>>()?;

        Ok(summaries)
    }

    pub fn count_events(&self, session_id: &str) -> SqliteResult<usize> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM session_events WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(count as usize)
    }

    pub fn delete_session_events(&self, session_id: &str) -> SqliteResult<()> {
        self.conn.execute(
            "DELETE FROM session_events WHERE session_id = ?1",
            [session_id],
        )?;
        Ok(())
    }

    pub fn get_recent_events(&self, limit: usize) -> SqliteResult<Vec<SessionEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, event_type, event_data, created_at FROM session_events
             ORDER BY created_at DESC LIMIT ?1",
        )?;

        let events = stmt
            .query_map([limit], |row| {
                let event_data_str: String = row.get(3)?;
                let event_type_str: String = row.get(2)?;
                let created_at_str: String = row.get(4)?;

                Ok(SessionEvent {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    event_type: SessionEventType::from(event_type_str.as_str()),
                    event_data: serde_json::from_str(&event_data_str)
                        .unwrap_or(serde_json::Value::Null),
                    created_at: DateTime::parse_from_rfc3339(&created_at_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .collect::<SqliteResult<Vec<_>>>()?;

        Ok(events)
    }

    fn compress_event_data(&self, data: &serde_json::Value) -> String {
        serde_json::to_string(data).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn get_tool_execution_count(&self, session_id: &str) -> SqliteResult<usize> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM session_events WHERE session_id = ?1 AND event_type = 'tool_executed'",
                [session_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(count as usize)
    }

    pub fn get_error_count(&self, session_id: &str) -> SqliteResult<usize> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM session_events WHERE session_id = ?1 AND event_type = 'error_occurred'",
                [session_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(count as usize)
    }

    pub fn has_events(&self, session_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM session_events WHERE session_id = ?1 LIMIT 1",
                [session_id],
                |_row| Ok(()),
            )
            .is_ok()
    }

    pub fn vacuum(&self) -> SqliteResult<()> {
        self.conn.execute("VACUUM", [])?;
        Ok(())
    }

    pub fn compress_old_events(&self, days_old: u32) -> SqliteResult<usize> {
        let cutoff = Utc::now() - chrono::Duration::days(days_old as i64);
        let cutoff_str = cutoff.to_rfc3339();

        let affected = self.conn.execute(
            "UPDATE session_events SET compressed = 1, event_data = '{}'
             WHERE compressed = 0 AND created_at < ?1",
            [cutoff_str],
        )?;

        Ok(affected)
    }

    pub fn get_compressed_count(&self, session_id: &str) -> SqliteResult<usize> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM session_events WHERE session_id = ?1 AND compressed = 1",
                [session_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(count as usize)
    }

    pub fn is_event_compressed(&self, event_id: &str) -> SqliteResult<bool> {
        let compressed: i32 = self
            .conn
            .query_row(
                "SELECT compressed FROM session_events WHERE id = ?1",
                [event_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(compressed != 0)
    }

    pub fn restore_compressed_event(&self, event_id: &str) -> SqliteResult<bool> {
        if self.is_event_compressed(event_id)? {
            self.conn.execute(
                "UPDATE session_events SET compressed = 0, event_data = '{\"restored\": true}'
                 WHERE id = ?1",
                [event_id],
            )?;
            return Ok(true);
        }

        Ok(false)
    }
}
