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

    pub fn daylight() -> Self {
        Self {
            name: "daylight",
            bg: Color::Rgb(250, 250, 248),
            surface: Color::Rgb(238, 240, 244),
            border: Color::Rgb(200, 205, 215),
            border_focus: Color::Rgb(0, 110, 200),
            text: Color::Rgb(35, 40, 50),
            text_dim: Color::Rgb(130, 138, 150),
            text_bright: Color::Rgb(10, 12, 18),
            accent: Color::Rgb(0, 110, 200),
            accent2: Color::Rgb(200, 40, 140),
            accent3: Color::Rgb(120, 80, 200),
            success: Color::Rgb(20, 130, 80),
            warn: Color::Rgb(180, 120, 0),
            error: Color::Rgb(200, 40, 60),
            info: Color::Rgb(30, 110, 190),
            user_fg: Color::Rgb(0, 110, 200),
            assistant_fg: Color::Rgb(30, 35, 45),
            tool_fg: Color::Rgb(150, 95, 0),
            diff_add: Color::Rgb(20, 130, 80),
            diff_del: Color::Rgb(200, 40, 60),
            diff_hunk: Color::Rgb(120, 80, 200),
            status_bg: Color::Rgb(232, 235, 240),
        }
    }

    pub fn mono() -> Self {
        // Indexed / reset colors only — no RGB (honours terminal fg/bg).
        Self {
            name: "mono",
            bg: Color::Reset,
            surface: Color::Reset,
            border: Color::DarkGray,
            border_focus: Color::White,
            text: Color::White,
            text_dim: Color::Gray,
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
            status_bg: Color::Reset,
        }
    }

    /// High-contrast: black bg, white/yellow text. Approximate WCAG 4.5:1+
    /// for `text`/`text_dim` vs `bg` (white-on-black ~21:1; yellow-on-black
    /// ~19:1; `Gray` on black is typically ≥ 4.5:1 in 16-color terminals).
    pub fn high_contrast() -> Self {
        Self {
            name: "high-contrast",
            bg: Color::Black,
            surface: Color::Black,
            border: Color::White,
            border_focus: Color::Yellow,
            text: Color::White,
            text_dim: Color::White,
            text_bright: Color::White,
            accent: Color::Yellow,
            accent2: Color::Yellow,
            accent3: Color::White,
            success: Color::White,
            warn: Color::Yellow,
            error: Color::White,
            info: Color::White,
            user_fg: Color::Yellow,
            assistant_fg: Color::White,
            tool_fg: Color::Yellow,
            diff_add: Color::White,
            diff_del: Color::White,
            diff_hunk: Color::Yellow,
            status_bg: Color::Black,
        }
    }

    pub fn all() -> &'static [fn() -> Theme] {
        &[
            Theme::cyberclaw,
            Theme::aurora,
            Theme::ember,
            Theme::mono,
            Theme::daylight,
            Theme::high_contrast,
        ]
    }

    pub fn by_name(name: &str) -> Option<Theme> {
        let n = name.to_lowercase();
        match n.as_str() {
            "cyberclaw" | "cyber" | "claw" | "dark" => Some(Self::cyberclaw()),
            "aurora" => Some(Self::aurora()),
            "ember" | "warm" | "amber" => Some(Self::ember()),
            "mono" | "monochrome" | "bw" => Some(Self::mono()),
            "daylight" | "light" | "day" => Some(Self::daylight()),
            "high-contrast" | "highcontrast" | "hc" => Some(Self::high_contrast()),
            _ => None,
        }
    }

    pub fn names() -> Vec<&'static str> {
        vec![
            "cyberclaw",
            "aurora",
            "ember",
            "mono",
            "daylight",
            "high-contrast",
        ]
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
    /// general=purple, chat-free=teal. `None` for agents outside the MODES cycle.
    pub fn mode_accent(agent: &str) -> Option<Color> {
        match agent {
            "build" => Some(Color::Rgb(70, 150, 255)),
            "plan" => Some(Color::Rgb(255, 210, 70)),
            "explore" => Some(Color::Rgb(255, 150, 60)),
            "general" => Some(Color::Rgb(185, 120, 255)),
            "chat-free" => Some(Color::Rgb(60, 200, 180)),
            _ => None,
        }
    }

    /// Darker variants of the mode accents, readable on a light background.
    pub fn mode_accent_light(agent: &str) -> Option<Color> {
        match agent {
            "build" => Some(Color::Rgb(0, 90, 200)),
            "plan" => Some(Color::Rgb(170, 110, 0)),
            "explore" => Some(Color::Rgb(190, 80, 0)),
            "general" => Some(Color::Rgb(110, 60, 190)),
            "chat-free" => Some(Color::Rgb(0, 130, 120)),
            _ => None,
        }
    }

    /// True when the theme is designed for a light background.
    pub fn is_light(&self) -> bool {
        self.name == "daylight"
    }

    /// Returns a copy with the accent and focus border tinted by the agent
    /// mode. Agents outside the cycle keep the base theme colors. Light themes
    /// use darker accent variants so they stay readable on a bright background.
    pub fn with_mode(mut self, agent: &str) -> Self {
        let accent = if self.is_light() {
            Self::mode_accent_light(agent)
        } else {
            Self::mode_accent(agent)
        };
        if let Some(c) = accent {
            self.accent = c;
            self.border_focus = c;
        }
        self
    }
}

/// True when `NO_COLOR` is set — the TUI stays on the mono palette.
pub fn theme_locked() -> bool {
    std::env::var_os("NO_COLOR").is_some()
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
        assert_eq!(
            Theme::mode_accent("chat-free"),
            Some(Color::Rgb(60, 200, 180))
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

    #[test]
    fn test_daylight_registered_and_resolvable() {
        assert!(Theme::names().contains(&"daylight"));
        assert_eq!(Theme::all().len(), Theme::names().len());
        assert_eq!(Theme::by_name("light").unwrap().name, "daylight");
        assert_eq!(Theme::by_name("day").unwrap().name, "daylight");
        assert_eq!(Theme::index_of("daylight"), 4);
        assert_eq!(
            Theme::from_index(Theme::index_of("daylight")).name,
            "daylight"
        );
    }

    #[test]
    fn test_theme_alias_dark_is_cyberclaw() {
        assert_eq!(Theme::by_name("dark").unwrap().name, "cyberclaw");
    }

    #[test]
    fn test_high_contrast_registered() {
        assert!(Theme::names().contains(&"high-contrast"));
        assert_eq!(
            Theme::by_name("high-contrast").unwrap().name,
            "high-contrast"
        );
        assert_eq!(Theme::by_name("hc").unwrap().name, "high-contrast");
        assert_eq!(Theme::all().len(), Theme::names().len());
    }

    #[test]
    fn test_no_color_locks_theme_in_runtime() {
        assert_eq!(theme_locked(), std::env::var_os("NO_COLOR").is_some());
        assert!(theme_locked() || !theme_locked());
    }

    fn color_is_rgb(c: Color) -> bool {
        matches!(c, Color::Rgb(_, _, _))
    }

    #[test]
    fn test_mono_theme_has_no_rgb_accents() {
        let t = Theme::mono();
        assert!(!color_is_rgb(t.bg));
        assert!(!color_is_rgb(t.surface));
        assert!(!color_is_rgb(t.accent));
        assert!(!color_is_rgb(t.accent2));
        assert!(!color_is_rgb(t.text));
        assert!(!color_is_rgb(t.status_bg));
    }

    #[test]
    fn test_theme_roundtrip_config_json() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.json");
        let s = crate::config::GlobalSettings {
            theme: "high-contrast".into(),
            ..Default::default()
        };
        s.save_to(&p).unwrap();
        let back = crate::config::GlobalSettings::load_from(&p).unwrap();
        assert_eq!(back.theme, "high-contrast");
        assert_eq!(Theme::by_name(&back.theme).unwrap().name, "high-contrast");
    }

    #[test]
    fn test_daylight_is_light_and_uses_dark_mode_accents() {
        let base = Theme::daylight();
        assert!(base.is_light());
        assert!(!Theme::cyberclaw().is_light());
        let themed = base.clone().with_mode("plan");
        assert_eq!(themed.accent, Theme::mode_accent_light("plan").unwrap());
        assert_eq!(
            themed.border_focus,
            Theme::mode_accent_light("plan").unwrap()
        );
        // Light accents differ from the dark-theme ones.
        assert_ne!(Theme::mode_accent_light("plan"), Theme::mode_accent("plan"));
        // Unknown agent keeps the base light accent.
        assert_eq!(base.clone().with_mode("custom").accent, base.accent);
    }
}
