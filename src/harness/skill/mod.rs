//! Skills = the per-session "memory". A skill is a reusable capability
//! (markdown instructions) attached to a session and, optionally, injected
//! into the system prompt for a given turn.
//!
//! Mental model:
//! - **prompt**: the user's current request (one turn).
//! - **session**: the record of what was asked/planned/implemented.
//! - **memory**: the list of skills chosen for that session (0..N, editable).

pub mod inject;
pub mod loader;

use serde::{Deserialize, Serialize};

/// A discovered skill from disk (catalog), not stored per session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillSpec {
    pub id: String,
    pub name: String,
    pub description: String,
    pub body: String,
    pub source: String,
}

/// A skill attached to a session's memory.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSkill {
    pub skill_id: String,
    /// Whether the skill is included in the next prompt by default
    /// (the user may toggle per-prompt via the checkbox UI).
    pub include_by_default: bool,
    /// Injection order.
    pub ord: u32,
}

/// Per-turn toggle state for a skill's checkbox in the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptSkillToggle {
    pub skill_id: String,
    pub include: bool,
}

impl SessionSkill {
    pub fn new(skill_id: impl Into<String>, include_by_default: bool) -> Self {
        Self {
            skill_id: skill_id.into(),
            include_by_default,
            ord: 0,
        }
    }
}

/// A catalog of discovered skills with lookup helpers.
#[derive(Clone, Debug, Default)]
pub struct SkillCatalog {
    pub skills: Vec<SkillSpec>,
}

impl SkillCatalog {
    pub fn get(&self, id: &str) -> Option<&SkillSpec> {
        self.skills.iter().find(|s| s.id == id)
    }

    pub fn names(&self) -> Vec<String> {
        self.skills.iter().map(|s| s.name.clone()).collect()
    }
}
