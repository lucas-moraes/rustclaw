use std::sync::Arc;
use std::sync::OnceLock;

#[derive(Clone)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn from_env() -> Self {
        std::env::var("LOG_LEVEL")
            .ok()
            .map(|l| match l.to_lowercase().as_str() {
                "debug" => LogLevel::Debug,
                "warn" | "warning" => LogLevel::Warn,
                "error" => LogLevel::Error,
                _ => LogLevel::Info,
            })
            .unwrap_or(LogLevel::Info)
    }

    pub fn should_log(&self, level: &LogLevel) -> bool {
        match (self, level) {
            (LogLevel::Debug, _) => true,
            (LogLevel::Info, LogLevel::Debug) => false,
            (LogLevel::Info, _) => true,
            (LogLevel::Warn, LogLevel::Debug) => false,
            (LogLevel::Warn, LogLevel::Info) => false,
            (LogLevel::Warn, _) => true,
            (LogLevel::Error, LogLevel::Debug) => false,
            (LogLevel::Error, LogLevel::Info) => false,
            (LogLevel::Error, LogLevel::Warn) => false,
            (LogLevel::Error, LogLevel::Error) => true,
        }
    }
}

pub trait OutputSink: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn write(&self, msg: &str);
    fn write_line(&self, msg: &str);
    fn write_tool(&self, tool: &str, input: &str, output: &str);
    fn write_thought(&self, thought: &str);
    fn write_error(&self, error: &str);
    fn write_browser(&self, path: &str, description: &str);
    fn flush(&self);
}

pub struct OutputManager {
    sinks: Vec<Arc<dyn OutputSink>>,
    log_level: LogLevel,
}

impl OutputManager {
    pub fn new() -> Self {
        Self {
            sinks: Vec::new(),
            log_level: LogLevel::from_env(),
        }
    }

    pub fn add_sink(&mut self, sink: Arc<dyn OutputSink>) {
        self.sinks.push(sink);
    }

    pub fn write(&self, msg: &str) {
        if !self.log_level.should_log(&LogLevel::Info) {
            return;
        }
        for sink in &self.sinks {
            sink.write(msg);
        }
    }

    pub fn write_line(&self, msg: &str) {
        if !self.log_level.should_log(&LogLevel::Info) {
            return;
        }
        for sink in &self.sinks {
            sink.write_line(msg);
        }
    }

    pub fn write_tool(&self, tool: &str, input: &str, output: &str) {
        if !self.log_level.should_log(&LogLevel::Info) {
            return;
        }
        for sink in &self.sinks {
            sink.write_tool(tool, input, output);
        }
    }

    pub fn write_thought(&self, thought: &str) {
        if !self.log_level.should_log(&LogLevel::Debug) {
            return;
        }
        for sink in &self.sinks {
            sink.write_thought(thought);
        }
    }

    pub fn write_error(&self, error: &str) {
        if !self.log_level.should_log(&LogLevel::Error) {
            return;
        }
        for sink in &self.sinks {
            sink.write_error(error);
        }
    }

    pub fn write_debug(&self, msg: &str) {
        if !self.log_level.should_log(&LogLevel::Debug) {
            return;
        }
        for sink in &self.sinks {
            sink.write_line(&format!("🐛 {}", msg));
        }
    }

    pub fn write_browser(&self, path: &str, description: &str) {
        if !self.log_level.should_log(&LogLevel::Info) {
            return;
        }
        for sink in &self.sinks {
            sink.write_browser(path, description);
        }
    }

    pub fn flush(&self) {
        for sink in &self.sinks {
            sink.flush();
        }
    }
}

impl Default for OutputManager {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ConsoleSink;

impl ConsoleSink {
    pub fn new() -> Self {
        Self
    }
}

impl OutputSink for ConsoleSink {
    fn name(&self) -> &str {
        "console"
    }

    fn write(&self, msg: &str) {
        print!("{}", msg);
    }

    fn write_line(&self, msg: &str) {
        println!("{}", msg);
    }

    fn write_tool(&self, tool: &str, input: &str, output: &str) {
        println!("🛠️  TOOL: {}", tool);
        println!("📦 Args: {}", input);
        println!("📤 Output: {}", output);
    }

    fn write_thought(&self, thought: &str) {
        println!("💭 {}", thought);
    }

    fn write_error(&self, error: &str) {
        eprintln!("❌ {}", error);
    }

    fn write_browser(&self, path: &str, description: &str) {
        println!("📸 {} - {}", description, path);
    }

    fn flush(&self) {
        std::io::Write::flush(&mut std::io::stdout()).ok();
    }
}
