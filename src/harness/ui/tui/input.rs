//! Input helpers and key binding documentation for the TUI.

/// Key binding help text (also mirrored in draw/help.rs).
#[allow(dead_code)]
pub const HELP: &str = "\
  Ctrl+C     quit (exit the project)
  Enter      send prompt
  Esc        cancel prompt / clear input / close overlay
  Up/Down    history (or autocomplete)
  PgUp/PgDn  scroll transcript
  Ctrl+P     command palette
  Ctrl+T     cycle theme
  Ctrl+L     clear transcript
  ? / F1     help overlay
  Tab        autocomplete (in /) / cycle mode
  y/n/a      permission modal
  1..n       question modal
  /help      slash commands
";
