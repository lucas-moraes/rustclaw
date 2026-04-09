//! Terminal utilities module
//! Provides cross-platform terminal operations using crossterm

use std::io::{self, Write};

/// Enable raw mode for terminal input
pub fn enable_raw() -> io::Result<()> {
    crossterm::terminal::enable_raw_mode()
}

/// Disable raw mode and restore normal terminal behavior  
pub fn disable_raw() -> io::Result<()> {
    crossterm::terminal::disable_raw_mode()
}

/// Check if raw mode is enabled
pub fn is_raw_mode() -> bool {
    crossterm::terminal::is_raw_mode_enabled().unwrap_or(false)
}

/// Read a key event (blocking) using crossterm
pub fn read_key_event() -> io::Result<Option<crossterm::event::Event>> {
    use crossterm::event;

    if event::poll(std::time::Duration::from_millis(100))? {
        event::read()
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    } else {
        Ok(None)
    }
}

/// RAII guard for raw mode
pub struct RawMode;

impl RawMode {
    pub fn enable() -> io::Result<RawModeGuard> {
        enable_raw()?;
        Ok(RawModeGuard)
    }
}

pub struct RawModeGuard;

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw();
    }
}

/// Clear the terminal screen
pub fn clear_screen() {
    print!("\x1b[2J\x1b[H");
    let _ = std::io::stdout().flush();
}

/// Move cursor to position (1-indexed)
pub fn move_cursor(row: u16, col: u16) {
    print!("\x1b[{};{}H", row, col);
    let _ = std::io::stdout().flush();
}

/// Hide the cursor
pub fn hide_cursor() {
    print!("\x1b[?25l");
    let _ = std::io::stdout().flush();
}

/// Show the cursor
pub fn show_cursor() {
    print!("\x1b[?25h");
    let _ = std::io::stdout().flush();
}
