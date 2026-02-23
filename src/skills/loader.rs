use crate::skills::{parser::SkillParser, Skill};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::{error, info, warn};

pub struct SkillLoader {
    skills_dir: PathBuf,
    loaded_skills: HashMap<String, (Skill, SystemTime)>,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Parse(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "IO error: {}", e),
            LoadError::Parse(msg) => write!(f, "Parse error: {}", msg),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        LoadError::Io(e)
    }
}

impl SkillLoader {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            loaded_skills: HashMap::new(),
        }
    }

    fn normalize_key(name: &str) -> String {
        name.trim().to_lowercase()
    }

    pub fn load_all(&mut self) -> Result<Vec<Skill>, LoadError> {
        self.loaded_skills.clear();

        if !self.skills_dir.exists() {
            warn!("Skills directory does not exist: {:?}", self.skills_dir);
            return Ok(vec![]);
        }

        let mut skills = vec![];

        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                let skill_file = path.join("skill.md");

                if skill_file.exists() {
                    match self.load_skill_file(&skill_file) {
                        Ok(skill) => {
                            let skill_name = skill.name.clone();
                            let key = Self::normalize_key(&skill_name);
                            let modified = skill.last_modified;
                            self.loaded_skills.insert(key, (skill.clone(), modified));
                            info!("Loaded skill: {}", skill_name);
                            skills.push(skill);
                        }
                        Err(e) => {
                            error!("Failed to load skill from {:?}: {}", skill_file, e);
                        }
                    }
                }
            }
        }

        // Verifica se tem skill general, se não cria uma padrão
        if !self
            .loaded_skills
            .contains_key(&Self::normalize_key("general"))
        {
            warn!("No 'general' skill found. Creating default.");
            if let Ok(default_skill) = self.create_default_general_skill() {
                let key = Self::normalize_key(&default_skill.name);
                self.loaded_skills
                    .insert(key, (default_skill.clone(), SystemTime::now()));
                skills.push(default_skill);
            }
        }

        info!("Loaded {} skills total", skills.len());
        Ok(skills)
    }

    fn load_skill_file(&self, path: &Path) -> Result<Skill, LoadError> {
        SkillParser::parse(path).map_err(|e| LoadError::Parse(e.to_string()))
    }

    pub fn check_modifications(&self) -> Vec<String> {
        let mut modified = vec![];

        for (name, (skill, last_loaded)) in &self.loaded_skills {
            if let Ok(metadata) = fs::metadata(&skill.file_path) {
                if let Ok(modified_time) = metadata.modified() {
                    if modified_time > *last_loaded {
                        if let Some(dir_name) = skill
                            .file_path
                            .parent()
                            .and_then(|p| p.file_name())
                            .and_then(|n| n.to_str())
                        {
                            modified.push(dir_name.to_string());
                        } else {
                            modified.push(name.clone());
                        }
                    }
                }
            }
        }

        // Verifica novos skills
        if let Ok(entries) = fs::read_dir(&self.skills_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let skill_file = path.join("skill.md");
                    if skill_file.exists() {
                        // Extrai nome do diretório como nome provisório
                        if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                            if !self
                                .loaded_skills
                                .contains_key(&Self::normalize_key(dir_name))
                            {
                                modified.push(dir_name.to_string());
                            }
                        }
                    }
                }
            }
        }

        modified
    }

    pub fn reload_skills(&mut self, skill_names: &[String]) -> Result<(), LoadError> {
        for name in skill_names {
            let skill_dir = self.skills_dir.join(name);
            let skill_file = skill_dir.join("skill.md");

            if skill_file.exists() {
                match self.load_skill_file(&skill_file) {
                    Ok(skill) => {
                        let dir_key = Self::normalize_key(name);
                        self.loaded_skills.remove(&dir_key);
                        let key = Self::normalize_key(&skill.name);
                        let modified = skill.last_modified;
                        self.loaded_skills.insert(key, (skill, modified));
                        info!("Reloaded skill: {}", name);
                    }
                    Err(e) => {
                        error!("Failed to reload skill '{}': {}", name, e);
                    }
                }
            } else {
                // Skill foi deletada
                let key = Self::normalize_key(name);
                self.loaded_skills.remove(&key);
                info!("Removed skill: {}", name);
            }
        }

        Ok(())
    }

    pub fn get_skill(&self, name: &str) -> Option<&Skill> {
        let key = Self::normalize_key(name);
        self.loaded_skills.get(&key).map(|(skill, _)| skill)
    }

    pub fn list_skills(&self) -> Vec<&Skill> {
        self.loaded_skills
            .values()
            .map(|(skill, _)| skill)
            .collect()
    }

    fn create_default_general_skill(&self) -> Result<Skill, LoadError> {
        Ok(Skill {
            name: "general".to_string(),
            description: "Assistente generalista útil e amigável".to_string(),
            context: "Você é o RustClaw, um assistente AI versátil. Você pode ajudar com diversas tarefas e adapta seu estilo conforme o contexto da conversa.".to_string(),
            keywords: vec!["ajuda".to_string(), "oi".to_string(), "olá".to_string()],
            behaviors: crate::skills::SkillBehaviors {
                always: vec!["Seja prestativo e amigável".to_string()],
                never: vec!["Seja rude ou condescendente".to_string()],
            },
            preferred_tools: vec![],
            examples: vec![],
            file_path: self.skills_dir.join("general/skill.md"),
            last_modified: SystemTime::now(),
        })
    }
}
