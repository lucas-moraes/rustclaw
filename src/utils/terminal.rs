//! Terminal utilities module
//! Provides cross-platform terminal operations using crossterm

use std::io::{self, Read};

/// Enable raw mode for terminal input
pub fn enable_raw() -> io::Result<()> {
    crossterm::terminal::enable_raw_mode()
}

/// Disable raw mode and restore normal terminal behavior
pub fn disable_raw() -> io::Result<()> {
    crossterm::terminal::disable_raw_mode()
}

/// Read a single byte from stdin (blocking) - useful for key detection
pub fn read_byte() -> io::Result<u8> {
    let mut buf = [0u8; 1];
    io::stdin().read_exact(&mut buf)?;
    Ok(buf[0])
}

/// Check if a key is available (non-blocking)
pub fn poll_key() -> io::Result<Option<u8>> {
    use std::time::Duration;

    // Use select to check if stdin has data
    use std::os::unix::io::AsRawFd;

    let mut fd_set = std::collections::HashSet::new();
    fd_set.insert(std::io::stdin().as_raw_fd());

    let mut timeout = std::time::Duration::ZERO;

    // Use select-style polling via poll
    let result = unsafe {
        let mut poll_fd = libc::pollfd {
            fd: std::io::stdin().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        libc::poll(&mut poll_fd, 1, 0)
    };

    if result > 0 {
        Ok(Some(read_byte()?))
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
