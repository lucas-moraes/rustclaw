use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub enum AuthSource {
    Environment,
    Keychain,
    ConfigFile,
}

pub trait AuthProvider: Send + Sync {
    fn get_token(&self) -> Option<(AuthTokens, AuthSource)>;
    fn source_name(&self) -> &'static str;
}

pub struct EnvironmentAuthProvider {
    api_key_var: String,
}

impl EnvironmentAuthProvider {
    pub fn new(api_key_var: &str) -> Self {
        Self {
            api_key_var: api_key_var.to_string(),
        }
    }
}

impl AuthProvider for EnvironmentAuthProvider {
    fn get_token(&self) -> Option<(AuthTokens, AuthSource)> {
        std::env::var(&self.api_key_var).ok().map(|key| {
            (
                AuthTokens {
                    access_token: key,
                    refresh_token: None,
                    expires_at: None,
                },
                AuthSource::Environment,
            )
        })
    }

    fn source_name(&self) -> &'static str {
        "environment"
    }
}

pub struct KeychainAuthProvider {
    service: String,
    account: String,
}

impl KeychainAuthProvider {
    pub fn new(service: &str, account: &str) -> Self {
        Self {
            service: service.to_string(),
            account: account.to_string(),
        }
    }
}

impl AuthProvider for KeychainAuthProvider {
    fn get_token(&self) -> Option<(AuthTokens, AuthSource)> {
        use keyring::Entry;

        let entry = Entry::new(&self.service, &self.account).ok()?;
        entry.get_password().ok().map(|key| {
            (
                AuthTokens {
                    access_token: key,
                    refresh_token: None,
                    expires_at: None,
                },
                AuthSource::Keychain,
            )
        })
    }

    fn source_name(&self) -> &'static str {
        "keychain"
    }
}

pub struct ConfigFileAuthProvider {
    config_path: std::path::PathBuf,
}

impl ConfigFileAuthProvider {
    pub fn new(config_path: &str) -> Self {
        Self {
            config_path: std::path::PathBuf::from(config_path),
        }
    }
}

impl AuthProvider for ConfigFileAuthProvider {
    fn get_token(&self) -> Option<(AuthTokens, AuthSource)> {
        if !self.config_path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&self.config_path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&content).ok()?;

        json.get("api_key")
            .or_else(|| json.get("token"))
            .and_then(|v| v.as_str())
            .map(|key| {
                (
                    AuthTokens {
                        access_token: key.to_string(),
                        refresh_token: None,
                        expires_at: None,
                    },
                    AuthSource::ConfigFile,
                )
            })
    }

    fn source_name(&self) -> &'static str {
        "config_file"
    }
}

pub struct ChainedAuthProvider {
    providers: Vec<Box<dyn AuthProvider>>,
}

impl ChainedAuthProvider {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn with_provider(mut self, provider: Box<dyn AuthProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    pub fn get_token(&self) -> Option<(AuthTokens, AuthSource)> {
        for provider in &self.providers {
            if let Some((tokens, source)) = provider.get_token() {
                tracing::info!("Using auth from {} source", provider.source_name());
                return Some((tokens, source));
            }
        }
        None
    }

    pub fn standard() -> Self {
        Self::new()
            .with_provider(Box::new(EnvironmentAuthProvider::new("OPENCODE_API_KEY")))
            .with_provider(Box::new(EnvironmentAuthProvider::new("TOKEN")))
            .with_provider(Box::new(KeychainAuthProvider::new("rustclaw", "api_key")))
            .with_provider(Box::new(ConfigFileAuthProvider::new(".env")))
    }
}

impl Default for ChainedAuthProvider {
    fn default() -> Self {
        Self::standard()
    }
}

pub static AUTH_PROVIDER: Lazy<ChainedAuthProvider> = Lazy::new(ChainedAuthProvider::standard);

pub fn get_api_key() -> Option<String> {
    AUTH_PROVIDER
        .get_token()
        .map(|(tokens, _)| tokens.access_token)
}

pub fn save_to_keychain(service: &str, account: &str, password: &str) -> Result<(), String> {
    use keyring::Entry;

    let entry = Entry::new(service, account)
        .map_err(|e| format!("Failed to create keyring entry: {}", e))?;

    entry
        .set_password(password)
        .map_err(|e| format!("Failed to save to keychain: {}", e))?;

    Ok(())
}
