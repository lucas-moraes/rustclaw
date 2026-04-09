use crate::skills::script_executor::{ScriptInfo, SkillScriptExecutor};
use crate::skills::Skill;
use crate::tools::Tool;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

pub struct SkillScriptTool {
    executor: Arc<SkillScriptExecutor>,
}

impl SkillScriptTool {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            executor: Arc::new(SkillScriptExecutor::new(skills_dir)),
        }
    }

    #[allow(dead_code)]
    pub fn list_scripts_for_skill(&self, skill: &Skill) -> Vec<ScriptInfo> {
        self.executor.list_scripts(&skill.name)
    }

    #[allow(dead_code)]
    pub fn has_scripts(&self, skill: &Skill) -> bool {
        self.executor.has_scripts(&skill.name)
    }
}

#[async_trait::async_trait]
impl Tool for SkillScriptTool {
    fn name(&self) -> &str {
        "skill_script"
    }

    fn description(&self) -> &str {
        "Executa um script de skill. Input: { \"skill\": \"nome-da-skill\", \"script\": \"nome-script\", \"args\": [\"arg1\", \"arg2\"] }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let skill_name = args["skill"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'skill' é obrigatório".to_string())?;

        let script_name = args["script"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'script' é obrigatório".to_string())?;

        let args_list: Vec<String> = args["args"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        self.executor.execute(skill_name, script_name, args_list)
    }
}

pub struct SkillScriptsListTool {
    executor: Arc<SkillScriptExecutor>,
}

impl SkillScriptsListTool {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            executor: Arc::new(SkillScriptExecutor::new(skills_dir)),
        }
    }
}

#[async_trait::async_trait]
impl Tool for SkillScriptsListTool {
    fn name(&self) -> &str {
        "skill_scripts_list"
    }

    fn description(&self) -> &str {
        "Lista todos os scripts disponíveis numa skill. Input: { \"skill\": \"nome-da-skill\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let skill_name = args["skill"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'skill' é obrigatório".to_string())?;

        let scripts = self.executor.list_scripts(skill_name);

        if scripts.is_empty() {
            return Ok(format!(
                "Nenhum script encontrado na skill '{}'",
                skill_name
            ));
        }

        let list: Vec<String> = scripts
            .iter()
            .map(|s| {
                format!(
                    "- {} ({})",
                    s.name,
                    match s.language {
                        crate::skills::script_executor::ScriptLanguage::Bash => "bash",
                        crate::skills::script_executor::ScriptLanguage::Python => "python",
                        crate::skills::script_executor::ScriptLanguage::Unknown => "unknown",
                    }
                )
            })
            .collect();

        Ok(format!(
            "Scripts disponíveis em '{}':\n{}",
            skill_name,
            list.join("\n")
        ))
    }
}
