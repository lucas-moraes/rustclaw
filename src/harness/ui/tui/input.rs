//! Input helpers and key binding documentation for the TUI.

/// Key binding help text (also mirrored in draw/help.rs).
#[allow(dead_code)]
pub const HELP: &str = "\
  Ctrl+C     quit (exit the project)
  Enter      send prompt
  Shift/Alt+Enter  line break
  Ctrl+J     line break (macOS fallback)  Esc        cancel prompt / clear input / close overlay
  Up/Down    history (single-line) / move between lines
  Ctrl+A/E   line start / line end
  Ctrl+U/W   kill to line start / kill word
  Del        delete char at cursor
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
