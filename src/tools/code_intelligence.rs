use crate::error::{AgentError, ToolError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub scope: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Module,
    Constant,
    Variable,
    Type,
    Method,
    Field,
    Other,
}

impl SymbolKind {
    pub fn from_string(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "function" => SymbolKind::Function,
            "struct" => SymbolKind::Struct,
            "enum" => SymbolKind::Enum,
            "trait" => SymbolKind::Trait,
            "impl" => SymbolKind::Impl,
            "module" => SymbolKind::Module,
            "constant" | "const" => SymbolKind::Constant,
            "variable" | "let" | "var" => SymbolKind::Variable,
            "type" | "typedef" => SymbolKind::Type,
            "method" => SymbolKind::Method,
            "field" => SymbolKind::Field,
            _ => SymbolKind::Other,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Definition {
    pub symbol: CodeSymbol,
    pub references: Vec<CodeSymbol>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeSearchResult {
    pub matches: Vec<SymbolMatch>,
    pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolMatch {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub context: String,
    pub symbol: Option<CodeSymbol>,
}

pub struct CodeIntelligence {
    symbols: HashMap<String, Vec<CodeSymbol>>,
    definitions: HashMap<String, Definition>,
}

impl CodeIntelligence {
    pub fn new() -> Self {
        Self {
            symbols: HashMap::new(),
            definitions: HashMap::new(),
        }
    }

    pub fn index_file<P: AsRef<Path>>(&mut self, path: P) -> Result<usize, AgentError> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path).map_err(|e| {
            AgentError::Tool(ToolError::ExecutionFailed(format!(
                "Failed to read file: {}",
                e
            )))
        })?;

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let symbols = self.extract_symbols(&content, ext, path.to_str().unwrap_or(""));

        let count = symbols.len();
        self.symbols
            .insert(path.to_str().unwrap_or("").to_string(), symbols);

        Ok(count)
    }

    pub fn extract_symbols(&self, content: &str, ext: &str, file: &str) -> Vec<CodeSymbol> {
        let mut symbols = Vec::new();

        match ext {
            "rs" => symbols.extend(self.extract_rust_symbols(content, file)),
            "js" | "ts" | "jsx" | "tsx" => symbols.extend(self.extract_js_symbols(content, file)),
            "py" => symbols.extend(self.extract_python_symbols(content, file)),
            "go" => symbols.extend(self.extract_go_symbols(content, file)),
            _ => symbols.extend(self.extract_generic_symbols(content, file)),
        }

        symbols
    }

    fn extract_rust_symbols(&self, content: &str, file: &str) -> Vec<CodeSymbol> {
        let mut symbols = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            if trimmed.starts_with("pub fn ")
                || trimmed.starts_with("fn ")
                || trimmed.starts_with("async fn ")
            {
                if let Some(name) =
                    self.extract_name_after(trimmed, &["fn ", "pub fn ", "async fn "])
                {
                    symbols.push(CodeSymbol {
                        name,
                        kind: if trimmed.contains("->") && trimmed.contains("Self") {
                            SymbolKind::Method
                        } else {
                            SymbolKind::Function
                        },
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: self.extract_rust_scope(content, i),
                    });
                }
            } else if trimmed.starts_with("struct ") {
                if let Some(name) = self.extract_name_after(trimmed, &["struct "]) {
                    symbols.push(CodeSymbol {
                        name,
                        kind: SymbolKind::Struct,
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: self.extract_rust_scope(content, i),
                    });
                }
            } else if trimmed.starts_with("enum ") {
                if let Some(name) = self.extract_name_after(trimmed, &["enum "]) {
                    symbols.push(CodeSymbol {
                        name,
                        kind: SymbolKind::Enum,
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: self.extract_rust_scope(content, i),
                    });
                }
            } else if trimmed.starts_with("trait ") {
                if let Some(name) = self.extract_name_after(trimmed, &["trait "]) {
                    symbols.push(CodeSymbol {
                        name,
                        kind: SymbolKind::Trait,
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: self.extract_rust_scope(content, i),
                    });
                }
            } else if trimmed.starts_with("impl ") {
                if let Some(name) = self.extract_impl_name(trimmed) {
                    symbols.push(CodeSymbol {
                        name,
                        kind: SymbolKind::Impl,
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: self.extract_rust_scope(content, i),
                    });
                }
            } else if trimmed.starts_with("mod ") {
                if let Some(name) = self.extract_name_after(trimmed, &["mod "]) {
                    symbols.push(CodeSymbol {
                        name,
                        kind: SymbolKind::Module,
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: self.extract_rust_scope(content, i),
                    });
                }
            } else if trimmed.starts_with("const ") || trimmed.starts_with("static ") {
                if let Some(name) = self.extract_const_name(trimmed) {
                    symbols.push(CodeSymbol {
                        name,
                        kind: SymbolKind::Constant,
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: self.extract_rust_scope(content, i),
                    });
                }
            }
        }

        symbols
    }

    fn extract_js_symbols(&self, content: &str, file: &str) -> Vec<CodeSymbol> {
        let mut symbols = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            if trimmed.starts_with("function ") || trimmed.starts_with("async function ") {
                if let Some(name) =
                    self.extract_name_after(trimmed, &["function ", "async function "])
                {
                    symbols.push(CodeSymbol {
                        name,
                        kind: SymbolKind::Function,
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: String::new(),
                    });
                }
            } else if trimmed.starts_with("class ") {
                if let Some(name) = self.extract_name_after(trimmed, &["class "]) {
                    symbols.push(CodeSymbol {
                        name,
                        kind: SymbolKind::Struct,
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: String::new(),
                    });
                }
            } else if trimmed.starts_with("const ")
                || trimmed.starts_with("let ")
                || trimmed.starts_with("var ")
            {
                if let Some(name) = self.extract_const_name(trimmed) {
                    symbols.push(CodeSymbol {
                        name,
                        kind: SymbolKind::Variable,
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: String::new(),
                    });
                }
            }
        }

        symbols
    }

    fn extract_python_symbols(&self, content: &str, file: &str) -> Vec<CodeSymbol> {
        let mut symbols = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
                if let Some(name) = self.extract_name_after(trimmed, &["def ", "async def "]) {
                    symbols.push(CodeSymbol {
                        name: name
                            .trim_start_matches(|c: char| c.is_alphanumeric() || c == '_')
                            .to_string(),
                        kind: SymbolKind::Function,
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: String::new(),
                    });
                }
            } else if trimmed.starts_with("class ") {
                if let Some(name) = self.extract_name_after(trimmed, &["class "]) {
                    symbols.push(CodeSymbol {
                        name,
                        kind: SymbolKind::Struct,
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: String::new(),
                    });
                }
            }
        }

        symbols
    }

    fn extract_go_symbols(&self, content: &str, file: &str) -> Vec<CodeSymbol> {
        let mut symbols = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            if trimmed.starts_with("func ") {
                if let Some(name) = self.extract_go_func_name(trimmed) {
                    let kind = if trimmed.contains("(") && !name.contains("(") {
                        SymbolKind::Method
                    } else {
                        SymbolKind::Function
                    };
                    symbols.push(CodeSymbol {
                        name,
                        kind,
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: String::new(),
                    });
                }
            } else if trimmed.starts_with("type ") && trimmed.contains("struct") {
                if let Some(name) = self.extract_name_after(trimmed, &["type "]) {
                    symbols.push(CodeSymbol {
                        name: name.split_whitespace().next().unwrap_or("").to_string(),
                        kind: SymbolKind::Struct,
                        file: file.to_string(),
                        line: i + 1,
                        column: line.len() - line.trim_start().len(),
                        scope: String::new(),
                    });
                }
            }
        }

        symbols
    }

    fn extract_generic_symbols(&self, content: &str, file: &str) -> Vec<CodeSymbol> {
        let mut symbols = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            if trimmed.contains("function ") || trimmed.contains("fn ") {
                let prefixes = ["function ", "fn ", "func "];
                for prefix in prefixes {
                    if trimmed.contains(prefix) {
                        if let Some(name) = self.extract_name_after(trimmed, &[prefix]) {
                            symbols.push(CodeSymbol {
                                name,
                                kind: SymbolKind::Function,
                                file: file.to_string(),
                                line: i + 1,
                                column: line.len() - line.trim_start().len(),
                                scope: String::new(),
                            });
                        }
                    }
                }
            }
        }

        symbols
    }

    fn extract_name_after<'a>(&self, s: &'a str, prefixes: &[&str]) -> Option<String> {
        for prefix in prefixes {
            if let Some(rest) = s.strip_prefix(prefix) {
                return rest
                    .split(&[' ', '{', '(', '<', ':'][..])
                    .next()
                    .map(|s| s.to_string());
            }
        }
        None
    }

    fn extract_const_name(&self, s: &str) -> Option<String> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() >= 2 {
            Some(
                parts[1]
                    .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_')
                    .to_string(),
            )
        } else {
            None
        }
    }

    fn extract_impl_name(&self, s: &str) -> Option<String> {
        if let Some(rest) = s.strip_prefix("impl") {
            let rest = rest.trim();
            if rest.starts_with('<') {
                if let Some(end) = rest.find('>') {
                    return Some(format!("impl{}", &rest[..=end]));
                }
            }
            Some(rest.split_whitespace().next().unwrap_or("").to_string())
        } else {
            None
        }
    }

    fn extract_go_func_name(&self, s: &str) -> Option<String> {
        if let Some(rest) = s.strip_prefix("func") {
            let rest = rest.trim();
            if rest.starts_with('(') {
                if let Some(end) = rest.find(')') {
                    let receiver = &rest[1..end];
                    if let Some(name) = receiver.split_whitespace().last() {
                        if let Some(after_paren) = rest[end + 1..].trim().strip_prefix('(') {
                            let method_name = after_paren.split('(').next().unwrap_or("");
                            return Some(format!(
                                "{}.{}",
                                name.trim_start_matches('*'),
                                method_name
                            ));
                        }
                    }
                }
            } else {
                return rest.split('(').next().map(|s| s.trim().to_string());
            }
        }
        None
    }

    fn extract_rust_scope(&self, content: &str, line_num: usize) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let mut scope = String::new();
        let mut depth = 0usize;

        for (i, line) in lines.iter().enumerate() {
            if i >= line_num {
                break;
            }

            let trimmed = line.trim();
            depth = depth.saturating_add(trimmed.matches('{').count());
            depth = depth.saturating_sub(trimmed.matches('}').count());

            if trimmed.starts_with("mod ")
                || trimmed.starts_with("impl")
                || trimmed.starts_with("trait ")
            {
                if let Some(name) = self.extract_name_after(trimmed, &["mod ", "impl ", "trait "]) {
                    if !scope.is_empty() {
                        scope.push_str("::");
                    }
                    scope.push_str(name.split(&[' ', '{', '<', '('][..]).next().unwrap_or(""));
                }
            }
        }

        scope
    }

    pub fn find_definition(&self, symbol_name: &str, file: &str) -> Option<Definition> {
        let key = format!("{}::{}", file, symbol_name);

        if let Some(def) = self.definitions.get(&key) {
            return Some(def.clone());
        }

        if let Some(symbols) = self.symbols.get(file) {
            for symbol in symbols {
                if symbol.name == symbol_name {
                    let references = self.find_references(symbol_name, file);
                    return Some(Definition {
                        symbol: symbol.clone(),
                        references,
                    });
                }
            }
        }

        for (file_path, symbols) in &self.symbols {
            if file_path != file {
                for symbol in symbols {
                    if symbol.name == symbol_name {
                        let references = self.find_references(symbol_name, file_path);
                        return Some(Definition {
                            symbol: symbol.clone(),
                            references,
                        });
                    }
                }
            }
        }

        None
    }

    pub fn find_references(&self, symbol_name: &str, _file: &str) -> Vec<CodeSymbol> {
        let mut refs = Vec::new();

        for (_file, symbols) in &self.symbols {
            for symbol in symbols {
                if symbol.name == symbol_name {
                    refs.push(symbol.clone());
                }
            }
        }

        refs
    }

    pub fn search_symbols(&self, query: &str) -> CodeSearchResult {
        let query_lower = query.to_lowercase();
        let mut matches = Vec::new();

        for (_file, symbols) in &self.symbols {
            for symbol in symbols {
                if symbol.name.to_lowercase().contains(&query_lower) {
                    matches.push(SymbolMatch {
                        file: symbol.file.clone(),
                        line: symbol.line,
                        column: symbol.column,
                        context: format!("{:?} {}", symbol.kind, symbol.name),
                        symbol: Some(symbol.clone()),
                    });
                }
            }
        }

        let total = matches.len();
        CodeSearchResult { matches, total }
    }

    pub fn get_symbols_in_file(&self, file: &str) -> Vec<CodeSymbol> {
        self.symbols.get(file).cloned().unwrap_or_default()
    }

    pub fn get_all_symbols(&self) -> Vec<CodeSymbol> {
        self.symbols.values().flatten().cloned().collect()
    }
}

impl Default for CodeIntelligence {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_rust_symbols() {
        let code = r#"
pub fn hello_world() {
    println!("Hello!");
}

struct MyStruct {
    field: i32,
}

impl MyStruct {
    pub fn new() -> Self {
        Self { field: 0 }
    }
}
"#;

        let ci = CodeIntelligence::new();
        let symbols = ci.extract_rust_symbols(code, "test.rs");

        assert!(symbols
            .iter()
            .any(|s| s.name == "hello_world" && s.kind == SymbolKind::Function));
        assert!(symbols
            .iter()
            .any(|s| s.name == "MyStruct" && s.kind == SymbolKind::Struct));
        assert!(symbols.iter().any(|s| s.kind == SymbolKind::Impl));
    }

    #[test]
    fn test_search_symbols() {
        let mut ci = CodeIntelligence::new();
        ci.index_file("test.rs").ok();

        let result = ci.search_symbols("test");
        assert_eq!(result.total, 0);
    }

    #[test]
    fn test_symbol_kind_from_string() {
        assert_eq!(SymbolKind::from_string("function"), SymbolKind::Function);
        assert_eq!(SymbolKind::from_string("struct"), SymbolKind::Struct);
        assert_eq!(SymbolKind::from_string("unknown"), SymbolKind::Other);
    }
}
