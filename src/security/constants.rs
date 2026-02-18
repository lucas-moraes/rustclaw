/// Security-related constants and patterns for prompt injection prevention

/// Maximum allowed input length (10KB)
pub const MAX_INPUT_LENGTH: usize = 10_240;

/// Maximum skill context size (4KB)
pub const MAX_SKILL_CONTEXT_SIZE: usize = 4_096;

/// Maximum tool output size (64KB)
pub const MAX_TOOL_OUTPUT_SIZE: usize = 65_536;

/// Control characters to remove (0x00-0x1F except tab, newline, carriage return)
pub const CONTROL_CHARS: &[char] = &[
    '\u{0000}', '\u{0001}', '\u{0002}', '\u{0003}', '\u{0004}', '\u{0005}', '\u{0006}', '\u{0007}',
    '\u{0008}', '\u{000B}', '\u{000C}', '\u{000E}', '\u{000F}', '\u{0010}', '\u{0011}', '\u{0012}',
    '\u{0013}', '\u{0014}', '\u{0015}', '\u{0016}', '\u{0017}', '\u{0018}', '\u{0019}', '\u{001A}',
    '\u{001B}', '\u{001C}', '\u{001D}', '\u{001E}', '\u{001F}',
];

/// Dangerous markdown sequences to sanitize
pub const DANGEROUS_MARKDOWN: &[&str] = &[
    "```system",
    "```json",
    "```yaml",
    "```python",
    "```javascript",
];

/// Keywords indicating system instructions
pub const SYSTEM_KEYWORDS: &[&str] = &[
    "system:",
    "system prompt",
    "system message",
    "system instruction",
    "ignore previous",
    "ignore all",
    "ignore above",
    "ignore instructions",
    "ignore commands",
    "forget everything",
    "forget all",
    "forget your",
    "forget training",
    "new system",
    "new master",
];

/// Roleplay and persona keywords
pub const PERSONA_KEYWORDS: &[&str] = &[
    "you are now",
    "you have been",
    "you will act as",
    "you will pretend to be",
    "act as",
    "pretend to be",
    "assume the role",
    "play the role",
    "roleplay as",
    "you are a",
    "you are an",
    "from now on you are",
];

/// Jailbreak patterns
pub const JAILBREAK_PATTERNS: &[&str] = &[
    "dan",
    "do anything now",
    "developer mode",
    "ignore previous instructions",
    "ignore your instructions",
    "bypass",
    "override",
    "disregard",
    "without restrictions",
    "unfiltered",
    "no ethics",
    "no moral",
    "no filter",
];

/// Sensitive data patterns to mask (regex)
pub const SENSITIVE_PATTERNS: &[&str] = &[
    "(?i)(api[_-]?key|apikey)[\"']?\\s*[:=]\\s*[\"']?([a-zA-Z0-9_\\-]{16,})[\"']?",
    "(?i)(password|senha|pwd)[\"']?\\s*[:=]\\s*[\"']?([^\\s\"']{8,})[\"']?",
    "(?i)(token|bearer)\\s+([a-zA-Z0-9_\\-\\.]{20,})",
    "(?i)(secret)[\"']?\\s*[:=]\\s*[\"']?([a-zA-Z0-9_\\-]{16,})[\"']?",
    "\\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\\b", // UUIDs
    "\\b[0-9a-fA-F]{32,}\\b",                                                            // Hashes
];

/// Unicode homoglyphs (characters that look like ASCII but are different)
pub const HOMOGLYPH_RANGES: &[(char, char)] = &[
    ('\u{0430}', '\u{044f}'), // Cyrillic а-я (looks like a-y)
    ('\u{0450}', '\u{045f}'), // Cyrillic ѐ-џ
    ('\u{03b1}', '\u{03c9}'), // Greek α-ω
    ('\u{FF10}', '\u{FF19}'), // Fullwidth digits ０-９
    ('\u{FF21}', '\u{FF3A}'), // Fullwidth A-Z Ａ-Ｚ
    ('\u{FF41}', '\u{FF5A}'), // Fullwidth a-z ａ-ｚ
];

/// Delimiter patterns that could break out of JSON
pub const JSON_BREAKOUT_PATTERNS: &[&str] = &[
    r#"""#,  // Double quote
    "[{[",   // JSON opening brackets
    "]}]",   // JSON closing brackets
    "\\\\",  // Escaped backslash
    r"\x00", // Null byte
];

/// Replacement characters for sanitization
pub const REPLACEMENTS: &[(char, char)] = &[('[', '【'), (']', '】'), ('{', '⟦'), ('}', '⟧')];

/// Trust levels for input sources
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    /// Fully trusted (system code)
    System,
    /// Partially trusted (user input)
    User,
    /// Untrusted (external sources, tool outputs)
    Untrusted,
}

impl TrustLevel {
    /// Get the sanitization level required
    pub fn sanitization_required(&self) -> SanitizationLevel {
        match self {
            TrustLevel::System => SanitizationLevel::None,
            TrustLevel::User => SanitizationLevel::Standard,
            TrustLevel::Untrusted => SanitizationLevel::Maximum,
        }
    }
}

/// Sanitization levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SanitizationLevel {
    /// No sanitization needed
    None,
    /// Standard sanitization (user input)
    Standard,
    /// Maximum sanitization (external data)
    Maximum,
}
