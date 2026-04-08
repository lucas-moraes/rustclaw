use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SessionType {
    Project,
    Subtask,
    Research,
    Chat,
}

impl std::fmt::Display for SessionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionType::Project => write!(f, "project"),
            SessionType::Subtask => write!(f, "subtask"),
            SessionType::Research => write!(f, "research"),
            SessionType::Chat => write!(f, "chat"),
        }
    }
}

impl From<&str> for SessionType {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "project" => SessionType::Project,
            "subtask" => SessionType::Subtask,
            "research" => SessionType::Research,
            "chat" => SessionType::Chat,
            _ => SessionType::Chat,
        }
    }
}

impl Default for SessionType {
    fn default() -> Self {
        SessionType::Chat
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub title: String,       // Título da conversa
    pub summary: String,     // Resumo da conversa (gerado por LLM)
    pub phase: String,       // Current phase
    pub state: String,       // Current state
    pub project_dir: String, // Project directory if any
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub first_input: String,       // Primeira mensagem do usuário
    pub last_input: String,        // Última mensagem do usuário
    pub message_count: usize,      // Total de mensagens trocadas
    pub topics: Vec<String>,       // Tópicos discutidos
    pub parent_id: Option<String>, // Parent session for hierarchy
    pub session_type: SessionType, // Type of session
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    pub current: SessionSummary,
    pub ancestors: Vec<SessionSummary>,
    pub children: Vec<SessionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecution {
    pub tool_name: String,
    pub input: String,
    pub output: String,
    pub iteration: usize,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStage {
    pub id: usize,
    pub name: String,
    pub description: String,
    pub validation: Option<String>,
}

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
                event_data TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
            [],
        )?;

        let _ = self.conn.execute(
            "ALTER TABLE session_events ADD COLUMN compressed INTEGER DEFAULT 0",
            [],
        );

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_events_session ON session_events(session_id)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_events_type ON session_events(event_type)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_events_created ON session_events(created_at)",
            [],
        )?;

        Ok(())
    }

    pub fn append_event(&self, event: &SessionEvent) -> SqliteResult<()> {
        let event_data_str = serde_json::to_string(&event.event_data).unwrap_or_default();

        self.conn.execute(
            "INSERT INTO session_events (id, session_id, event_type, event_data, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                event.id,
                event.session_id,
                event.event_type.to_string(),
                event_data_str,
                event.created_at.to_rfc3339(),
            ],
        )?;

        Ok(())
    }

    pub fn get_events(
        &self,
        session_id: &str,
        from_ts: Option<DateTime<Utc>>,
        to_ts: Option<DateTime<Utc>>,
    ) -> SqliteResult<Vec<SessionEvent>> {
        let query = "SELECT id, session_id, event_type, event_data, created_at FROM session_events WHERE session_id = ?1";
        let (query_with_filter, param_count) = match (from_ts, to_ts) {
            (Some(_), Some(_)) => (
                format!(
                    "{} AND created_at >= ?2 AND created_at <= ?3 ORDER BY created_at ASC",
                    query
                ),
                3,
            ),
            (Some(_), None) => (
                format!("{} AND created_at >= ?2 ORDER BY created_at ASC", query),
                2,
            ),
            (None, Some(_)) => (
                format!("{} AND created_at <= ?2 ORDER BY created_at ASC", query),
                2,
            ),
            (None, None) => (format!("{} ORDER BY created_at ASC", query), 1),
        };

        let mut stmt = self.conn.prepare(&query_with_filter)?;

        let rows = match (from_ts, to_ts) {
            (Some(from), Some(to)) => stmt
                .query_map(
                    params![session_id, from.to_rfc3339(), to.to_rfc3339()],
                    |row| self.row_to_event(row),
                )?
                .collect::<Result<Vec<_>, _>>()?,
            (Some(from), None) => stmt
                .query_map(params![session_id, from.to_rfc3339()], |row| {
                    self.row_to_event(row)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, Some(to)) => stmt
                .query_map(params![session_id, to.to_rfc3339()], |row| {
                    self.row_to_event(row)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, None) => stmt
                .query_map([session_id], |row| self.row_to_event(row))?
                .collect::<Result<Vec<_>, _>>()?,
        };

        Ok(rows)
    }

    fn row_to_event(&self, row: &rusqlite::Row) -> SqliteResult<SessionEvent> {
        let id: String = row.get(0)?;
        let session_id: String = row.get(1)?;
        let event_type_str: String = row.get(2)?;
        let event_data_str: String = row.get(3)?;
        let created_at_str: String = row.get(4)?;

        let event_data: serde_json::Value =
            serde_json::from_str(&event_data_str).unwrap_or(serde_json::Value::Null);

        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok(SessionEvent {
            id,
            session_id,
            event_type: SessionEventType::from(event_type_str.as_str()),
            event_data,
            created_at,
        })
    }

    pub fn get_session_timeline(
        &self,
        session_id: &str,
    ) -> SqliteResult<Vec<(DateTime<Utc>, SessionEventType, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT created_at, event_type, event_data FROM session_events 
             WHERE session_id = ?1 ORDER BY created_at ASC",
        )?;

        let rows = stmt.query_map([session_id], |row| {
            let created_at_str: String = row.get(0)?;
            let event_type_str: String = row.get(1)?;
            let event_data_str: String = row.get(2)?;

            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let summary = if event_type_str == "tool_executed" {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&event_data_str) {
                    data.get("tool_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string()
                } else {
                    event_type_str.clone()
                }
            } else if event_type_str == "error_occurred" {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&event_data_str) {
                    data.get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error")
                        .to_string()
                } else {
                    event_type_str.clone()
                }
            } else {
                event_type_str.clone()
            };

            Ok((
                created_at,
                SessionEventType::from(event_type_str.as_str()),
                summary,
            ))
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    pub fn get_event_summary(&self, session_id: &str) -> SqliteResult<Vec<EventSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_type, COUNT(*) as count, MAX(created_at) as last_occurrence 
             FROM session_events WHERE session_id = ?1 GROUP BY event_type ORDER BY count DESC",
        )?;

        let rows = stmt.query_map([session_id], |row| {
            let event_type_str: String = row.get(0)?;
            let count: usize = row.get(1)?;
            let last_occurrence_str: String = row.get(2)?;

            let last_occurrence = DateTime::parse_from_rfc3339(&last_occurrence_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            Ok(EventSummary {
                event_type: SessionEventType::from(event_type_str.as_str()),
                count,
                last_occurrence,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    pub fn count_events(&self, session_id: &str) -> SqliteResult<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM session_events WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn delete_events_for_session(&self, session_id: &str) -> SqliteResult<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM session_events WHERE session_id = ?1",
            [session_id],
        )?;
        Ok(deleted)
    }

    pub fn compress_event_data(data: &str) -> String {
        const COMPRESSION_THRESHOLD: usize = 500;

        if data.len() < COMPRESSION_THRESHOLD {
            return data.to_string();
        }

        use std::io::Read;

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());

        if std::io::Write::write_all(&mut encoder, data.as_bytes()).is_ok() {
            let compressed = encoder.finish().unwrap_or_default();
            if compressed.len() < data.len() {
                return base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &compressed,
                );
            }
        }

        data.to_string()
    }

    pub fn decompress_event_data(data: &str, is_compressed: bool) -> String {
        if !is_compressed {
            return data.to_string();
        }

        use std::io::Read;

        let compressed =
            match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data) {
                Ok(c) => c,
                Err(_) => return data.to_string(),
            };

        let mut decoder = flate2::read::GzDecoder::new(&compressed[..]);
        let mut decompressed = String::new();

        if std::io::Read::read_to_string(&mut decoder, &mut decompressed).is_ok() {
            decompressed
        } else {
            data.to_string()
        }
    }

    pub fn should_compress(data: &str) -> bool {
        data.len() > 500
    }

    pub fn compress_stored_event(&self, event_id: &str) -> SqliteResult<bool> {
        let result = self.conn.query_row(
            "SELECT event_data, LENGTH(event_data) FROM session_events WHERE id = ?1 AND compressed = 0",
            [event_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        );

        let (data, current_len) = match result.optional() {
            Ok(Some(pair)) => pair,
            Ok(None) => return Ok(false),
            Err(e) => return Err(e),
        };

        if current_len < 500 {
            return Ok(false);
        }

        let compressed = Self::compress_event_data(&data);

        if compressed.len() < data.len() {
            self.conn.execute(
                "UPDATE session_events SET event_data = ?1, compressed = 1 WHERE id = ?2",
                params![compressed, event_id],
            )?;
            return Ok(true);
        }

        Ok(false)
    }
}

pub struct DevelopmentCheckpoint {
    pub id: String,
    pub user_input: String,
    pub session_name: Option<String>, // Human-readable session title
    pub current_iteration: usize,
    pub messages_json: String,
    pub completed_tools_json: String,
    pub plan_text: String,
    pub project_dir: String,
    pub plan_file: String,
    pub active_skill: Option<String>, // Skill ativa durante o desenvolvimento
    pub phase: PlanPhase,
    pub state: DevelopmentState,
    pub current_step: usize,
    pub completed_steps: Vec<usize>,
    pub retry_count: usize,         // NEW: número de tentativas no step atual
    pub last_error: Option<String>, // NEW: último erro encontrado
    pub auto_loop_enabled: bool,    // NEW: se está em modo auto loop
    pub parent_id: Option<String>,  // Parent session for hierarchy
    pub session_type: Option<SessionType>, // Type of session
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PlanPhase {
    AwaitingDir,
    AwaitingIdea,
    AwaitingPlanEdit,
    AwaitingApproval,
    Executing,
    Completed,
}

impl std::fmt::Display for PlanPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanPhase::AwaitingDir => write!(f, "awaiting_dir"),
            PlanPhase::AwaitingIdea => write!(f, "awaiting_idea"),
            PlanPhase::AwaitingPlanEdit => write!(f, "awaiting_plan_edit"),
            PlanPhase::AwaitingApproval => write!(f, "awaiting_approval"),
            PlanPhase::Executing => write!(f, "executing"),
            PlanPhase::Completed => write!(f, "completed"),
        }
    }
}

impl From<&str> for PlanPhase {
    fn from(s: &str) -> Self {
        match s {
            "awaiting_dir" => PlanPhase::AwaitingDir,
            "awaiting_idea" => PlanPhase::AwaitingIdea,
            "awaiting_plan_edit" => PlanPhase::AwaitingPlanEdit,
            "awaiting_approval" => PlanPhase::AwaitingApproval,
            "completed" => PlanPhase::Completed,
            _ => PlanPhase::AwaitingDir,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DevelopmentState {
    InProgress,
    Completed,
    Failed,
    Interrupted,
}

impl std::fmt::Display for DevelopmentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DevelopmentState::InProgress => write!(f, "in_progress"),
            DevelopmentState::Completed => write!(f, "completed"),
            DevelopmentState::Failed => write!(f, "failed"),
            DevelopmentState::Interrupted => write!(f, "interrupted"),
        }
    }
}

impl From<&str> for DevelopmentState {
    fn from(s: &str) -> Self {
        match s {
            "in_progress" => DevelopmentState::InProgress,
            "completed" => DevelopmentState::Completed,
            "failed" => DevelopmentState::Failed,
            "interrupted" => DevelopmentState::Interrupted,
            _ => DevelopmentState::Interrupted,
        }
    }
}

pub struct CheckpointStore {
    conn: Connection,
}

impl CheckpointStore {
    pub fn new(db_path: &Path) -> SqliteResult<Self> {
        let conn = Connection::open(db_path)?;
        let store = Self { conn };
        store.init_table()?;
        Ok(store)
    }

    fn init_table(&self) -> SqliteResult<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS checkpoints (
                id TEXT PRIMARY KEY,
                user_input TEXT NOT NULL,
                current_iteration INTEGER DEFAULT 0,
                messages_json TEXT NOT NULL,
                completed_tools_json TEXT NOT NULL,
                plan_text TEXT NOT NULL DEFAULT '',
                project_dir TEXT NOT NULL DEFAULT '',
                plan_file TEXT NOT NULL DEFAULT '',
                active_skill TEXT,
                phase TEXT NOT NULL DEFAULT 'executing',
                state TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            [],
        )?;

        // Migration: add active_skill column if it doesn't exist
        let _ = self
            .conn
            .execute("ALTER TABLE checkpoints ADD COLUMN active_skill TEXT", []);

        // Migration: add retry_count column if it doesn't exist
        let _ = self.conn.execute(
            "ALTER TABLE checkpoints ADD COLUMN retry_count INTEGER DEFAULT 0",
            [],
        );

        // Migration: add active_skill column if it doesn't exist (duplicate, safe to run)
        let _ = self
            .conn
            .execute("ALTER TABLE checkpoints ADD COLUMN active_skill TEXT", []);

        // Migration: add session_name column for human-readable session titles
        let _ = self
            .conn
            .execute("ALTER TABLE checkpoints ADD COLUMN session_name TEXT", []);

        // Migration: add parent_id for session hierarchy
        let _ = self
            .conn
            .execute("ALTER TABLE checkpoints ADD COLUMN parent_id TEXT", []);

        // Migration: add session_type for session type
        let _ = self.conn.execute(
            "ALTER TABLE checkpoints ADD COLUMN session_type TEXT DEFAULT 'chat'",
            [],
        );

        // Create session_summaries table for fast session listing
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS session_summaries (
                session_id TEXT PRIMARY KEY,
                title TEXT DEFAULT '',
                summary TEXT NOT NULL,
                phase TEXT NOT NULL DEFAULT 'executing',
                state TEXT NOT NULL,
                project_dir TEXT DEFAULT '',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                first_input TEXT DEFAULT '',
                last_input TEXT DEFAULT '',
                message_count INTEGER DEFAULT 0,
                topics TEXT DEFAULT '[]'
            )",
            [],
        )?;

        // Migration: add new columns if they don't exist
        let _ = self.conn.execute(
            "ALTER TABLE session_summaries ADD COLUMN title TEXT DEFAULT ''",
            [],
        );
        let _ = self.conn.execute(
            "ALTER TABLE session_summaries ADD COLUMN first_input TEXT DEFAULT ''",
            [],
        );
        let _ = self.conn.execute(
            "ALTER TABLE session_summaries ADD COLUMN last_input TEXT DEFAULT ''",
            [],
        );
        let _ = self.conn.execute(
            "ALTER TABLE session_summaries ADD COLUMN message_count INTEGER DEFAULT 0",
            [],
        );
        let _ = self.conn.execute(
            "ALTER TABLE session_summaries ADD COLUMN topics TEXT DEFAULT '[]'",
            [],
        );
        let _ = self.conn.execute(
            "ALTER TABLE session_summaries ADD COLUMN parent_id TEXT",
            [],
        );
        let _ = self.conn.execute(
            "ALTER TABLE session_summaries ADD COLUMN session_type TEXT DEFAULT 'chat'",
            [],
        );

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_checkpoints_state ON checkpoints(state)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_checkpoints_user ON checkpoints(user_input)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_summaries_parent ON session_summaries(parent_id)",
            [],
        )?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS session_events (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                event_data TEXT NOT NULL,
                created_at TEXT NOT NULL,
                compressed INTEGER DEFAULT 0
            )",
            [],
        )?;

        let _ = self.conn.execute(
            "ALTER TABLE session_events ADD COLUMN compressed INTEGER DEFAULT 0",
            [],
        );

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_events_session ON session_events(session_id)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_events_type ON session_events(event_type)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_session_events_created ON session_events(created_at)",
            [],
        )?;

        Ok(())
    }

    pub fn emit_event(
        &self,
        session_id: &str,
        event_type: &SessionEventType,
        event_data: &serde_json::Value,
    ) -> SqliteResult<()> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        self.conn.execute(
            "INSERT INTO session_events (id, session_id, event_type, event_data, created_at, compressed)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![
                id,
                session_id,
                event_type.to_string(),
                event_data.to_string(),
                now.to_rfc3339(),
            ],
        )?;

        Ok(())
    }

    pub fn emit_checkpoint_created(&self, checkpoint: &DevelopmentCheckpoint) -> SqliteResult<()> {
        let event_data = serde_json::json!({
            "checkpoint_id": checkpoint.id,
            "phase": checkpoint.phase.to_string(),
            "state": checkpoint.state.to_string(),
            "iteration": checkpoint.current_iteration,
            "has_plan": !checkpoint.plan_text.is_empty(),
            "project_dir": checkpoint.project_dir,
        });

        self.emit_event(
            &checkpoint.id,
            &SessionEventType::CheckpointCreated,
            &event_data,
        )
    }

    pub fn emit_phase_changed(
        &self,
        session_id: &str,
        from: &PlanPhase,
        to: &PlanPhase,
        reason: &str,
    ) -> SqliteResult<()> {
        let event_data = serde_json::json!({
            "from": from.to_string(),
            "to": to.to_string(),
            "reason": reason,
        });

        self.emit_event(session_id, &SessionEventType::PhaseChanged, &event_data)
    }

    pub fn emit_state_changed(
        &self,
        session_id: &str,
        from: &DevelopmentState,
        to: &DevelopmentState,
    ) -> SqliteResult<()> {
        let event_data = serde_json::json!({
            "from": from.to_string(),
            "to": to.to_string(),
        });

        self.emit_event(session_id, &SessionEventType::StateChanged, &event_data)
    }

    pub fn emit_error(&self, session_id: &str, error: &str, context: &str) -> SqliteResult<()> {
        let event_data = serde_json::json!({
            "error": error,
            "context": context,
        });

        self.emit_event(session_id, &SessionEventType::ErrorOccurred, &event_data)
    }

    /// Save or update a session summary
    pub fn save_session_summary(&self, summary: &SessionSummary) -> SqliteResult<()> {
        let topics_json =
            serde_json::to_string(&summary.topics).unwrap_or_else(|_| "[]".to_string());
        self.conn.execute(
            "INSERT OR REPLACE INTO session_summaries 
             (session_id, title, summary, phase, state, project_dir, created_at, updated_at, first_input, last_input, message_count, topics, parent_id, session_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                summary.session_id,
                summary.title,
                summary.summary,
                summary.phase,
                summary.state,
                summary.project_dir,
                summary.created_at.to_rfc3339(),
                summary.updated_at.to_rfc3339(),
                summary.first_input,
                summary.last_input,
                summary.message_count as i64,
                topics_json,
                summary.parent_id,
                summary.session_type.to_string(),
            ],
        )?;
        Ok(())
    }

    /// Get a session summary by session_id
    pub fn get_session_summary(&self, session_id: &str) -> SqliteResult<Option<SessionSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, title, summary, phase, state, project_dir, created_at, updated_at, 
                    first_input, last_input, message_count, topics, parent_id, session_type
             FROM session_summaries WHERE session_id = ?1",
        )?;

        let mut rows = stmt.query(params![session_id])?;
        if let Some(row) = rows.next()? {
            let topics_str: String = row.get(11).ok().unwrap_or_else(|| "[]".to_string());
            let topics: Vec<String> = serde_json::from_str(&topics_str).unwrap_or_default();
            let parent_id: Option<String> = row.get(12).ok();
            let session_type_str: String = row.get(13).ok().unwrap_or_else(|| "chat".to_string());
            Ok(Some(SessionSummary {
                session_id: row.get(0)?,
                title: row.get(1).ok().unwrap_or_default(),
                summary: row.get(2)?,
                phase: row.get(3)?,
                state: row.get(4)?,
                project_dir: row.get(5)?,
                created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                first_input: row.get(8).ok().unwrap_or_default(),
                last_input: row.get(9).ok().unwrap_or_default(),
                message_count: row.get::<_, i64>(10).ok().unwrap_or(0) as usize,
                topics,
                parent_id,
                session_type: SessionType::from(session_type_str.as_str()),
            }))
        } else {
            Ok(None)
        }
    }

    /// Delete a session summary
    pub fn delete_session_summary(&self, session_id: &str) -> SqliteResult<()> {
        tracing::debug!("Deleting session summary for: {}", session_id);
        let deleted = self.conn.execute(
            "DELETE FROM session_summaries WHERE session_id = ?1",
            params![session_id],
        )?;
        tracing::debug!("Deleted {} session summary rows", deleted);
        Ok(())
    }

    /// Update session with new message (call after each exchange)
    pub fn update_session_message(&self, session_id: &str, user_input: &str) -> SqliteResult<()> {
        // Get current session
        if let Some(mut session) = self.get_session_summary(session_id)? {
            // Update last_input and increment message_count
            session.last_input = user_input.to_string();
            session.message_count += 1;
            session.updated_at = Utc::now();

            // If first_input is empty, set it
            if session.first_input.is_empty() {
                session.first_input = user_input.to_string();
            }

            // Auto-generate title from first input if empty
            if session.title.is_empty() {
                session.title = user_input.chars().take(40).collect::<String>();
                if user_input.len() > 40 {
                    session.title.push_str("...");
                }
            }

            self.save_session_summary(&session)?;
        }
        Ok(())
    }

    /// List all session summaries ordered by most recent
    pub fn list_session_summaries(&self, limit: usize) -> SqliteResult<Vec<SessionSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, title, summary, phase, state, project_dir, created_at, updated_at, 
                    first_input, last_input, message_count, topics, parent_id, session_type
             FROM session_summaries
             ORDER BY updated_at DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            let topics_str: String = row.get(11).ok().unwrap_or_else(|| "[]".to_string());
            let topics: Vec<String> = serde_json::from_str(&topics_str).unwrap_or_default();
            let parent_id: Option<String> = row.get(12).ok();
            let session_type_str: String = row.get(13).ok().unwrap_or_else(|| "chat".to_string());
            Ok(SessionSummary {
                session_id: row.get(0)?,
                title: row.get(1).ok().unwrap_or_default(),
                summary: row.get(2)?,
                phase: row.get(3)?,
                state: row.get(4)?,
                project_dir: row.get(5)?,
                created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                first_input: row.get(8).ok().unwrap_or_default(),
                last_input: row.get(9).ok().unwrap_or_default(),
                message_count: row.get::<_, i64>(10).ok().unwrap_or(0) as usize,
                topics,
                parent_id,
                session_type: SessionType::from(session_type_str.as_str()),
            })
        })?;

        let mut summaries = Vec::new();
        for summary in rows {
            summaries.push(summary?);
        }
        Ok(summaries)
    }

    /// List session summaries filtered by parent_id
    pub fn list_session_summaries_by_parent(
        &self,
        parent_id: &str,
        limit: usize,
    ) -> SqliteResult<Vec<SessionSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, title, summary, phase, state, project_dir, created_at, updated_at, 
                    first_input, last_input, message_count, topics, parent_id, session_type
             FROM session_summaries
             WHERE parent_id = ?1
             ORDER BY updated_at DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![parent_id, limit as i64], |row| {
            let topics_str: String = row.get(11).ok().unwrap_or_else(|| "[]".to_string());
            let topics: Vec<String> = serde_json::from_str(&topics_str).unwrap_or_default();
            let parent_id: Option<String> = row.get(12).ok();
            let session_type_str: String = row.get(13).ok().unwrap_or_else(|| "chat".to_string());
            Ok(SessionSummary {
                session_id: row.get(0)?,
                title: row.get(1).ok().unwrap_or_default(),
                summary: row.get(2)?,
                phase: row.get(3)?,
                state: row.get(4)?,
                project_dir: row.get(5)?,
                created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                first_input: row.get(8).ok().unwrap_or_default(),
                last_input: row.get(9).ok().unwrap_or_default(),
                message_count: row.get::<_, i64>(10).ok().unwrap_or(0) as usize,
                topics,
                parent_id,
                session_type: SessionType::from(session_type_str.as_str()),
            })
        })?;

        let mut summaries = Vec::new();
        for summary in rows {
            summaries.push(summary?);
        }
        Ok(summaries)
    }

    /// Get all ancestors of a session (parent chain to root)
    pub fn get_ancestors(&self, session_id: &str) -> SqliteResult<Vec<SessionSummary>> {
        let mut ancestors = Vec::new();
        let mut current_id = Some(session_id.to_string());

        while let Some(id) = current_id {
            if let Some(summary) = self.get_session_summary(&id)? {
                if summary.session_id == id
                    && ancestors
                        .iter()
                        .any(|a: &SessionSummary| a.session_id == id)
                {
                    break;
                }
                if summary.session_id != session_id {
                    ancestors.insert(0, summary.clone());
                }
                current_id = summary.parent_id.clone();
            } else {
                break;
            }
        }

        Ok(ancestors)
    }

    /// Get the root session of a hierarchy
    pub fn get_root_session(&self, session_id: &str) -> SqliteResult<Option<SessionSummary>> {
        let ancestors = self.get_ancestors(session_id)?;
        Ok(ancestors.into_iter().next())
    }

    /// Get full context for a session including parent context
    pub fn get_full_context(&self, session_id: &str) -> SqliteResult<Option<SessionContext>> {
        if let Some(current) = self.get_session_summary(session_id)? {
            let ancestors = self.get_ancestors(session_id)?;
            let children = self.list_session_summaries_by_parent(session_id, 10)?;

            Ok(Some(SessionContext {
                current,
                ancestors,
                children,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn save(&self, checkpoint: &DevelopmentCheckpoint) -> SqliteResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO checkpoints (id, user_input, session_name, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at, retry_count, parent_id, session_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                checkpoint.id,
                checkpoint.user_input,
                checkpoint.session_name,
                checkpoint.current_iteration,
                checkpoint.messages_json,
                checkpoint.completed_tools_json,
                checkpoint.plan_text,
                checkpoint.project_dir,
                checkpoint.plan_file,
                checkpoint.active_skill,
                checkpoint.phase.to_string(),
                checkpoint.state.to_string(),
                checkpoint.created_at.to_rfc3339(),
                checkpoint.updated_at.to_rfc3339(),
                checkpoint.retry_count,
                checkpoint.parent_id,
                checkpoint.session_type.map(|t| t.to_string()),
            ],
        )?;

        // Also update session summary
        let summary = SessionSummary {
            session_id: checkpoint.id.clone(),
            title: checkpoint.session_name.clone().unwrap_or_default(),
            summary: checkpoint.session_name.clone().unwrap_or_else(|| {
                let truncated: String = checkpoint.user_input.chars().take(500).collect();
                if truncated.len() == 500 {
                    format!("{}...", truncated)
                } else {
                    truncated
                }
            }),
            phase: checkpoint.phase.to_string(),
            state: checkpoint.state.to_string(),
            project_dir: checkpoint.project_dir.clone(),
            created_at: checkpoint.created_at,
            updated_at: chrono::Utc::now(),
            first_input: checkpoint.user_input.clone(),
            last_input: checkpoint.user_input.clone(),
            message_count: checkpoint.current_iteration,
            topics: vec![],
            parent_id: checkpoint.parent_id.clone(),
            session_type: checkpoint.session_type.unwrap_or_default(),
        };
        self.save_session_summary(&summary)?;

        if let Err(e) = self.emit_checkpoint_created(checkpoint) {
            tracing::warn!("Failed to emit checkpoint created event: {}", e);
        }

        Ok(())
    }

    pub fn get(&self, id: &str) -> SqliteResult<Option<DevelopmentCheckpoint>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user_input, session_name, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at, retry_count
             FROM checkpoints WHERE id = ?1"
        )?;

        let mut rows = stmt.query(params![id])?;

        if let Some(row) = rows.next()? {
            Ok(Some(self.row_to_checkpoint(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn get_active(&self) -> SqliteResult<Vec<DevelopmentCheckpoint>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user_input, session_name, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at, retry_count
             FROM checkpoints WHERE state IN ('in_progress', 'interrupted', 'completed') AND phase != 'executing'
             ORDER BY updated_at DESC"
        )?;

        let rows = stmt.query_map([], |row| self.row_to_checkpoint(row))?;

        let mut checkpoints = Vec::new();
        for checkpoint in rows {
            checkpoints.push(checkpoint?);
        }

        Ok(checkpoints)
    }

    pub fn get_recent_with_plans(&self, limit: usize) -> SqliteResult<Vec<DevelopmentCheckpoint>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user_input, session_name, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at, retry_count
             FROM checkpoints WHERE plan_text != ''
             ORDER BY updated_at DESC LIMIT ?1"
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| self.row_to_checkpoint(row))?;
        let mut checkpoints = Vec::new();
        for checkpoint in rows {
            checkpoints.push(checkpoint?);
        }
        Ok(checkpoints)
    }

    /// List all sessions ordered by most recent
    pub fn list_all(&self, limit: usize) -> SqliteResult<Vec<DevelopmentCheckpoint>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user_input, session_name, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at, retry_count
             FROM checkpoints
             ORDER BY created_at DESC LIMIT ?1"
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| self.row_to_checkpoint(row))?;
        let mut checkpoints = Vec::new();
        for checkpoint in rows {
            checkpoints.push(checkpoint?);
        }
        Ok(checkpoints)
    }

    pub fn find_by_id_prefix(&self, prefix: &str) -> SqliteResult<Option<DevelopmentCheckpoint>> {
        let search_pattern = format!("{}%", prefix);
        let mut stmt = self.conn.prepare(
            "SELECT id, user_input, session_name, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at, retry_count
             FROM checkpoints WHERE id LIKE ?1
             ORDER BY updated_at DESC LIMIT 1"
        )?;

        let mut rows = stmt.query(params![search_pattern])?;
        if let Some(row) = rows.next()? {
            Ok(Some(self.row_to_checkpoint(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn find_by_input(&self, user_input: &str) -> SqliteResult<Option<DevelopmentCheckpoint>> {
        let search_pattern = format!("%{}%", user_input);

        let mut stmt = self.conn.prepare(
            "SELECT id, user_input, session_name, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at, retry_count
             FROM checkpoints WHERE user_input LIKE ?1 AND state IN ('in_progress', 'interrupted')
             ORDER BY updated_at DESC LIMIT 1"
        )?;

        let mut rows = stmt.query(params![search_pattern])?;

        if let Some(row) = rows.next()? {
            Ok(Some(self.row_to_checkpoint(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn delete(&self, id: &str) -> SqliteResult<()> {
        // Try to delete from checkpoints first
        let deleted = self
            .conn
            .execute("DELETE FROM checkpoints WHERE id = ?1", params![id])?;

        // Also try to delete session summary (works even if checkpoint doesn't exist)
        let _ = self.delete_session_summary(id);

        Ok(())
    }

    pub fn delete_completed_older_than(&self, days: i64) -> SqliteResult<usize> {
        let cutoff = Utc::now() - chrono::Duration::days(days);

        let deleted = self.conn.execute(
            "DELETE FROM checkpoints WHERE state IN ('completed', 'failed') AND updated_at < ?1",
            params![cutoff.to_rfc3339()],
        )?;

        Ok(deleted)
    }

    pub fn cleanup_old(&self, max_per_user: usize) -> SqliteResult<usize> {
        let mut deleted = 0;

        let to_delete = self.conn.query_row(
            "SELECT id FROM checkpoints WHERE state IN ('completed', 'failed') 
             ORDER BY updated_at DESC LIMIT -1 OFFSET ?1",
            params![max_per_user as i64],
            |row| row.get::<_, String>(0),
        );

        if let Ok(id) = to_delete {
            deleted = self.conn.execute(
                "DELETE FROM checkpoints WHERE state IN ('completed', 'failed') AND updated_at < (
                    SELECT updated_at FROM checkpoints WHERE id = ?1
                )",
                params![id],
            )?;
        }

        Ok(deleted)
    }

    fn row_to_checkpoint(&self, row: &rusqlite::Row) -> SqliteResult<DevelopmentCheckpoint> {
        let state_str: String = row.get(11)?;
        let phase_str: String = row.get(10)?;

        Ok(DevelopmentCheckpoint {
            id: row.get(0)?,
            user_input: row.get(1)?,
            session_name: row.get(2).ok(),
            current_iteration: row.get(3)?,
            messages_json: row.get(4)?,
            completed_tools_json: row.get(5)?,
            plan_text: row.get(6)?,
            project_dir: row.get(7)?,
            plan_file: row.get(8)?,
            active_skill: row.get(9).ok(),
            phase: PlanPhase::from(phase_str.as_str()),
            state: DevelopmentState::from(state_str.as_str()),
            current_step: 0,
            completed_steps: vec![],
            retry_count: row.get(14).unwrap_or(0),
            last_error: None,
            auto_loop_enabled: false,
            parent_id: row.get(15).ok(),
            session_type: row
                .get::<_, Option<String>>(16)?
                .map(|s| SessionType::from(s.as_str())),
            created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(12)?)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(13)?)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}

impl DevelopmentCheckpoint {
    pub fn new(user_input: String) -> Self {
        let now = Utc::now();
        let session_name: String = user_input.chars().take(40).collect();
        let session_name = if session_name.len() == 40 {
            format!("{}...", session_name)
        } else {
            session_name
        };
        Self {
            id: Uuid::new_v4().to_string(),
            user_input,
            session_name: Some(session_name),
            current_iteration: 0,
            messages_json: "[]".to_string(),
            completed_tools_json: "[]".to_string(),
            plan_text: String::new(),
            project_dir: String::new(),
            plan_file: String::new(),
            active_skill: None,
            phase: PlanPhase::Executing,
            state: DevelopmentState::InProgress,
            current_step: 0,
            completed_steps: vec![],
            retry_count: 0,
            last_error: None,
            auto_loop_enabled: false,
            parent_id: None,
            session_type: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_messages(mut self, messages_json: String) -> Self {
        self.messages_json = messages_json;
        self
    }

    pub fn with_tools(mut self, tools_json: String) -> Self {
        self.completed_tools_json = tools_json;
        self
    }

    pub fn increment_iteration(&mut self) {
        self.current_iteration += 1;
        self.updated_at = Utc::now();
    }

    pub fn set_state(&mut self, state: DevelopmentState) {
        self.state = state;
        self.updated_at = Utc::now();
    }

    pub fn set_phase(&mut self, phase: PlanPhase) {
        self.phase = phase;
        self.updated_at = Utc::now();
    }

    pub fn set_plan_text(&mut self, plan_text: String) {
        self.plan_text = plan_text;
        self.updated_at = Utc::now();
    }

    pub fn set_project_dir(&mut self, project_dir: String) {
        self.project_dir = project_dir;
        self.updated_at = Utc::now();
    }

    pub fn set_plan_file(&mut self, plan_file: String) {
        self.plan_file = plan_file;
        self.updated_at = Utc::now();
    }

    pub fn is_development_task(input: &str) -> bool {
        let dev_keywords = [
            "criar",
            "implementar",
            "desenvolver",
            "construir",
            "fazer",
            "crie",
            "implemente",
            "desenvolva",
            "construa",
            "faça",
            "bug",
            "erro",
            "corrigir",
            "fix",
            "create",
            "implement",
            "develop",
            "build",
            "make",
            "add",
            "remove",
            "update",
            "escrever",
            "codar",
            "programar",
            "code",
            "program",
            "file",
            "arquivo",
            "function",
            "função",
            "class",
            "classe",
            "api",
            "endpoint",
            "service",
            "serviço",
            "test",
            "teste",
        ];

        let input_lower = input.to_lowercase();
        dev_keywords.iter().any(|kw| input_lower.contains(kw))
    }

    pub fn set_current_step(&mut self, step: usize) {
        self.current_step = step;
        self.updated_at = Utc::now();
    }

    pub fn mark_step_done(&mut self, step: usize) {
        if !self.completed_steps.contains(&step) {
            self.completed_steps.push(step);
            self.completed_steps.sort();
        }
        self.updated_at = Utc::now();
    }

    pub fn is_step_done(&self, step: usize) -> bool {
        self.completed_steps.contains(&step)
    }

    pub fn parse_plan_steps(&self) -> Vec<String> {
        if self.plan_text.is_empty() {
            return vec![];
        }

        let mut steps = Vec::new();
        for line in self.plan_text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with(|c: char| c.is_ascii_digit()) && trimmed.contains(')') {
                let step_text = trimmed
                    .trim_start_matches(|c: char| c.is_ascii_digit())
                    .trim_start_matches(')')
                    .trim_start_matches('.')
                    .trim();
                if !step_text.is_empty() {
                    steps.push(step_text.to_string());
                }
            } else if trimmed.starts_with('-') || trimmed.starts_with('*') {
                let step_text = trimmed[1..].trim().to_string();
                if !step_text.is_empty() {
                    steps.push(step_text);
                }
            }
        }

        steps
    }

    pub fn total_steps(&self) -> usize {
        self.parse_plan_steps().len()
    }

    pub fn is_plan_mode(&self) -> bool {
        self.phase == PlanPhase::Executing && !self.plan_text.is_empty()
    }

    // Auto loop methods
    pub fn set_auto_loop(&mut self, enabled: bool) {
        self.auto_loop_enabled = enabled;
    }

    pub fn increment_retry(&mut self) {
        self.retry_count += 1;
    }

    pub fn reset_retry(&mut self) {
        self.retry_count = 0;
        self.last_error = None;
    }

    pub fn set_last_error(&mut self, error: String) {
        self.last_error = Some(error);
    }

    pub fn should_retry(&self, max_retries: usize) -> bool {
        self.retry_count < max_retries
    }

    pub fn with_parent(mut self, parent_id: String) -> Self {
        self.parent_id = Some(parent_id);
        self
    }

    pub fn with_session_type(mut self, session_type: SessionType) -> Self {
        self.session_type = Some(session_type);
        self
    }

    pub fn as_subtask_of(parent: &SessionSummary, user_input: String) -> Self {
        let mut checkpoint = Self::new(user_input);
        checkpoint.parent_id = Some(parent.session_id.clone());
        checkpoint.session_type = Some(SessionType::Subtask);
        if parent.project_dir.is_empty() {
            checkpoint.project_dir = parent.project_dir.clone();
        }
        checkpoint
    }

    pub fn as_project(user_input: String) -> Self {
        let mut checkpoint = Self::new(user_input);
        checkpoint.session_type = Some(SessionType::Project);
        checkpoint
    }

    pub fn as_research(user_input: String) -> Self {
        let mut checkpoint = Self::new(user_input);
        checkpoint.session_type = Some(SessionType::Research);
        checkpoint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plan_steps_numbered() {
        let mut cp = DevelopmentCheckpoint::new("test".to_string());
        cp.set_plan_text("1) Criar diretório\n2) Escrever código\n3) Testar".to_string());
        let steps = cp.parse_plan_steps();
        assert_eq!(steps.len(), 3);
        assert!(steps[0].contains("Criar diretório"));
        assert!(steps[1].contains("Escrever código"));
        assert!(steps[2].contains("Testar"));
    }

    #[test]
    fn test_parse_plan_steps_bullet() {
        let mut cp = DevelopmentCheckpoint::new("test".to_string());
        cp.set_plan_text("- Criar arquivo\n- Editar conteúdo".to_string());
        let steps = cp.parse_plan_steps();
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn test_mark_step_done() {
        let mut cp = DevelopmentCheckpoint::new("test".to_string());
        cp.mark_step_done(0);
        cp.mark_step_done(2);
        cp.mark_step_done(0);
        assert!(cp.is_step_done(0));
        assert!(!cp.is_step_done(1));
        assert!(cp.is_step_done(2));
        assert_eq!(cp.completed_steps, vec![0, 2]);
    }

    #[test]
    fn test_is_plan_mode() {
        let mut cp = DevelopmentCheckpoint::new("test".to_string());
        assert!(!cp.is_plan_mode());
        cp.set_phase(PlanPhase::Executing);
        assert!(!cp.is_plan_mode());
        cp.set_plan_text("1) Passo 1".to_string());
        assert!(cp.is_plan_mode());
    }
}
