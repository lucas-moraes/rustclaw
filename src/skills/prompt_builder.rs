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
            prompt.push_str(&format!(
                "\n\n═══════════════════════════════════════════════════\n"
            ));
            prompt.push_str(&format!("🎭 MODO ATIVO: {}\n", skill.name.to_uppercase()));
            prompt.push_str(&format!(
                "═══════════════════════════════════════════════════\n\n"
            ));

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
                    prompt.push_str("\n");
                }

                if !skill.behaviors.never.is_empty() {
                    prompt.push_str("❌ VOCÊ NUNCA DEVE:\n");
                    for item in &skill.behaviors.never {
                        prompt.push_str(&format!("  • {}\n", item));
                    }
                    prompt.push_str("\n");
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
                    prompt.push_str("\n");
                }
            }

            prompt.push_str("═══════════════════════════════════════════════════\n\n");
        }

        // Adiciona ferramentas e memória
        prompt.push_str("FERRAMENTAS DISPONÍVEIS:\n");
        prompt.push_str(tool_list);
        prompt.push_str("\n\n");
        prompt.push_str(memory_context);

        prompt
    }
}
