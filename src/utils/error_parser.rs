use regex::Regex;
use std::fmt;

/// Erro estruturado extraído do output de build
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedError {
    pub file: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub error_type: String,
    pub message: String,
    pub suggestion: Option<String>,
}

impl fmt::Display for ParsedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let location = if let Some(line) = self.line {
            if let Some(col) = self.column {
                format!("{}:{}:{}", self.file, line, col)
            } else {
                format!("{}:{}", self.file, line)
            }
        } else {
            self.file.clone()
        };

        write!(f, "{} - {}: {}", location, self.error_type, self.message)?;

        if let Some(suggestion) = &self.suggestion {
            write!(f, "\n  Sugestão: {}", suggestion)?;
        }

        Ok(())
    }
}

/// Resultado da validação de build
#[derive(Debug, Clone)]
pub enum BuildValidation {
    Success,
    Failed { errors: Vec<ParsedError> },
}

impl BuildValidation {
    #[allow(dead_code)]
    pub fn is_success(&self) -> bool {
        matches!(self, BuildValidation::Success)
    }

    #[allow(dead_code)]
    pub fn error_count(&self) -> usize {
        match self {
            BuildValidation::Success => 0,
            BuildValidation::Failed { errors } => errors.len(),
        }
    }

    #[allow(dead_code)]
    pub fn format_for_llm(&self) -> String {
        match self {
            BuildValidation::Success => "✅ Build passou com sucesso!".to_string(),
            BuildValidation::Failed { errors } => {
                let mut output = format!("❌ Build falhou com {} erro(s):\n\n", errors.len());

                for (idx, error) in errors.iter().enumerate() {
                    output.push_str(&format!("{}. {}\n", idx + 1, error));
                }

                output.push_str("\n🔧 Por favor, corrija estes erros e tente novamente.");
                output
            }
        }
    }
}

/// Parser de erros de diferentes linguagens
pub struct ErrorParser;

impl ErrorParser {
    /// Parseia output de build e extrai erros estruturados
    pub fn parse(output: &str, project_type: &str) -> BuildValidation {
        match project_type.to_lowercase().as_str() {
            "rust" => Self::parse_rust(output),
            "javascript" | "typescript" => Self::parse_typescript(output),
            "python" => Self::parse_python(output),
            "go" => Self::parse_go(output),
            "java" => Self::parse_java(output),
            _ => Self::parse_generic(output),
        }
    }

    /// Parseia erros do Rust/Cargo
    fn parse_rust(output: &str) -> BuildValidation {
        let mut errors = Vec::new();

        // Regex para erros do Rust: error[E0XXX]: message
        //   --> src/file.rs:42:10
        let error_re = Regex::new(r"error(?:\[E\d+\])?: (.+)").unwrap();
        let location_re = Regex::new(r"-->\s+(.+?):(\d+):(\d+)").unwrap();
        let help_re = Regex::new(r"help: (.+)").unwrap();

        let lines: Vec<&str> = output.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];

            if let Some(error_cap) = error_re.captures(line) {
                let message = error_cap[1].trim().to_string();
                let mut file = String::new();
                let mut line_num = None;
                let mut col_num = None;
                let mut suggestion = None;

                // Procura pela localização nas próximas linhas
                for line in lines.iter().skip(i + 1).take(5) {
                    if let Some(loc_cap) = location_re.captures(line) {
                        file = loc_cap[1].to_string();
                        line_num = loc_cap[2].parse().ok();
                        col_num = loc_cap[3].parse().ok();
                        break;
                    }
                }

                // Procura por sugestões
                for line in lines.iter().skip(i + 1).take(10) {
                    if let Some(help_cap) = help_re.captures(line) {
                        suggestion = Some(help_cap[1].trim().to_string());
                        break;
                    }
                }

                if !file.is_empty() {
                    errors.push(ParsedError {
                        file,
                        line: line_num,
                        column: col_num,
                        error_type: "compilation error".to_string(),
                        message,
                        suggestion,
                    });
                }
            }

            i += 1;
        }

        if errors.is_empty() {
            BuildValidation::Success
        } else {
            BuildValidation::Failed { errors }
        }
    }

    /// Parseia erros do TypeScript/JavaScript
    fn parse_typescript(output: &str) -> BuildValidation {
        let mut errors = Vec::new();

        // Regex para erros do tsc: file.ts(42,10): error TS2304: message
        let error_re = Regex::new(r"(.+?)\((\d+),(\d+)\): error (TS\d+): (.+)").unwrap();

        for line in output.lines() {
            if let Some(cap) = error_re.captures(line) {
                errors.push(ParsedError {
                    file: cap[1].to_string(),
                    line: cap[2].parse().ok(),
                    column: cap[3].parse().ok(),
                    error_type: cap[4].to_string(),
                    message: cap[5].trim().to_string(),
                    suggestion: None,
                });
            }
        }

        if errors.is_empty() {
            BuildValidation::Success
        } else {
            BuildValidation::Failed { errors }
        }
    }

    /// Parseia erros do Python
    fn parse_python(output: &str) -> BuildValidation {
        let mut errors = Vec::new();

        // Python traceback: File "file.py", line 42, in <module>
        let error_re = Regex::new(r#"File "(.+?)", line (\d+)"#).unwrap();
        let exception_re = Regex::new(r"^(\w+Error|Exception): (.+)").unwrap();

        let lines: Vec<&str> = output.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            if let Some(cap) = error_re.captures(lines[i]) {
                let file = cap[1].to_string();
                let line_num = cap[2].parse().ok();

                // Procura pela exception nas próximas linhas
                for line in lines.iter().skip(i + 1).take(5) {
                    if let Some(exc_cap) = exception_re.captures(line) {
                        errors.push(ParsedError {
                            file: file.clone(),
                            line: line_num,
                            column: None,
                            error_type: exc_cap[1].to_string(),
                            message: exc_cap[2].trim().to_string(),
                            suggestion: None,
                        });
                        break;
                    }
                }
            }
            i += 1;
        }

        if errors.is_empty() {
            BuildValidation::Success
        } else {
            BuildValidation::Failed { errors }
        }
    }

    /// Parseia erros do Go
    fn parse_go(output: &str) -> BuildValidation {
        let mut errors = Vec::new();

        // Go errors: file.go:42:10: error message
        let error_re = Regex::new(r"(.+?):(\d+):(\d+): (.+)").unwrap();

        for line in output.lines() {
            if let Some(cap) = error_re.captures(line) {
                errors.push(ParsedError {
                    file: cap[1].to_string(),
                    line: cap[2].parse().ok(),
                    column: cap[3].parse().ok(),
                    error_type: "compilation error".to_string(),
                    message: cap[4].trim().to_string(),
                    suggestion: None,
                });
            }
        }

        if errors.is_empty() {
            BuildValidation::Success
        } else {
            BuildValidation::Failed { errors }
        }
    }

    /// Parseia erros do Java
    fn parse_java(output: &str) -> BuildValidation {
        let mut errors = Vec::new();

        // Java errors: File.java:42: error: message
        let error_re = Regex::new(r"(.+?):(\d+): error: (.+)").unwrap();

        for line in output.lines() {
            if let Some(cap) = error_re.captures(line) {
                errors.push(ParsedError {
                    file: cap[1].to_string(),
                    line: cap[2].parse().ok(),
                    column: None,
                    error_type: "compilation error".to_string(),
                    message: cap[3].trim().to_string(),
                    suggestion: None,
                });
            }
        }

        if errors.is_empty() {
            BuildValidation::Success
        } else {
            BuildValidation::Failed { errors }
        }
    }

    /// Parser genérico - procura por padrões comuns de erro
    fn parse_generic(output: &str) -> BuildValidation {
        let contains_error = output.to_lowercase().contains("error")
            || output.to_lowercase().contains("failed")
            || output.to_lowercase().contains("exception");

        if contains_error {
            BuildValidation::Failed {
                errors: vec![ParsedError {
                    file: "unknown".to_string(),
                    line: None,
                    column: None,
                    error_type: "build error".to_string(),
                    message: output.lines().take(10).collect::<Vec<_>>().join("\n"),
                    suggestion: None,
                }],
            }
        } else {
            BuildValidation::Success
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rust_error() {
        let output = r#"
error[E0425]: cannot find value `x` in this scope
  --> src/main.rs:5:13
   |
5  |     println!("{}", x);
   |                    ^ not found in this scope
help: consider importing this function
   |
1  | use std::x;
   |
        "#;

        let result = ErrorParser::parse_rust(output);
        assert!(!result.is_success());
        assert_eq!(result.error_count(), 1);

        if let BuildValidation::Failed { errors } = result {
            assert_eq!(errors[0].file, "src/main.rs");
            assert_eq!(errors[0].line, Some(5));
            assert_eq!(errors[0].column, Some(13));
            assert!(errors[0].message.contains("cannot find value"));
        }
    }

    #[test]
    fn test_build_validation_success() {
        let validation = BuildValidation::Success;
        assert!(validation.is_success());
        assert_eq!(validation.error_count(), 0);
        assert!(validation.format_for_llm().contains("sucesso"));
    }

    #[test]
    fn test_build_validation_failed() {
        let validation = BuildValidation::Failed {
            errors: vec![ParsedError {
                file: "test.rs".to_string(),
                line: Some(10),
                column: Some(5),
                error_type: "syntax".to_string(),
                message: "expected `;`".to_string(),
                suggestion: None,
            }],
        };

        assert!(!validation.is_success());
        assert_eq!(validation.error_count(), 1);

        let formatted = validation.format_for_llm();
        assert!(formatted.contains("falhou"));
        assert!(formatted.contains("test.rs:10:5"));
    }
}
