//! TUI application state and main loop.
//!
//! Split across cohesive submodules; this file only wires them together and
//! re-exports the public surface used by `draw/` and `tui/mod.rs`.

pub mod events;
pub mod keys;
pub mod pickers;
mod pickers_state;
pub mod runner;
pub mod settings_keys;
pub mod skills;
pub mod state;
pub mod undo;
pub mod usage;

#[cfg(test)]
mod tests;

// Public surface consumed by `draw/`, `codeblock.rs`, `editor.rs`, etc.
#[allow(unused_imports)]
pub use crate::harness::ui::tui::subagent::SubagentPanel;
#[allow(unused_imports)]
pub use crate::harness::ui::tui::transcript::{
    preview, tool_arg_label, ActiveTool, LineKind, ToolBatch, TranscriptLine,
};

#[allow(unused_imports)]
pub use pickers_state::{
    AddProviderForm, AuthItem, AuthPickerState, AuthPromptState, ModelPickerState,
    ResumePickerState, SearchState, SkillPickerState, ThemePickerState,
};
pub use runner::run_tui;
#[allow(unused_imports)]
pub use state::{
    search_lines, should_flush_stream, App, CursorModelTarget, DoomLevel, Modal,
    STREAM_FLUSH_CHARS, STREAM_FLUSH_MS,
};
pub use undo::copy_to_clipboard;

/// Modes cycled by the prompt mode selector (like opencode's primary agents).
/// `general` stays available via `/agent general` and the palette.
pub const MODES: &[&str] = &["build", "reason", "chat-free"];
