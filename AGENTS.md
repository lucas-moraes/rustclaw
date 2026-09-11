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
    ├── runtime/     # SessionRuntime (facade)
    │   ├── mod.rs       # SessionRuntime + PromptResult + re-exports
    │   ├── registry.rs  # build_default_registry
    │   ├── task_runner.rs # TaskRunner (subagentes via tool::task)
    │   └── context.rs   # frozen_summary_for + memory_block_for
    ├── auth.rs      # Auth token store (~/Library/.../rustclaw/auth.json)
    ├── budget.rs    # BudgetTracker (token usage + cost estimation)
    ├── hooks.rs     # Pre/post tool hooks (run_pre_tool, spawn_post_tool)
    ├── skill/       # Skills = memória da sessão (prompt/session/memory)
    │   ├── mod.rs   # SkillSpec, SessionSkill, PromptSkillToggle
    │   ├── loader.rs# Discovery de SKILL.md (projeto+home+env) + parse
    │   └── inject.rs# Render de skills habilitadas no system prompt
    ├── session/
    │   ├── mod.rs       # Session, Message, Part, ToolPart, ToolStatus, preview()
    │   ├── store.rs     # SessionStore (SQLite)
    │   ├── processor/   # Loop central (native tool calling + paralelo)
    │   │   ├── mod.rs       # SessionProcessor + run_turn
    │   │   └── stream_loop.rs # consume_stream + StreamOutcome
    │   ├── tool_exec.rs # Execução de tools (JoinSet + permissões + catch_unwind)
    │   ├── doom_loop.rs # DoomLoopDetector (detecção de ciclos de tool calls)
    │   ├── compaction.rs# Compactação de contexto em overflow
    │   └── image.rs     # Suporte a imagens nas mensagens
    ├── provider/
    │   ├── mod.rs       # Trait Provider + ProviderEvent + SSE parser
    │   ├── catalog.rs   # Catálogo builtin + merge com user providers
    │   ├── user_store.rs# providers.json (provedores personalizados)
    │   ├── openai.rs    # /chat/completions + tool_calls
    │   ├── anthropic.rs # /messages + tool_use
    │   ├── opencode_go.rs # roteia minimax→/messages, senão→/chat/completions
    │   └── retry.rs     # RetryPolicy para falhas de provider
    ├── tool/
    │   ├── mod.rs       # Trait Tool (JSON Schema) + ToolResult + ToolSpec
    │   ├── registry.rs  # ToolRegistry (builder)
    │   ├── context.rs   # ToolContext (cwd, abort, permission, askers, todos)
    │   ├── bash.rs read.rs write.rs edit.rs glob.rs grep.rs
    │   ├── ast_search.rs web_search.rs fetch_webpage.rs
    │   ├── todo.rs question.rs task.rs remember.rs
    │   ├── diff.rs git.rs diagnostics.rs checkpoint.rs
    │   ├── env.rs jobs.rs truncate.rs
    │   └── mod.rs
    ├── permission/mod.rs # allow/ask/deny engine
    ├── project/       # Memória persistente do projeto
    │   ├── mod.rs
    │   ├── memory.rs  # ProjectMemoryStore (SQLite, facts CRUD, dedup, GC)
    │   ├── scoring.rs # Scoring/ranking de facts (recência+uso+BM25) + render
    │   ├── profiler.rs# ProjectContext, StackKind, análise de código
    │   ├── table.rs   # table_name() helper para nomes de tabela SQLite
    │   └── config_file.rs # rustclaw.json per-project config
    ├── mcp/          # MCP (Model Context Protocol) client
    │   ├── mod.rs    # McpManager (connect_all, status, restart, health)
    │   ├── config.rs # McpConfig (mcpServers) + load/merge global+projeto
    │   ├── client.rs # McpClient (stdio/HTTP, handshake, list/call, reconnect)
    │   └── tool.rs   # McpTool impl Tool (mcp_<server>_<tool>)
    ├── agent/
    │   ├── mod.rs        # AgentSpec + build_system_prompt
    │   ├── builtin.rs    # build/plan/explore/general
    │   └── custom.rs     # Agentes customizados (user-defined)
    └── ui/
        ├── mod.rs
        ├── cli.rs       # streaming CLI (fallback / RUSTCLAW_UI=cli)
        ├── commands/    # slash commands (/help, /settings, /undo, /fork, ...)
        │   ├── mod.rs   # handle() dispatcher + tests
        │   ├── memory.rs# /memory list|rm|clear|promote|gc
        │   ├── export.rs# /export (Markdown/JSON)
        │   ├── apply_plan.rs # /apply-plan
        │   └── replay.rs# /record, /replay
        └── tui/         # TUI ratatui + crossterm
            ├── mod.rs   # entry + TTY selection + askers wiring
            ├── app/     # App state + loop (split modules)
            │   ├── mod.rs    # re-exports + MODES
            │   ├── state.rs  # App + Modal + picker states
            │   ├── events.rs # apply_event + transcript rebuild
            │   ├── skills.rs # toggles/pickers/comando /skills
            │   ├── usage.rs  # contabilidade de custo
            │   ├── undo.rs   # undo/revert helpers
            │   ├── keys.rs   # handle_key + submit_input
            │   ├── pickers.rs# key handlers dos modais
            │   ├── runner.rs # run_tui + TerminalGuard
            │   └── tests.rs  # input/code_block tests
            ├── editor.rs# prompt input editor (cursor/editing/history)
            ├── subagent.rs # live subagent panels (task tool)
            ├── codeblock.rs # last-code-block copy/save helpers
            ├── transcript.rs # LineKind/TranscriptLine/ToolBatch types
            ├── draw/    # widgets
            │   ├── mod.rs
            │   ├── header.rs transcript.rs status.rs input.rs help.rs
            │   ├── modal.rs sidebar.rs splash.rs
            │   ├── model_picker.rs resume_picker.rs skill_picker.rs
            │   ├── palette_view.rs search.rs
            │   └── mod.rs
            ├── input.rs # key bindings
            ├── askers.rs# TuiAsker/TuiUserAsker (channels oneshot)
            ├── theme.rs # Tema e cores
            ├── palette.rs # Paleta de comandos (Ctrl+P)
            ├── selection.rs # Seleção de texto no transcript
            ├── anim.rs  # Animações (spinner, fade)
            └── markdown.rs # Renderização de Markdown
```

## Configuration (file-based, no `.env`)

- `~/.local/share/rustclaw/auth.json` — API token per provider (0600, via `/auth`)
  (macOS: `~/Library/Application Support/rustclaw/auth.json`)
- `~/.local/share/rustclaw/config.json` — provider/model, `max_iterations`,
  `max_context_tokens`, theme (via `/settings`, `/models`)
- `~/.local/share/rustclaw/providers.json` — user-defined providers/models
  (via `/provider add|rm|list`, `/models add`, or the `/models` picker
  "add provider…"). Merged with the builtin catalog at runtime; a user
  provider with the same name as a builtin overrides it.
- `~/.local/share/rustclaw/mcp.json` — MCP servers (`mcpServers` format);
  merged with the `mcp` section of `rustclaw.json` (project wins by name)
- `rustclaw.json` in the project root — per-project provider/model override
  + optional `mcp` section
- Precedence: catalog (builtin + user) → config.json → rustclaw.json → auth token
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
    // opcional: fn read_only(&self) -> bool { true }  // MCP readOnlyHint
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String>;
}
```

### ast_search (Tree-Sitter)
- `tool/ast_search.rs` usa `tree-sitter` 0.25 + `tree-sitter-rust` 0.24 para
  busca sintática em `.rs` (structs/enums/traits/functions/impls) sem regex.
- API da crate: `tree_sitter_rust::LANGUAGE` (constante `Language`); cria-se
  `tree_sitter::Language::new(LANGUAGE)` e `Parser::set_language(&language)`
  recebe referência.
- Nó `impl_item` NÃO tem campo `name` — o nome é montado dos fields `trait`
  e `type` ("Trait for Type"). `child_by_field_name("name")` vale para
  function/struct/enum/trait items.
- `read_only() = true`; registrado em `build_default_registry` e na lista
  `Allow` do `PermissionEngine`.
- Nós >3000 chars são truncados para o cabeçalho (protege a janela de contexto).

### Provider
- `provider::Provider` tem `stream(LlmRequest)` e `complete(LlmRequest)`
- Adaptadores convertem `Message`/`Part` e emitem `ProviderEvent` unificado
- `ProviderEvent::ToolCallEnd` carrega os argumentos completos da tool call
- Prompt caching: `AnthropicProvider.prompt_cache` injeta até 3 breakpoints
  `cache_control` (system/tools/última message); flag efetiva =
  `config.json prompt_caching` && (providers.json `prompt_cache` ?? default
  por provider — `true` só para `anthropic`); custo via
  `catalog::estimate_cost_cached` (Anthropic write 1.25×/read 0.1×,
  OpenAI cached 0.5×)

### Loop do processor
- `session/processor/mod.rs::run_turn`: stream → tool_calls → execução paralela
  (JoinSet) com `ctx.check_permission` → results → repete até resposta final
- Tool execution extraída para `session/tool_exec.rs` (JoinSet + permissões +
  catch_unwind)
- Doom-loop detection em `session/doom_loop.rs`: `DoomLoopDetector` com
  `record()` e `should_stop()`; mesma tool call 3x (warn) / 5x (stop);
  detecta ciclos multi-call (A,B,A,B...)
- Compaction automática em overflow de contexto (`session/compaction.rs`)

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

### MCP (Model Context Protocol)
- `McpManager` (`mcp/mod.rs`): `connect_all` paralelo no boot (falha de um
  server não bloqueia os outros; timeout de conexão 10s), `restart`,
  health check a cada 60s, reconnect 1x em falha de transporte
- Config: formato padrão `mcpServers` (Claude/Cursor); `command` (stdio) XOR
  `url` (streamable HTTP + `headers.Authorization`); `${VAR}` expandido em
  `args`/`env`; `timeout_secs` por chamada (default 60)
- `McpTool` (`mcp/tool.rs`) impl `Tool` com nome `mcp_<server>_<tool>`
  (sanitizado `[a-z0-9_]`); descrição truncada 200 chars; cap 50 tools/server
- Registro: `SessionRuntime::init_mcp()` (chamado no boot do CLI/TUI) injeta
  as tools no `ToolRegistry` via `registry.with_tool`
- Permissões: MCP tools caem no default `Ask`; `/permissions set mcp_... allow`
  persiste. Todos os modos admitem tools MCP com `readOnlyHint` via marcador
  `mcp_readonly` na allowlist (`Tool::read_only()` no trait)
- Comandos: `/mcp list|status|restart` (`ui/commands/mod.rs`)

### Project Memory (remember tool)
- `project/memory.rs`: `ProjectMemoryStore` — SQLite per-project facts table
  (`<cwd_hash>_facts`) com CRUD: `append_fact`, `list_fact_rows`, `active_facts`,
  `delete_fact_by_index`, `clear_memory`, `bump_usage`, `set_archived`, `dedup`,
  `archive_stale`, `compact`, `auto_promote` (→ skills), `search_facts` (FTS5)
- `project/scoring.rs`: `MemoryFact` struct, `score_fact`/`score_fact_bm25`
  (recência + hit_count + lexical + BM25), `render_memory`/`render_memory_ranked`
  (top-N por score, orçamento `MAX_MEMORY_CHARS=2048`), `is_memory_block`
- `project/profiler.rs`: `ProjectContext`, `StackKind` (Rust/Node/Python/Go)
- `project/table.rs`: `table_name()` — nomes de tabela sanitizados por projeto
- `project/config_file.rs`: `rustclaw.json` per-project config
- Constantes de kind: `KIND_FACT`, `KIND_COMMAND`, `KIND_CONVENTION`,
  `KIND_PATTERN`, `KIND_DECISION`, `KIND_TRAP`
- Confiança: `CONFIDENCE_INFERRED`, `CONFIDENCE_CONFIRMED`
- Comandos: `/memory list|rm|clear|promote <id>|gc` (`ui/commands/memory.rs`)

### Auth & Budget
- `auth.rs`: `AuthStore` — gerencia tokens de API por provider em
  `dirs::data_local_dir()/rustclaw/auth.json` (0600)
- `budget.rs`: `BudgetTracker` — contabilidade de uso de tokens + estimativa
  de custo via `catalog::estimate_cost` e `catalog::estimate_cost_cached`

### Hooks
- `hooks.rs`: `run_pre_tool` (antes da execução) e `spawn_post_tool` (depois,
  fire-and-forget) — pontos de extensão para plugins/observability
