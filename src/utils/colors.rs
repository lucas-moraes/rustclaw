use std::fmt;

/// RustClaw brand palette: warm gradient from gold to dark red
///
/// #ffe400 → #ffae00 → #ee5c00 → #c11100 → #840000
pub struct Colors;

impl Colors {
    /// #ffe400 — Gold/Yellow (top of gradient, logo line 6)
    pub const GOLD: &'static str = "\x1b[38;5;220m";

    /// #ffae00 — Amber (primary brand color, prompts, spinner)
    pub const AMBER: &'static str = "\x1b[38;5;214m";

    /// #ee5c00 — Orange (tool calls, actions)
    pub const ORANGE: &'static str = "\x1b[38;5;202m";

    /// #c11100 — Red (errors, critical)
    pub const RED: &'static str = "\x1b[38;5;160m";

    /// #840000 — Dark Red (decorative, borders, bottom of gradient)
    pub const DARK_RED: &'static str = "\x1b[38;5;88m";

    /// Dim gray for secondary text (labels, hints)
    pub const DIM: &'static str = "\x1b[90m";

    /// Light gray for subtle text (250)
    pub const LIGHT_GRAY: &'static str = "\x1b[38;5;250m";

    /// Bold attribute
    pub const BOLD: &'static str = "\x1b[1m";

    /// Reset all attributes
    pub const RESET: &'static str = "\x1b[0m";

    /// Erase to end of line (for spinner cleanup)
    pub const CLEAR_LINE: &'static str = "\r\x1b[K";

    /// Brand gradient as array [dark_red, red, orange, orange, amber, gold]
    pub fn logo_gradient() -> [&'static str; 6] {
        [
            Self::DARK_RED,
            Self::RED,
            Self::ORANGE,
            Self::ORANGE,
            Self::AMBER,
            Self::GOLD,
        ]
    }
}

impl fmt::Display for Colors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Colors")
    }
}
