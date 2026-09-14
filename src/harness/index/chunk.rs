//! Symbol-aware chunking for the semantic index.
//!
//! Splits a source file into indexable chunks. For Rust files the chunker
//! reuses the Tree-Sitter grammar (same as `ast_search`) to cut on symbol
//! boundaries (structs/enums/traits/functions/impls), so a chunk is a whole
//! definition rather than an arbitrary line window. Non-Rust files (and Rust
//! files that fail to parse) fall back to fixed-size line windows.
//!
//! Each chunk carries the file path, the 1-based start/end line, an optional
//! symbol name/kind, and the source text.

use std::path::Path;

/// Max chars of a chunk's source kept in the index (protects the DB and the
/// LLM context). Larger symbols are truncated, keeping the header.
pub const MAX_CHUNK_CHARS: usize = 2000;
/// Lines per chunk for the non-Rust fallback.
const FALLBACK_LINES: usize = 40;
/// Overlap between fallback windows, so a definition split across a boundary
/// is still findable.
const FALLBACK_OVERLAP: usize = 8;

/// One indexable unit of source.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    /// 1-based first line of the chunk.
    pub start_line: usize,
    /// 1-based last line of the chunk (inclusive).
    pub end_line: usize,
    /// Symbol name when the chunk came from a Rust definition.
    pub symbol: Option<String>,
    /// Symbol kind (`function`, `struct`, ...) when known.
    pub kind: Option<String>,
    /// Chunk source text (possibly truncated to `MAX_CHUNK_CHARS`).
    pub text: String,
}

/// Tree-Sitter node kinds indexed as symbols (mirrors `ast_search`).
const SYMBOL_KINDS: &[&str] = &[
    "function_item",
    "struct_item",
    "enum_item",
    "trait_item",
    "impl_item",
    "mod_item",
    "const_item",
    "static_item",
    "type_item",
    "macro_definition",
];

/// Splits `source` into chunks. `path` decides the strategy: `.rs` files are
/// chunked by symbol, everything else by line windows.
pub fn chunk_source(path: &Path, source: &str) -> Vec<Chunk> {
    if path.extension().and_then(|e| e.to_str()) == Some("rs") {
        if let Some(chunks) = chunk_rust(source) {
            if !chunks.is_empty() {
                return chunks;
            }
        }
    }
    chunk_lines(source)
}

/// Chunks a Rust source by symbol. Returns `None` when the file cannot be
/// parsed (caller falls back to line windows).
fn chunk_rust(source: &str) -> Option<Vec<Chunk>> {
    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter::Language::new(tree_sitter_rust::LANGUAGE);
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;

    let mut chunks = Vec::new();
    collect(tree.root_node(), source, &mut chunks);
    Some(chunks)
}

/// Recursively collects symbol nodes as chunks. Nested items (e.g. a method
/// inside an `impl`) are collected too, so both the impl block and its methods
/// are searchable.
fn collect(node: tree_sitter::Node, source: &str, out: &mut Vec<Chunk>) {
    if SYMBOL_KINDS.contains(&node.kind()) {
        let start = node.start_position().row + 1;
        let end = node.end_position().row + 1;
        let text = truncate(&source[node.byte_range()]);
        out.push(Chunk {
            start_line: start,
            end_line: end,
            symbol: symbol_name(node, source),
            kind: Some(kind_label(node.kind())),
            text,
        });
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect(child, source, out);
    }
}

/// Human label for a Tree-Sitter node kind.
fn kind_label(kind: &str) -> String {
    match kind {
        "function_item" => "function",
        "struct_item" => "struct",
        "enum_item" => "enum",
        "trait_item" => "trait",
        "impl_item" => "impl",
        "mod_item" => "mod",
        "const_item" => "const",
        "static_item" => "static",
        "type_item" => "type",
        "macro_definition" => "macro",
        other => other,
    }
    .to_string()
}

/// Extracts a human-readable symbol name. `impl_item` has no `name` field, so
/// the name is built from its `trait`/`type` fields (`Trait for Type`).
fn symbol_name(node: tree_sitter::Node, source: &str) -> Option<String> {
    if node.kind() == "impl_item" {
        let trait_name = node
            .child_by_field_name("trait")
            .map(|n| n.utf8_text(source.as_bytes()).unwrap_or("").to_string());
        let type_name = node
            .child_by_field_name("type")
            .map(|n| n.utf8_text(source.as_bytes()).unwrap_or("").to_string());
        return match (trait_name, type_name) {
            (Some(t), Some(ty)) => Some(format!("{} for {}", t, ty)),
            (Some(t), None) => Some(t),
            (None, Some(ty)) => Some(ty),
            (None, None) => None,
        };
    }
    node.child_by_field_name("name")
        .map(|n| n.utf8_text(source.as_bytes()).unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
}

/// Fixed-size line windows with overlap (fallback for non-Rust files).
fn chunk_lines(source: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < lines.len() {
        let end = (start + FALLBACK_LINES).min(lines.len());
        let text = truncate(&lines[start..end].join("\n"));
        chunks.push(Chunk {
            start_line: start + 1,
            end_line: end,
            symbol: None,
            kind: None,
            text,
        });
        if end == lines.len() {
            break;
        }
        start = end.saturating_sub(FALLBACK_OVERLAP);
    }
    chunks
}

/// Truncates a chunk to `MAX_CHUNK_CHARS` chars, keeping the header.
fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_CHUNK_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(MAX_CHUNK_CHARS).collect();
    format!("{}…", head)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SAMPLE: &str = r#"pub struct ToolRegistry {
    pub name: String,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { name: String::new() }
    }
}

pub fn execute(cmd: &str) -> i32 {
    cmd.len() as i32
}
"#;

    #[test]
    fn test_rust_chunks_by_symbol() {
        let chunks = chunk_source(&PathBuf::from("a.rs"), SAMPLE);
        let symbols: Vec<_> = chunks.iter().filter_map(|c| c.symbol.clone()).collect();
        assert!(symbols.contains(&"ToolRegistry".to_string()));
        assert!(symbols.contains(&"execute".to_string()));
        // The impl block and its method are both indexed.
        assert!(symbols.iter().any(|s| s == "ToolRegistry"));
        assert!(symbols.contains(&"new".to_string()));
    }

    #[test]
    fn test_rust_chunk_lines_are_1_based() {
        let chunks = chunk_source(&PathBuf::from("a.rs"), SAMPLE);
        let exec = chunks
            .iter()
            .find(|c| c.symbol.as_deref() == Some("execute"));
        let exec = exec.expect("execute chunk");
        assert_eq!(exec.start_line, 11);
        assert_eq!(exec.end_line, 13);
        assert!(exec.text.contains("pub fn execute"));
    }

    #[test]
    fn test_non_rust_falls_back_to_lines() {
        let src = (1..=100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = chunk_source(&PathBuf::from("a.txt"), &src);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.symbol.is_none()));
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, FALLBACK_LINES);
    }

    #[test]
    fn test_empty_source_no_chunks() {
        assert!(chunk_source(&PathBuf::from("a.txt"), "").is_empty());
    }

    #[test]
    fn test_large_symbol_is_truncated() {
        let big = format!("pub fn big() {{\n{}\n}}", "let x = 1;\n".repeat(500));
        let chunks = chunk_source(&PathBuf::from("a.rs"), &big);
        let big_chunk = chunks.iter().find(|c| c.symbol.as_deref() == Some("big"));
        let big_chunk = big_chunk.expect("big chunk");
        assert!(big_chunk.text.chars().count() <= MAX_CHUNK_CHARS + 1);
    }
}
