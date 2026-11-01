//! Swarm coordination: message bus and shared memory between agents.
//!
//! Both live in the same SQLite database as sessions, partitioned per project
//! (`project_<h>_messages` / `project_<h>_shared_memory`), so a subagent can
//! talk to its parent (or a sibling) and share findings without routing
//! everything through the parent's context window.
//!
//! The bus is deliberately *pull-based*: `message_send` never blocks, and
//! `message_inbox` drains what is waiting. That keeps a subagent from stalling
//! on a peer that may never answer.

pub mod bus;
pub mod memory;

pub use bus::MessageBus;
pub use memory::SharedMemory;
