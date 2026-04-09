#![allow(dead_code)]

use crate::utils::output::OutputManager;
use crate::utils::tmux::TmuxManager;
use std::sync::OnceLock;

static OUTPUT_MANAGER: OnceLock<OutputManager> = OnceLock::new();
static TMUX_MANAGER: OnceLock<TmuxManager> = OnceLock::new();

pub fn init_tmux(skill_name: &str) {
    if TmuxManager::is_enabled() {
        let mut manager = TmuxManager::new(skill_name);
        if let Err(e) = manager.create_sessions() {
            eprintln!("⚠️  Erro ao criar sessões TMUX: {}", e);
        }

        let mut output = OutputManager::new();
        output.add_sink(std::sync::Arc::new(crate::utils::output::ConsoleSink::new()));

        let _ = TMUX_MANAGER.set(manager);
        let _ = OUTPUT_MANAGER.set(output);
    }
}

pub fn get_tmux_manager() -> Option<&'static TmuxManager> {
    TMUX_MANAGER.get()
}

pub fn get_output_manager() -> Option<&'static OutputManager> {
    OUTPUT_MANAGER.get()
}

pub fn output_write(msg: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write(msg);
    }
    print!("{}", msg);
}

pub fn output_write_line(msg: &str) {
    if let Some(output) = OUTPUT_MANAGER.get() {
        output.write_line(msg);
    }
    println!("{}", msg);
}

pub fn output_write_tool(tool: &str, input: &str, output: &str) {
    let formatted = format!("🔧 [{}] Input: {}\nOutput: {}", tool, input, output);
    output_write_line(&formatted);
}

pub fn output_write_thought(thought: &str) {
    let formatted = format!("💭 {}", thought);
    output_write_line(&formatted);
}

pub fn output_write_error(error: &str) {
    let formatted = format!("❌ Erro: {}", error);
    output_write_line(&formatted);
}

pub fn output_write_debug(msg: &str) {
    #[cfg(debug_assertions)]
    {
        let formatted = format!("🔍 Debug: {}", msg);
        output_write_line(&formatted);
    }
}

pub fn output_write_browser(path: &str, description: &str) {
    let formatted = format!("🌐 Browser: {} - {}", path, description);
    output_write_line(&formatted);
}
