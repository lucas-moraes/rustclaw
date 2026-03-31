# RustClaw

AI Agent in Rust with persistent memory, skills, and ReAct architecture. Compatible with multiple LLM providers.

## ✨ Features

- 🤖 **AI Agent** with ReAct architecture
- 💾 **Persistent memory** via SQLite
- 🔍 **Web search** via Tavily or web search
- 🧠 **Semantic embeddings** for memory search
- ⏰ **Reminder system** scheduled
- 🎯 **Skills** - Customizable commands with YAML frontmatter
- 📦 **MCP Client** - stdio and HTTP support
- 🔐 **Credential chaining** - Environment, keychain, config file
- 🎨 **Colored UI** - Terminal with colors
- 💻 **CLI Mode** and **Telegram Bot**
- 🔄 **Automatic fallback** - Multiple providers
- 🛡️ **Workspace trust** - Trust system

## Supported Providers

| Provider | Model |
|----------|-------|
| **OpenCode Go** (default) | MiniMax M2.5 |
| OpenRouter | MiniMax M2.5, Qwen |
| VillaMarket | MiniMax M2.5 |
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

# API Key (required)
OPENCODE_API_KEY=your_api_key_here

# Provider (default: opencode-go)
PROVIDER=opencode-go

# Model (default: minimax-m2.5)
MODEL=minimax-m2.5

# Optional settings
MAX_TOKENS=4000
MAX_ITERATIONS=20
TZ=America/Sao_Paulo
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

### Commands

| Command | Description |
|---------|-------------|
| `exit` | Exit |
| `clear-memory` | Clear memories |
| `/skill` | List skills |
| `/skill:name` | Activate skill |
| `/clear` | Clear conversation |

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
