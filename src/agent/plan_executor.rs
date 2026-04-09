#![allow(dead_code)]

use crate::utils::build_detector::BuildDetector;
use crate::utils::error_parser::{BuildValidation, ErrorParser};

pub struct BuildValidator;

impl BuildValidator {
    pub async fn validate_build(
        tools: &crate::tools::ToolRegistry,
        project_dir: &str,
    ) -> anyhow::Result<BuildValidation> {
        let build_info = BuildDetector::detect(project_dir);

        if build_info.build_command.is_empty() {
            tracing::info!(
                "No build command detected for {}, skipping validation",
                project_dir
            );
            return Ok(BuildValidation::Success);
        }

        tracing::info!(
            "Running build command: {} in {}",
            build_info.build_command,
            project_dir
        );

        let shell_tool = tools
            .get("shell")
            .ok_or_else(|| anyhow::anyhow!("Shell tool not found"))?;

        let args = serde_json::json!({
            "command": build_info.build_command
        });

        let build_result = shell_tool
            .call(args)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let success = !build_result.contains("❌ Erro");

        if success {
            tracing::info!("Build successful for {}", project_dir);
            return Ok(BuildValidation::Success);
        }

        tracing::info!("Build failed, parsing errors...");
        let project_type = format!("{:?}", build_info.project_type);
        let validation = ErrorParser::parse(&build_result, &project_type);

        Ok(validation)
    }
}

pub struct PlanExecutor;

impl PlanExecutor {
    pub fn count_plan_steps(plan: &str) -> usize {
        plan.lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                trimmed.starts_with(|c: char| c.is_ascii_digit()) && trimmed.contains(')')
            })
            .count()
    }

    pub fn update_plan_progress(
        plan_file: &str,
        _steps: &[String],
        completed: &[usize],
    ) -> anyhow::Result<()> {
        if plan_file.is_empty() || !std::path::Path::new(plan_file).exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(plan_file)?;

        let re = regex::Regex::new(r"(?m)^(\s*\d+)\.\s*(\[[ xX]\])\s+(.*)$")?;

        let updated = re
            .replace_all(&content, |caps: &regex::Captures| {
                let number = &caps[1];
                let step_text = &caps[3];
                let step_idx: usize = number
                    .trim()
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_sub(1);

                if completed.contains(&step_idx) {
                    format!("{}. [x] {}", number, step_text)
                } else {
                    format!("{}. [ ] {}", number, step_text)
                }
            })
            .to_string();

        std::fs::write(plan_file, updated)?;

        Ok(())
    }

    pub async fn generate_plan(
        call_llm_fn: impl Fn(serde_json::Value) -> anyhow::Result<String>,
        user_input: &str,
    ) -> anyhow::Result<String> {
        let plan_prompt = format!(
            "Voce e um planejador. Crie um plano em passos numerados, conciso e executavel, para a tarefa abaixo.\n\nTarefa: {}\n\nRegras:\n- Use 5-10 passos\n- Cada passo deve ser uma acao concreta\n- Nao execute nada, apenas planeje\n\nFormato:\n1) ...\n2) ...\n3) ...",
            user_input
        );

        let messages = serde_json::json!([
            {
                "role": "user",
                "content": plan_prompt
            }
        ]);

        let response = call_llm_fn(messages)?;
        Ok(response.trim().to_string())
    }
}
