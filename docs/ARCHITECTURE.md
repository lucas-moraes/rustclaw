# RustClaw Architecture

> Last updated: Apr 2026

## Overview

RustClaw is an AI assistant with ReAct architecture, long-term memory, and skill execution capabilities. It can run as a CLI tool or Telegram bot.

## Core Components

### 1. Agent (`src/agent/`)
The main orchestration engine implementing the ReAct loop.

| Module | Lines | Purpose |
|--------|-------|---------|
| `mod.rs` | ~3200 | Main Agent struct, ReAct loop execution |
| `llm_client.rs` | ~200 | LLM API calls, prompt building |
| `response_parser.rs` | ~360 | Parse LLM responses, extract JSON |
| `session.rs` | ~250 | Session management, history |
| `plan_executor.rs` | ~120 | Development plan execution |
| `build_validator.rs` | ~40 | Build validation |
| `output.rs` | ~80 | Output formatting, TMUX integration |

### 2. Memory (`src/memory/`)
SQLite-based memory system with embeddings.

| Module | Lines | Purpose |
|--------|-------|---------|
| `store.rs` | ~590 | Memory CRUD operations |
| `checkpoint.rs` | ~2340 | Session/checkpoint persistence |
| `embeddings.rs` | ~100 | Text embedding generation |
| `search.rs` | ~100 | Semantic search |
| `reminder.rs` | ~50 | Reminder management |
| `skill_context.rs` | ~60 | Skill state per chat |

### 3. Tools (`src/tools/`)
Tool registry and implementations. Tools are async functions that agents can call.

**Core Tools:**
- `shell.rs` - Execute shell commands
- `file_read.rs` - Read files (with path validation)
- `file_write.rs` - Write files (with path validation)
- `file_edit.rs` - Edit files
- `file_search.rs` - Search files
- `file_list.rs` - List directory contents
- `http.rs` - HTTP requests
- `browser.rs` - Browser automation
- `system.rs` - System info

**Skill Tools:**
- `skill_manager.rs` - Skill loading/execution
- `skill_import.rs` - Import skills
- `skill_script.rs` - Run skill scripts
- `reminder.rs` - Set reminders
- `capabilities.rs` - List available tools

### 4. Skills (`src/skills/`)
Dynamic skill system with YAML definitions.

| Module | Purpose |
|--------|---------|
| `manager.rs` | Skill loading and execution |
| `loader.rs` | Load skill from directory |
| `parser.rs` | Parse skill metadata |
| `detector.rs` | Auto-detect applicable skills |
| `prompt_builder.rs` | Inject skill into prompts |
| `mcp_client.rs` | MCP protocol support |

### 5. Security (`src/security/`)
Input validation and prompt injection prevention.

| Module | Purpose |
|--------|---------|
| `sanitizer.rs` | Clean model output |
| `validator.rs` | Validate user input |
| `injection_detector.rs` | Detect prompt injection |
| `defense_prompt.rs` | Defense instructions |
| `output_cleaner.rs` | Clean tool output |

### 6. Utilities (`src/utils/`)
- `output.rs` - Output management
- `colors.rs` - Terminal colors
- `error_parser.rs` - Parse build errors
- `build_detector.rs` - Detect project type
- `tmux.rs` - TMUX session management
- `spinner.rs` - Loading spinner

### 7. CLI (`src/cli.rs`)
Terminal UI with raw mode (uses libc for keyboard input).

### 8. Telegram (`src/telegram/`)
Bot integration using teloxide.

## Data Flow

```
User Input
    │
    ▼
┌─────────────────┐
│  Security Layer │ ◄── Input validation, injection detection
└────────┬────────┘
         │
    ┌────▼────────┐
    │    Agent     │ ◄── ReAct loop
    │  (mod.rs)    │
    └────┬────────┘
         │
    ┌────▼────────────────────────────────┐
    │  LLM Client (llm_client.rs)        │ ◄── API calls
    └────┬───────────────────────────────┘
         │
    ┌────▼────────────────────────────────┐
    │  Response Parser (response_parser) │ ◄── Parse ReAct response
    └────┬───────────────────────────────┘
         │
    ┌────▼────────────────────────────────┐
    │    Tool Registry (tools/mod.rs)    │ ◄── Route to tools
    └────┬───────────────────────────────┘
         │
    ┌────▼───────────────────────────────┐
    │  Tool Execution + Trust Check      │
    │  (workspace_trust.rs)               │
    └────┬───────────────────────────────┘
         │
    ┌────▼───────────────────────────────┐
    │  Memory Store (memory/store.rs)     │ ◄── Save to SQLite
    │  Checkpoint Store (checkpoint.rs)   │
    └────────────────────────────────────┘
```

## Key Patterns

### ReAct Loop
1. Build system prompt with memory context
2. Call LLM with user input + history
3. Parse response into Thought/Action/Action Input
4. Execute tool, get observation
5. Verify result, loop until Final Answer

### Memory System
- **Episodes**: User-assistant conversations
- **Facts**: Single-occurrence facts
- **Tool Results**: Tool execution outputs
- Embeddings stored in SQLite, searched via cosine similarity

### Trust System
- `workspace_trust.rs` evaluates operations
- Levels: Blocked, ReadOnly, ReadWrite, Full
- File operations validated against trust level
- Shell execution restricted by directory trust

### Skills
- Defined in `skills/` directory
- YAML frontmatter with metadata
- Scripts in `scripts/`, references in `references/`
- Auto-loaded based on keywords in user input

## Database Schema

### memories (SQLite)
```sql
CREATE TABLE memories (
    id TEXT PRIMARY KEY,
    session_id TEXT,
    content TEXT,
    embedding BLOB,
    timestamp TEXT,
    importance REAL,
    memory_type TEXT,
    search_count INTEGER DEFAULT 0
);
```

### checkpoints (SQLite)
```sql
CREATE TABLE checkpoints (
    id TEXT PRIMARY KEY,
    user_input TEXT,
    current_iteration INTEGER,
    messages_json TEXT,
    completed_tools_json TEXT,
    plan_text TEXT,
    project_dir TEXT,
    phase TEXT,
    state TEXT,
    created_at TEXT,
    updated_at TEXT
);
```

## Dependencies

**Core:**
- `tokio` - Async runtime
- `reqwest` - HTTP client
- `rusqlite` - SQLite
- `serde` - Serialization

**Optional:**
- `teloxide` - Telegram bot
- `clap` - CLI args
- `rustyline` - Line editing

## Configuration

Via `config/.env`:
- `TOKEN` - API key (required)
- `TAVILY_API_KEY` - Web search
- `MAX_TOKENS` - LLM max tokens
- `MAX_ITERATIONS` - ReAct max loops

## Entry Points

- `cargo run` - Start in Telegram mode
- `cargo run -- --mode cli` - Start in CLI mode