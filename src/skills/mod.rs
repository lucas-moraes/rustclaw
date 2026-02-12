use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub context: String,
    pub keywords: Vec<String>,
    pub behaviors: SkillBehaviors,
    pub preferred_tools: Vec<String>,
    pub examples: Vec<SkillExample>,
    pub file_path: PathBuf,
    pub last_modified: SystemTime,
}

#[derive(Debug, Clone)]
pub struct SkillBehaviors {
    pub always: Vec<String>,
    pub never: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SkillExample {
    pub input: String,
    pub good: String,
    pub bad: String,
}

impl Skill {
    pub fn combined_keywords(&self) -> Vec<String> {
        let mut all_keywords = self.keywords.clone();

        // Adiciona nome da skill como keyword
        all_keywords.push(self.name.to_lowercase());

        // Extrai palavras importantes da descrição
        let desc_words: Vec<String> = self
            .description
            .to_lowercase()
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .map(|w| w.to_string())
            .collect();

        all_keywords.extend(desc_words);
        all_keywords.sort();
        all_keywords.dedup();

        all_keywords
    }
}

pub mod detector;
pub mod loader;
pub mod manager;
pub mod parser;
pub mod prompt_builder;
