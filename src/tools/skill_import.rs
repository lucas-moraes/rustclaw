use super::Tool;
use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const SKILLS_DIR: &str = "skills";

pub struct SkillImportFromUrlTool;

impl SkillImportFromUrlTool {
    pub fn new() -> Self {
        Self
    }

    fn extract_domain(url: &str) -> Option<String> {
        url.split("//")
            .nth(1)?
            .split('/')
            .next()
            .map(|s| s.to_string())
    }

    async fn fetch_url(&self, url: &str) -> Result<String, String> {
        let client = Client::builder()
            .user_agent("Mozilla/5.0 (compatible; RustClaw/1.0)")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Erro ao criar cliente HTTP: {}", e))?;

        let response = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Erro ao acessar URL: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}: {}", response.status(), response.status()));
        }

        let content = response
            .text()
            .await
            .map_err(|e| format!("Erro ao ler conteúdo: {}", e))?;

        Ok(content)
    }

    fn is_html(&self, content: &str) -> bool {
        content.contains("<!DOCTYPE html>") ||
        content.contains("<html") ||
        content.contains("<body")
    }

    fn extract_markdown_from_html(&self, html: &str) -> Result<String, String> {
        let document = Html::parse_document(html);
        
        // Try to find main content area
        let selectors = vec![
            "article",
            "main",
            "[role='main']",
            ".content",
            ".documentation",
            ".markdown-body",
            ".readme",
            "#content",
            "#readme",
            "body",
        ];

        for selector_str in selectors {
            if let Ok(selector) = Selector::parse(selector_str) {
                if let Some(element) = document.select(&selector).next() {
                    let text = self.html_to_markdown(&element);
                    if !text.trim().is_empty() {
                        return Ok(text);
                    }
                }
            }
        }

        // Fallback: extract all text
        let body_selector = Selector::parse("body").unwrap();
        if let Some(body) = document.select(&body_selector).next() {
            Ok(self.html_to_markdown(&body))
        } else {
            Err("Não foi possível extrair conteúdo do HTML".to_string())
        }
    }

    fn html_to_markdown(&self, element: &scraper::ElementRef) -> String {
        let mut markdown = String::new();
        
        for child in element.children() {
            match child.value() {
                scraper::Node::Text(text) => {
                    markdown.push_str(&text.text);
                }
                scraper::Node::Element(elem) => {
                    let tag_name = elem.name();
                    let child_ref = scraper::ElementRef::wrap(child).unwrap();
                    
                    match tag_name {
                        "h1" => markdown.push_str(&format!("\n# {}\n", self.html_to_markdown(&child_ref).trim())),
                        "h2" => markdown.push_str(&format!("\n## {}\n", self.html_to_markdown(&child_ref).trim())),
                        "h3" => markdown.push_str(&format!("\n### {}\n", self.html_to_markdown(&child_ref).trim())),
                        "h4" => markdown.push_str(&format!("\n#### {}\n", self.html_to_markdown(&child_ref).trim())),
                        "p" => markdown.push_str(&format!("\n{}\n", self.html_to_markdown(&child_ref).trim())),
                        "br" => markdown.push('\n'),
                        "strong" | "b" => markdown.push_str(&format!("**{}**", self.html_to_markdown(&child_ref))),
                        "em" | "i" => markdown.push_str(&format!("*{}*", self.html_to_markdown(&child_ref))),
                        "code" => markdown.push_str(&format!("`{}`", self.html_to_markdown(&child_ref))),
                        "pre" => {
                            let code = self.html_to_markdown(&child_ref);
                            markdown.push_str(&format!("\n```\n{}\n```\n", code.trim()));
                        }
                        "ul" => {
                            let items: Vec<String> = child_ref
                                .select(&Selector::parse("li").unwrap())
                                .map(|li| format!("- {}", self.html_to_markdown(&li).trim()))
                                .collect();
                            markdown.push_str(&format!("\n{}\n", items.join("\n")));
                        }
                        "ol" => {
                            let items: Vec<String> = child_ref
                                .select(&Selector::parse("li").unwrap())
                                .enumerate()
                                .map(|(i, li)| format!("{}. {}", i + 1, self.html_to_markdown(&li).trim()))
                                .collect();
                            markdown.push_str(&format!("\n{}\n", items.join("\n")));
                        }
                        "a" => {
                            let href = elem.attr("href").unwrap_or("#");
                            let text = self.html_to_markdown(&child_ref);
                            markdown.push_str(&format!("[{}]({})", text, href));
                        }
                        _ => markdown.push_str(&self.html_to_markdown(&child_ref)),
                    }
                }
                _ => {}
            }
        }

        markdown
    }

    fn convert_to_skill_format(&self, content: &str, skill_name: &str, url: &str) -> String {
        // Check if already in skill format
        if content.contains("# Skill:") && content.contains("## Contexto") {
            return content.to_string();
        }

        // Extract metadata
        let lines: Vec<&str> = content.lines().collect();
        let _title = skill_name.to_string();
        let mut description = String::new();
        let mut keywords = HashSet::new();

        // Try to find title (first h1)
        let h1_regex = Regex::new(r"#\s+(.+)").unwrap();
        let _extracted_title = 'title_block: {
            for line in &lines {
                if let Some(caps) = h1_regex.captures(line) {
                    break 'title_block caps.get(1).unwrap().as_str().to_string();
                }
            }
            skill_name.to_string()
        };

        // Try to find description (first paragraph after title)
        let mut found_title = false;
        for line in &lines {
            if found_title && !line.trim().is_empty() && !line.starts_with('#') {
                description = line.trim().to_string();
                if description.len() > 200 {
                    description = format!("{}...", &description[..200]);
                }
                break;
            }
            if line.starts_with("# ") {
                found_title = true;
            }
        }

        if description.is_empty() {
            description = format!("Skill importada de {}", Self::extract_domain(url).unwrap_or_else(|| "URL".to_string()));
        }

        // Extract keywords from content
        let common_words: HashSet<&str> = [
            "the", "be", "to", "of", "and", "a", "in", "that", "have",
            "i", "it", "for", "not", "on", "with", "he", "as", "you",
            "do", "at", "this", "but", "his", "by", "from", "they",
            "we", "say", "her", "she", "or", "an", "will", "my",
            "one", "all", "would", "there", "their", "what", "so",
            "up", "out", "if", "about", "who", "get", "which", "go",
            "me", "when", "make", "can", "like", "time", "no", "just",
            "him", "know", "take", "people", "into", "year", "your",
            "good", "some", "could", "them", "see", "other", "than",
            "then", "now", "look", "only", "come", "its", "over",
            "think", "also", "back", "after", "use", "two", "how",
            "our", "work", "first", "well", "way", "even", "new",
            "want", "because", "any", "these", "give", "day", "most",
            "us", "é", "o", "a", "os", "as", "um", "uma", "para",
            "de", "da", "do", "dos", "das", "no", "na", "nos", "nas",
            "em", "com", "por", "que", "se", "ou", "mas", "como",
        ].iter().cloned().collect();

        let word_regex = Regex::new(r"\b[a-zA-Záéíóúàèìòùãõâêîôûäëïöüç]{4,}\b").unwrap();
        for cap in word_regex.captures_iter(&content.to_lowercase()) {
            let word = cap.get(0).unwrap().as_str();
            if !common_words.contains(word) {
                keywords.insert(word.to_string());
            }
        }

        // Select top keywords
        let keywords_vec: Vec<String> = keywords.into_iter().take(7).collect();

        // Build skill content
        format!(r#"# Skill: {}

## Descrição
{}

## Contexto
Contexto baseado em documentação importada de {}.

Use este contexto quando o usuário estiver trabalhando com tópicos relacionados.

## Keywords
{}

## Comportamento

### SEMPRE
- Baseie suas respostas no contexto importado
- Seja prestativo e preciso
- Use ferramentas apropriadas quando necessário

### NUNCA
- Ignore o contexto importado
- Forneça informações contraditórias à documentação

## Ferramentas Prioritárias
1. file_read
2. file_search
3. shell

## Exemplos

### Input: "Me ajude com isso"
**Bom:** Resposta baseada no contexto importado
**Ruim:** Resposta genérica ignorando o contexto

---

## Conteúdo Importado Original

{}

---

*Skill importada automaticamente de {}*
"#,
            skill_name,
            description,
            url,
            keywords_vec.iter().map(|k| format!("- {}", k)).collect::<Vec<_>>().join("\n"),
            content,
            url
        )
    }

    fn validate_skill_name(name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("Nome da skill não pode estar vazio".to_string());
        }

        if name.contains('/') || name.contains('\\') || name.contains(' ') {
            return Err("Nome da skill inválido. Use kebab-case (ex: minha-skill)".to_string());
        }

        if name == "general" {
            return Err("Não é possível sobrescrever a skill 'general'".to_string());
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl Tool for SkillImportFromUrlTool {
    fn name(&self) -> &str {
        "skill_import_from_url"
    }

    fn description(&self) -> &str {
        "Importa e converte documentação de uma URL em skill. Input: { \"url\": \"https://example.com/doc.md\", \"skill_name\": \"minha-skill\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'url' é obrigatório".to_string())?;

        let skill_name = args["skill_name"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'skill_name' é obrigatório".to_string())?;

        // Validate skill name
        Self::validate_skill_name(skill_name)?;

        // Check if skill already exists
        let skill_dir = Path::new(SKILLS_DIR).join(skill_name);
        if skill_dir.exists() {
            return Err(format!("Skill '{}' já existe. Use outro nome ou remova a existente primeiro.", skill_name));
        }

        // Fetch content
        let content = self.fetch_url(url).await?;

        // Convert HTML to markdown if needed
        let markdown_content = if self.is_html(&content) {
            self.extract_markdown_from_html(&content)?
        } else {
            content
        };

        // Convert to skill format
        let skill_content = self.convert_to_skill_format(&markdown_content, skill_name, url);

        // Create skill directory and file
        fs::create_dir_all(&skill_dir)
            .map_err(|e| format!("Erro ao criar diretório: {}", e))?;

        let skill_file = skill_dir.join("skill.md");
        fs::write(&skill_file, &skill_content)
            .map_err(|e| format!("Erro ao escrever arquivo: {}", e))?;

        // Validate
        match crate::skills::parser::SkillParser::parse(&skill_file) {
            Ok(skill) => {
                let result = format!(
                    "✅ Skill '{}' importada com sucesso!\n\n📊 Detalhes:\n- Nome: {}\n- Descrição: {}\n- Keywords: {}\n- Arquivo: {:?}",
                    skill_name,
                    skill.name,
                    skill.description,
                    skill.keywords.join(", "),
                    skill_file
                );
                Ok(result)
            }
            Err(e) => {
                // Clean up on validation error
                let _ = fs::remove_dir_all(&skill_dir);
                Err(format!("❌ Skill criada mas com erro de validação: {}. Diretório removido.", e))
            }
        }
    }
}

impl Default for SkillImportFromUrlTool {
    fn default() -> Self {
        Self::new()
    }
}
