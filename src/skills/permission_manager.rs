use crate::skills::Skill;
use crate::tools::Tool;
use std::collections::HashSet;
use tracing::debug;

pub struct ToolPermissionManager;

impl ToolPermissionManager {
    pub fn new() -> Self {
        Self
    }

    pub fn can_use_tool(skill: &Skill, tool_name: &str) -> bool {
        if skill.preferred_tools.is_empty() {
            return true;
        }

        let tool_lower = tool_name.to_lowercase();
        skill.preferred_tools.iter().any(|t| {
            let t_lower = t.to_lowercase();
            tool_lower == t_lower
                || tool_lower.starts_with(&t_lower)
                || t_lower.starts_with(&tool_lower)
        })
    }

    pub fn get_suggested_tools(skill: &Skill) -> Vec<String> {
        if skill.preferred_tools.is_empty() {
            vec![]
        } else {
            skill.preferred_tools.clone()
        }
    }

    pub fn filter_tools_by_skill(
        current_skill: Option<&Skill>,
        tool_names: &[String],
    ) -> Vec<String> {
        match current_skill {
            Some(skill) => {
                if skill.preferred_tools.is_empty() {
                    return tool_names.to_vec();
                }

                let suggested: HashSet<String> = skill
                    .preferred_tools
                    .iter()
                    .map(|t| t.to_lowercase())
                    .collect();

                let filtered: Vec<String> = tool_names
                    .iter()
                    .filter(|name| {
                        let name_lower = name.to_lowercase();
                        suggested.contains(&name_lower)
                            || suggested.iter().any(|s| name_lower.starts_with(s))
                    })
                    .cloned()
                    .collect();

                if filtered.is_empty() {
                    debug!(
                        "No matching tools found for skill '{}', returning all",
                        skill.name
                    );
                    tool_names.to_vec()
                } else {
                    debug!(
                        "Filtered tools for skill '{}': {} of {}",
                        skill.name,
                        filtered.len(),
                        tool_names.len()
                    );
                    filtered
                }
            }
            None => tool_names.to_vec(),
        }
    }

    pub fn format_tool_suggestions(skill: &Skill) -> String {
        let tools = Self::get_suggested_tools(skill);
        if tools.is_empty() {
            String::new()
        } else {
            format!("Ferramentas sugeridas: {}", tools.join(", "))
        }
    }
}

impl Default for ToolPermissionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::SkillBehaviors;
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn create_test_skill(name: &str, preferred_tools: Vec<String>) -> Skill {
        Skill {
            name: name.to_string(),
            description: "Test".to_string(),
            context: "Test".to_string(),
            keywords: vec![],
            behaviors: SkillBehaviors {
                always: vec![],
                never: vec![],
            },
            preferred_tools,
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
        }
    }

    #[test]
    fn test_empty_preferred_tools_allows_all() {
        let skill = create_test_skill("test", vec![]);
        assert!(ToolPermissionManager::can_use_tool(&skill, "any_tool"));
    }

    #[test]
    fn test_exact_match() {
        let skill = create_test_skill("test", vec!["Read".to_string()]);
        assert!(ToolPermissionManager::can_use_tool(&skill, "Read"));
    }

    #[test]
    fn test_case_insensitive() {
        let skill = create_test_skill("test", vec!["Read".to_string()]);
        assert!(ToolPermissionManager::can_use_tool(&skill, "read"));
    }
}
