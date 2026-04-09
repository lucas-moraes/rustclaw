use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Skill {
    // Level 1: Always loaded at startup
    pub name: String,
    pub description: String,
    // Level 2: Loaded when skill is activated
    pub context: String,
    pub keywords: Vec<String>,
    pub behaviors: SkillBehaviors,
    #[allow(dead_code)]
    pub preferred_tools: Vec<String>,
    pub examples: Vec<SkillExample>,
    // Metadata
    pub file_path: PathBuf,
    pub last_modified: SystemTime,
    // Claude Code fields
    pub user_invocable: bool,
    pub disable_model_invocation: bool,
    #[allow(dead_code)]
    pub internal: bool,
    // Resource directories
    pub has_scripts: bool,
    pub has_references: bool,
    pub has_assets: bool,
    // Additional metadata
    pub license: Option<String>,
    pub version: Option<String>,
    pub compatibility: Option<String>,
    // Lazy loading
    pub full_content_loaded: bool,
    // Model/effort override
    pub model: Option<String>,
    pub effort: Option<String>,
    // Skill dependencies
    pub dependencies: Vec<String>,
}

impl Skill {
    #[allow(dead_code)]
    pub fn load_level_1(&self) -> String {
        format!("Skill: {}\nDescription: {}", self.name, self.description)
    }

    #[allow(dead_code)]
    pub fn load_level_2(&self) -> String {
        self.context.clone()
    }
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
pub mod hook_manager;
pub mod loader;
pub mod manager;
pub mod marketplace;
pub mod mcp_client;
pub mod parser;
pub mod permission_manager;
pub mod prompt_builder;
pub mod reference_loader;
pub mod script_executor;
