# RustClaw

AI Agent in Rust with persistent memory, skills, and ReAct architecture. Compatible with multiple LLM providers.

## ✨ Features

- 🤖 **AI Agent** with ReAct architecture
- 💾 **Persistent memory** via SQLite (with session linking)
- 🔍 **Web search** via Tavily (1000 queries/month free)
- 🧠 **Semantic embeddings** for memory search
- ⏰ **Reminder system** scheduled
- 🎯 **Skills** - Customizable commands with YAML frontmatter
- 📦 **MCP Client** - stdio and HTTP support
- 🔐 **Credential chaining** - Environment, keychain, config file
- 🎨 **Colored UI** - Terminal with colors
- 💻 **CLI Mode** and **Telegram Bot**
- 🔄 **Automatic fallback** - Multiple providers
- 🛡️ **Workspace trust** - Trust system
- 🔁 **Self-review loop** - Agent evaluates and refines responses
- 🔧 **Agent loop config** - Configurable retry/validation behavior
- 📋 **Session management** - List, resume, delete sessions via `/sessions`
- 🗺️ **Structured development** - Parse PLANO.md and execute in stages

## Supported Providers

| Provider | Model |
|----------|-------|
| **OpenCode Go** (default) | MiniMax M2.7 |
| OpenRouter | MiniMax M2.7, Qwen |
| VillaMarket | MiniMax M2.7 |
| Moonshot | Kimi K2.5 |
| HuggingFace | Qwen3-Coder-Next |

## 🚀 Installation

```bash
# Clone the project
git clone https://github.com/your-user/rustclaw
cd rustclaw

# Copy config template
cp .env.example config/.env

# Edit configuration
nano config/.env
```

## ⚙️ Configuration

```bash
# config/.env

# API Key (required) - Get from https://opencode.ai or your provider
TOKEN=your_token_here

# Provider (default: opencode-go)
# Options: opencode-go, openrouter, villamarket, moonshot, huggingface, custom
PROVIDER=opencode-go

# Model (optional - uses provider default if empty)
# Default for opencode-go: minimax-m2.7
MODEL=

# Base URL (optional - uses provider default if empty)
# Default for opencode-go: https://opencode.ai/zen/go/v1
BASE_URL=

# Tavily API Key (optional) - Get free key at https://tavily.com/api
# Free tier: 1000 queries/month
# TAVILY_API_KEY=

# Max tokens per response (recommended: 16000-32000 for development)
MAX_TOKENS=32000

# Max iterations per conversation (default: 50 for complex projects)
MAX_ITERATIONS=100

# Timezone (default: America/Sao_Paulo)
TZ=America/Sao_Paulo
```

### Agent Loop Configuration

```bash
# Auto-retry failed steps (default: true)
AGENT_AUTO_RETRY=true

# Max retries per failed step (default: 3)
AGENT_MAX_RETRIES_PER_STEP=3

# Require build validation before continuing (default: true)
AGENT_VALIDATION_REQUIRED=true

# Exit behavior on max retries: task, session, never (default: task)
AGENT_EXIT_ON_ERROR=task

# Force tool use for development tasks (default: true)
AGENT_FORCE_TOOL_USE=true
```

### Self-Review Configuration

```bash
# Enable self-review loop (default: true)
SELF_REVIEW_ENABLED=true

# Max self-review loops (default: 3)
SELF_REVIEW_MAX_LOOPS=3

# Show self-review process to user (default: true)
SELF_REVIEW_SHOW_PROCESS=true
```

### Feature Flags

```bash
# Enable features
PROACTIVE=1          # Proactive mode
BRIDGE_MODE=1        # IDE bridge
DEBUG=1              # Debug mode
VERBOSE=1            # Detailed logging
```

## 🤖 Usage

### CLI Mode

```bash
cargo run -- --mode cli
```

### Multi-line Input

O CLI suporta entrada multi-linha:
- **Linha terminada com `\`** = continua para próxima linha
- **Linha vazia** = envia o prompt completo
- **Duplo backslash `\\`** = literal `\` no final

Exemplo:
```
› primeira linha \
· segunda linha \
· terceira linha
```

### Commands

| Command | Description |
|---------|-------------|
| `exit` | Exit |
| `clear-memory` | Clear memories |
| `/skill` | List skills |
| `/skill:name` | Activate skill |
| `/clear` | Clear conversation |
| `/sessions` | List/resume sessions (interactive) |
| `/session <id>` | Resume specific session |
| `/desenvolver` | Structured development (parse PLANO.md) |

### Skills

Skills are in `skills/`:
- `/SKILL.md` with YAML frontmatter
- Supports `user_invocable`, `disable_model_invocation`
- Resources: `scripts/`, `references/`, `assets/`

## 🛠️ Available Tools

1. **file_list** - List directories
2. **file_read** - Read files
3. **file_write** - Write files
4. **file_search** - Search files
5. **shell** - Shell commands (with security)
6. **http_get/http_post** - HTTP requests
7. **tavily_search** - Tavily search
8. **web_search** - Web search
9. **browser** - Browser automation
10. **reminder** - Scheduled reminders

## 📁 Structure

```
rustclaw/
├── src/
│   ├── agent.rs      # Main agent
│   ├── config.rs     # Configuration
│   ├── skills/      # Skills system
│   ├── tools/       # Tools
│   ├── memory/      # SQLite memory
│   └── utils/       # Utilities
├── skills/          # User skills
├── config/          # Local config
└── .env             # Environment variables
```

## 🔐 Security

- **Shell**: Blocking dangerous commands (rm -rf, dd)
- **Heredoc**: Support for `cat > file << EOF`
- **Workspace Trust**: Trust levels per directory
- **Credential Chaining**: Multiple API key sources

## 📊 Comparison

| Aspect | Claude Code | RustClaw |
|--------|-------------|----------|
| Language | TypeScript | Rust |
| Store | Zustand | Custom (Zustand-like) |
| Memory | SQLite/JSON | SQLite |
| Skills | SKILL.md | SKILL.md |
| MCP | stdio | stdio + HTTP |

## 🧪 Development

```bash
# Development
cargo run

# Release build
cargo build --release

# Tests
cargo test
```

## 📝 License

MIT License - see LICENSE file.
