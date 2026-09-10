//! `ast_search` tool: syntactic search over `.rs` files using the Tree-Sitter
//! Rust grammar. Extracts structs, enums, traits, functions and impl blocks
//! from the AST without regex false positives or reading the whole file.

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};

/// Max chars of a node's source shown in the result (protects the LLM context).
const MAX_NODE_CHARS: usize = 3000;
/// Max number of nodes returned in one call.
const MAX_NODES: usize = 50;

/// Maps a `symbol_kind` argument to the Tree-Sitter node kinds it matches.
fn node_kinds(symbol_kind: &str) -> Option<&'static [&'static str]> {
    match symbol_kind {
        "function" => Some(&["function_item"]),
        "struct" => Some(&["struct_item"]),
        "enum" => Some(&["enum_item"]),
        "trait" => Some(&["trait_item"]),
        "impl" => Some(&["impl_item"]),
        "all" => Some(&[
            "function_item",
            "struct_item",
            "enum_item",
            "trait_item",
            "impl_item",
        ]),
        _ => None,
    }
}

/// Human label for a Tree-Sitter node kind.
fn kind_label(kind: &str) -> String {
    match kind {
        "function_item" => "function".to_string(),
        "struct_item" => "struct".to_string(),
        "enum_item" => "enum".to_string(),
        "trait_item" => "trait".to_string(),
        "impl_item" => "impl".to_string(),
        _ => kind.to_string(),
    }
}

pub struct AstSearchTool;

#[async_trait::async_trait]
impl Tool for AstSearchTool {
    fn name(&self) -> &str {
        "ast_search"
    }

    fn description(&self) -> &str {
        "Realiza busca sintática em arquivos .rs usando a AST (Tree-Sitter). \
Permite extrair definições de structs, enums, traits, funções e impls sem ler \
o arquivo inteiro e sem falsos positivos."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Caminho do arquivo .rs a ser analisado (ex: \"src/harness/tool/mod.rs\")"
                },
                "symbol_kind": {
                    "type": "string",
                    "description": "Tipo de nó a buscar: function, struct, enum, trait, impl ou all (padrão all)",
                    "enum": ["function", "struct", "enum", "trait", "impl", "all"]
                },
                "name": {
                    "type": "string",
                    "description": "Nome do símbolo a filtrar (ex: \"ToolRegistry\" ou \"execute\"). Se omitido, retorna todos os símbolos do tipo especificado"
                }
            },
            "required": ["path"]
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let path_str = args["path"]
            .as_str()
            .ok_or_else(|| "missing required argument: path".to_string())?;
        let symbol_kind = args["symbol_kind"].as_str().unwrap_or("all");
        let name_filter = args["name"].as_str().map(|s| s.to_string());

        let kinds = node_kinds(symbol_kind)
            .ok_or_else(|| format!("invalid symbol_kind `{}`", symbol_kind))?;

        let path = ctx.cwd.resolve(path_str);
        if !path.is_file() {
            return Err(format!("file not found: {}", path.display()));
        }
        let code = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;

        // Parse with the official Rust grammar.
        let mut parser = tree_sitter::Parser::new();
        let language = tree_sitter::Language::new(tree_sitter_rust::LANGUAGE);
        parser
            .set_language(&language)
            .map_err(|e| format!("failed to load Rust grammar: {}", e))?;
        let tree = parser
            .parse(&code, None)
            .ok_or_else(|| format!("failed to parse {}", path.display()))?;

        // Collect matching nodes via recursive traversal.
        let mut matches: Vec<(String, String, usize, usize, String)> = Vec::new();
        collect_nodes(
            tree.root_node(),
            &code,
            kinds,
            name_filter.as_deref(),
            &mut matches,
        );

        if matches.is_empty() {
            let what = match name_filter {
                Some(n) => format!("`{}` ({})", n, symbol_kind),
                None => symbol_kind.to_string(),
            };
            return Ok(ToolResult::simple(
                format!("ast_search {}", preview(path_str, 40)),
                format!("(no {} symbols found in {})", what, path.display()),
            ));
        }

        let truncated = matches.len() > MAX_NODES;
        let mut body = String::new();
        for (name, kind, start, end, src) in matches.iter().take(MAX_NODES) {
            body.push_str(&format!(
                "### {} `{}` (lines {}-{})\n```rust\n{}\n```\n\n",
                kind_label(kind),
                name,
                start + 1,
                end + 1,
                src
            ));
        }
        if truncated {
            body.push_str(&format!(
                "[showing first {} of {} symbols]\n",
                MAX_NODES,
                matches.len()
            ));
        }

        Ok(ToolResult::simple(
            format!("ast_search {} ({})", preview(path_str, 40), symbol_kind),
            body,
        ))
    }
}

/// Extracts a human-readable symbol name for a node. Most items carry a
/// `name` field; `impl_item` has no name, so we build `Trait for Type`.
fn symbol_name(node: tree_sitter::Node, code: &str) -> String {
    if node.kind() == "impl_item" {
        let trait_name = node
            .child_by_field_name("trait")
            .map(|n| n.utf8_text(code.as_bytes()).unwrap_or("").to_string());
        let type_name = node
            .child_by_field_name("type")
            .map(|n| n.utf8_text(code.as_bytes()).unwrap_or("").to_string());
        return match (trait_name, type_name) {
            (Some(t), Some(ty)) => format!("{} for {}", t, ty),
            (Some(t), None) => t,
            (None, Some(ty)) => ty,
            (None, None) => String::new(),
        };
    }
    node.child_by_field_name("name")
        .map(|n| n.utf8_text(code.as_bytes()).unwrap_or("").to_string())
        .unwrap_or_default()
}

/// Recursively walks the tree, collecting nodes whose kind is in `kinds` and
/// whose name (if any) matches `name_filter`. `start`/`end` are 0-based rows.
fn collect_nodes(
    node: tree_sitter::Node,
    code: &str,
    kinds: &[&str],
    name_filter: Option<&str>,
    out: &mut Vec<(String, String, usize, usize, String)>,
) {
    if kinds.contains(&node.kind()) {
        let name = symbol_name(node, code);
        if name_filter.is_none_or(|f| name == f) {
            let start = node.start_position().row;
            let end = node.end_position().row;
            let src = &code[node.byte_range()];
            let src = if src.chars().count() > MAX_NODE_CHARS {
                // Truncate the body, keeping the signature/header.
                let head: String = src.chars().take(MAX_NODE_CHARS).collect();
                format!("{}…\n[truncated {} chars]", head, src.chars().count())
            } else {
                src.to_string()
            };
            out.push((name, node.kind().to_string(), start, end, src));
            if out.len() >= MAX_NODES * 2 {
                return;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_nodes(child, code, kinds, name_filter, out);
        if out.len() >= MAX_NODES * 2 {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::permission::PermissionEngine;
    use crate::harness::tool::context::ToolContext;
    use std::sync::Arc;

    const SAMPLE: &str = r#"
pub struct ToolRegistry {
    pub name: String,
}

enum Color {
    Red,
    Green,
}

trait Runner {
    fn run(&self) -> String;
}

impl Runner for ToolRegistry {
    fn run(&self) -> String {
        self.name.clone()
    }
}

pub fn execute(cmd: &str) -> i32 {
    cmd.len() as i32
}
"#;

    fn write_sample(dir: &std::path::Path) -> std::path::PathBuf {
        let p = dir.join("sample.rs");
        std::fs::write(&p, SAMPLE).unwrap();
        p
    }

    fn ctx_with_cwd(cwd: std::path::PathBuf) -> ToolContext {
        ToolContext {
            session_id: "s".into(),
            agent: "build".into(),
            agent_tools: vec![],
            cwd: crate::harness::tool::context::PathBufGuard(cwd),
            abort: crate::harness::tool::context::AbortSignal::new(),
            permission: Arc::new(PermissionEngine::default()),
            asker: Arc::new(AllowAsker),
            user_asker: Arc::new(NoUserAsker),
            todos: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            task_runner: None,
            events: crate::harness::event::event_channel().0,
            project_memory: None,
            hooks: Default::default(),
            checkpoints: std::sync::Arc::new(
                crate::harness::tool::checkpoint::FileCheckpoints::new(),
            ),
            jobs: std::sync::Arc::new(crate::harness::tool::jobs::JobRegistry::new()),
        }
    }

    struct AllowAsker;
    #[async_trait::async_trait]
    impl crate::harness::tool::context::PermissionAsker for AllowAsker {
        async fn ask(&self, _: crate::harness::tool::context::PermissionAskInput) -> bool {
            true
        }
    }
    struct NoUserAsker;
    #[async_trait::async_trait]
    impl crate::harness::tool::context::UserAsker for NoUserAsker {
        async fn ask(&self, _: String, _: Vec<String>) -> Option<String> {
            None
        }
    }

    #[tokio::test]
    async fn test_finds_specific_struct() {
        let dir = tempfile::tempdir().unwrap();
        let _path = write_sample(dir.path());
        let ctx = ctx_with_cwd(dir.path().to_path_buf());
        let result = AstSearchTool
            .execute(
                json!({"path": "sample.rs", "symbol_kind": "struct", "name": "ToolRegistry"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.output.contains("struct `ToolRegistry`"));
        assert!(result.output.contains("pub struct ToolRegistry"));
        assert!(!result.output.contains("enum"));
    }

    #[tokio::test]
    async fn test_filters_by_function_and_name() {
        let dir = tempfile::tempdir().unwrap();
        let _path = write_sample(dir.path());
        let ctx = ctx_with_cwd(dir.path().to_path_buf());
        let result = AstSearchTool
            .execute(
                json!({"path": "sample.rs", "symbol_kind": "function", "name": "execute"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.output.contains("function `execute`"));
        assert!(result.output.contains("pub fn execute"));
        // The impl's `run` method must not appear.
        assert!(!result.output.contains("`run`"));
    }

    #[tokio::test]
    async fn test_all_kinds_returns_everything() {
        let dir = tempfile::tempdir().unwrap();
        let _path = write_sample(dir.path());
        let ctx = ctx_with_cwd(dir.path().to_path_buf());
        let result = AstSearchTool
            .execute(json!({"path": "sample.rs", "symbol_kind": "all"}), &ctx)
            .await
            .unwrap();
        for needle in [
            "struct `ToolRegistry`",
            "enum `Color`",
            "trait `Runner`",
            "impl `Runner for ToolRegistry`",
            "function `execute`",
        ] {
            assert!(result.output.contains(needle), "missing {needle}");
        }
    }

    #[tokio::test]
    async fn test_missing_symbol_returns_no_match() {
        let dir = tempfile::tempdir().unwrap();
        let _path = write_sample(dir.path());
        let ctx = ctx_with_cwd(dir.path().to_path_buf());
        let result = AstSearchTool
            .execute(
                json!({"path": "sample.rs", "symbol_kind": "struct", "name": "Nope"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.output.contains("no `Nope` (struct) symbols found"));
    }

    #[tokio::test]
    async fn test_missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_with_cwd(dir.path().to_path_buf());
        let err = AstSearchTool
            .execute(json!({"path": "nope.rs"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.contains("file not found"));
    }

    #[tokio::test]
    async fn test_invalid_symbol_kind_errors() {
        let dir = tempfile::tempdir().unwrap();
        let _path = write_sample(dir.path());
        let ctx = ctx_with_cwd(dir.path().to_path_buf());
        let err = AstSearchTool
            .execute(json!({"path": "sample.rs", "symbol_kind": "bogus"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.contains("invalid symbol_kind"));
    }
}
