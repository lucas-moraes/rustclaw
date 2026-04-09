#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

use crate::memory::checkpoint::SessionSummary;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SessionType {
    #[default]
    Chat,
    Project,
    Subtask,
    Research,
}

impl std::fmt::Display for SessionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionType::Chat => write!(f, "chat"),
            SessionType::Project => write!(f, "project"),
            SessionType::Subtask => write!(f, "subtask"),
            SessionType::Research => write!(f, "research"),
        }
    }
}

impl From<&str> for SessionType {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "chat" => SessionType::Chat,
            "project" => SessionType::Project,
            "subtask" => SessionType::Subtask,
            "research" => SessionType::Research,
            _ => SessionType::Chat,
        }
    }
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionFingerprint {
    pub has_git: bool,
    pub has_cargo: bool,
    pub has_package_json: bool,
    pub has_requirements_txt: bool,
    pub has_go_mod: bool,
    pub has_pyproject_toml: bool,
    pub language: Option<String>,
    pub repo_url: Option<String>,
    pub project_name: Option<String>,
}

impl SessionFingerprint {
    pub fn detect(cwd: &Path) -> Self {
        let mut fp = Self::default();

        fp.has_git = cwd.join(".git").exists();

        fp.has_cargo = cwd.join("Cargo.toml").exists();
        fp.has_package_json = cwd.join("package.json").exists();
        fp.has_requirements_txt = cwd.join("requirements.txt").exists();
        fp.has_go_mod = cwd.join("go.mod").exists();
        fp.has_pyproject_toml = cwd.join("pyproject.toml").exists();

        if fp.has_cargo {
            fp.language = Some("Rust".to_string());
            fp.project_name = Self::extract_project_name(cwd, "Cargo.toml");
        } else if fp.has_package_json {
            fp.language = Some("JavaScript/TypeScript".to_string());
            fp.project_name = Self::extract_project_name(cwd, "package.json");
        } else if fp.has_go_mod {
            fp.language = Some("Go".to_string());
            fp.project_name = Self::extract_project_name(cwd, "go.mod");
        } else if fp.has_pyproject_toml || fp.has_requirements_txt {
            fp.language = Some("Python".to_string());
        }

        fp.repo_url = Self::detect_git_remote(cwd);

        fp
    }

    fn extract_project_name(cwd: &Path, file: &str) -> Option<String> {
        let content = std::fs::read_to_string(cwd.join(file)).ok()?;
        if file == "Cargo.toml" {
            content
                .lines()
                .find(|l| l.starts_with("name = "))
                .and_then(|l| l.split('=').nth(1))
                .map(|n| n.trim().trim_matches('"').to_string())
        } else if file == "package.json" {
            serde_json::from_str::<serde_json::Value>(&content)
                .ok()
                .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
        } else if file == "go.mod" {
            content
                .lines()
                .find(|l| l.starts_with("module "))
                .and_then(|l| l.split_whitespace().nth(1))
                .map(|s| s.split('/').next_back().unwrap_or(s).to_string())
        } else {
            None
        }
    }

    fn detect_git_remote(cwd: &Path) -> Option<String> {
        let output = std::process::Command::new("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(cwd)
            .output()
            .ok()?;

        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            None
        }
    }

    pub fn is_project_mode(&self) -> bool {
        self.has_git || self.has_cargo || self.has_package_json || self.has_go_mod
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextChange {
    pub is_new_project: bool,
    pub is_new_language: bool,
    pub is_continuing: bool,
    pub previous_fingerprint: Option<SessionFingerprint>,
    pub current_fingerprint: SessionFingerprint,
    pub suggestion: Option<String>,
}

impl ContextChange {
    pub fn detect(previous: Option<&SessionFingerprint>, current: &SessionFingerprint) -> Self {
        let prev = previous.cloned();

        let is_new_project = match &prev {
            Some(p) => p.project_name.as_ref() != current.project_name.as_ref(),
            None => current.is_project_mode(),
        };

        let is_new_language = match &prev {
            Some(p) => p.language != current.language,
            None => false,
        };

        let is_continuing = prev
            .as_ref()
            .is_some_and(|p| p.project_name == current.project_name && !is_new_project);

        let suggestion = if is_new_project {
            Some(format!(
                "Novo projeto detectado: {}. Deseja criar uma nova sessão?",
                current.project_name.as_deref().unwrap_or("desconhecido")
            ))
        } else {
            None
        };

        Self {
            is_new_project,
            is_new_language,
            is_continuing,
            previous_fingerprint: prev,
            current_fingerprint: current.clone(),
            suggestion,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DevelopmentCheckpoint {
    pub id: String,
    pub user_input: String,
    pub session_name: Option<String>,
    pub current_iteration: usize,
    pub messages_json: String,
    pub completed_tools_json: String,
    pub plan_text: String,
    pub project_dir: String,
    pub plan_file: String,
    pub active_skill: Option<String>,
    pub phase: PlanPhase,
    pub state: DevelopmentState,
    pub current_step: usize,
    pub completed_steps: Vec<usize>,
    pub retry_count: usize,
    pub last_error: Option<String>,
    pub auto_loop_enabled: bool,
    pub parent_id: Option<String>,
    pub session_type: Option<SessionType>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
