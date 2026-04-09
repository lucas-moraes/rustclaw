#![allow(dead_code)]

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::memory::checkpoint::types::{DevelopmentState, PlanPhase, SessionType};
use crate::memory::checkpoint::DevelopmentCheckpoint;
use crate::memory::checkpoint::SessionSummary;
use crate::memory::{MemoryEntry, MemoryScope, MemoryType};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotTrigger {
    AfterSuccessfulBuild,
    AfterFailedBuild,
    BeforeMajorRefactor,
    OnUserRequest,
    OnPhaseTransition,
    Periodic(u32),
    OnSessionResume,
}

impl std::fmt::Display for SnapshotTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotTrigger::AfterSuccessfulBuild => write!(f, "after_successful_build"),
            SnapshotTrigger::AfterFailedBuild => write!(f, "after_failed_build"),
            SnapshotTrigger::BeforeMajorRefactor => write!(f, "before_major_refactor"),
            SnapshotTrigger::OnUserRequest => write!(f, "on_user_request"),
            SnapshotTrigger::OnPhaseTransition => write!(f, "on_phase_transition"),
            SnapshotTrigger::Periodic(n) => write!(f, "periodic_{}", n),
            SnapshotTrigger::OnSessionResume => write!(f, "on_session_resume"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotPolicy {
    pub trigger_on_build_success: bool,
    pub trigger_on_build_fail: bool,
    pub trigger_on_phase_change: bool,
    pub trigger_on_user_request: bool,
    pub periodic_interval: Option<u32>,
    pub debounce_seconds: u64,
    pub max_snapshots_per_session: usize,
}

impl Default for SnapshotPolicy {
    fn default() -> Self {
        Self {
            trigger_on_build_success: true,
            trigger_on_build_fail: false,
            trigger_on_phase_change: true,
            trigger_on_user_request: true,
            periodic_interval: Some(50),
            debounce_seconds: 60,
            max_snapshots_per_session: 10,
        }
    }
}

impl SnapshotPolicy {
    pub fn should_snapshot(&self, trigger: SnapshotTrigger, message_count: usize) -> bool {
        match trigger {
            SnapshotTrigger::AfterSuccessfulBuild => self.trigger_on_build_success,
            SnapshotTrigger::AfterFailedBuild => self.trigger_on_build_fail,
            SnapshotTrigger::BeforeMajorRefactor => true,
            SnapshotTrigger::OnUserRequest => self.trigger_on_user_request,
            SnapshotTrigger::OnPhaseTransition => self.trigger_on_phase_change,
            SnapshotTrigger::Periodic(_n) => self
                .periodic_interval
                .is_some_and(|interval| message_count.is_multiple_of(interval as usize)),
            SnapshotTrigger::OnSessionResume => true,
        }
    }

    pub fn from_env() -> Self {
        Self {
            trigger_on_build_success: std::env::var("SNAPSHOT_ON_SUCCESS")
                .map(|v| v == "true")
                .unwrap_or(true),
            trigger_on_build_fail: std::env::var("SNAPSHOT_ON_FAILURE")
                .map(|v| v == "true")
                .unwrap_or(false),
            trigger_on_phase_change: std::env::var("SNAPSHOT_ON_PHASE_CHANGE")
                .map(|v| v == "true")
                .unwrap_or(true),
            trigger_on_user_request: true,
            periodic_interval: std::env::var("SNAPSHOT_PERIODIC_MESSAGES")
                .ok()
                .and_then(|v| v.parse().ok()),
            debounce_seconds: std::env::var("SNAPSHOT_DEBOUNCE_SECONDS")
                .map(|v| v.parse().unwrap_or(60))
                .unwrap_or(60),
            max_snapshots_per_session: std::env::var("SNAPSHOT_MAX_PER_SESSION")
                .map(|v| v.parse().unwrap_or(10))
                .unwrap_or(10),
        }
    }
}

pub struct SnapshotManager {
    policy: SnapshotPolicy,
    last_snapshot_time: HashMap<String, DateTime<Utc>>,
    snapshot_counts: HashMap<String, usize>,
}

impl SnapshotManager {
    pub fn new(policy: SnapshotPolicy) -> Self {
        Self {
            policy,
            last_snapshot_time: HashMap::new(),
            snapshot_counts: HashMap::new(),
        }
    }

    pub fn with_default_policy() -> Self {
        Self::new(SnapshotPolicy::default())
    }

    pub fn from_env() -> Self {
        Self::new(SnapshotPolicy::from_env())
    }

    pub fn should_snapshot(
        &self,
        session_id: &str,
        trigger: SnapshotTrigger,
        message_count: usize,
    ) -> bool {
        if !self.policy.should_snapshot(trigger, message_count) {
            return false;
        }

        if let Some(&count) = self.snapshot_counts.get(session_id) {
            if count >= self.policy.max_snapshots_per_session {
                tracing::debug!("Max snapshots reached for session {}", session_id);
                return false;
            }
        }

        if let Some(&last_time) = self.last_snapshot_time.get(session_id) {
            let elapsed = Utc::now() - last_time;
            if elapsed.num_seconds() < self.policy.debounce_seconds as i64 {
                tracing::debug!("Debouncing snapshot for session {}", session_id);
                return false;
            }
        }

        true
    }

    pub fn record_snapshot(&mut self, session_id: &str) {
        let now = Utc::now();
        self.last_snapshot_time.insert(session_id.to_string(), now);
        *self
            .snapshot_counts
            .entry(session_id.to_string())
            .or_insert(0) += 1;
    }

    pub fn get_snapshot_count(&self, session_id: &str) -> usize {
        self.snapshot_counts.get(session_id).copied().unwrap_or(0)
    }

    pub fn get_last_snapshot_time(&self, session_id: &str) -> Option<DateTime<Utc>> {
        self.last_snapshot_time.get(session_id).copied()
    }

    pub fn reset_session(&mut self, session_id: &str) {
        self.last_snapshot_time.remove(session_id);
        self.snapshot_counts.remove(session_id);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecyclePolicy {
    pub session_memory_ttl_days: i64,
    pub checkpoint_ttl_days: i64,
    pub project_memory_ttl_days: i64,
    pub archive_after_days: i64,
    pub low_importance_threshold: f32,
    pub cleanup_batch_size: usize,
}

impl Default for LifecyclePolicy {
    fn default() -> Self {
        Self {
            session_memory_ttl_days: 7,
            checkpoint_ttl_days: 14,
            project_memory_ttl_days: 30,
            archive_after_days: 90,
            low_importance_threshold: 0.2,
            cleanup_batch_size: 100,
        }
    }
}

impl LifecyclePolicy {
    pub fn from_env() -> Self {
        Self {
            session_memory_ttl_days: std::env::var("LIFECYCLE_SESSION_MEMORY_TTL_DAYS")
                .map(|v| v.parse().unwrap_or(7))
                .unwrap_or(7),
            checkpoint_ttl_days: std::env::var("LIFECYCLE_CHECKPOINT_TTL_DAYS")
                .map(|v| v.parse().unwrap_or(14))
                .unwrap_or(14),
            project_memory_ttl_days: std::env::var("LIFECYCLE_PROJECT_MEMORY_TTL_DAYS")
                .map(|v| v.parse().unwrap_or(30))
                .unwrap_or(30),
            archive_after_days: std::env::var("LIFECYCLE_ARCHIVE_AFTER_DAYS")
                .map(|v| v.parse().unwrap_or(90))
                .unwrap_or(90),
            low_importance_threshold: std::env::var("LIFECYCLE_LOW_IMPORTANCE_THRESHOLD")
                .map(|v| v.parse().unwrap_or(0.2))
                .unwrap_or(0.2),
            cleanup_batch_size: std::env::var("LIFECYCLE_CLEANUP_BATCH_SIZE")
                .map(|v| v.parse().unwrap_or(100))
                .unwrap_or(100),
        }
    }
}

pub struct LifecycleManager {
    policy: LifecyclePolicy,
    conn: Connection,
}

impl LifecycleManager {
    pub fn new(db_path: &Path, policy: LifecyclePolicy) -> SqliteResult<Self> {
        let conn = Connection::open(db_path)?;
        Ok(Self { policy, conn })
    }

    pub fn with_default_policy(db_path: &Path) -> SqliteResult<Self> {
        Self::new(db_path, LifecyclePolicy::default())
    }

    pub fn from_env(db_path: &Path) -> SqliteResult<Self> {
        Self::new(db_path, LifecyclePolicy::from_env())
    }

    pub fn run_cleanup(&self) -> SqliteResult<CleanupStats> {
        let mut stats = CleanupStats::default();

        stats.deleted_checkpoints = self.cleanup_old_checkpoints()?;
        stats.downgraded_memories = self.downgrade_low_importance_memories()?;
        stats.archived_sessions = self.archive_old_sessions()?;

        Ok(stats)
    }

    fn cleanup_old_checkpoints(&self) -> SqliteResult<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM checkpoints WHERE updated_at < datetime('now', ?1)",
            [format!("-{} days", self.policy.checkpoint_ttl_days)],
        )?;
        Ok(deleted)
    }

    fn downgrade_low_importance_memories(&self) -> SqliteResult<usize> {
        let updated = self.conn.execute(
            "UPDATE memories SET importance = importance * 0.8 
             WHERE importance < ?1 AND access_count < 2 
             AND timestamp < datetime('now', ?2)",
            params![
                self.policy.low_importance_threshold,
                format!("-{} days", self.policy.session_memory_ttl_days)
            ],
        )?;
        Ok(updated)
    }

    fn archive_old_sessions(&self) -> SqliteResult<usize> {
        let archived = self.conn.execute(
            "UPDATE session_summaries SET state = 'archived' 
             WHERE state != 'archived' 
             AND updated_at < datetime('now', ?1)",
            [format!("-{} days", self.policy.archive_after_days)],
        )?;
        Ok(archived)
    }

    pub fn archive_session(&self, session_id: &str, archive_dir: &Path) -> SqliteResult<String> {
        let checkpoint = {
            let mut stmt = self.conn.prepare(
                "SELECT id, user_input, session_name, current_iteration, messages_json, 
                        completed_tools_json, plan_text, project_dir, plan_file, active_skill, 
                        phase, state, created_at, updated_at, retry_count, parent_id, session_type
                 FROM checkpoints WHERE id = ?1",
            )?;
            stmt.query_row([session_id], |row| {
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
                    phase: PlanPhase::from(row.get::<_, String>(10)?.as_str()),
                    state: DevelopmentState::from(row.get::<_, String>(11)?.as_str()),
                    current_step: 0,
                    completed_steps: vec![],
                    retry_count: row.get::<_, i64>(13).unwrap_or(0) as usize,
                    last_error: None,
                    auto_loop_enabled: false,
                    parent_id: row.get(14).ok(),
                    session_type: row
                        .get::<_, Option<String>>(15)?
                        .map(|s| SessionType::from(s.as_str())),
                    created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(12)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(13)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .optional()?
        };

        let session_summary = {
            let mut stmt = self.conn.prepare(
                "SELECT session_id, title, summary, phase, state, project_dir, created_at, updated_at,
                        first_input, last_input, message_count, topics, parent_id, session_type
                 FROM session_summaries WHERE session_id = ?1"
            )?;
            stmt.query_row([session_id], |row| {
                let topics_str: String = row.get(11).ok().unwrap_or_else(|| "[]".to_string());
                let topics: Vec<String> = serde_json::from_str(&topics_str).unwrap_or_default();
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
                    message_count: row.get::<_, i64>(10).unwrap_or(0) as usize,
                    topics,
                    parent_id: row.get(12).ok(),
                    session_type: SessionType::from(
                        row.get::<_, Option<String>>(13)?
                            .unwrap_or_default()
                            .as_str(),
                    ),
                })
            })
            .optional()?
        };

        let memories: Vec<MemoryEntry> = {
            let mut stmt = self.conn.prepare(
                "SELECT id, session_id, content, embedding, timestamp, importance, memory_type, 
                        metadata, search_count, scope, access_count, last_accessed
                 FROM memories WHERE session_id = ?1",
            )?;
            let rows = stmt.query_map([session_id], |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    session_id: row.get(1).ok(),
                    content: row.get(2)?,
                    embedding: vec![],
                    timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    importance: row.get(5)?,
                    memory_type: MemoryType::from(row.get::<_, String>(6)?.as_str()),
                    metadata: serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default(),
                    search_count: row.get(8)?,
                    scope: MemoryScope::from(
                        row.get::<_, Option<String>>(9)?
                            .unwrap_or_default()
                            .as_str(),
                    ),
                    access_count: row.get(10)?,
                    last_accessed: DateTime::parse_from_rfc3339(&row.get::<_, String>(11)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?;
            rows.filter_map(|r| r.ok()).collect()
        };

        let archive = SessionArchive {
            checkpoint,
            session_summary,
            memories,
            archived_at: Utc::now(),
            version: 1,
        };

        let json = serde_json::to_string_pretty(&archive)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;

        std::fs::create_dir_all(archive_dir).map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!("Failed to create archive dir: {}", e))
        })?;

        let archive_path = archive_dir.join(format!("{}.json", session_id));
        std::fs::write(&archive_path, &json).map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!("Failed to write archive: {}", e))
        })?;

        self.conn
            .execute("DELETE FROM memories WHERE session_id = ?1", [session_id])?;
        self.conn
            .execute("DELETE FROM checkpoints WHERE id = ?1", [session_id])?;
        self.conn.execute(
            "DELETE FROM session_summaries WHERE session_id = ?1",
            [session_id],
        )?;
        self.conn.execute(
            "DELETE FROM session_events WHERE session_id = ?1",
            [session_id],
        )?;

        Ok(archive_path.to_string_lossy().to_string())
    }

    pub fn restore_session(&self, archive_path: &Path) -> SqliteResult<String> {
        let json = std::fs::read_to_string(archive_path).map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!("Failed to read archive: {}", e))
        })?;

        let archive: SessionArchive = serde_json::from_str(&json).map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!("Failed to parse archive: {}", e))
        })?;

        if let Some(checkpoint) = archive.checkpoint {
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
                    checkpoint.retry_count as i64,
                    checkpoint.parent_id,
                    checkpoint.session_type.map(|t| t.to_string()),
                ],
            )?;
        }

        let session_id = archive
            .session_summary
            .as_ref()
            .map(|s| s.session_id.clone())
            .unwrap_or_default();

        if let Some(summary) = archive.session_summary {
            let topics_json =
                serde_json::to_string(&summary.topics).unwrap_or_else(|_| "[]".to_string());
            self.conn.execute(
                "INSERT OR REPLACE INTO session_summaries (session_id, title, summary, phase, state, project_dir, created_at, updated_at, first_input, last_input, message_count, topics, parent_id, session_type)
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
        }

        for memory in archive.memories {
            let metadata =
                serde_json::to_string(&memory.metadata).unwrap_or_else(|_| "{}".to_string());
            let embedding_bytes = Self::vec_f32_to_bytes(&memory.embedding);
            self.conn.execute(
                "INSERT OR REPLACE INTO memories (id, session_id, content, embedding, timestamp, importance, memory_type, metadata, search_count, scope, access_count, last_accessed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    memory.id,
                    memory.session_id,
                    memory.content,
                    embedding_bytes,
                    memory.timestamp.to_rfc3339(),
                    memory.importance,
                    memory.memory_type.to_string(),
                    metadata,
                    memory.search_count,
                    memory.scope.to_string(),
                    memory.access_count,
                    memory.last_accessed.to_rfc3339(),
                ],
            )?;
        }

        Ok(session_id)
    }

    fn vec_f32_to_bytes(vec: &[f32]) -> Vec<u8> {
        vec.iter().flat_map(|&f| f.to_le_bytes()).collect()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CleanupStats {
    pub deleted_checkpoints: usize,
    pub downgraded_memories: usize,
    pub archived_sessions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionArchive {
    pub checkpoint: Option<DevelopmentCheckpoint>,
    pub session_summary: Option<SessionSummary>,
    pub memories: Vec<MemoryEntry>,
    pub archived_at: DateTime<Utc>,
    pub version: u32,
}
