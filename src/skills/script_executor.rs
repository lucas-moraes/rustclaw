use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, error};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ScriptInfo {
    pub name: String,
    pub path: PathBuf,
    pub language: ScriptLanguage,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScriptLanguage {
    Bash,
    Python,
    Unknown,
}

impl ScriptLanguage {
    #[allow(dead_code)]
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "sh" | "bash" => ScriptLanguage::Bash,
            "py" | "python" => ScriptLanguage::Python,
            _ => ScriptLanguage::Unknown,
        }
    }

    pub fn from_filename(filename: &str) -> Self {
        let lower = filename.to_lowercase();
        if lower.ends_with(".sh") || lower.ends_with(".bash") {
            ScriptLanguage::Bash
        } else if lower.ends_with(".py") || lower.ends_with(".python") {
            ScriptLanguage::Python
        } else {
            ScriptLanguage::Unknown
        }
    }
}

pub struct SkillScriptExecutor {
    skills_dir: PathBuf,
}

impl SkillScriptExecutor {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self { skills_dir }
    }

    pub fn list_scripts(&self, skill_name: &str) -> Vec<ScriptInfo> {
        let skill_dir = self.skills_dir.join(skill_name).join("scripts");
        if !skill_dir.exists() {
            return vec![];
        }

        let mut scripts = vec![];
        if let Ok(entries) = std::fs::read_dir(&skill_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                    let language = ScriptLanguage::from_filename(filename);
                    if language != ScriptLanguage::Unknown {
                        scripts.push(ScriptInfo {
                            name: filename.to_string(),
                            path,
                            language,
                            description: None,
                        });
                    }
                }
            }
        }

        debug!("Found {} scripts for skill '{}'", scripts.len(), skill_name);
        scripts
    }

    pub fn get_script(&self, skill_name: &str, script_name: &str) -> Option<ScriptInfo> {
        let scripts = self.list_scripts(skill_name);
        scripts
            .into_iter()
            .find(|s| s.name == script_name || s.name == format!("{}.sh", script_name))
    }

    pub fn execute(
        &self,
        skill_name: &str,
        script_name: &str,
        args: Vec<String>,
    ) -> Result<String, String> {
        let script = self.get_script(skill_name, script_name).ok_or_else(|| {
            format!(
                "Script '{}' not found in skill '{}'",
                script_name, skill_name
            )
        })?;

        debug!(
            "Executing script '{}' from skill '{}'",
            script_name, skill_name
        );

        match script.language {
            ScriptLanguage::Bash => self.execute_bash(&script.path, args),
            ScriptLanguage::Python => self.execute_python(&script.path, args),
            ScriptLanguage::Unknown => Err("Unknown script language".to_string()),
        }
    }

    fn execute_bash(&self, script_path: &Path, args: Vec<String>) -> Result<String, String> {
        let output = Command::new("bash")
            .arg(script_path)
            .args(&args)
            .output()
            .map_err(|e| format!("Failed to execute bash script: {}", e))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            debug!("Script stdout: {}", stdout);
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            error!("Script error: {}", stderr);
            Err(stderr)
        }
    }

    fn execute_python(&self, script_path: &Path, args: Vec<String>) -> Result<String, String> {
        let output = Command::new("python3")
            .arg(script_path)
            .args(&args)
            .output()
            .map_err(|e| format!("Failed to execute python script: {}", e))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            debug!("Script stdout: {}", stdout);
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            error!("Script error: {}", stderr);
            Err(stderr)
        }
    }

    pub fn validate_script(&self, skill_name: &str, script_name: &str) -> bool {
        self.get_script(skill_name, script_name).is_some()
    }

    pub fn has_scripts(&self, skill_name: &str) -> bool {
        !self.list_scripts(skill_name).is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_language_detection() {
        assert_eq!(
            ScriptLanguage::from_filename("script.sh"),
            ScriptLanguage::Bash
        );
        assert_eq!(
            ScriptLanguage::from_filename("script.bash"),
            ScriptLanguage::Bash
        );
        assert_eq!(
            ScriptLanguage::from_filename("script.py"),
            ScriptLanguage::Python
        );
        assert_eq!(
            ScriptLanguage::from_filename("script.python"),
            ScriptLanguage::Python
        );
        assert_eq!(
            ScriptLanguage::from_filename("script.txt"),
            ScriptLanguage::Unknown
        );
    }
}
