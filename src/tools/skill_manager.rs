use super::Tool;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use crate::skills::parser::SkillParser;

const SKILLS_DIR: &str = "skills";

pub struct SkillListTool;

impl SkillListTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for SkillListTool {
    fn name(&self) -> &str {
        "skill_list"
    }

    fn description(&self) -> &str {
        "Lista todas as skills disponíveis. Input: {} (vazio)"
    }

    async fn call(&self, _args: Value) -> Result<String, String> {
        let skills_path = Path::new(SKILLS_DIR);
        
        if !skills_path.exists() {
            return Ok("Diretório de skills não encontrado.".to_string());
        }

        let mut skills = vec![];
        
        if let Ok(entries) = fs::read_dir(skills_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let skill_file = path.join("skill.md");
                    if skill_file.exists() {
                        let skill_name = path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown");
                        
                        // Try to parse and get description
                        let description = match SkillParser::parse(&skill_file) {
                            Ok(skill) => skill.description,
                            Err(_) => "(erro ao carregar)".to_string(),
                        };
                        
                        skills.push(format!("- **{}**: {}", skill_name, description));
                    }
                }
            }
        }

        if skills.is_empty() {
            Ok("Nenhuma skill encontrada.".to_string())
        } else {
            skills.sort();
            Ok(format!("Skills disponíveis ({}):\n{}", skills.len(), skills.join("\n")))
        }
    }
}

impl Default for SkillListTool {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SkillCreateTool;

impl SkillCreateTool {
    pub fn new() -> Self {
        Self
    }

    fn get_skill_template(name: &str) -> String {
        format!(r#"# Skill: {}

## Descrição
Descrição breve do que esta skill faz

## Contexto
Contexto detalhado sobre como o assistente deve se comportar quando esta skill está ativa.
Explique a personalidade, tom de voz, e abordagem recomendada.

## Keywords
- palavra-chave1
- palavra-chave2
- palavra-chave3

## Comportamento

### SEMPRE
- Comportamento obrigatório 1
- Comportamento obrigatório 2

### NUNCA
- Comportamento proibido 1
- Comportamento proibido 2

## Ferramentas Prioritárias
1. tool_name1
2. tool_name2

## Exemplos

### Input: "exemplo de pergunta"
**Bom:** resposta desejada
**Ruim:** resposta a ser evitada

### Input: "outro exemplo"
**Bom:** outra resposta desejada
"#, name)
    }
}

#[async_trait::async_trait]
impl Tool for SkillCreateTool {
    fn name(&self) -> &str {
        "skill_create"
    }

    fn description(&self) -> &str {
        "Cria uma nova skill. Input: { \"name\": \"minha-skill\", \"content\": \"# Skill: ...\" }. Por padrão não valida o formato - use validate:true para validar."
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let name = args["name"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'name' é obrigatório".to_string())?;

        // Validate name
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            return Err("Nome de skill inválido".to_string());
        }

        let skills_path = Path::new(SKILLS_DIR);
        let skill_dir = skills_path.join(name);
        
        if skill_dir.exists() {
            return Err(format!("Skill '{}' já existe", name));
        }

        // Create directory
        fs::create_dir_all(&skill_dir)
            .map_err(|e| format!("Erro ao criar diretório: {}", e))?;

        // Get content - either custom or template
        let content = args["content"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| Self::get_skill_template(name));

        let skill_file = skill_dir.join("skill.md");
        fs::write(&skill_file, &content)
            .map_err(|e| format!("Erro ao escrever arquivo: {}", e))?;

        // Check if validation is requested
        let validate = args["validate"]
            .as_bool()
            .unwrap_or(false);

        if validate {
            // Validate the created skill
            match SkillParser::parse(&skill_file) {
                Ok(_) => Ok(format!("✅ Skill '{}' criada e validada com sucesso em {:?}", name, skill_file)),
                Err(e) => {
                    let _ = fs::remove_dir_all(&skill_dir);
                    Err(format!("Erro de validação: {}. Diretório removido.", e))
                }
            }
        } else {
            Ok(format!("✅ Skill '{}' criada com sucesso em {:?}\n💡 Use validate:true para validar o formato.", name, skill_file))
        }
    }
}

impl Default for SkillCreateTool {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SkillDeleteTool;

impl SkillDeleteTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for SkillDeleteTool {
    fn name(&self) -> &str {
        "skill_delete"
    }

    fn description(&self) -> &str {
        "Remove uma skill existente. Input: { \"name\": \"minha-skill\", \"confirm\": true }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let name = args["name"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'name' é obrigatório".to_string())?;

        let confirmed = args["confirm"].as_bool().unwrap_or(false);
        
        if !confirmed {
            return Err("Adicione 'confirm': true para confirmar a remoção".to_string());
        }

        // Prevent deletion of 'general' skill
        if name == "general" {
            return Err("Não é possível remover a skill 'general'".to_string());
        }

        let skill_dir = Path::new(SKILLS_DIR).join(name);
        
        if !skill_dir.exists() {
            return Err(format!("Skill '{}' não encontrada", name));
        }

        // Create backup before deletion
        let backup_dir = Path::new(SKILLS_DIR).join(format!("{}.backup", name));
        if let Err(e) = fs::rename(&skill_dir, &backup_dir) {
            return Err(format!("Erro ao criar backup: {}", e));
        }

        // Actually delete backup after a while
        let _ = fs::remove_dir_all(&backup_dir);

        Ok(format!("✅ Skill '{}' removida com sucesso", name))
    }
}

impl Default for SkillDeleteTool {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SkillValidateTool;

impl SkillValidateTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for SkillValidateTool {
    fn name(&self) -> &str {
        "skill_validate"
    }

    fn description(&self) -> &str {
        "Valida a sintaxe de uma ou todas as skills. Input: { \"name\": \"minha-skill\" } ou {} para todas"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let skills_path = Path::new(SKILLS_DIR);
        
        if !skills_path.exists() {
            return Ok("Diretório de skills não encontrado.".to_string());
        }

        // Validate specific skill or all
        if let Some(name) = args["name"].as_str() {
            let skill_file = skills_path.join(name).join("skill.md");
            
            if !skill_file.exists() {
                return Err(format!("Skill '{}' não encontrada", name));
            }

            match SkillParser::parse(&skill_file) {
                Ok(skill) => {
                    let info = format!(
                        "✅ Skill '{}' válida\n\nNome: {}\nDescrição: {}\nKeywords: {}\nComportamentos SEMPRE: {}\nComportamentos NUNCA: {}",
                        name,
                        skill.name,
                        skill.description,
                        skill.keywords.join(", "),
                        skill.behaviors.always.len(),
                        skill.behaviors.never.len()
                    );
                    Ok(info)
                }
                Err(e) => Err(format!("❌ Skill '{}' com erro: {}", name, e)),
            }
        } else {
            // Validate all
            let mut valid = vec![];
            let mut invalid = vec![];

            if let Ok(entries) = fs::read_dir(skills_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let skill_file = path.join("skill.md");
                        if skill_file.exists() {
                            let name = path.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("unknown");
                            
                            match SkillParser::parse(&skill_file) {
                                Ok(_) => valid.push(name.to_string()),
                                Err(e) => invalid.push(format!("{}: {}", name, e)),
                            }
                        }
                    }
                }
            }

            let mut result = format!("Validação de {} skills:\n\n", valid.len() + invalid.len());
            
            if !valid.is_empty() {
                result.push_str(&format!("✅ Válidas ({}): {}\n", valid.len(), valid.join(", ")));
            }
            
            if !invalid.is_empty() {
                result.push_str(&format!("\n❌ Com erros ({}):\n{}", invalid.len(), invalid.join("\n")));
            }

            Ok(result)
        }
    }
}

impl Default for SkillValidateTool {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SkillEditTool {
    skills_dir: PathBuf,
}

impl SkillEditTool {
    pub fn new<P: AsRef<Path>>(skills_dir: P) -> Self {
        Self {
            skills_dir: skills_dir.as_ref().to_path_buf(),
        }
    }
}

#[async_trait::async_trait]
impl Tool for SkillEditTool {
    fn name(&self) -> &str {
        "skill_edit"
    }

    fn description(&self) -> &str {
        "Lê conteúdo de uma skill para edição. Input: { \"name\": \"minha-skill\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let name = args["name"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'name' é obrigatório".to_string())?;

        let skill_file = self.skills_dir.join(name).join("skill.md");
        
        if !skill_file.exists() {
            return Err(format!("Skill '{}' não encontrada", name));
        }

        let content = fs::read_to_string(&skill_file)
            .map_err(|e| format!("Erro ao ler arquivo: {}", e))?;

        Ok(format!(
            "Conteúdo atual da skill '{}':\n\n```markdown\n{}\n```\n\nPara editar, use file_write com o caminho: {}",
            name,
            content,
            skill_file.display()
        ))
    }
}

impl Default for SkillEditTool {
    fn default() -> Self {
        Self::new(SKILLS_DIR)
    }
}

pub struct SkillRenameTool;

impl SkillRenameTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for SkillRenameTool {
    fn name(&self) -> &str {
        "skill_rename"
    }

    fn description(&self) -> &str {
        "Renomeia uma skill existente. Input: { \"old_name\": \"antigo\", \"new_name\": \"novo\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let old_name = args["old_name"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'old_name' é obrigatório".to_string())?;

        let new_name = args["new_name"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'new_name' é obrigatório".to_string())?;

        // Validate names
        if old_name.is_empty() || new_name.is_empty() {
            return Err("Nomes não podem estar vazios".to_string());
        }

        if old_name == "general" {
            return Err("Não é possível renomear a skill 'general'".to_string());
        }

        if new_name.contains('/') || new_name.contains('\\') {
            return Err("Nome de skill inválido".to_string());
        }

        let skills_path = Path::new(SKILLS_DIR);
        let old_dir = skills_path.join(old_name);
        let new_dir = skills_path.join(new_name);

        if !old_dir.exists() {
            return Err(format!("Skill '{}' não encontrada", old_name));
        }

        if new_dir.exists() {
            return Err(format!("Já existe uma skill chamada '{}'", new_name));
        }

        // Rename directory
        fs::rename(&old_dir, &new_dir)
            .map_err(|e| format!("Erro ao renomear: {}", e))?;

        // Update skill name inside the file
        let skill_file = new_dir.join("skill.md");
        if let Ok(content) = fs::read_to_string(&skill_file) {
            // Update the title
            let new_content = content.replacen(
                &format!("# Skill: {}", old_name),
                &format!("# Skill: {}", new_name),
                1
            );
            
            let _ = fs::write(&skill_file, new_content);
        }

        Ok(format!("✅ Skill '{}' renomeada para '{}'", old_name, new_name))
    }
}

impl Default for SkillRenameTool {
    fn default() -> Self {
        Self::new()
    }
}
