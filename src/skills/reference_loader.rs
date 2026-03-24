use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, error};

#[derive(Debug, Clone)]
pub struct ReferenceInfo {
    pub name: String,
    pub path: PathBuf,
    pub description: Option<String>,
}

pub struct SkillReferenceLoader {
    skills_dir: PathBuf,
    cache: HashMap<String, String>,
}

impl SkillReferenceLoader {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            cache: HashMap::new(),
        }
    }

    pub fn list_references(&self, skill_name: &str) -> Vec<ReferenceInfo> {
        let references_dir = self.skills_dir.join(skill_name).join("references");
        if !references_dir.exists() {
            return vec![];
        }

        let mut references = vec![];
        if let Ok(entries) = fs::read_dir(&references_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                    if filename.ends_with(".md") || filename.ends_with(".txt") {
                        references.push(ReferenceInfo {
                            name: filename.to_string(),
                            path,
                            description: None,
                        });
                    }
                }
            }
        }

        debug!(
            "Found {} references for skill '{}'",
            references.len(),
            skill_name
        );
        references
    }

    pub fn get_reference(&self, skill_name: &str, ref_name: &str) -> Option<ReferenceInfo> {
        let references = self.list_references(skill_name);
        references.into_iter().find(|r| {
            r.name == ref_name
                || r.name == format!("{}.md", ref_name)
                || r.name == format!("{}.txt", ref_name)
        })
    }

    pub fn load_reference(&mut self, skill_name: &str, ref_name: &str) -> Result<String, String> {
        let cache_key = format!("{}:{}", skill_name, ref_name);

        if let Some(cached) = self.cache.get(&cache_key) {
            debug!(
                "Returning cached reference '{}' for skill '{}'",
                ref_name, skill_name
            );
            return Ok(cached.clone());
        }

        let reference = self.get_reference(skill_name, ref_name).ok_or_else(|| {
            format!(
                "Reference '{}' not found in skill '{}'",
                ref_name, skill_name
            )
        })?;

        let content = fs::read_to_string(&reference.path)
            .map_err(|e| format!("Failed to read reference file: {}", e))?;

        self.cache.insert(cache_key, content.clone());
        debug!("Loaded reference '{}' for skill '{}'", ref_name, skill_name);

        Ok(content)
    }

    pub fn has_references(&self, skill_name: &str) -> bool {
        !self.list_references(skill_name).is_empty()
    }

    pub fn resolve_at_references(
        &mut self,
        skill_name: &str,
        markdown: &str,
    ) -> Result<String, String> {
        let mut result = markdown.to_string();
        let pattern = regex::Regex::new(r"@(\S+?)(?:\.md|\.txt)?(?:\s|$)").unwrap();

        let matches: Vec<_> = pattern.captures_iter(markdown).collect();

        for captures in matches {
            if let Some(full_match) = captures.get(0) {
                let ref_name = captures.get(1).map(|m| m.as_str()).unwrap_or("");

                if !ref_name.is_empty() {
                    match self.load_reference(skill_name, ref_name) {
                        Ok(content) => {
                            let replacement = format!("\n```\n{}\n```\n", content.trim());
                            result = result.replace(full_match.as_str(), &replacement);
                        }
                        Err(e) => {
                            error!("Failed to resolve reference '{}': {}", ref_name, e);
                        }
                    }
                }
            }
        }

        Ok(result)
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    pub fn clear_skill_cache(&mut self, skill_name: &str) {
        self.cache.retain(|key, _| !key.starts_with(skill_name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reference_pattern() {
        let pattern = regex::Regex::new(r"@(\S+?)(?:\.md|\.txt)?(?:\s|$)").unwrap();

        assert!(pattern.is_match("@reference"));
        assert!(pattern.is_match("@reference.md"));
        assert!(pattern.is_match("@reference.txt"));
        assert!(pattern.is_match("@reference "));
        assert!(pattern.is_match("@reference\n"));
    }
}
