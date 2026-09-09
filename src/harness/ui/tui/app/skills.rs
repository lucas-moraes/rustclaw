//! Session-skill toggles, picker and `/skills` command.

use crate::harness::skill::PromptSkillToggle;
use anyhow::Result;

use super::state::{App, SkillPickerState};

impl App {
    pub fn sync_prompt_toggles(&mut self) {
        self.prompt_toggles = Some(
            self.session
                .skills
                .iter()
                .map(|s| PromptSkillToggle {
                    skill_id: s.skill_id.clone(),
                    include: s.include_by_default,
                })
                .collect(),
        );
    }

    /// Toggles the include flag for the skill at `index` (in prompt_toggles order).
    pub fn toggle_prompt_skill(&mut self, index: usize) {
        if let Some(toggles) = &mut self.prompt_toggles {
            if let Some(t) = toggles.get_mut(index) {
                t.include = !t.include;
            }
        }
    }

    /// Cycle which UI element is focused: input <-> skill chips.
    pub fn cycle_focus(&mut self) {
        self.skills_focused = !self.skills_focused;
    }

    /// Skill ids currently checked for the upcoming prompt.
    pub fn enabled_skill_ids(&self) -> Vec<String> {
        match &self.prompt_toggles {
            Some(toggles) => toggles
                .iter()
                .filter(|t| t.include)
                .map(|t| t.skill_id.clone())
                .collect(),
            None => self
                .session
                .skills
                .iter()
                .filter(|s| s.include_by_default)
                .map(|s| s.skill_id.clone())
                .collect(),
        }
    }

    /// Applies the picker selection to the session and persists it.
    pub fn apply_skill_picker(&mut self) -> Result<()> {
        if let Some(picker) = self.skill_picker.take() {
            self.session.skills = picker.build_skills();
            self.runtime.store.save_session(&self.session)?;
        }
        self.sync_prompt_toggles();
        self.skills_focused = false;
        Ok(())
    }

    /// Opens the skill picker for a fresh session (or `/new`). Returns true if opened.
    pub fn open_skill_picker(&mut self) -> bool {
        if let Some(state) = SkillPickerState::open(self) {
            self.skill_picker = Some(state);
            true
        } else {
            self.sync_prompt_toggles();
            false
        }
    }

    /// Handles `/skills [list|add <id>|rm <id>|default <id> on|off]`.
    pub fn handle_skills_command(&mut self, text: &str) -> Result<()> {
        let mut parts = text.splitn(3, char::is_whitespace);
        let _cmd = parts.next();
        let sub = parts.next().unwrap_or("").trim().to_string();
        let arg = parts.next().unwrap_or("").trim().to_string();

        match sub.as_str() {
            "" => {
                if self.runtime.skills.skills.is_empty() {
                    self.add_system(
                        "no skills discovered (look for .agents/skills/SKILL.md or RUSTCLAW_SKILLS_DIR)",
                    );
                } else {
                    self.open_skill_picker();
                }
            }
            "list" => {
                let lines: Vec<String> = self
                    .session
                    .skills
                    .iter()
                    .map(|s| {
                        let on = if s.include_by_default { "on" } else { "off" };
                        format!("  {} [{}]", s.skill_id, on)
                    })
                    .collect();
                for l in lines {
                    self.add_system(&l);
                }
            }
            "add" => {
                let mut existing: std::collections::HashSet<String> = self
                    .session
                    .skills
                    .iter()
                    .map(|s| s.skill_id.clone())
                    .collect();
                let mut added = 0;
                let mut unknown: Vec<String> = Vec::new();
                for id in arg.split(',') {
                    let id = id.trim();
                    if id.is_empty() {
                        continue;
                    }
                    if self.runtime.skills.get(id).is_none() {
                        unknown.push(id.to_string());
                        continue;
                    }
                    if existing.insert(id.to_string()) {
                        self.session
                            .skills
                            .push(crate::harness::skill::SessionSkill::new(id, true));
                        added += 1;
                    }
                }
                if !unknown.is_empty() {
                    self.add_system(&format!(
                        "unknown: {} (available: {})",
                        unknown.join(", "),
                        self.runtime.skills.names().join(", ")
                    ));
                }
                self.runtime.store.save_session(&self.session)?;
                self.sync_prompt_toggles();
                self.add_system(&format!("skills updated (+{})", added));
            }
            "rm" => {
                let remove: std::collections::HashSet<String> = arg
                    .split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect();
                self.session
                    .skills
                    .retain(|s| !remove.contains(&s.skill_id));
                self.runtime.store.save_session(&self.session)?;
                self.sync_prompt_toggles();
                self.add_system("skills updated");
            }
            "default" => {
                let mut it = arg.split_whitespace();
                let id = it.next().unwrap_or("").trim().to_string();
                let on = it.next().unwrap_or("").trim().to_string();
                if id.is_empty() {
                    self.add_system("usage: /skills default <id> on|off");
                    return Ok(());
                }
                let val = on == "on" || on == "true";
                let found = self.session.skills.iter().position(|s| s.skill_id == id);
                match found {
                    Some(idx) => {
                        self.session.skills[idx].include_by_default = val;
                        self.runtime.store.save_session(&self.session)?;
                        self.sync_prompt_toggles();
                        self.add_system(&format!(
                            "{} default → {}",
                            id,
                            if val { "on" } else { "off" }
                        ));
                    }
                    None => self.add_system(&format!("skill not in session memory: {}", id)),
                }
            }
            "picker" | "open" => {
                self.open_skill_picker();
            }
            _ => self.add_system(
                "usage: /skills [list] [add <id>] [rm <id>] [default <id> on|off] [picker]",
            ),
        }
        Ok(())
    }
}
