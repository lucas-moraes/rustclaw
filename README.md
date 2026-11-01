# RustClaw

A coding agent harness in Rust, in the style of OpenCode / Claude Code. Agent loop with **native tool calling**, sessions persisted in SQLite, **skills** (session memory), an animated TUI, human-in-the-loop permissions, builtin agents and subagents.

## ✨ Features

- 🤖 **Native tool calling** — tools described via JSON Schema; no free-text ReAct parsing
- ⚡ **Parallel execution** — multiple independent tool calls run concurrently
- 💾 **SQLite sessions** — messages/parts persisted; list, resume, delete
- 🧠 **Skills (session memory)** — pick skills per session, injected into the prompt each turn (checkbox)
- 🔐 **HITL permissions** — allow / ask / deny per tool and path (y/n/always in the CLI)
- 🎭 **Agents** — `build`, `plan`, `explore`, `general`, `chat-free` + subagent via the `task` tool
- 🛠️ **Coding tools** — bash, read, write, edit, glob, grep, ast_search, todo, question, task, remember, git_status, git_diff, git_log
- 🔍 **Syntactic search (Tree-Sitter)** — `ast_search` extracts structs, enums, traits, functions and impls from `.rs` files via AST, with no regex or false positives
- 🔌 **MCP (Model Context Protocol)** — connects to external MCP servers (stdio or streamable HTTP) and exposes their tools as native tools (`mcp_<server>_<tool>`)
- 🌐 **Free Web Research** — DuckDuckGo search (`web_search`) and HTML documentation converted to clean Markdown (`fetch_webpage`), with no paid API keys
- 📋 **Quick Code Export** — copy (`Ctrl+Y`) or save to a file (`Ctrl+S`) the code blocks the agent generates, straight from the TUI
- 🎨 **Cyberclaw TUI (ratatui + crossterm)** — colorful theme (4 switchable themes), animated splash, transcript in bubbles, status bar with tokens/context, command palette, permission/question modals, colored diff. Streaming CLI as fallback (`RUSTCLAW_UI=cli`)
- 🔁 **Compaction** — context summarization on overflow
- 🐛 **Doom-loop detection** — stops when the agent repeats the same tool call
- ⚡ **Prompt caching** — `cache_control` breakpoints on Anthropic (system/tools/last message), cache-aware cost in `/usage` (write 1.25×/read 0.1×; OpenAI cached 0.5× + write premium 1.25×), `prompt_caching` kill switch in config.json — see `docs/FEATURES.md`
- 🎛️ **Configurable provider parameters** — `fail_fast`, `service_tier`, `reasoning_effort`, `response_format` and sampling (`top_p`, `presence_penalty`, `frequency_penalty`, `stop`, `seed`) via `/settings` — emitted only when explicitly set
- ✅ **Verification gate** — nudges the agent to run `cargo test`/`check`/`clippy` after code mutations (write/edit) before ending the turn
- 🎬 **Living TUI** — fade animation on reasoning (braille spinner while "thinking"), live status inside the transcript (streaming/working/tools), floating confirmation toasts (copy, save, attach image)

## Supported Providers

| Provider | Base URL (default) |
|----------|--------------------|
| **DeepInfra** | `https://api.deepinfra.com/v1/openai` |
| **opencode-go** | `https://opencode.ai/zen/go/v1` |
| OpenRouter | `https://openrouter.ai/api/v1` |
| Moonshot | `https://api.moonshot.ai/v1` |
| VillaMarket | `https://api.minimax.villamarket.ai/v1` |
| HuggingFace | `https://router.huggingface.co/v1` |

Any OpenAI-compatible provider works via `BASE_URL` + `Authorization: Bearer`.

## 🚀 Installation

macOS (Apple Silicon) and Linux x64, via prebuilt release:

```bash
curl -fsSL https://raw.githubusercontent.com/lucas-moraes/rustclaw/main/scripts/install.sh | bash
```

The installer downloads the latest release, validates the SHA-256, copies it to `~/.local/bin/rustclaw`
and adjusts PATH if needed. Specific version:

```bash
RUSTCLAW_VERSION=v1.2.0 curl -fsSL .../install.sh | bash
```

Install from source (development):

```bash
git clone git@github.com:lucas-moraes/rustclaw.git && cd rustclaw
./scripts/link-local.sh   # cargo build --release + copies to ~/.local/bin
# or: cargo install --path .
```

## ⚙️ Configuration (no `.env`)

All configuration lives in files — no mandatory environment variables:

| File | Scope | Contents |
|------|-------|----------|
| `~/.local/share/rustclaw/auth.json` | global | API key per provider (chmod 600) |
| `~/.local/share/rustclaw/config.json` | global | provider/model, `max_iterations`, `max_context_tokens`, theme |
| `~/.local/share/rustclaw/mcp.json` | global | MCP servers (`mcpServers`) |
| `rustclaw.json` (project root) | per project | provider/model/base_url (written by `/models`), `mcp` section |

On **first run**, the TUI opens a wizard: pick **provider → model** (`/models`)
and paste the **token** (`/auth <provider>`) — nothing is edited by hand. Config commands:

| Command | What it does |
|---------|--------------|
| `/models` | provider/model picker (writes the project's `rustclaw.json`) |
| `/auth <provider>` | saves the token to `auth.json` (masked input) |
| `/settings` | view/edit `max_iterations`, `max_context_tokens`, theme |
| `/provider <name>` | switch provider (catalog default model) |
| `/model <name>` | switch model |

Precedence: **builtin catalog → `config.json` → `rustclaw.json` → token from `auth.json`**.

## 🤖 Usage

```bash
rustclaw                 # TUI (TTY); CLI if stdout/stdin is not a terminal
RUSTCLAW_UI=cli rustclaw # force streaming CLI
RUSTCLAWUI=cli rustclaw  # alias (same effect)
cargo run                # in the project directory (dev)
```

### UI

The default interface is the **TUI** when stdin and stdout are a TTY. Use the CLI when you
need a screen reader, CI, a pipe, or a session without an alternate screen.

| Variable / command | Effect |
|--------------------|--------|
| `RUSTCLAW_UI=cli` / `RUSTCLAWUI=cli` | forces the streaming REPL (no ratatui) |
| `NO_COLOR` | `mono` theme (terminal colors) and **locks** `/theme` and Ctrl+T |
| `RUSTCLAW_THEME` | initial preset (`cyberclaw`, `aurora`, `ember`, `mono`, `daylight`, `high-contrast`; `dark` = cyberclaw, `light` = daylight) |
| `/theme` · `/theme list` · `/theme <name>` · Ctrl+T | picker (TUI) or list/apply (CLI); persists to `config.json` |
| `?` / F1 | in-app help (keys, commands, agents, tips) |

TUI actions have an equivalent slash command (`/models`, `/skills`, `/sessions`, `/cursor`, `/settings`, `/theme`, …). TUI-only exceptions: animated splash, mouse selection, kill-ring yank (Ctrl+Y). Copy/save the last code block: **Ctrl+Y** (empty ring) / **Ctrl+S**, or `/copy-code` / `/save-code` in the TUI; in the CLI use `/export`.

### Skills: session memory

When you open a **new session**, the TUI opens a **skills picker** — choose the skills that
make up that session's memory (you can select none and change it later).

Before **each prompt**, the chosen skills appear as **checkbox chips** above the input;
only the checked ones enter that turn's prompt.

### TUI shortcuts

| Key | Action |
|-----|--------|
| `Ctrl+C` | cancels the active run / exits when idle |
| `Enter` | sends the prompt |
| `Ctrl+P` | command palette (commands/agents/themes/actions) |
| `Ctrl+T` | theme picker (`NO_COLOR` locks it to mono) |
| `Ctrl+L` | clears the local transcript |
| `Ctrl+Y` | yank from the kill-ring, or the last code block if the ring is empty (`/copy-code`) |
| `Ctrl+S` | saves the last code block (`/save-code`) |
| `Ctrl+←/→` | previous/next word |
| `Ctrl+Home/End` | top / bottom of the transcript |
| `?` / `F1` | help overlay (sections with `Tab`) |
| `Esc` | clears input / closes overlay |
| `↑/↓` | history (or navigate focused chips) |
| `PgUp/PgDn` or mouse wheel | scroll the transcript |
| `y`/`n`/`a` | permission modal (allow/deny/always) |
| `1..n` | question modal (pick an option) |
| `Space` | toggles a skill in the picker and in the chips |
| `Tab` | `/` autocomplete / next help section |

Simple prompt:
```
› List the .rs files in src/ using glob and read src/main.rs
```

### Slash commands

| Command | Description |
|---------|-------------|
| `/new` | New session (opens the skills picker) |
| `/sessions` | Manage sessions (picker: list/select/delete/rename) |
| `/sessions select <id>` | Select a session by id |
| `/agent <name>` | Switch agent (build/plan/explore/general/chat-free) |
| `/compact` | Compact the context manually |
| `/skills` | Manage skills (`list`·`add <id>`·`rm <id>`·`default <id> on\|off`·`picker`) |
| `/theme [name]` | List or apply a theme |
| `/mcp list\|status\|restart` | MCP servers (list/status/reconnect) |
| `/usage` | In/out tokens + session context window |
| `/memory list\|rm\|clear\|promote\|gc` | Project memory (facts ranked by recency/usage) |
| `/permissions set\|rm <tool> <rule>` | Per-tool permission rules (allow/ask/deny) |
| `/allow-all-permissions` | Grants the harness full freedom to modify any project file (persisted in `rustclaw.json`) |
| `/help` / `/exit` | Help / quit |

### Permissions

Destructive tools (`bash`, `write`, `edit`, `task`) ask for confirmation:
```
[permission] write (path: /proj/x.rs) {...}
  allow? [y]es/[n]o/[a]lways:
```
`a`/`always` approves the tool for the rest of the session. Paths outside the working directory are always escalated to `ask`.

To give the harness full freedom to modify **any project file** (without asking for
confirmation on every tool), use `/allow-all-permissions`. It marks every tool as
`allow` and persists to `rustclaw.json` (survives restarts). Paths **outside** the project
still require approval — the freedom stays scoped to the project directory.

### MCP (external servers)

Connect MCP servers (filesystem, github, postgres, etc.) via `~/.local/share/rustclaw/mcp.json`
or the `mcp` section of the project's `rustclaw.json` — standard `mcpServers` format:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    },
    "remote": {
      "url": "https://example.com/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

- Tools show up as `mcp_<server>_<tool>` (e.g. `mcp_filesystem_read_file`) and ask for
  permission like any mutable tool (`/permissions set mcp_... allow` to allow them).
- `${VAR}` in `args`/`env` is expanded from the environment; `timeout_secs` controls the per-call
  timeout (default 60); `enabled: false` disables a server.
- In `plan`/`explore` mode, MCP tools annotated with `readOnlyHint: true` are admitted
  automatically (`mcp_readonly` marker); all other MCP tools fall back to the `Ask` default.

Commands:

| Command | Description |
|---------|-------------|
| `/mcp list` | configured servers + tool count |
| `/mcp status` | connection state (connected/failed/disabled) |
| `/mcp restart <name>` | reconnects a server |

If a server's subprocess dies, the next call reconnects automatically (1 retry);
a health check every 60s flags dead servers in `/mcp status`.

## 🧱 Architecture

```
src/
├── main.rs          # Entry point
├── config.rs        # GlobalSettings (config.json) + resolution
├── error.rs         # Error types
└── harness/         # The harness itself
    ├── event.rs     # Event bus
    ├── runtime.rs   # SessionRuntime (facade)
    ├── skill/       # Skills = session memory (loader + inject)
    ├── session/     # Session/Message/Part + SQLite store + processor + compaction
    ├── provider/    # OpenAI, Anthropic, opencode-go adapters (streaming + native tools)
    ├── tool/        # Tool trait + registry + bash/read/write/edit/glob/grep/ast_search/todo/question/task
    ├── mcp/         # MCP client (mcpServers config, stdio/HTTP, McpManager, McpTool)
    ├── permission/  # allow/ask/deny
    ├── agent/       # AgentSpec + builtin (build/plan/explore/general/chat-free)
    └── ui/          # tui/ (app, draw, input, askers) + cli/ (fallback) + commands/
```

The central loop (`session/processor.rs`): sends the prompt to the provider with the native
tools, consumes the stream, executes tool calls (parallel via JoinSet, with permission checks),
and repeats until the model answers without tools. The **system prompt** is assembled by
`agent/build_system_prompt` (identity + Environment + AGENTS.md + checked skills +
operating rules); the session **history** becomes the messages. See `docs/ARCHITECTURE.md`.

## 🧪 Tests

```bash
cargo test            # unit suite (harness)
cargo clippy          # lint
cargo fmt --check     # formatting
```

A live smoke test (`--ignored`) validates native tool calling against a real provider using the token from the auth store (`~/.local/share/rustclaw/auth.json`):

```bash
cargo test --bin rustclaw smoke_native_tool_calling -- --ignored --nocapture
```

## 📄 License

MIT
