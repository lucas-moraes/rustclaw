use crate::security::constants::*;
use crate::security::injection_detector::InjectionDetector;

/// Validation result
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationResult {
    pub fn valid() -> Self {
        Self {
            valid: true,
            errors: vec![],
            warnings: vec![],
        }
    }

    pub fn invalid(errors: Vec<String>) -> Self {
        Self {
            valid: false,
            errors,
            warnings: vec![],
        }
    }

    pub fn with_warnings(valid: bool, errors: Vec<String>, warnings: Vec<String>) -> Self {
        Self {
            valid,
            errors,
            warnings,
        }
    }

    pub fn add_error(&mut self, error: impl Into<String>) {
        self.errors.push(error.into());
        self.valid = false;
    }

    pub fn add_warning(&mut self, warning: impl Into<String>) {
        self.warnings.push(warning.into());
    }
}

/// Validates different types of input
pub struct Validator;

impl Validator {
    /// Validate user input
    pub fn user_input(text: &str) -> ValidationResult {
        let mut result = ValidationResult::valid();

        // Check length
        if text.len() > MAX_INPUT_LENGTH {
            result.add_error(format!(
                "Input exceeds maximum length of {} bytes",
                MAX_INPUT_LENGTH
            ));
        }

        // Check for empty input
        if text.trim().is_empty() {
            result.add_error("Input cannot be empty");
        }

        // Check for null bytes
        if text.contains('\x00') {
            result.add_error("Input contains null bytes");
        }

        // Check for injection
        let detector = InjectionDetector::new();
        let detection = detector.detect(text);
        if detection.detected {
            result.add_error(format!(
                "Potential security issue detected: {}. Confidence: {:.0}%",
                detection.attack_type.description(),
                detection.confidence * 100.0
            ));
        }

        // Check for suspicious unicode (basic check)
        if Self::has_suspicious_unicode(text) {
            result.add_warning("Suspicious unicode characters detected");
        }

        result
    }

    /// Validate skill context
    pub fn skill_context(text: &str) -> ValidationResult {
        let mut result = ValidationResult::valid();

        // Check length
        if text.len() > MAX_SKILL_CONTEXT_SIZE {
            result.add_error(format!(
                "Skill context exceeds maximum length of {} bytes",
                MAX_SKILL_CONTEXT_SIZE
            ));
        }

        // Check for system keywords
        let lowercase = text.to_lowercase();
        for keyword in SYSTEM_KEYWORDS {
            if lowercase.contains(&keyword.to_lowercase()) {
                result.add_warning(format!(
                    "Skill context contains potentially dangerous keyword: {}",
                    keyword
                ));
            }
        }

        // Check structure (basic markdown check)
        if !text.contains("# Skill:") {
            result.add_warning("Skill context should contain '# Skill:' header");
        }

        result
    }

    /// Validate tool arguments
    pub fn tool_args(args: &serde_json::Value) -> ValidationResult {
        let mut result = ValidationResult::valid();

        // Check if args is an object
        if !args.is_object() {
            result.add_error("Tool arguments must be a JSON object");
            return result;
        }

        // Check for injection in string values
        if let Some(obj) = args.as_object() {
            for (key, value) in obj {
                if let Some(text) = value.as_str() {
                    if text.len() > MAX_INPUT_LENGTH {
                        result.add_error(format!("Argument '{}' exceeds maximum length", key));
                    }

                    // Check for injection
                    let detector = InjectionDetector::new();
                    let detection = detector.detect(text);
                    if detection.detected {
                        result.add_error(format!(
                            "Potential injection in argument '{}': {}",
                            key,
                            detection.attack_type.description()
                        ));
                    }
                }
            }
        }

        result
    }

    /// Validate memory content
    pub fn memory_content(content: &str) -> ValidationResult {
        let mut result = ValidationResult::valid();

        // Check length
        if content.len() > 10000 {
            result.add_warning("Memory content is very long, consider summarizing");
        }

        // Basic injection check
        let detector = InjectionDetector::new();
        let detection = detector.detect(content);
        if detection.detected {
            result.add_warning(format!(
                "Memory content may contain suspicious patterns: {}",
                detection.attack_type.description()
            ));
        }

        result
    }

    /// Check for suspicious unicode characters
    fn has_suspicious_unicode(text: &str) -> bool {
        for c in text.chars() {
            // Check control characters (excluding common ones)
            if c.is_control() && !matches!(c, '\n' | '\r' | '\t') {
                return true;
            }

            // Check for homoglyph ranges
            for (start, end) in HOMOGLYPH_RANGES {
                if *start <= c && c <= *end {
                    return true;
                }
            }
        }
        false
    }

    /// Validate file path for directory traversal
    pub fn file_path(path: &str) -> ValidationResult {
        let mut result = ValidationResult::valid();

        // Check for directory traversal
        if path.contains("..") || path.contains("~") {
            result.add_error("Path contains potentially dangerous characters");
        }

        // Check for absolute paths
        if path.starts_with('/') || (path.len() > 1 && path[1..].starts_with(":")) {
            result.add_warning("Absolute paths may not work as expected");
        }

        // Check for null bytes
        if path.contains('\x00') {
            result.add_error("Path contains null bytes");
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_user_input() {
        let result = Validator::user_input("Hello world");
        assert!(result.valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_empty_input() {
        let result = Validator::user_input("");
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.contains("empty")));
    }

    #[test]
    fn test_input_with_null_bytes() {
        let result = Validator::user_input("Hello\x00World");
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.contains("null bytes")));
    }

    #[test]
    fn test_skill_context_with_system_keyword() {
        let result = Validator::skill_context("This contains system: prompt");
        assert!(result.warnings.iter().any(|w| w.contains("system")));
    }

    #[test]
    fn test_file_path_traversal() {
        let result = Validator::file_path("../etc/passwd");
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.contains("dangerous")));
    }
}
