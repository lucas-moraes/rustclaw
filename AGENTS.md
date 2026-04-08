# AGENTS.md - RustClaw Development Guide

## Build Commands

```bash
# Development build
cargo build

# Run in development mode
cargo run

# Release build (optimized for size)
cargo build --release

# Run in CLI mode
cargo run -- --mode cli
```

## Linting & Testing

```bash
# Run all tests
cargo test

# Run a single test by name
cargo test test_name

# Run tests in a specific file
cargo test --test file_name

# Run clippy for linting
cargo clippy

# Format code
cargo fmt

# Check formatting
cargo fmt --check

# Full check (tests + clippy + fmt)
cargo check
```

## Code Style Guidelines

### Imports
- Use absolute imports within crate: `use crate::module::Item`
- Group std, external crates, and local modules with blank lines between
- Order: std → external → crate
- Example:
  ```rust
  use std::path::{Path, PathBuf};
  use anyhow::Result;
  use serde::{Deserialize, Serialize};
  use crate::config::Config;
  ```

### Formatting
- Use `cargo fmt` for automatic formatting
- Maximum line length: 100 characters
- Use 4 spaces for indentation (Rust standard)
- Keep related items together

### Types & Naming

**Structs & Enums:**
- Use PascalCase: `struct AgentConfig`, `enum ExitBehavior`
- Add doc comments with `///`
- Derive `Clone`, `Debug`, `Serialize`, `Deserialize` where appropriate

**Functions & Variables:**
- Use snake_case: `fn get_config()`, `let memory_store`
- Be descriptive: `memory_store` not `ms`
- Avoid abbreviations unless well-known (e.g., `config`, `api`)

**Constants:**
- Use SCREAMING_SNAKE_CASE: `const USER_AGENT: &str = "RustClaw/1.0";`
- Place at module level, not inside impl blocks

**Visibility:**
- Use `pub` for public API
- Keep private by default, expose only what's needed
- Use `pub(crate)` for intra-crate public items

### Error Handling

- Use `anyhow::Result<T>` for application code with context
- Use `std::io::Result<T>` for file/network operations
- Add context with `map_err(|e| anyhow!("failed to X: {}", e))`
- Avoid bare `unwrap()` in production code
- Use `expect()` only for unrecoverable init errors
- Propagate errors with `?` operator

### Async Code

- Use `tokio` for async runtime (already in dependencies)
- Prefer `async fn` for functions that await
- Use `Arc<T>` for shared state across async tasks
- Avoid blocking calls in async context

### Database (SQLite)

- Use `rusqlite` with connection pooling pattern
- Close connections explicitly or use `Connection` per operation
- Use transactions for multi-step operations
- Table naming: snake_case (e.g., `memory_entries`)

### Testing

- Create tests in same file with `#[cfg(test)]` module
- Use descriptive test names: `fn test_loads_config_from_env()`
- Use `tempfile` for temporary database/files in tests
- Assert on specific error types when testing failures

### Security Considerations

- Sanitize shell command inputs (see `src/security/`)
- Implement workspace trust levels
- Validate HTTP inputs
- Clean tool outputs before showing to user

## Project Structure

```
src/
├── agent.rs         # Main agent with ReAct architecture
├── config.rs        # Configuration from environment
├── cli.rs           # CLI mode
├── main.rs          # Entry point
├── memory/          # SQLite memory + embeddings
├── skills/          # Skills system (load, execute, MCP)
├── tools/           # Tool registry and implementations
├── security/        # Security (sanitizer, validator, trust)
├── telegram/        # Telegram bot integration
└── utils/           # Helpers (colors, output, error parsing)
```

## Configuration

- Config via `config/.env` (copy from `.env.example`)
- Required: `TOKEN` (API key)
- Optional: `TAVILY_API_KEY`, `MAX_TOKENS`, `MAX_ITERATIONS`, `TZ`

## Key Patterns

**Tool Registration:**
- Add tools in `src/tools/mod.rs`
- Implement `Tool` trait with `name()`, `execute()`, `description()`

**Skill System:**
- Skills defined in `skills/` directory
- Use YAML frontmatter in `SKILL.md`
- Supports resources: scripts/, references/, assets/

**Memory:**
- SQLite-based with session linking
- Semantic embeddings for search
- Checkpoint system for development phases