use crate::skills::Skill;
use std::collections::HashMap;
use tracing::debug;

pub struct SkillDetector {
    keyword_map: HashMap<String, Vec<String>>, // keyword -> [skill_names]
    confidence_threshold: f32,
}

impl SkillDetector {
    pub fn new(skills: &[Skill]) -> Self {
        let mut keyword_map: HashMap<String, Vec<String>> = HashMap::new();

        for skill in skills {
            let keywords = skill.combined_keywords();
            for keyword in keywords {
                keyword_map
                    .entry(keyword)
                    .or_default()
                    .push(skill.name.clone());
            }
        }

        debug!("Built keyword map with {} keywords", keyword_map.len());

        Self {
            keyword_map,
            confidence_threshold: 0.3, // Ajustável
        }
    }

    pub fn detect(&self, message: &str, active_skill: Option<&str>) -> Option<String> {
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
