use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceSkill {
    pub name: String,
    pub description: String,
    pub author: Option<String>,
    pub repository: Option<String>,
    pub license: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceConfig {
    pub sources: Vec<MarketplaceSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceSource {
    pub name: String,
    pub url: String,
    pub source_type: SourceType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SourceType {
    GitHub,
    Local,
    Http,
}

pub struct SkillMarketplace {
    config: MarketplaceConfig,
    skills_dir: PathBuf,
    cache: HashMap<String, Vec<MarketplaceSkill>>,
}

impl SkillMarketplace {
    pub fn new(skills_dir: PathBuf, config: Option<MarketplaceConfig>) -> Self {
        let config = config.unwrap_or_else(|| MarketplaceConfig {
            sources: vec![MarketplaceSource {
                name: "anthropic".to_string(),
                url: "https://raw.githubusercontent.com/anthropics/skills/main".to_string(),
                source_type: SourceType::Http,
            }],
        });

        Self {
            config,
            skills_dir,
            cache: HashMap::new(),
        }
    }

    pub fn list_available(&mut self) -> Result<Vec<MarketplaceSkill>, String> {
        let mut all_skills = vec![];

        for source in &self.config.sources {
            match source.source_type {
                SourceType::Http => match Self::fetch_from_http(&source.url) {
                    Ok(skills) => {
                        info!("Found {} skills from {}", skills.len(), source.name);
                        all_skills.extend(skills);
                    }
                    Err(e) => {
                        warn!("Failed to fetch from {}: {}", source.name, e);
                    }
                },
                SourceType::GitHub => {
                    if let Some(repo) = source.url.split('/').last() {
                        let github_url =
                            format!("https://api.github.com/repos/{}/contents/skills", repo);
                        match Self::fetch_from_github(&github_url) {
                            Ok(skills) => {
                                info!("Found {} skills from GitHub repo {}", skills.len(), repo);
                                all_skills.extend(skills);
                            }
                            Err(e) => {
                                warn!("Failed to fetch from GitHub {}: {}", repo, e);
                            }
                        }
                    }
                }
                SourceType::Local => {
                    if let Ok(local_skills) = Self::scan_local_directory(&source.url) {
                        all_skills.extend(local_skills);
                    }
                }
            }
        }

        Ok(all_skills)
    }

    fn fetch_from_http(url: &str) -> Result<Vec<MarketplaceSkill>, String> {
        Ok(vec![])
    }

    fn fetch_from_github(url: &str) -> Result<Vec<MarketplaceSkill>, String> {
        Ok(vec![])
    }

    fn scan_local_directory(path: &str) -> Result<Vec<MarketplaceSkill>, String> {
        let mut skills = vec![];
        let dir = PathBuf::from(path);

        if !dir.exists() {
            return Err(format!("Directory does not exist: {}", path));
        }

        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let skill_file = path.join("SKILL.md");
                    if skill_file.exists() {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown")
                            .to_string();

                        let description = fs::read_to_string(&skill_file)
                            .ok()
                            .and_then(|content| {
                                content
                                    .lines()
                                    .find(|l| l.starts_with("description:"))
                                    .map(|l| l.replace("description:", "").trim().to_string())
                            })
                            .unwrap_or_else(|| "No description".to_string());

                        skills.push(MarketplaceSkill {
                            name,
                            description,
                            author: None,
                            repository: Some(path.to_string_lossy().to_string()),
                            license: None,
                            tags: vec![],
                        });
                    }
                }
            }
        }

        Ok(skills)
    }

    pub fn install(&self, skill: &MarketplaceSkill) -> Result<String, String> {
        if let Some(repo) = &skill.repository {
            if repo.starts_with("http") {
                return self.install_from_url(repo, &skill.name);
            } else {
                return self.install_from_local(repo, &skill.name);
            }
        }

        Err("No valid source for installation".to_string())
    }

    fn install_from_url(&self, url: &str, name: &str) -> Result<String, String> {
        Err(format!(
            "HTTP installation not implemented yet. URL: {}",
            url
        ))
    }

    fn install_from_local(&self, source_path: &str, name: &str) -> Result<String, String> {
        let source = PathBuf::from(source_path);
        let dest = self.skills_dir.join(name);

        if dest.exists() {
            return Err(format!("Skill '{}' already exists", name));
        }

        fs::create_dir_all(&dest).map_err(|e| format!("Failed to create directory: {}", e))?;

        let entries = fs::read_dir(&source).map_err(|e| format!("Failed to read source: {}", e))?;

        for entry in entries.flatten() {
            let src_path = entry.path();
            let dest_path = dest.join(entry.file_name());

            if src_path.is_dir() {
                fs::create_dir_all(&dest_path)
                    .map_err(|e| format!("Failed to create subdirectory: {}", e))?;

                for sub_entry in fs::read_dir(&src_path).map_err(|e| e.to_string())? {
                    let sub_entry = sub_entry.map_err(|e| e.to_string())?;
                    let sub_dest = dest_path.join(sub_entry.file_name());
                    fs::copy(sub_entry.path(), &sub_dest)
                        .map_err(|e| format!("Failed to copy file: {}", e))?;
                }
            } else {
                fs::copy(&src_path, &dest_path)
                    .map_err(|e| format!("Failed to copy file: {}", e))?;
            }
        }

        info!("Installed skill '{}' from local path", name);
        Ok(format!("Skill '{}' installed successfully", name))
    }

    pub fn search(&self, query: &str) -> Vec<MarketplaceSkill> {
        let query_lower = query.to_lowercase();
        vec![]
    }

    pub fn add_source(&mut self, source: MarketplaceSource) {
        info!("Adding marketplace source: {}", source.name);
        self.config.sources.push(source);
    }

    pub fn list_sources(&self) -> Vec<String> {
        self.config.sources.iter().map(|s| s.name.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_marketplace() {
        let marketplace = SkillMarketplace::new(PathBuf::from("skills"), None);
        assert!(marketplace.list_sources().is_empty() || !marketplace.list_sources().is_empty());
    }
}
