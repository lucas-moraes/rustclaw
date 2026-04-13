# New Features Plan - RustClaw

Based on project analysis (April 2026)

---

## Phase 1: Quick Wins ✅ Complete

### Feature F1: Streaming Responses ✅
- **Description**: Token-by-token display instead of waiting for full response
- **Complexity**: MEDIUM
- **Priority**: HIGH
- **Location**: `src/agent/llm_client.rs`, `src/cli.rs`
- **Tasks**:
  - [x] Implement SSE endpoint for LLM responses
  - [ ] Add streaming mode to CLI display (deferred)
  - [ ] Handle backpressure gracefully (deferred)
- **Related**: Currently `call_llm_with_config()` returns full response

### Feature F3: Conversation Export/Import ✅
- **Description**: Export conversations to Markdown/JSON for sharing or backup
- **Complexity**: LOW
- **Priority**: HIGH
- **Location**: New `src/memory/export.rs`
- **Tasks**:
  - [x] Implement export to Markdown format
  - [x] Implement export to JSON format
  - [ ] Implement import functionality (deferred)
  - [ ] Add CLI commands for export/import (deferred)

### Feature F4: Configuration Validation ✅
- **Description**: Validate config on startup with helpful error messages
- **Complexity**: LOW
- **Priority**: HIGH
- **Location**: `src/config.rs`
- **Tasks**:
  - [x] Add required field validation
  - [x] Add API key format validation
  - [x] Add helpful error messages with setup instructions
  - [ ] Add config wizard (deferred)

---

## Phase 2: Core Enhancements ✅ Complete

### Feature F6: Dry Run Mode ✅
- **Description**: Preview agent actions without execution
- **Complexity**: MEDIUM
- **Priority**: HIGH
- **Location**: `src/agent/mod.rs`
- **Tasks**:
  - [x] Add dry_run flag to Agent struct
  - [x] Add set_dry_run() and is_dry_run() methods
  - [x] Preview actions in dry-run mode
  - [ ] Add CLI --dry-run flag (deferred)
  - [ ] Add confirmation prompts (deferred)

### Feature F7: Undo/Rollback System ✅
- **Description**: File operation journaling with undo capability
- **Complexity**: MEDIUM
- **Priority**: HIGH
- **Location**: New `src/memory/journal.rs`
- **Tasks**:
  - [x] Implement operation journal
  - [x] Track file changes with backups
  - [x] Implement undo_last() method
  - [x] Add N-operations rollback (undo_n)
  - [ ] Add CLI /undo command (deferred)

### Feature F8: Hierarchical Memory System ✅
- **Description**: Memory tiers with automatic promotion
- **Complexity**: HIGH
- **Priority**: HIGH
- **Location**: New `src/memory/hierarchical.rs`
- **Tasks**:
  - [x] Implement Working Memory tier
  - [x] Implement Short-term Memory tier
  - [x] Implement Long-term Memory tier
  - [x] Implement automatic promotion based on usage
  - [x] Add memory consolidation
  - [x] Add memory decay mechanism

---

## Phase 4: Developer Experience (1-2 months)

### Feature D1: Plugin System
- **Description**: Dynamic plugin loading for extensibility
- **Complexity**: HIGH
- **Priority**: MEDIUM
- **Location**: New `src/plugins/` module
- **Tasks**:
  - [ ] Define plugin trait/interface
  - [ ] Implement WASM plugin loader
  - [ ] Implement external process plugins
  - [ ] Add plugin registry
  - [ ] Create plugin API documentation

### Feature D2: API Server Mode
- **Description**: REST/gRPC API for programmatic access
- **Complexity**: MEDIUM
- **Priority**: MEDIUM
- **Location**: New `src/api/` module
- **Tasks**:
  - [ ] Implement REST API endpoints
  - [ ] Add authentication
  - [ ] Implement streaming responses
  - [ ] Add WebSocket support

### Feature D3: Custom Prompt Templates
- **Description**: User-defined prompt templates
- **Complexity**: LOW
- **Priority**: LOW
- **Location**: New `src/config/prompts.rs`, `config/prompts/`
- **Tasks**:
  - [ ] Define template format
  - [ ] Implement template loader
  - [ ] Add built-in templates
  - [ ] Add CLI command to list templates

---

## Phase 5: Advanced Features (1-2 months)

### Feature A1: Multi-Agent Orchestration
- **Description**: Spawn sub-agents for complex tasks
- **Complexity**: HIGH
- **Priority**: MEDIUM
- **Location**: New `src/agent/multi_agent/` module
- **Tasks**:
  - [ ] Define SubAgent trait
  - [ ] Implement agent delegation
  - [ ] Add agent-to-agent communication
  - [ ] Implement result aggregation
  - [ ] Add agent pool management

### Feature A2: Code Intelligence (tree-sitter)
- **Description**: AST parsing for semantic code understanding
- **Complexity**: HIGH
- **Priority**: MEDIUM
- **Location**: New `src/tools/code_intelligence.rs`
- **Tasks**:
  - [ ] Integrate tree-sitter
  - [ ] Implement symbol extraction
  - [ ] Add go-to-definition support
  - [ ] Implement code search by AST

### Feature A3: Knowledge Graph
- **Description**: Entity relationships, not just vectors
- **Complexity**: HIGH
- **Priority**: MEDIUM
- **Location**: New `src/memory/knowledge_graph.rs`
- **Tasks**:
  - [ ] Define entity schema
  - [ ] Implement relationship extraction
  - [ ] Add graph queries
  - [ ] Integrate with memory system

### Feature A4: Image/Vision Support
- **Description**: Support vision models for image analysis
- **Complexity**: MEDIUM
- **Priority**: MEDIUM
- **Location**: `src/agent/llm_client.rs`, new `src/tools/vision.rs`
- **Tasks**:
  - [ ] Add image upload handling
  - [ ] Implement multi-modal messages
  - [ ] Add screenshot capture tool
  - [ ] Support GPT-4V/Claude Vision

---

## Progress Tracking

| Feature | Status | Complexity | Priority |
|---------|--------|-----------|----------|
| F1: Streaming Responses | ✅ Done | MEDIUM | HIGH |
| F3: Conversation Export | ✅ Done | LOW | HIGH |
| F4: Config Validation | ✅ Done | LOW | HIGH |
| F6: Dry Run Mode | ✅ Done | MEDIUM | HIGH |
| F7: Undo/Rollback | ✅ Done | MEDIUM | HIGH |
| F8: Hierarchical Memory | ✅ Done | HIGH | HIGH |
| T1: Database Tool | 🔲 Pending | MEDIUM | MEDIUM |
| T2: Docker Integration | 🔲 Pending | MEDIUM | MEDIUM |
| T3: Cloud Storage | 🔲 Pending | MEDIUM | LOW |
| D1: Plugin System | 🔲 Pending | HIGH | MEDIUM |
| D2: API Server | 🔲 Pending | MEDIUM | MEDIUM |
| D3: Prompt Templates | 🔲 Pending | LOW | LOW |
| A1: Multi-Agent | 🔲 Pending | HIGH | MEDIUM |
| A2: Code Intelligence | 🔲 Pending | HIGH | MEDIUM |
| A3: Knowledge Graph | 🔲 Pending | HIGH | MEDIUM |
| A4: Image/Vision | 🔲 Pending | MEDIUM | MEDIUM |

---

## Phase 1 Completed ✅

Phase 1 (Quick Wins) completed:
- **F1: Streaming Responses** - Added `call_llm_streaming()` method
- **F3: Conversation Export** - Created `src/memory/export.rs` with Markdown/JSON export
- **F4: Configuration Validation** - Added `validate()` method with helpful error messages

---

## Phase 2 Completed ✅

Phase 2 (Core Enhancements) completed:
- **F6: Dry Run Mode** - Added dry_run flag to Agent, preview actions
- **F7: Undo/Rollback** - Created `src/memory/journal.rs` with OperationJournal
- **F8: Hierarchical Memory** - Created `src/memory/hierarchical.rs` with MemoryTier system

---

## Technical Debt to Address

1. **ParallelExecutor** - Marked for implementation but not fully integrated
2. **Dead code** - Multiple `#[allow(dead_code)]` annotations
3. **MCP Client** - Basic implementation, needs full protocol support
4. **Browser automation** - Needs better error handling

---

## Notes

- This plan focuses on user-facing features first
- Complexity estimates assume single developer
- Priority based on user impact vs effort
- Features can be implemented in parallel by different developers

---

Last updated: April 12, 2026
