use std::error::Error as StdError;
use std::fmt;

#[derive(Debug)]
pub enum AgentError {
    Config(ConfigError),
    LLM(LLMError),
    Tool(ToolError),
    Trust(TrustError),
    Memory(MemoryError),
    Session(SessionError),
    Parse(ParseError),
    Internal(InternalError),
}

#[derive(Debug)]
pub enum ConfigError {
    MissingToken,
    InvalidModel(String),
    InvalidUrl(String),
    IoError(String),
}

#[derive(Debug)]
pub enum LLMError {
    ApiCallFailed(String),
    InvalidResponse(String),
    NoChoices,
    NoContent,
    NoMessage,
    ParsingFailed(String),
    RateLimited,
    Timeout,
}

#[derive(Debug)]
pub enum ToolError {
    NotFound(String),
    ExecutionFailed(String),
    SecurityViolation(String),
    InvalidInput(String),
    Timeout,
    OutputTooLarge(usize),
}

#[derive(Debug)]
pub enum TrustError {
    WorkspaceNotTrusted(String),
    OperationBlocked(String),
    NetworkBlocked(String),
    InsufficientTrust,
}

#[derive(Debug)]
pub enum MemoryError {
    StorageFailed(String),
    EmbeddingFailed(String),
    NotFound(String),
    QueryFailed(String),
}

#[derive(Debug)]
pub enum SessionError {
    NotFound(String),
    Expired(String),
    Corrupted(String),
}

#[derive(Debug)]
pub enum ParseError {
    InvalidFormat(String),
    JsonError(String),
    MissingField(String),
    InvalidRegex(String),
}

#[derive(Debug)]
pub enum InternalError {
    LockPoisoned(String),
    ThreadPanic(String),
    Unexpected(String),
}

impl StdError for AgentError {}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentError::Config(e) => write!(f, "Config error: {}", e),
            AgentError::LLM(e) => write!(f, "LLM error: {}", e),
            AgentError::Tool(e) => write!(f, "Tool error: {}", e),
            AgentError::Trust(e) => write!(f, "Trust error: {}", e),
            AgentError::Memory(e) => write!(f, "Memory error: {}", e),
            AgentError::Session(e) => write!(f, "Session error: {}", e),
            AgentError::Parse(e) => write!(f, "Parse error: {}", e),
            AgentError::Internal(e) => write!(f, "Internal error: {}", e),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::MissingToken => write!(f, "TOKEN environment variable not set"),
            ConfigError::InvalidModel(s) => write!(f, "Invalid model: {}", s),
            ConfigError::InvalidUrl(s) => write!(f, "Invalid URL: {}", s),
            ConfigError::IoError(s) => write!(f, "IO error: {}", s),
        }
    }
}

impl fmt::Display for LLMError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LLMError::ApiCallFailed(s) => write!(f, "API call failed: {}", s),
            LLMError::InvalidResponse(s) => write!(f, "Invalid response: {}", s),
            LLMError::NoChoices => write!(f, "No choices in response"),
            LLMError::NoContent => write!(f, "No content in message"),
            LLMError::NoMessage => write!(f, "No message in choice"),
            LLMError::ParsingFailed(s) => write!(f, "Parsing failed: {}", s),
            LLMError::RateLimited => write!(f, "Rate limited"),
            LLMError::Timeout => write!(f, "Request timeout"),
        }
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::NotFound(s) => write!(f, "Tool '{}' not found", s),
            ToolError::ExecutionFailed(s) => write!(f, "Tool execution failed: {}", s),
            ToolError::SecurityViolation(s) => write!(f, "Security violation: {}", s),
            ToolError::InvalidInput(s) => write!(f, "Invalid input: {}", s),
            ToolError::Timeout => write!(f, "Tool execution timeout"),
            ToolError::OutputTooLarge(s) => write!(f, "Output too large: {} bytes", s),
        }
    }
}

impl fmt::Display for TrustError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrustError::WorkspaceNotTrusted(s) => write!(f, "Workspace not trusted: {}", s),
            TrustError::OperationBlocked(s) => write!(f, "Operation blocked: {}", s),
            TrustError::NetworkBlocked(s) => write!(f, "Network request blocked: {}", s),
            TrustError::InsufficientTrust => write!(f, "Trust level insufficient"),
        }
    }
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryError::StorageFailed(s) => write!(f, "Storage failed: {}", s),
            MemoryError::EmbeddingFailed(s) => write!(f, "Embedding failed: {}", s),
            MemoryError::NotFound(s) => write!(f, "Memory not found: {}", s),
            MemoryError::QueryFailed(s) => write!(f, "Query failed: {}", s),
        }
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::NotFound(s) => write!(f, "Session not found: {}", s),
            SessionError::Expired(s) => write!(f, "Session expired: {}", s),
            SessionError::Corrupted(s) => write!(f, "Session corrupted: {}", s),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::InvalidFormat(s) => write!(f, "Invalid format: {}", s),
            ParseError::JsonError(s) => write!(f, "JSON error: {}", s),
            ParseError::MissingField(s) => write!(f, "Missing field: {}", s),
            ParseError::InvalidRegex(s) => write!(f, "Invalid regex: {}", s),
        }
    }
}

impl fmt::Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InternalError::LockPoisoned(s) => write!(f, "Lock poisoned: {}", s),
            InternalError::ThreadPanic(s) => write!(f, "Thread panic: {}", s),
            InternalError::Unexpected(s) => write!(f, "Unexpected error: {}", s),
        }
    }
}

impl From<ConfigError> for AgentError {
    fn from(e: ConfigError) -> Self {
        AgentError::Config(e)
    }
}

impl From<LLMError> for AgentError {
    fn from(e: LLMError) -> Self {
        AgentError::LLM(e)
    }
}

impl From<ToolError> for AgentError {
    fn from(e: ToolError) -> Self {
        AgentError::Tool(e)
    }
}

impl From<TrustError> for AgentError {
    fn from(e: TrustError) -> Self {
        AgentError::Trust(e)
    }
}

impl From<MemoryError> for AgentError {
    fn from(e: MemoryError) -> Self {
        AgentError::Memory(e)
    }
}

impl From<SessionError> for AgentError {
    fn from(e: SessionError) -> Self {
        AgentError::Session(e)
    }
}

impl From<ParseError> for AgentError {
    fn from(e: ParseError) -> Self {
        AgentError::Parse(e)
    }
}

impl From<InternalError> for AgentError {
    fn from(e: InternalError) -> Self {
        AgentError::Internal(e)
    }
}

impl From<std::io::Error> for AgentError {
    fn from(e: std::io::Error) -> Self {
        AgentError::Config(ConfigError::IoError(e.to_string())).into()
    }
}

impl From<reqwest::Error> for AgentError {
    fn from(e: reqwest::Error) -> Self {
        AgentError::LLM(LLMError::ApiCallFailed(e.to_string())).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_error_display() {
        let err = AgentError::LLM(LLMError::NoChoices);
        assert_eq!(format!("{}", err), "LLM error: No choices in response");
    }

    #[test]
    fn test_tool_error_display() {
        let err = ToolError::NotFound("shell".to_string());
        assert_eq!(format!("{}", err), "Tool 'shell' not found");
    }

    #[test]
    fn test_error_from() {
        let llm_err = LLMError::Timeout;
        let agent_err: AgentError = llm_err.into();
        match agent_err {
            AgentError::LLM(e) => assert_eq!(format!("{}", e), "Request timeout"),
            _ => panic!("Expected LLM error"),
        }
    }
}
