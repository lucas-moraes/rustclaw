use crate::security::SecurityManager;
use crate::skills::{Skill, SkillBehaviors, SkillExample};
use regex::Regex;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct SkillParser;

#[derive(Debug, Deserialize)]
struct YamlFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    allowed_tools: Vec<String>,
    #[serde(default)]
    user_invocable: bool,
    #[serde(default)]
    disable_model_invocation: bool,
    #[serde(default)]
    internal: bool,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    compatibility: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    metadata: YamlMetadata,
}

#[derive(Debug, Deserialize, Default)]
struct YamlMetadata {
    #[serde(default)]
    internal: bool,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    MissingField(String),
    #[allow(dead_code)]
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

        // Detect format: YAML frontmatter (Claude Code format) or Markdown (Legacy RustClaw)
        if Self::has_yaml_frontmatter(&content) {
            Self::parse_yaml_format(&content, path, &metadata)
        } else {
            Self::parse_markdown_format(&content, path, &metadata)
        }
    }

    pub fn supports_cli_invocation(skill: &Skill) -> bool {
        skill.user_invocable
    }

    pub fn allows_auto_invocation(skill: &Skill) -> bool {
        !skill.disable_model_invocation
    }

    fn has_yaml_frontmatter(content: &str) -> bool {
        content.trim().starts_with("---")
    }

    fn detect_resource_directories(skill_dir: &Path) -> (bool, bool, bool) {
        let mut has_scripts = false;
        let mut has_references = false;
        let mut has_assets = false;

        if let Ok(entries) = fs::read_dir(skill_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy().to_lowercase();
                match name_str.as_str() {
                    "scripts" => has_scripts = entry.path().is_dir(),
                    "references" => has_references = entry.path().is_dir(),
                    "assets" => has_assets = entry.path().is_dir(),
                    _ => {}
                }
            }
        }

        (has_scripts, has_references, has_assets)
    }

    fn get_skill_directory(skill_file_path: &Path) -> PathBuf {
        skill_file_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default()
    }

    fn parse_yaml_format(
        content: &str,
        path: &Path,
        file_meta: &std::fs::Metadata,
    ) -> Result<Skill, ParseError> {
        // Extract YAML frontmatter between --- markers
        let trimmed = content.trim();
        if !trimmed.starts_with("---") {
            return Err(ParseError::InvalidFormat(
                "Expected YAML frontmatter".to_string(),
            ));
        }

        let start = trimmed[3..]
            .find("---")
            .ok_or_else(|| ParseError::InvalidFormat("Missing closing ---".to_string()))?;
        let yaml_content = &trimmed[3..start + 3];
        let markdown_content = &trimmed[start + 6..];

        // Parse YAML
        let frontmatter: YamlFrontmatter = serde_yaml::from_str(yaml_content)
            .map_err(|e| ParseError::InvalidFormat(format!("Invalid YAML: {}", e)))?;

        let name = frontmatter
            .name
            .ok_or_else(|| ParseError::MissingField("name".to_string()))?;
        let description = frontmatter.description.unwrap_or_default();

        // Use markdown content as context, or build from description if empty
        let context = if markdown_content.trim().is_empty() {
            description.clone()
        } else {
            Self::extract_context_from_markdown(markdown_content)
        };

        // Extract keywords from markdown content
        let keywords = Self::extract_keywords_from_markdown(markdown_content);

        // Convert allowed-tools to preferred_tools
        let preferred_tools = frontmatter.allowed_tools;

        let behaviors = SkillBehaviors {
            always: vec!["Follow the guidelines in the skill context".to_string()],
            never: vec!["Ignore the skill context".to_string()],
        };

        // Detect resource directories
        let skill_dir = Self::get_skill_directory(path);
        let (has_scripts, has_references, has_assets) =
            Self::detect_resource_directories(&skill_dir);

        // Get version from frontmatter or metadata
        let version = frontmatter
            .version
            .clone()
            .or_else(|| frontmatter.metadata.version.clone());

        Ok(Skill {
            name,
            description,
            context,
            keywords,
            behaviors,
            preferred_tools,
            examples: vec![],
            file_path: path.to_path_buf(),
            last_modified: file_meta.modified().unwrap_or(SystemTime::now()),
            user_invocable: frontmatter.user_invocable,
            disable_model_invocation: frontmatter.disable_model_invocation,
            internal: frontmatter.internal,
            has_scripts,
            has_references,
            has_assets,
            license: frontmatter.license,
            version,
            compatibility: frontmatter.compatibility,
            full_content_loaded: true,
            model: frontmatter.model,
            effort: frontmatter.effort,
            dependencies: frontmatter.dependencies,
        })
    }

    fn parse_markdown_format(
        content: &str,
        path: &Path,
        file_meta: &std::fs::Metadata,
    ) -> Result<Skill, ParseError> {
        let name = Self::extract_name(content)?;
        let description = Self::extract_section(content, "Descrição")?;
        let context = Self::extract_section(content, "Contexto")?;
        let keywords = Self::extract_keywords(content);
        let behaviors = SkillBehaviors {
            always: Self::extract_behaviors(content, "SEMPRE"),
            never: Self::extract_behaviors(content, "NUNCA"),
        };
        let preferred_tools = Self::extract_tools(content);
        let examples = Self::extract_examples(content);

        // Detect resource directories
        let skill_dir = Self::get_skill_directory(path);
        let (has_scripts, has_references, has_assets) =
            Self::detect_resource_directories(&skill_dir);

        Ok(Skill {
            name,
            description,
            context,
            keywords,
            behaviors,
            preferred_tools,
            examples,
            file_path: path.to_path_buf(),
            last_modified: file_meta.modified().unwrap_or(SystemTime::now()),
            user_invocable: true, // Default for markdown format
            disable_model_invocation: false,
            internal: false,
            has_scripts,
            has_references,
            has_assets,
            license: None,
            version: None,
            compatibility: None,
            full_content_loaded: true,
            model: None,
            effort: None,
            dependencies: vec![],
        })
    }

    fn extract_context_from_markdown(markdown: &str) -> String {
        // Remove title (#) and section headers to get context
        let mut context = markdown.to_string();

        // Remove title
        if let Some(pos) = context.find("\n#") {
            context = context[pos + 1..].to_string();
        }

        // Remove all ## sections for cleaner context
        while let Some(pos) = context.find("\n##") {
            if let Some(end) = context[pos + 3..].find("\n##") {
                context = context[..pos].to_string() + &context[pos + 3 + end..];
            } else {
                break;
            }
        }

        let sanitized = SecurityManager::sanitize_skill_context(context.trim());
        sanitized
    }

    fn extract_keywords_from_markdown(markdown: &str) -> Vec<String> {
        // Look for ## Keywords or similar patterns
        let patterns = [
            "## Keywords",
            "## Keywords\n",
            "## when to use",
            "## When to Use",
        ];

        for pattern in patterns {
            if let Some(pos) = markdown.to_lowercase().find(&pattern.to_lowercase()) {
                let start = pos + pattern.len();
                let remaining = &markdown[start..];
                let end = remaining.find("\n## ").unwrap_or(remaining.len());
                let keywords_content = &remaining[..end];

                return keywords_content
                    .lines()
                    .filter_map(|line| {
                        let trimmed = line.trim();
                        if trimmed.starts_with('-') || trimmed.starts_with('*') {
                            trimmed
                                .strip_prefix('-')
                                .or_else(|| trimmed.strip_prefix('*'))
                                .map(|s| s.trim().to_lowercase())
                        } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
                            Some(trimmed.to_lowercase())
                        } else {
                            None
                        }
                    })
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }

        vec![]
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
