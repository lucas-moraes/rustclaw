//! Input helpers and key binding documentation for the TUI.

/// Key binding help text (also mirrored in draw/help.rs).
#[allow(dead_code)]
pub const HELP: &str = "\
  Ctrl+C     quit (exit the project)
  Enter      send prompt
  Shift/Alt+Enter  line break
  Ctrl+J     line break (macOS fallback)
  Esc        cancel streaming/run · close overlay · clear draft
  Up/Down    history (single-line) / move between lines
  Ctrl+A/E   line start / line end
  Ctrl+U/W   kill to line start / kill word
  Ctrl+Z     reset prompt input
  Del        delete char at cursor
  PgUp/PgDn  scroll transcript
  Drag       select transcript text (auto-copy on release)
  Ctrl+C     copy selection · quit (no selection)
  Esc        (same: cancel in-flight action first)
  Ctrl+P     command palette
  Ctrl+T     cycle theme
  Ctrl+L     clear transcript
  ? / F1     help overlay
  Tab        autocomplete (in /) / cycle mode
  y/n/a      permission modal
  1..n / type  question modal (pick option or free-text + Enter)
  /help      slash commands
";
