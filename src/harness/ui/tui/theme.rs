//! Color themes for the Cyberclaw TUI.

use ratatui::style::{Color, Modifier, Style};

/// Named color palette driving every widget.
#[derive(Clone, Debug)]
pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub surface: Color,
    pub border: Color,
    pub border_focus: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_bright: Color,
    pub accent: Color,
    pub accent2: Color,
    pub accent3: Color,
    pub success: Color,
    pub warn: Color,
    pub error: Color,
    pub info: Color,
    pub user_fg: Color,
    pub assistant_fg: Color,
    pub tool_fg: Color,
    pub diff_add: Color,
    pub diff_del: Color,
    pub diff_hunk: Color,
    pub status_bg: Color,
}

impl Theme {
    pub fn cyberclaw() -> Self {
        Self {
            name: "cyberclaw",
            bg: Color::Rgb(10, 14, 20),
            surface: Color::Rgb(18, 24, 34),
            border: Color::Rgb(40, 55, 75),
            border_focus: Color::Rgb(0, 240, 255),
            text: Color::Rgb(220, 230, 240),
            text_dim: Color::Rgb(90, 110, 130),
            text_bright: Color::Rgb(255, 255, 255),
            accent: Color::Rgb(0, 240, 255),
            accent2: Color::Rgb(255, 43, 214),
            accent3: Color::Rgb(179, 136, 255),
            success: Color::Rgb(80, 250, 160),
            warn: Color::Rgb(255, 200, 80),
            error: Color::Rgb(255, 85, 120),
            info: Color::Rgb(120, 180, 255),
            user_fg: Color::Rgb(0, 240, 255),
            assistant_fg: Color::Rgb(230, 235, 245),
            tool_fg: Color::Rgb(255, 200, 80),
            diff_add: Color::Rgb(80, 250, 160),
            diff_del: Color::Rgb(255, 85, 120),
            diff_hunk: Color::Rgb(179, 136, 255),
            status_bg: Color::Rgb(14, 20, 30),
        }
    }

    pub fn aurora() -> Self {
        Self {
            name: "aurora",
            bg: Color::Rgb(8, 12, 24),
            surface: Color::Rgb(14, 22, 38),
            border: Color::Rgb(30, 50, 70),
            border_focus: Color::Rgb(100, 255, 200),
            text: Color::Rgb(210, 230, 240),
            text_dim: Color::Rgb(80, 110, 130),
            text_bright: Color::Rgb(255, 255, 255),
            accent: Color::Rgb(100, 255, 200),
            accent2: Color::Rgb(140, 160, 255),
            accent3: Color::Rgb(200, 120, 255),
            success: Color::Rgb(100, 255, 180),
            warn: Color::Rgb(255, 210, 100),
            error: Color::Rgb(255, 100, 140),
            info: Color::Rgb(140, 200, 255),
            user_fg: Color::Rgb(100, 255, 200),
            assistant_fg: Color::Rgb(220, 235, 245),
            tool_fg: Color::Rgb(255, 210, 100),
            diff_add: Color::Rgb(100, 255, 180),
            diff_del: Color::Rgb(255, 100, 140),
            diff_hunk: Color::Rgb(200, 120, 255),
            status_bg: Color::Rgb(10, 16, 28),
        }
    }

    pub fn ember() -> Self {
        Self {
            name: "ember",
            bg: Color::Rgb(16, 10, 8),
            surface: Color::Rgb(28, 18, 12),
            border: Color::Rgb(70, 45, 30),
            border_focus: Color::Rgb(255, 160, 60),
            text: Color::Rgb(245, 230, 210),
            text_dim: Color::Rgb(130, 100, 70),
            text_bright: Color::Rgb(255, 250, 240),
            accent: Color::Rgb(255, 160, 60),
            accent2: Color::Rgb(255, 90, 50),
            accent3: Color::Rgb(255, 200, 120),
            success: Color::Rgb(160, 220, 100),
            warn: Color::Rgb(255, 180, 60),
            error: Color::Rgb(255, 80, 60),
            info: Color::Rgb(255, 200, 140),
            user_fg: Color::Rgb(255, 160, 60),
            assistant_fg: Color::Rgb(245, 230, 210),
            tool_fg: Color::Rgb(255, 200, 100),
            diff_add: Color::Rgb(160, 220, 100),
            diff_del: Color::Rgb(255, 80, 60),
            diff_hunk: Color::Rgb(255, 200, 120),
            status_bg: Color::Rgb(20, 12, 8),
        }
    }

    pub fn mono() -> Self {
        Self {
            name: "mono",
            bg: Color::Black,
            surface: Color::Rgb(20, 20, 20),
            border: Color::DarkGray,
            border_focus: Color::White,
            text: Color::White,
            text_dim: Color::DarkGray,
            text_bright: Color::White,
            accent: Color::White,
            accent2: Color::Gray,
            accent3: Color::DarkGray,
            success: Color::White,
            warn: Color::Gray,
            error: Color::White,
            info: Color::Gray,
            user_fg: Color::White,
            assistant_fg: Color::Gray,
            tool_fg: Color::White,
            diff_add: Color::White,
            diff_del: Color::DarkGray,
            diff_hunk: Color::Gray,
            status_bg: Color::Black,
        }
    }

    pub fn all() -> &'static [fn() -> Theme] {
        &[Theme::cyberclaw, Theme::aurora, Theme::ember, Theme::mono]
    }

    pub fn by_name(name: &str) -> Option<Theme> {
        let n = name.to_lowercase();
        match n.as_str() {
            "cyberclaw" | "cyber" | "claw" => Some(Self::cyberclaw()),
            "aurora" => Some(Self::aurora()),
            "ember" | "warm" | "amber" => Some(Self::ember()),
            "mono" | "monochrome" | "bw" => Some(Self::mono()),
            _ => None,
        }
    }

    pub fn names() -> Vec<&'static str> {
        vec!["cyberclaw", "aurora", "ember", "mono"]
    }

    pub fn from_index(i: usize) -> Theme {
        let all = Self::all();
        all[i % all.len()]()
    }

    pub fn index_of(name: &str) -> usize {
        Self::names().iter().position(|n| *n == name).unwrap_or(0)
    }

    pub fn error_style(&self) -> Style {
        Style::default().fg(self.error).add_modifier(Modifier::BOLD)
    }

    /// Accent color per agent mode: build=blue, plan=yellow, explore=orange,
    /// general=purple. `None` for agents outside the MODES cycle.
    pub fn mode_accent(agent: &str) -> Option<Color> {
        match agent {
            "build" => Some(Color::Rgb(70, 150, 255)),
            "plan" => Some(Color::Rgb(255, 210, 70)),
            "explore" => Some(Color::Rgb(255, 150, 60)),
            "general" => Some(Color::Rgb(185, 120, 255)),
            _ => None,
        }
    }

    /// Returns a copy with the accent and focus border tinted by the agent
    /// mode. Agents outside the cycle keep the base theme colors.
    pub fn with_mode(mut self, agent: &str) -> Self {
        if let Some(c) = Self::mode_accent(agent) {
            self.accent = c;
            self.border_focus = c;
        }
        self
    }
}

/// Resolve initial theme from env or default cyberclaw.
pub fn initial_theme() -> (Theme, usize) {
    if std::env::var_os("NO_COLOR").is_some() {
        return (Theme::mono(), Theme::index_of("mono"));
    }
    if let Ok(name) = std::env::var("RUSTCLAW_THEME") {
        if let Some(t) = Theme::by_name(&name) {
            let idx = Theme::index_of(t.name);
            return (t, idx);
        }
    }
    // Theme persisted in the global config.json (set via Ctrl+T).
    if let Ok(s) = crate::config::GlobalSettings::load_from(&crate::config::GlobalSettings::path())
    {
        if let Some(t) = Theme::by_name(&s.theme) {
            let idx = Theme::index_of(t.name);
            return (t, idx);
        }
    }
    (Theme::cyberclaw(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_accent_mapping() {
        assert_eq!(Theme::mode_accent("build"), Some(Color::Rgb(70, 150, 255)));
        assert_eq!(Theme::mode_accent("plan"), Some(Color::Rgb(255, 210, 70)));
        assert_eq!(
            Theme::mode_accent("explore"),
            Some(Color::Rgb(255, 150, 60))
        );
        assert_eq!(
            Theme::mode_accent("general"),
            Some(Color::Rgb(185, 120, 255))
        );
        assert_eq!(Theme::mode_accent("custom"), None);
    }

    #[test]
    fn test_with_mode_overrides_accent_and_focus() {
        let base = Theme::cyberclaw();
        let themed = base.clone().with_mode("plan");
        assert_eq!(themed.accent, Theme::mode_accent("plan").unwrap());
        assert_eq!(themed.border_focus, Theme::mode_accent("plan").unwrap());
        // Unrelated fields stay untouched.
        assert_eq!(themed.bg, base.bg);
        assert_eq!(themed.name, base.name);
    }

    #[test]
    fn test_with_mode_unknown_agent_keeps_base() {
        let base = Theme::cyberclaw();
        let themed = base.clone().with_mode("custom-agent");
        assert_eq!(themed.accent, base.accent);
        assert_eq!(themed.border_focus, base.border_focus);
    }
}
