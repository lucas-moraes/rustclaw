use crate::skills::Skill;
use std::collections::HashMap;
use tracing::debug;

pub struct SkillDetector {
    keyword_map: HashMap<String, Vec<String>>,
    skills_by_name: HashMap<String, Skill>,
    confidence_threshold: f32,
}

impl SkillDetector {
    pub fn new(skills: &[Skill]) -> Self {
        let mut keyword_map: HashMap<String, Vec<String>> = HashMap::new();
        let mut skills_by_name: HashMap<String, Skill> = HashMap::new();

        for skill in skills {
            let keywords = skill.combined_keywords();
            for keyword in keywords {
                keyword_map
                    .entry(keyword)
                    .or_default()
                    .push(skill.name.clone());
            }
            skills_by_name.insert(skill.name.clone(), skill.clone());
        }

        debug!("Built keyword map with {} keywords", keyword_map.len());

        Self {
            keyword_map,
            skills_by_name,
            confidence_threshold: 0.3,
        }
    }

    pub fn detect(&self, message: &str, active_skill: Option<&str>) -> Option<String> {
        // Check for /skill-name invocation (CLI-style)
        if let Some(skill_name) = Self::parse_slash_command(message) {
            if let Some(skill) = self.skills_by_name.get(&skill_name) {
                if skill.user_invocable {
                    debug!("Slash command invoked skill '{}'", skill_name);
                    return Some(skill_name);
                } else {
                    debug!("Skill '{}' is not user-invocable", skill_name);
                    return None;
                }
            }
        }

        let tokens = Self::tokenize(message);

        if tokens.is_empty() {
            return active_skill.map(|s| s.to_string());
        }

        let mut scores: HashMap<String, f32> = HashMap::new();

        // Pontua por keywords encontradas
        for token in &tokens {
            if let Some(skills) = self.keyword_map.get(token) {
                for skill in skills {
                    *scores.entry(skill.clone()).or_insert(0.0) += 1.0;
                }
            }
        }

        // Normaliza scores pelo número de tokens
        let total_tokens = tokens.len() as f32;
        for score in scores.values_mut() {
            *score /= total_tokens;
        }

        // Boost para skill já ativa (evita mudanças frequentes)
        if let Some(active) = active_skill {
            if let Some(score) = scores.get_mut(active) {
                *score *= 1.2; // 20% boost
            }
        }

        // Encontra skill com maior score
        let best = scores
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(name, score)| (name.clone(), *score));

        match best {
            Some((name, score)) if score >= self.confidence_threshold => {
                debug!("Detected skill '{}' with confidence {:.2}", name, score);
                Some(name)
            }
            _ => {
                debug!("No skill detected with sufficient confidence");
                None
            }
        }
    }

    fn tokenize(message: &str) -> Vec<String> {
        message
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty() && s.len() > 2)
            .map(|s| s.to_string())
            .collect()
    }

    fn parse_slash_command(message: &str) -> Option<String> {
        let trimmed = message.trim();
        if !trimmed.starts_with('/') {
            return None;
        }

        // Extract skill name after /
        let after_slash = &trimmed[1..];
        let skill_name = after_slash.split_whitespace().next().unwrap_or(after_slash);

        if skill_name.is_empty() {
            return None;
        }

        Some(skill_name.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{Skill, SkillBehaviors};
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn create_test_skill(name: &str, keywords: Vec<&str>) -> Skill {
        Skill {
            name: name.to_string(),
            description: "Test".to_string(),
            context: "Test".to_string(),
            keywords: keywords.into_iter().map(|s| s.to_string()).collect(),
            behaviors: SkillBehaviors {
                always: vec![],
                never: vec![],
            },
            preferred_tools: vec![],
            examples: vec![],
            file_path: PathBuf::new(),
            last_modified: SystemTime::now(),
            user_invocable: true,
            disable_model_invocation: false,
            internal: false,
            has_scripts: false,
            has_references: false,
            has_assets: false,
            license: None,
            version: None,
            compatibility: None,
            full_content_loaded: true,
            model: None,
            effort: None,
            dependencies: vec![],
        }
    }

    #[test]
    fn test_detect_coder() {
        let coder = create_test_skill("coder", vec!["rust", "código", "programar", "debug"]);
        let detector = SkillDetector::new(&[coder]);

        assert_eq!(
            detector.detect("Me ajude com esse código Rust", None),
            Some("coder".to_string())
        );
    }
}
