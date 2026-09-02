//! Project auto-discovery & persistent project memory.
//!
//! On startup the profiler inspects the project root (no LLM) to detect the
//! stack, build/test/run commands, and entry points. That structural summary
//! is cached in SQLite (`project_memory`) and injected into the system prompt
//! under `# Project context` each turn.

pub mod config_file;
pub mod memory;
pub mod profiler;
pub mod table;

pub use memory::ProjectMemoryStore;
pub use profiler::ProjectProfiler;
