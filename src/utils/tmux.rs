use super::output::{OutputManager, OutputSink};
use chrono::Local;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

pub struct TmuxSink {
    session_name: String,
    log_file: PathBuf,
}

impl TmuxSink {
    pub fn new(session_name: &str) -> Self {
        let temp_dir = std::env::temp_dir();
        let log_dir = temp_dir
            .join("rustclaw")
            .join(session_name.replace("rustclaw-", ""));
        std::fs::create_dir_all(&log_dir).ok();

        let log_file = log_dir.join(format!(
            "{}.log",
            session_name.split('-').next_back().unwrap_or("log")
        ));

        Self {
            session_name: session_name.to_string(),
            log_file,
        }
    }

    fn timestamp() -> String {
        Local::now().format("%H:%M:%S").to_string()
    }

    fn write_to_tmux(&self, text: &str) {
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");

        Command::new("tmux")
            .args(["send-keys", "-t", &self.session_name, &escaped, "C-m"])
            .output()
            .ok();

        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file)
            .and_then(|mut f| {
                use std::io::Write;
                writeln!(f, "{}", text)
            })
            .ok();
    }
}

impl OutputSink for TmuxSink {
    fn name(&self) -> &str {
        &self.session_name
    }

    fn write(&self, msg: &str) {
        self.write_to_tmux(msg);
    }

    fn write_line(&self, msg: &str) {
        self.write_to_tmux(msg);
    }

    fn write_tool(&self, tool: &str, input: &str, output: &str) {
        let msg = format!(
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n🛠️  [{}] TOOL: {}\n📦 [{}] Args: {}\n📤 [{}] Output: {}\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
            Self::timestamp(), tool, Self::timestamp(), input, Self::timestamp(), output
        );
        self.write_to_tmux(&msg);
    }

    fn write_thought(&self, thought: &str) {
        let msg = format!("💭 [{}] {}", Self::timestamp(), thought);
        self.write_to_tmux(&msg);
    }

    fn write_error(&self, error: &str) {
        let msg = format!("🔴 [{}] ERROR: {}", Self::timestamp(), error);
        self.write_to_tmux(&msg);
    }

    fn write_browser(&self, path: &str, description: &str) {
        let msg = format!("📸 [{}] {} - {}", Self::timestamp(), description, path);
        self.write_to_tmux(&msg);
    }

    fn flush(&self) {
        // No flush needed for tmux
    }
}

pub struct TmuxManager {
    base_name: String,
    sessions: HashMap<String, String>,
    output_manager: OutputManager,
}

impl TmuxManager {
    pub fn new(skill_name: &str) -> Self {
        let base_name = format!("rustclaw-{}", skill_name);

        Self {
            base_name,
            sessions: HashMap::new(),
            output_manager: OutputManager::new(),
        }
    }

    pub fn is_enabled() -> bool {
        std::env::var("TMUX_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() == "true")
            .unwrap_or(false)
    }

    pub fn create_sessions(&mut self) -> Result<(), String> {
        if !Self::is_enabled() {
            return Ok(());
        }

        let session_types = vec!["agent", "tools", "debug", "browser"];

        for session_type in session_types {
            let session_name = format!("{}-{}", self.base_name, session_type);

            // Kill existing session if exists
            Command::new("tmux")
                .args(["kill-session", "-t", &session_name])
                .output()
                .ok();

            // Create new session (detached)
            let result = Command::new("tmux")
                .args([
                    "new-session",
                    "-d",
                    "-s",
                    &session_name,
                    "-c",
                    &std::env::temp_dir().join("rustclaw").to_string_lossy(),
                ])
                .output()
                .map_err(|e| format!("Failed to create session: {}", e))?;

            if result.status.success() {
                self.sessions
                    .insert(session_type.to_string(), session_name.clone());

                // Create log directory
                let log_dir = std::env::temp_dir()
                    .join("rustclaw")
                    .join(session_name.replace("rustclaw-", ""));
                std::fs::create_dir_all(&log_dir).ok();

                // Add sink to output manager
                self.output_manager
                    .add_sink(std::sync::Arc::new(TmuxSink::new(&session_name)));

                println!("✅ {} (interativo)", session_name);
            } else {
                eprintln!("❌ Failed to create session: {}", session_name);
            }
        }

        // Add console sink for local output
        self.output_manager
            .add_sink(std::sync::Arc::new(super::output::ConsoleSink::new()));

        println!("✅ Sessões TMUX criadas com sucesso!");
        println!(
            "📁 Logs em: {}/rustclaw-{}/",
            std::env::temp_dir().to_string_lossy(),
            self.base_name.replace("rustclaw-", "")
        );
        println!("🔗 Conectar: tmux attach -t {}-agent", self.base_name);

        Ok(())
    }

    pub fn get_output_manager(&self) -> &OutputManager {
        &self.output_manager
    }

    pub fn get_output_manager_mut(&mut self) -> &mut OutputManager {
        &mut self.output_manager
    }

    pub fn session_dir(&self) -> PathBuf {
        std::env::temp_dir()
            .join("rustclaw")
            .join(self.base_name.replace("rustclaw-", ""))
    }

    pub fn browser_dir(&self) -> PathBuf {
        self.session_dir().join("browser")
    }

    pub fn create_browser_screenshot(&self, path: &str, description: &str) {
        self.output_manager.write_browser(path, description);
    }

    pub fn write_agent(&self, msg: &str) {
        if let Some(session) = self.sessions.get("agent") {
            let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
            Command::new("tmux")
                .args(["send-keys", "-t", session, &escaped, "C-m"])
                .output()
                .ok();
        }
    }

    pub fn cleanup(&self) {
        for session in self.sessions.values() {
            Command::new("tmux")
                .args(["kill-session", "-t", session])
                .output()
                .ok();
        }
    }
}

impl Drop for TmuxManager {
    fn drop(&mut self) {
        if Self::is_enabled() {
            println!("\n🧹 Limpando sessões TMUX...");
            self.cleanup();
        }
    }
}
