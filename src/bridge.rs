use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdeMessage {
    pub msg_type: String,
    pub uuid: Option<String>,
    pub content: Option<String>,
    pub subtype: Option<String>,
}

impl IdeMessage {
    pub fn user(content: String) -> Self {
        Self {
            msg_type: "user".to_string(),
            uuid: Some(uuid::Uuid::new_v4().to_string()),
            content: Some(content),
            subtype: None,
        }
    }

    pub fn assistant(content: String) -> Self {
        Self {
            msg_type: "assistant".to_string(),
            uuid: Some(uuid::Uuid::new_v4().to_string()),
            content: Some(content),
            subtype: None,
        }
    }

    pub fn result(subtype: &str) -> Self {
        Self {
            msg_type: "result".to_string(),
            uuid: None,
            content: None,
            subtype: Some(subtype.to_string()),
        }
    }
}

pub struct IdeBridge {
    command: Option<String>,
    args: Vec<String>,
    env: std::collections::HashMap<String, String>,
    running: bool,
}

impl IdeBridge {
    pub fn new() -> Self {
        Self {
            command: None,
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            running: false,
        }
    }

    pub fn with_command(mut self, command: String) -> Self {
        self.command = Some(command);
        self
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }

    pub fn start(&mut self) -> Result<mpsc::UnboundedSender<IdeMessage>, String> {
        let command = self.command.take()
            .ok_or("No command specified")?;

        let mut cmd = Command::new(&command);
        cmd.args(&self.args);
        cmd.envs(&self.env);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn()
            .map_err(|e| format!("Failed to start IDE bridge: {}", e))?;

        let stdin = child.stdin.take()
            .ok_or("Failed to take stdin")?;
        let stdout = child.stdout.take()
            .ok_or("Failed to take stdout")?;

        let (tx, mut rx) = mpsc::unbounded_channel::<IdeMessage>();

        let writer = std::sync::Mutex::new(stdin);
        
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);

            for line in reader.lines() {
                if let Ok(line) = line {
                    debug!("IDE bridge received: {}", line);
                }
            }
        });

        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Ok(json) = serde_json::to_string(&msg) {
                    if let Ok(mut w) = writer.lock() {
                        let _ = writeln!(&mut w, "{}", json);
                    }
                }
            }
        });

        self.running = true;
        Ok(tx)
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn stop(&mut self) {
        self.running = false;
    }
}

impl Default for IdeBridge {
    fn default() -> Self {
        Self::new()
    }
}

pub fn create_stdout_bridge() -> IdeBridge {
    let mut bridge = IdeBridge::new();
    if let Some(exe) = std::env::current_exe().ok() {
        bridge = bridge.with_command(exe.to_string_lossy().to_string());
    }
    bridge.with_args(vec!["--mode=bridge".to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ide_bridge_creation() {
        let bridge = IdeBridge::new();
        assert!(!bridge.is_running());
    }

    #[test]
    fn test_ide_message_user() {
        let msg = IdeMessage::user("Hello".to_string());
        assert_eq!(msg.msg_type, "user");
        assert!(msg.uuid.is_some());
    }
}
