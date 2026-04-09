#![allow(dead_code)]

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

use crate::memory::checkpoint::events::SessionEventType;
use crate::memory::checkpoint::types::{
    DevelopmentCheckpoint, DevelopmentState, PlanPhase, SessionType,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub title: String,
    pub summary: String,
    pub phase: String,
    pub state: String,
    pub project_dir: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub first_input: String,
    pub last_input: String,
    pub message_count: usize,
    pub topics: Vec<String>,
    pub parent_id: Option<String>,
    pub session_type: SessionType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    pub current: SessionSummary,
    pub ancestors: Vec<SessionSummary>,
    pub children: Vec<SessionSummary>,
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

        let _ = self
            .conn
            .execute("ALTER TABLE checkpoints ADD COLUMN active_skill TEXT", []);

        let _ = self.conn.execute(
            "ALTER TABLE checkpoints ADD COLUMN retry_count INTEGER DEFAULT 0",
            [],
        );

        let _ = self
            .conn
            .execute("ALTER TABLE checkpoints ADD COLUMN session_name TEXT", []);

        let _ = self
            .conn
            .execute("ALTER TABLE checkpoints ADD COLUMN parent_id TEXT", []);

        let _ = self.conn.execute(
            "ALTER TABLE checkpoints ADD COLUMN session_type TEXT DEFAULT 'chat'",
            [],
        );

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

    pub fn delete_session_summary(&self, session_id: &str) -> SqliteResult<()> {
        tracing::debug!("Deleting session summary for: {}", session_id);
        let deleted = self.conn.execute(
            "DELETE FROM session_summaries WHERE session_id = ?1",
            params![session_id],
        )?;
        tracing::debug!("Deleted {} session summary rows", deleted);
        Ok(())
    }

    pub fn update_session_message(&self, session_id: &str, user_input: &str) -> SqliteResult<()> {
        if let Some(mut session) = self.get_session_summary(session_id)? {
            session.last_input = user_input.to_string();
            session.message_count += 1;
            session.updated_at = Utc::now();

            if session.first_input.is_empty() {
                session.first_input = user_input.to_string();
            }

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

    pub fn get_root_session(&self, session_id: &str) -> SqliteResult<Option<SessionSummary>> {
        let ancestors = self.get_ancestors(session_id)?;
        Ok(ancestors.into_iter().next())
    }

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
             FROM checkpoints WHERE id = ?1",
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
             ORDER BY updated_at DESC",
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
             ORDER BY updated_at DESC LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| self.row_to_checkpoint(row))?;
        let mut checkpoints = Vec::new();
        for checkpoint in rows {
            checkpoints.push(checkpoint?);
        }
        Ok(checkpoints)
    }

    pub fn list_all(&self, limit: usize) -> SqliteResult<Vec<DevelopmentCheckpoint>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user_input, session_name, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at, retry_count
             FROM checkpoints
             ORDER BY created_at DESC LIMIT ?1",
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
             ORDER BY updated_at DESC LIMIT 1",
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
             ORDER BY updated_at DESC LIMIT 1",
        )?;

        let mut rows = stmt.query(params![search_pattern])?;

        if let Some(row) = rows.next()? {
            Ok(Some(self.row_to_checkpoint(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn delete(&self, id: &str) -> SqliteResult<()> {
        let _deleted = self
            .conn
            .execute("DELETE FROM checkpoints WHERE id = ?1", params![id])?;

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
