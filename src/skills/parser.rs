use crate::security::SecurityManager;
use crate::skills::{Skill, SkillBehaviors, SkillExample};
use regex::Regex;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

pub struct SkillParser;

#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    MissingField(String),
    InvalidFormat(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Io(e) => write!(f, "IO error: {}", e),
            ParseError::MissingField(field) => write!(f, "Missing field: {}", field),
            ParseError::InvalidFormat(msg) => write!(f, "Invalid format: {}", msg),
        }
    }
}

impl std::error::Error for ParseError {}

impl From<std::io::Error> for ParseError {
    fn from(e: std::io::Error) -> Self {
        ParseError::Io(e)
    }
}

impl SkillParser {
    pub fn parse(path: &Path) -> Result<Skill, ParseError> {
        let content = fs::read_to_string(path)?;
        let metadata = fs::metadata(path)?;

        let name = Self::extract_name(&content)?;
        let description = Self::extract_section(&content, "Descrição")?;
        let context = Self::extract_section(&content, "Contexto")?;
        let keywords = Self::extract_keywords(&content);
        let behaviors = SkillBehaviors {
            always: Self::extract_behaviors(&content, "SEMPRE"),
            never: Self::extract_behaviors(&content, "NUNCA"),
        };
        let preferred_tools = Self::extract_tools(&content);
        let examples = Self::extract_examples(&content);

        Ok(Skill {
            name,
            description,
            context,
            keywords,
            behaviors,
            preferred_tools,
            examples,
            file_path: path.to_path_buf(),
            last_modified: metadata.modified().unwrap_or(SystemTime::now()),
        })
    }

    fn extract_name(content: &str) -> Result<String, ParseError> {
        // Use multiline mode to match end of line properly
        let re = Regex::new(r"(?m)^#\s*Skill:\s*(.+)$").unwrap();
        re.captures(content)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().trim().to_string())
            .ok_or_else(|| ParseError::MissingField("Nome da skill (título)".to_string()))
    }

    fn extract_section(content: &str, section: &str) -> Result<String, ParseError> {
        // Split by ## to find the section
        let section_header = format!("## {}", section);

        if let Some(pos) = content.find(&section_header) {
            let start = pos + section_header.len();
            let remaining = &content[start..];

            // Find the next section or end of content
            let end = remaining.find("\n## ").unwrap_or(remaining.len());
            let section_content = &remaining[..end];

            // Remove leading newlines and trim
            let content = section_content.trim_start_matches('\n').trim();

            if !content.is_empty() {
                // SECURITY: Sanitize skill context
                let sanitized = SecurityManager::sanitize_skill_context(content);
                return Ok(sanitized);
            }
        }

        Err(ParseError::MissingField(format!("Seção '{}'", section)))
    }

    fn extract_keywords(content: &str) -> Vec<String> {
        let section_header = "## Keywords";

        if let Some(pos) = content.find(section_header) {
            let start = pos + section_header.len();
            let remaining = &content[start..];

            // Find end of keywords section (next ## section)
            let end = remaining.find("\n## ").unwrap_or(remaining.len());
            let keywords_content = &remaining[..end];

            return keywords_content
                .lines()
                .filter_map(|line| {
                    line.trim()
                        .strip_prefix("- ")
                        .map(|s| s.trim().to_lowercase().to_string())
                })
                .collect();
        }

        vec![]
    }

    fn extract_behaviors(content: &str, behavior_type: &str) -> Vec<String> {
        // Find the behavior section (e.g., "### SEMPRE" or "### NUNCA")
        let patterns = [
            format!("### {} (✅)", behavior_type),
            format!("### {} (❌)", behavior_type),
            format!("### {}", behavior_type),
        ];

        for pattern in &patterns {
            if let Some(pos) = content.find(pattern) {
                let start = pos + pattern.len();
                let remaining = &content[start..];

                // Find end of this subsection (next ### or ##)
                let end = remaining
                    .find("\n### ")
                    .or_else(|| remaining.find("\n## "))
                    .unwrap_or(remaining.len());
                let behavior_content = &remaining[..end];

                return behavior_content
                    .lines()
                    .filter_map(|line| line.trim().strip_prefix("- ").map(|s| s.trim().to_string()))
                    .collect();
            }
        }

        vec![]
    }

    fn extract_tools(content: &str) -> Vec<String> {
        let section_header = "## Ferramentas Prioritárias";

        if let Some(pos) = content.find(section_header) {
            let start = pos + section_header.len();
            let remaining = &content[start..];

            // Find end of section
            let end = remaining.find("\n## ").unwrap_or(remaining.len());
            let tools_content = &remaining[..end];

            return tools_content
                .lines()
                .filter_map(|line| {
                    let trimmed = line.trim();
                    trimmed
                        .split_once(". ")
                        .map(|(_, tool)| tool.trim().to_string())
                })
                .collect();
        }

        vec![]
    }

    fn extract_examples(_content: &str) -> Vec<SkillExample> {
        // Examples extraction is optional - return empty for now
        // This would require complex parsing without look-behind regex
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_name() {
        let content = "# Skill: Coder\n\n## Descrição\nTeste";
        assert_eq!(SkillParser::extract_name(content).unwrap(), "Coder");
    }

    #[test]
    fn test_extract_keywords() {
        let content = "## Keywords\n- rust\n- código\n- programar\n\n## Outra";
        let keywords = SkillParser::extract_keywords(content);
        assert_eq!(keywords, vec!["rust", "código", "programar"]);
    }
}
