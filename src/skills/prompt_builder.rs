use crate::skills::Skill;

pub struct SkillPromptBuilder;

impl SkillPromptBuilder {
    pub fn build(
        base_prompt: &str,
        skill: Option<&Skill>,
        tool_list: &str,
        memory_context: &str,
    ) -> String {
        let mut prompt = base_prompt.to_string();

        if let Some(skill) = skill {
            prompt.push_str("\n\n═══════════════════════════════════════════════════\n");
            prompt.push_str(&format!("🎭 MODO ATIVO: {}\n", skill.name.to_uppercase()));
            prompt.push_str("═══════════════════════════════════════════════════\n\n");

            // Contexto da skill
            prompt.push_str(&skill.context);
            prompt.push_str("\n\n");

            // Diretrizes de comportamento
            if !skill.behaviors.always.is_empty() || !skill.behaviors.never.is_empty() {
                prompt.push_str("### DIRETRIZES DE COMPORTAMENTO\n\n");

                if !skill.behaviors.always.is_empty() {
                    prompt.push_str("✅ VOCÊ DEVE SEMPRE:\n");
                    for item in &skill.behaviors.always {
                        prompt.push_str(&format!("  • {}\n", item));
                    }
                    prompt.push('\n');
                }

                if !skill.behaviors.never.is_empty() {
                    prompt.push_str("❌ VOCÊ NUNCA DEVE:\n");
                    for item in &skill.behaviors.never {
                        prompt.push_str(&format!("  • {}\n", item));
                    }
                    prompt.push('\n');
                }
            }

            // Exemplos
            if !skill.examples.is_empty() {
                prompt.push_str("### EXEMPLOS DE ESTILO\n\n");
                for (i, example) in skill.examples.iter().enumerate() {
                    prompt.push_str(&format!("Exemplo {}:\n", i + 1));
                    prompt.push_str(&format!("  Input: {}\n", example.input));
                    prompt.push_str(&format!(
                        "  ✅ Bom: {}\n",
                        example.good.chars().take(100).collect::<String>()
                    ));
                    if !example.bad.is_empty() {
                        prompt.push_str(&format!(
                            "  ❌ Ruim: {}\n",
                            example.bad.chars().take(100).collect::<String>()
                        ));
                    }
                    prompt.push('\n');
                }
            }

            // Resource directories info
            Self::add_resource_info(&mut prompt, skill);

            prompt.push_str("═══════════════════════════════════════════════════\n\n");
        }

        // Adiciona ferramentas e memória
        prompt.push_str("FERRAMENTAS DISPONÍVEIS:\n");
        prompt.push_str(tool_list);
        prompt.push_str("\n\n");
        prompt.push_str(memory_context);

        prompt
    }

    fn add_resource_info(prompt: &mut String, skill: &Skill) {
        let mut resources = vec![];

        if skill.has_scripts {
            resources.push("scripts/ - Executáveis disponíveis");
        }
        if skill.has_references {
            resources.push("references/ - Documentação de referência");
        }
        if skill.has_assets {
            resources.push("assets/ - Recursos e templates");
        }

        if !resources.is_empty() {
            prompt.push_str("### RECURSOS DISPONÍVEIS\n\n");
            for res in resources {
                prompt.push_str(&format!("  • {}\n", res));
            }
            prompt.push_str("\nUse a ferramenta skill_script para executar scripts.\n");
            prompt.push_str("Use @nome-do-arquivo para referenciar arquivos em references/.\n\n");
        }
    }
}
