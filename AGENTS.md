# AGENTS.md - RustClaw Development Guide

RustClaw é um **coding agent harness** (estilo OpenCode / Claude Code) em Rust.
O core é o módulo `src/harness/`; o restante são módulos de suporte.

## Build Commands

```bash
# Development build
cargo build

# Run (CLI harness)
cargo run

# Run a live smoke test (uses the token in ~/.local/share/rustclaw/auth.json)
cargo test --bin rustclaw smoke_native_tool_calling -- --ignored --nocapture
```

## Linting & Testing

```bash
cargo test            # all tests
cargo test harness    # only harness tests
cargo test test_name  # single test
cargo clippy          # lint
cargo fmt             # format
cargo fmt --check     # formatting check
cargo check           # full check
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

### Types & Naming
- Structs/Enums: PascalCase with `///` doc comments; derive `Clone`, `Debug`,
  `Serialize`, `Deserialize` where appropriate
- Functions/variables: snake_case, descriptive
- Constants: SCREAMING_SNAKE_CASE at module level
- `pub` for public API, `pub(crate)` for intra-crate, keep private by default

### Error Handling
- Use `anyhow::Result<T>` for application code
- Add context with `.context("failed to X")` or `map_err(...)`
- Avoid bare `unwrap()` in production code
- Propagate with `?`

### Async Code
- Use `tokio` runtime
- Prefer `async fn`
- Use `Arc<T>` / `Arc<RwLock<T>>` for shared state
- Avoid blocking calls in async context (use `spawn_blocking` when needed)

### SQLite
- `rusqlite`; wrap `Connection` in `Mutex` when shared across tasks
- `SessionStore` (sessions) é o único store; skills vivem em `skills_json` na session

### Testing
- Tests in same file under `#[cfg(test)]`
- Descriptive names: `test_loads_config_from_env()`
- Use `tempfile` for temp DB/files

## Project Structure

```
src/
├── main.rs          # Entry point (runs harness CLI)
├── config.rs        # Configuração por env (TOKEN/PROVIDER/MODEL/BASE_URL/limits)
├── error.rs         # Tipos de erro (AgentError, ConfigError)
└── harness/         # O harness em si
    ├── mod.rs
    ├── event.rs     # Event bus (HarnessEvent)
    ├── runtime.rs   # SessionRuntime (facade) + build_default_registry + TaskRunner
    ├── skill/       # Skills = memória da sessão (prompt/session/memory)
    │   ├── mod.rs   # SkillSpec, SessionSkill, PromptSkillToggle
    │   ├── loader.rs# Discovery de SKILL.md (projecto+home+env) + parse
    │   └── inject.rs# Render de skills habilitadas no system prompt
    ├── session/
    │   ├── mod.rs       # Session, Message, Part, TodoItem
    │   ├── store.rs     # SessionStore (SQLite)
    │   ├── processor.rs # Loop central (native tool calling + paralelo)
    │   └── compaction.rs
    ├── provider/
    │   ├── mod.rs       # Trait Provider + ProviderEvent + SSE parser
    │   ├── openai.rs    # /chat/completions + tool_calls
    │   ├── anthropic.rs # /messages + tool_use
    │   └── opencode_go.rs # roteia minimax→/messages, senão→/chat/completions
    ├── tool/
    │   ├── mod.rs       # Trait Tool (JSON Schema) + ToolResult + ToolSpec
    │   ├── registry.rs  # ToolRegistry (builder)
    │   ├── context.rs   # ToolContext (cwd, abort, permission, askers, todos)
    │   ├── bash.rs read.rs write.rs edit.rs glob.rs grep.rs
    │   ├── todo.rs question.rs task.rs
    │   └── truncate.rs
    ├── permission/mod.rs # allow/ask/deny engine
    ├── agent/
    │   ├── mod.rs        # AgentSpec + build_system_prompt
    │   └── builtin.rs    # build/plan/explore/general
    └── ui/
        ├── mod.rs
        ├── commands.rs  # slash commands compartilhados (TUI + CLI)
        ├── cli.rs       # streaming CLI (fallback / RUSTCLAW_UI=cli)
        └── tui/         # TUI ratatui + crossterm
            ├── mod.rs   # entry + TTY selection + askers wiring
            ├── app.rs   # App state, apply_event, loop principal, modals
            ├── draw.rs  # widgets (header/transcript/status/input/help/modal)
            ├── input.rs # key bindings
            └── askers.rs# TuiAsker/TuiUserAsker (channels oneshot)
```

## Configuration (file-based, no `.env`)

- `~/.local/share/rustclaw/auth.json` — API token per provider (0600, via `/auth`)
- `~/.local/share/rustclaw/config.json` — provider/model, `max_iterations`,
  `max_context_tokens`, theme (via `/settings`, `/models`)
- `rustclaw.json` in the project root — per-project provider/model override
- Precedence: catalog → config.json → rustclaw.json → auth token
- UX env vars still honored: `RUSTCLAWUI`/`RUSTCLAW_UI`, `RUSTCLAW_THEME`,
  `RUSTCLAW_SKILLS_DIR`, `NO_COLOR`
- Install: `scripts/install.sh` (curl releases) / `scripts/link-local.sh` (dev)

## Key Patterns

### Tool trait (native tool calling)
Adicionar tools em `src/harness/tool/` e registrá-las em
`runtime::build_default_registry`. Implementar:

```rust
#[async_trait::async_trait]
impl Tool for MyTool {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value; // JSON Schema
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String>;
}
```

### Provider
- `provider::Provider` tem `stream(LlmRequest)` e `complete(LlmRequest)`
- Adaptadores convertem `Message`/`Part` e emitem `ProviderEvent` unificado
- `ProviderEvent::ToolCallEnd` carrega os argumentos completos da tool call

### Loop do processor
- `session/processor.rs::run_turn`: stream → tool_calls → execução paralela
  (JoinSet) com `ctx.check_permission` → results → repete até resposta final
- Doom-loop detection: mesma tool call 3x (warn) / 5x (stop)
- Compaction automática em overflow de contexto

### Agents
- Builtins em `agent/builtin.rs` (`build`/`plan`/`explore`/`general`)
- `tool::task` dispara subagent via `runtime::TaskRunner` (child session isolada)

### Memory (skills)
- Modelo: **prompt** (pedido atual) + **session** (histórico) + **memory** (skills)
- Skills escolhidas na criação da session (SkillPicker) ou via `/skills`
- Skills marcadas no turno (checkbox) entram no system prompt (`# Session skills`)
- Persistidas em `harness_sessions.skills_json` (DB: `dirs::data_local_dir()/rustclaw/harness.db`)

### Permissions
- `PermissionEngine` decide Allow/Ask/Deny por tool + path (fora do CWD → Ask)
- Tools mutáveis pedem confirmação no CLI (y/n/always)
