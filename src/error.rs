// Variants are the public error surface; some are not constructed by the binary yet.
#![allow(dead_code)]

use std::error::Error as StdError;
use std::fmt;

/// Top-level error type for the harness.
#[derive(Debug)]
pub enum AgentError {
    Config(ConfigError),
    Internal(String),
}

#[derive(Debug)]
pub enum ConfigError {
    MissingToken,
    InvalidModel(String),
    InvalidUrl(String),
    IoError(String),
}

impl StdError for AgentError {}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentError::Config(e) => write!(f, "Config error: {}", e),
            AgentError::Internal(e) => write!(f, "Internal error: {}", e),
        }
    }
}

impl From<ConfigError> for AgentError {
    fn from(e: ConfigError) -> Self {
        AgentError::Config(e)
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::MissingToken => {
                write!(f, "Missing TOKEN/OPENCODE_API_KEY environment variable")
            }
            ConfigError::InvalidModel(m) => write!(f, "Invalid model/API key: {}", m),
            ConfigError::InvalidUrl(u) => write!(f, "Invalid base URL: {}", u),
            ConfigError::IoError(e) => write!(f, "IO error: {}", e),
        }
    }
}
