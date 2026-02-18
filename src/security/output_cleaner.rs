use crate::security::constants::*;
use regex::Regex;

/// Cleans and sanitizes tool output before sending to LLM
pub struct OutputCleaner;

impl OutputCleaner {
    /// Clean generic tool output
    pub fn clean(output: &str) -> String {
        let mut cleaned = output.to_string();

        // 1. Limit size
        if cleaned.len() > MAX_TOOL_OUTPUT_SIZE {
            cleaned = format!(
                "{}\n\n[Output truncated: {} bytes removed]",
                &cleaned[..MAX_TOOL_OUTPUT_SIZE],
                cleaned.len() - MAX_TOOL_OUTPUT_SIZE
            );
        }

        // 2. Remove control sequences
        cleaned = Self::remove_control_sequences(&cleaned);

        // 3. Remove prompt injection attempts from output
        cleaned = Self::remove_prompt_injection(&cleaned);

        // 4. Mask sensitive data
        cleaned = Self::mask_sensitive_patterns(&cleaned);

        // 5. Normalize line endings
        cleaned = cleaned.replace("\r\n", "\n").replace("\r", "\n");

        cleaned
    }

    /// Clean shell command output
    pub fn clean_shell(output: &str) -> String {
        let mut cleaned = Self::clean(output);

        // Remove shell prompts that might confuse the LLM
        let prompt_re = Regex::new(r"^\s*\$\s+|^\s*>\s+|^\s*%\s+").unwrap();
        cleaned = prompt_re
            .replace_all(&cleaned, "")
            .lines()
            .collect::<Vec<_>>()
            .join("\n");

        cleaned
    }

    /// Clean file content
    pub fn clean_file(content: &str, file_type: &str) -> String {
        let mut cleaned = Self::clean(content);

        match file_type {
            "json" => {
                // Validate JSON
                if let Err(e) = serde_json::from_str::<serde_json::Value>(&cleaned) {
                    cleaned = format!(
                        "[Invalid JSON: {}]\n{}",
                        e,
                        &cleaned[..cleaned.len().min(200)]
                    );
                }
            }
            "html" | "xml" => {
                // Remove scripts from HTML/XML
                cleaned = Self::remove_scripts(&cleaned);
            }
            _ => {}
        }

        cleaned
    }

    /// Clean HTTP response
    pub fn clean_http(response: &str) -> String {
        let mut cleaned = Self::clean(response);

        // Remove sensitive headers
        let sensitive_headers = [
            "authorization:",
            "cookie:",
            "set-cookie:",
            "x-api-key:",
            "api-key:",
        ];

        for header in &sensitive_headers {
            let re = Regex::new(&format!(r"(?im)^{}\s*.+$", regex::escape(header))).unwrap();
            cleaned = re.replace_all(&cleaned, "$1 [REDACTED]").to_string();
        }

        cleaned
    }

    /// Remove ANSI escape sequences
    fn remove_control_sequences(text: &str) -> String {
        // ANSI color codes
        let ansi_re = Regex::new(r"\x1b\[[0-9;]*m").unwrap();
        let mut cleaned = ansi_re.replace_all(text, "").to_string();

        // Other control sequences
        cleaned = cleaned
            .replace("\x07", "") // Bell
            .replace("\x08", "") // Backspace
            .replace("\x0b", "") // Vertical tab
            .replace("\x0c", ""); // Form feed

        cleaned
    }

    /// Remove potential prompt injection from output
    fn remove_prompt_injection(text: &str) -> String {
        let mut cleaned = text.to_string();

        // Patterns that might be prompt injection
        let injection_patterns = [
            r"(?i)ignore\s+previous\s+instructions",
            r"(?i)system\s*:",
            r"(?i)you\s+are\s+now",
            r"(?i)final\s+answer\s*:",
            r"(?i)action\s*:\s*\{[^}]+\}",
            r"(?i)thought\s*:",
        ];

        for pattern in &injection_patterns {
            if let Ok(re) = Regex::new(pattern) {
                cleaned = re.replace_all(&cleaned, "[REDACTED]").to_string();
            }
        }

        cleaned
    }

    /// Mask sensitive patterns
    fn mask_sensitive_patterns(text: &str) -> String {
        let mut cleaned = text.to_string();

        for pattern in SENSITIVE_PATTERNS {
            if let Ok(re) = Regex::new(pattern) {
                cleaned = re.replace_all(&cleaned, "[REDACTED]").to_string();
            }
        }

        cleaned
    }

    /// Remove scripts from HTML
    fn remove_scripts(html: &str) -> String {
        let script_re = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
        let style_re = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();

        let mut cleaned = script_re.replace_all(html, "[SCRIPT REMOVED]").to_string();
        cleaned = style_re.replace_all(&cleaned, "").to_string();

        cleaned
    }
}

/// Clean output based on tool type
pub fn clean_tool_output(output: &str, tool_name: &str) -> String {
    match tool_name {
        "shell" => OutputCleaner::clean_shell(output),
        "file_read" => OutputCleaner::clean_file(output, "text"),
        "http_get" | "http_post" => OutputCleaner::clean_http(output),
        _ => OutputCleaner::clean(output),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_control_sequences() {
        let input = "\x1b[31mRed Text\x1b[0m";
        let cleaned = OutputCleaner::clean(input);
        assert!(!cleaned.contains("\x1b["));
        assert!(cleaned.contains("Red Text"));
    }

    #[test]
    fn test_truncate_long_output() {
        let input = "x".repeat(MAX_TOOL_OUTPUT_SIZE + 100);
        let cleaned = OutputCleaner::clean(&input);
        assert!(cleaned.contains("truncated"));
        assert!(cleaned.len() < input.len());
    }

    #[test]
    fn test_mask_sensitive_data() {
        let input = "api_key=sk-1234567890abcdef";
        let cleaned = OutputCleaner::clean(input);
        assert!(cleaned.contains("[REDACTED]"));
        assert!(!cleaned.contains("sk-1234567890abcdef"));
    }

    #[test]
    fn test_remove_scripts_from_html() {
        let input = r#"<html><script>alert('xss')</script><body>Hello</body></html>"#;
        let cleaned = OutputCleaner::clean_file(input, "html");
        assert!(cleaned.contains("[SCRIPT REMOVED]"));
        assert!(!cleaned.contains("alert"));
    }

    #[test]
    fn test_clean_http_removes_auth() {
        let input = "Authorization: Bearer secret123\nContent-Type: text/plain";
        let cleaned = OutputCleaner::clean_http(input);
        assert!(cleaned.contains("[REDACTED]"));
        assert!(!cleaned.contains("secret123"));
    }
}
