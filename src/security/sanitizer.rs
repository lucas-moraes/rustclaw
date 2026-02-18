use crate::security::constants::*;
use crate::security::TrustLevel;

/// Result of sanitization
#[derive(Debug, Clone)]
pub struct SanitizedInput {
    pub text: String,
    pub was_modified: bool,
    pub original_length: usize,
    pub sanitized_length: usize,
}

impl SanitizedInput {
    /// Create sanitized input
    pub fn new(text: String, was_modified: bool, original_length: usize) -> Self {
        let sanitized_length = text.len();
        Self {
            text,
            was_modified,
            original_length,
            sanitized_length,
        }
    }

    /// Check if text was truncated
    pub fn was_truncated(&self) -> bool {
        self.sanitized_length < self.original_length
    }
}

/// Text sanitizer for different contexts
pub struct Sanitizer;

impl Sanitizer {
    /// Sanitize user input
    pub fn user_input(input: &str) -> SanitizedInput {
        let original_length = input.len();

        // 1. Limit length
        let mut text = if input.len() > MAX_INPUT_LENGTH {
            input[..MAX_INPUT_LENGTH].to_string()
        } else {
            input.to_string()
        };

        let was_modified = text.len() < original_length;

        // 2. Remove control characters
        text = Self::remove_control_chars(&text);

        // 3. Normalize unicode (prevent homoglyph attacks)
        text = Self::normalize_unicode(&text);

        // 4. Escape dangerous sequences
        text = Self::escape_dangerous_sequences(&text);

        // 5. Clean markdown
        text = Self::sanitize_markdown(&text);

        SanitizedInput::new(text, was_modified, original_length)
    }

    /// Sanitize skill context
    pub fn skill_context(input: &str) -> String {
        // 1. Limit length
        let mut text = if input.len() > MAX_SKILL_CONTEXT_SIZE {
            input[..MAX_SKILL_CONTEXT_SIZE].to_string()
        } else {
            input.to_string()
        };

        // 2. Remove system instruction keywords
        text = Self::remove_system_keywords(&text);

        // 3. Escape dangerous sequences
        text = Self::escape_dangerous_sequences(&text);

        // 4. Remove markdown code blocks that could be dangerous
        text = Self::sanitize_markdown(&text);

        // 5. Normalize whitespace
        text = Self::normalize_whitespace(&text);

        text
    }

    /// Sanitize tool output
    pub fn tool_output(output: &str, tool_name: &str) -> String {
        let original_len = output.len();

        // 1. Limit length
        let mut text = if output.len() > MAX_TOOL_OUTPUT_SIZE {
            format!(
                "{}\n[Output truncated - {} bytes removed]",
                &output[..MAX_TOOL_OUTPUT_SIZE],
                original_len - MAX_TOOL_OUTPUT_SIZE
            )
        } else {
            output.to_string()
        };

        // 2. Mask sensitive data
        text = Self::mask_sensitive_data(&text);

        // 3. Remove control characters
        text = Self::remove_control_chars(&text);

        // 4. Sanitize based on tool type
        text = match tool_name {
            "shell" | "system_info" => Self::sanitize_shell_output(&text),
            "file_read" => Self::sanitize_file_content(&text),
            "http_get" | "http_post" => Self::sanitize_http_response(&text),
            _ => text,
        };

        // 5. Prevent prompt injection via tool output
        text = Self::escape_dangerous_sequences(&text);

        text
    }

    /// Remove control characters
    fn remove_control_chars(text: &str) -> String {
        text.chars()
            .filter(|c| !CONTROL_CHARS.contains(c))
            .collect()
    }

    /// Normalize unicode to prevent homoglyph attacks
    fn normalize_unicode(text: &str) -> String {
        // Simple normalization: replace common homoglyphs with ASCII equivalents
        text.chars()
            .map(|c| {
                // Check if character is in homoglyph ranges
                for (start, end) in HOMOGLYPH_RANGES {
                    if *start <= c && c <= *end {
                        // Replace with '?' to flag suspicious characters
                        return '�';
                    }
                }
                c
            })
            .collect()
    }

    /// Escape dangerous sequences that could break out of JSON/prompt
    fn escape_dangerous_sequences(text: &str) -> String {
        let mut result = text.to_string();

        for (from, to) in REPLACEMENTS {
            result = result.replace(*from, &to.to_string());
        }

        result
    }

    /// Sanitize markdown content
    fn sanitize_markdown(text: &str) -> String {
        let mut result = text.to_string();

        // Replace dangerous code blocks with safe markers
        for pattern in DANGEROUS_MARKDOWN {
            result = result.replace(pattern, "【CODE BLOCK REMOVED】");
        }

        // Remove HTML tags
        let html_re = regex::Regex::new(r"<[^>]+>").unwrap();
        result = html_re.replace_all(&result, "").to_string();

        result
    }

    /// Remove system instruction keywords from text
    fn remove_system_keywords(text: &str) -> String {
        let mut result = text.to_string();
        let lowercase = text.to_lowercase();

        for keyword in SYSTEM_KEYWORDS.iter().chain(PERSONA_KEYWORDS.iter()) {
            if lowercase.contains(&keyword.to_lowercase()) {
                // Replace with neutral placeholder
                result = result.replace(&keyword.to_lowercase(), "[REDACTED]");
            }
        }

        result
    }

    /// Mask sensitive data patterns
    fn mask_sensitive_data(text: &str) -> String {
        let mut result = text.to_string();

        for pattern in SENSITIVE_PATTERNS {
            if let Ok(re) = regex::Regex::new(pattern) {
                result = re.replace_all(&result, "[REDACTED]").to_string();
            }
        }

        result
    }

    /// Normalize whitespace
    fn normalize_whitespace(text: &str) -> String {
        text.lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Sanitize shell command output
    fn sanitize_shell_output(text: &str) -> String {
        // Remove potential ANSI escape sequences
        let ansi_re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
        ansi_re.replace_all(text, "").to_string()
    }

    /// Sanitize file content
    fn sanitize_file_content(text: &str) -> String {
        // Limit lines to prevent overwhelming the LLM
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > 1000 {
            let truncated: Vec<&str> = lines[..1000].to_vec();
            format!(
                "{}\n[File truncated - {} lines removed]",
                truncated.join("\n"),
                lines.len() - 1000
            )
        } else {
            text.to_string()
        }
    }

    /// Sanitize HTTP response
    fn sanitize_http_response(text: &str) -> String {
        // Remove headers that might contain sensitive info
        let mut result = text.to_string();

        // Mask cookies and auth headers
        let cookie_re = regex::Regex::new(r"(?i)(Set-Cookie|Cookie):\s*[^\r\n]+").unwrap();
        result = cookie_re.replace_all(&result, "$1: [REDACTED]").to_string();

        let auth_re = regex::Regex::new(r"(?i)(Authorization|X-API-Key):\s*[^\r\n]+").unwrap();
        result = auth_re.replace_all(&result, "$1: [REDACTED]").to_string();

        result
    }
}

/// Sanitize based on trust level
pub fn sanitize_with_trust_level(text: &str, trust_level: TrustLevel, context: &str) -> String {
    match trust_level {
        TrustLevel::System => text.to_string(),
        TrustLevel::User => {
            let sanitized = Sanitizer::user_input(text);
            sanitized.text
        }
        TrustLevel::Untrusted => {
            let sanitized = match context {
                "skill" => Sanitizer::skill_context(text),
                "tool" => Sanitizer::tool_output(text, "unknown"),
                _ => {
                    let s = Sanitizer::user_input(text);
                    s.text
                }
            };
            sanitized
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_input_sanitization() {
        let input = "Hello\x00World";
        let result = Sanitizer::user_input(input);
        assert!(!result.text.contains('\x00'));
        assert!(result.was_modified);
    }

    #[test]
    fn test_skill_context_removes_system_keywords() {
        let input = "This is system: prompt and instructions";
        let result = Sanitizer::skill_context(input);
        assert!(!result.to_lowercase().contains("system:"));
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_mask_sensitive_data() {
        let input = "api_key=sk-1234567890abcdef";
        let result = Sanitizer::tool_output(input, "test");
        assert!(result.contains("[REDACTED]"));
        assert!(!result.contains("sk-1234567890abcdef"));
    }

    #[test]
    fn test_escape_dangerous_sequences() {
        let input = "Hello [world] {test}";
        let result = Sanitizer::user_input(input);
        assert!(!result.text.contains('['));
        assert!(result.text.contains('【'));
    }

    #[test]
    fn test_truncate_long_input() {
        let input = "x".repeat(MAX_INPUT_LENGTH + 100);
        let result = Sanitizer::user_input(&input);
        assert_eq!(result.text.len(), MAX_INPUT_LENGTH);
        assert!(result.was_truncated());
    }
}
