use crate::skills::{detector::SkillDetector, loader::SkillLoader, Skill};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

pub struct SkillManager {
    loader: SkillLoader,
    detector: SkillDetector,
    active_skill: Option<String>,
    last_check: Instant,
    check_interval: Duration,
}

impl SkillManager {
    pub fn new(skills_dir: PathBuf) -> anyhow::Result<Self> {
        let mut loader = SkillLoader::new(skills_dir);
        let skills = loader.load_all()?;

        if skills.is_empty() {
            warn!("No skills loaded!");
        }

        let detector = SkillDetector::new(&skills);

        Ok(Self {
            loader,
            detector,
            active_skill: None,
            last_check: Instant::now(),
            check_interval: Duration::from_secs(0), // Verifica a cada mensagem
        })
    }

    pub fn process_message(&mut self, message: &str) -> Option<&Skill> {
        // Verifica modificações (hot reload)
        let now = Instant::now();
        if now.duration_since(self.last_check) >= self.check_interval {
            let modified = self.loader.check_modifications();
            if !modified.is_empty() {
                info!("Detected modifications in skills: {:?}", modified);
                if let Err(e) = self.loader.reload_skills(&modified) {
                    error!("Failed to reload skills: {}", e);
                } else {
                    // Reconstrói detector com skills atualizadas
                    let skill_refs = self.loader.list_skills();
                    let skills: Vec<Skill> = skill_refs.iter().map(|&s| s.clone()).collect();
                    self.detector = SkillDetector::new(&skills);
                    info!("Skills reloaded successfully");
                }
            }
            self.last_check = now;
        }

        // Detecta skill pela mensagem
        let detected = self.detector.detect(message, self.active_skill.as_deref());

        // Se mudou, atualiza
        if detected != self.active_skill {
            if let Some(ref name) = detected {
                info!("🎭 Switching to skill: {}", name);
            } else if self.active_skill.is_some() {
                info!("🎭 Returning to general mode");
            }
            self.active_skill = detected;
        }

        // Retorna skill ativa ou general
        self.get_active_skill()
    }

    pub fn get_active_skill(&self) -> Option<&Skill> {
        self.active_skill
            .as_ref()
            .and_then(|name| self.loader.get_skill(name))
            .or_else(|| self.loader.get_skill("general"))
    }

    #[allow(dead_code)]
    pub fn get_active_skill_name(&self) -> Option<String> {
        self.active_skill.clone()
    }

    #[allow(dead_code)]
    pub fn list_available_skills(&self) -> Vec<String> {
        self.loader
            .list_skills()
            .iter()
            .map(|s| s.name.clone())
            .collect()
    }

    pub fn force_skill(&mut self, skill_name: &str) -> Result<(), String> {
        if self.loader.get_skill(skill_name).is_some() {
            self.active_skill = Some(skill_name.to_string());
            info!("🎭 Forced skill: {}", skill_name);
            Ok(())
        } else {
            Err(format!("Skill '{}' not found", skill_name))
        }
    }
}
