use once_cell::sync::Lazy;
use std::sync::RwLock;

#[derive(Clone, Debug, Default)]
pub struct FeatureFlags {
    pub voice_mode: bool,
    pub proactive_mode: bool,
    pub bridge_mode: bool,
    pub daemon_mode: bool,
    pub agent_triggers: bool,
    pub monitor_tool: bool,
    pub debug_mode: bool,
    pub verbose_logging: bool,
}

impl FeatureFlags {
    pub fn load_from_env() -> Self {
        Self {
            voice_mode: std::env::var("VOICE_MODE")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            proactive_mode: std::env::var("PROACTIVE")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            bridge_mode: std::env::var("BRIDGE_MODE")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            daemon_mode: std::env::var("DAEMON")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            agent_triggers: std::env::var("AGENT_TRIGGERS")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            monitor_tool: std::env::var("MONITOR_TOOL")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            debug_mode: std::env::var("DEBUG")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            verbose_logging: std::env::var("VERBOSE")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
        }
    }

    pub fn is_enabled(&self, feature: &str) -> bool {
        match feature {
            "voice" => self.voice_mode,
            "proactive" => self.proactive_mode,
            "bridge" => self.bridge_mode,
            "daemon" => self.daemon_mode,
            "agent_triggers" => self.agent_triggers,
            "monitor_tool" => self.monitor_tool,
            "debug" => self.debug_mode,
            "verbose" => self.verbose_logging,
            _ => false,
        }
    }

    pub fn enable(&mut self, feature: &str) {
        match feature {
            "voice" => self.voice_mode = true,
            "proactive" => self.proactive_mode = true,
            "bridge" => self.bridge_mode = true,
            "daemon" => self.daemon_mode = true,
            "agent_triggers" => self.agent_triggers = true,
            "monitor_tool" => self.monitor_tool = true,
            "debug" => self.debug_mode = true,
            "verbose" => self.verbose_logging = true,
            _ => {}
        }
    }

    pub fn disable(&mut self, feature: &str) {
        match feature {
            "voice" => self.voice_mode = false,
            "proactive" => self.proactive_mode = false,
            "bridge" => self.bridge_mode = false,
            "daemon" => self.daemon_mode = false,
            "agent_triggers" => self.agent_triggers = false,
            "monitor_tool" => self.monitor_tool = false,
            "debug" => self.debug_mode = false,
            "verbose" => self.verbose_logging = false,
            _ => {}
        }
    }
}

static FEATURE_FLAGS: Lazy<RwLock<FeatureFlags>> =
    Lazy::new(|| RwLock::new(FeatureFlags::load_from_env()));

pub fn get_feature_flags() -> FeatureFlags {
    FEATURE_FLAGS.read().unwrap().clone()
}

pub fn is_feature_enabled(feature: &str) -> bool {
    FEATURE_FLAGS.read().unwrap().is_enabled(feature)
}

pub fn enable_feature(feature: &str) {
    FEATURE_FLAGS.write().unwrap().enable(feature)
}

pub fn disable_feature(feature: &str) {
    FEATURE_FLAGS.write().unwrap().disable(feature)
}

pub fn reload_features() {
    let mut flags = FEATURE_FLAGS.write().unwrap();
    *flags = FeatureFlags::load_from_env();
}
