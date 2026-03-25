use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecution {
    pub tool_name: String,
    pub input: String,
    pub output: String,
    pub iteration: usize,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevelopmentCheckpoint {
    pub id: String,
    pub user_input: String,
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

        // Migration: add active_skill column if it doesn't exist
        let _ = self
            .conn
            .execute("ALTER TABLE checkpoints ADD COLUMN active_skill TEXT", []);

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_checkpoints_state ON checkpoints(state)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_checkpoints_user ON checkpoints(user_input)",
            [],
        )?;

        Ok(())
    }

    pub fn save(&self, checkpoint: &DevelopmentCheckpoint) -> SqliteResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO checkpoints 
             (id, user_input, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                checkpoint.id,
                checkpoint.user_input,
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
            ],
        )?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> SqliteResult<Option<DevelopmentCheckpoint>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user_input, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at
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
            "SELECT id, user_input, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at
             FROM checkpoints WHERE state IN ('in_progress', 'interrupted')
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
            "SELECT id, user_input, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at
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

    pub fn find_by_id_prefix(&self, prefix: &str) -> SqliteResult<Option<DevelopmentCheckpoint>> {
        let search_pattern = format!("{}%", prefix);
        let mut stmt = self.conn.prepare(
            "SELECT id, user_input, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at
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
            "SELECT id, user_input, current_iteration, messages_json, completed_tools_json, plan_text, project_dir, plan_file, active_skill, phase, state, created_at, updated_at
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
        self.conn
            .execute("DELETE FROM checkpoints WHERE id = ?1", params![id])?;
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
        let state_str: String = row.get(9)?;
        let phase_str: String = row.get(8)?;

        Ok(DevelopmentCheckpoint {
            id: row.get(0)?,
            user_input: row.get(1)?,
            current_iteration: row.get(2)?,
            messages_json: row.get(3)?,
            completed_tools_json: row.get(4)?,
            plan_text: row.get(5)?,
            project_dir: row.get(6)?,
            plan_file: row.get(7)?,
            active_skill: row.get(12).ok(),
            phase: PlanPhase::from(phase_str.as_str()),
            state: DevelopmentState::from(state_str.as_str()),
            current_step: 0,
            completed_steps: vec![],
            retry_count: 0,
            last_error: None,
            auto_loop_enabled: false,
            created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(10)?)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(11)?)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}

impl DevelopmentCheckpoint {
    pub fn new(user_input: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            user_input,
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
