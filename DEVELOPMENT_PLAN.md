# RustClaw Development Plan

## Overview

This plan addresses technical debt and missing features in RustClaw to make it a more capable software development agent. Issues are listed in growing priority order (easiest to hardest).

---

## Getting Started

Recommended order based on impact/difficulty ratio:

1. **#5 Trust Model** - Quick win, improves security
2. **#3 Embedding Fallback** - Improves memory quality without API key
3. **#4 Context Compaction** - Critical for long sessions
4. **#6 Cost Tracking** - Operational visibility
5. **#1 Parallel Execution** - Performance improvement
6. **#2 Internationalization** - Low priority, large effort

---

## 1. Trust Model Consistency

**Problem:** `WorkspaceTrustStore` exists but is not consistently checked in all tool operations.

**Priority:** High | **Difficulty:** Medium

### Checkpoints

#### Phase 1: Audit Trust Checks
- [ ] Audit `shell` tool for existing trust checks
- [ ] Audit `file_write` / `file_edit` for trust checks
- [ ] Audit `http_get` / `http_post` for trust checks
- [ ] Document findings in `checkpoint_01_trust_model/phase_1_audit/audit_tool_trust_checks.md`

#### Phase 2: Implement TrustChecker Middleware
- [ ] Create `src/security/trust_checker.rs`
- [ ] Implement `TrustChecker` struct with `check()` method
- [ ] Add `require_trust!` macro
- [ ] Define `Operation` enum for different operations

#### Phase 3: Wire Trust Into Tools
- [ ] Add trust check to `file_write` tool
- [ ] Add trust check to `file_edit` tool
- [ ] Add trust check to `http_get` / `http_post` tools
- [ ] Update `shell` tool to use `TrustChecker`

#### Phase 4: Agent Integration & CLI
- [ ] Add `workspace_trust` field to `Agent`
- [ ] Initialize trust store from config in Agent
- [ ] Add `trust` CLI command to view/elevate trust level
- [ ] Add `/trust` chat command

#### Phase 5: Verification
- [ ] Run `cargo clippy --quiet` - 0 warnings
- [ ] Run `cargo test` - all pass
- [ ] Test trust elevation flow in CLI mode
- [ ] Test trust blocks in untrusted workspace

---

## 2. Embedding Fallback Improvement

**Problem:** Without OpenAI API key, the hash-based fallback embedding produces poor semantic search results.

**Priority:** High | **Difficulty:** Medium

### Checkpoints

#### Phase 1: TF-IDF Fallback Implementation
- [ ] Create `src/memory/embeddings_tfidf.rs`
- [ ] Implement tokenization (whitespace + punctuation)
- [ ] Add stopword list
- [ ] Implement TF-IDF weighting
- [ ] Implement hashing trick for fixed-dimension vectors
- [ ] Normalize vectors

#### Phase 2: Config Options
- [ ] Add `EMBEDDING_MODEL` config option (openai/cohere/local)
- [ ] Add `EmbeddingModel` enum to `config.rs`
- [ ] Update `EmbeddingService::new()` to use config
- [ ] Add warning when fallback is active

#### Phase 3: BM25 Secondary Ranking
- [ ] Create `src/memory/bm25.rs`
- [ ] Implement BM25 scoring
- [ ] Integrate BM25 as secondary signal in `MemoryStore::search()`
- [ ] Combine embedding similarity + BM25 for final ranking

#### Phase 4: Quality Metrics
- [ ] Add `EmbeddingQuality` enum (High/Medium/Low)
- [ ] Add `embedding_quality()` method to `EmbeddingService`
- [ ] Log warning when low quality embeddings used
- [ ] Add to `/stats` output

#### Phase 5: Verification
- [ ] Run `cargo clippy --quiet` - 0 warnings
- [ ] Run `cargo test` - all pass
- [ ] Test search with local embeddings (no API key)
- [ ] Verify BM25 improves results on fallback mode

---

## 3. Context Compaction / Summarization

**Problem:** Long conversations grow unbounded and will eventually overflow the LLM context window.

**Priority:** High | **Difficulty:** Hard

### Checkpoints

#### Phase 1: Token Counting
- [ ] Create `src/agent/token_counter.rs`
- [ ] Implement simple tokenizer (estimate tokens from characters)
- [ ] Add `count_tokens()` function
- [ ] Add `count_messages_tokens()` for conversation history
- [ ] Add `max_context_tokens` to `AgentConfig`

#### Phase 2: ConversationSummarizer
- [ ] Create `src/agent/conversation_summarizer.rs`
- [ ] Implement `ConversationSummarizer` struct
- [ ] Add `should_summarize()` method
- [ ] Add `summarize()` method using LLM
- [ ] Add `max_messages_to_preserve` config

#### Phase 3: Integrate Into ReAct Loop
- [ ] Add token counting before each LLM call
- [ ] Trigger summarization at threshold (default 80% of context)
- [ ] Preserve system prompt and current task during compression
- [ ] Track `compression_count` in session state

#### Phase 4: Manual Summarization Command
- [ ] Add `/summarize` command
- [ ] Add `/compress` alias
- [ ] Update CLI to support manual trigger
- [ ] Add compression stats to `/stats`

#### Phase 5: Verification
- [ ] Run `cargo clippy --quiet` - 0 warnings
- [ ] Run `cargo test` - all pass
- [ ] Test with long conversation (simulate context growth)
- [ ] Verify summarization preserves critical context

---

## 4. Rate Limiting and Cost Tracking

**Problem:** No tracking of API usage, costs, or iteration count beyond basic `max_iterations`.

**Priority:** Medium | **Difficulty:** Medium

### Checkpoints

#### Phase 1: CostTracker Implementation
- [ ] Create `src/agent/cost_tracker.rs`
- [ ] Implement `CostTracker` struct with all fields
- [ ] Add `record_call()` method (tokens, cost)
- [ ] Add `record_iteration()` method
- [ ] Add `reset()` method

#### Phase 2: LLM Client Integration
- [ ] Add `CostTracker` field to `LlmClient`
- [ ] Count tokens before LLM call
- [ ] Count tokens in response
- [ ] Update cost estimate using model pricing

#### Phase 3: RateLimiter
- [ ] Create `src/agent/rate_limiter.rs`
- [ ] Implement `RateLimiter` struct
- [ ] Add `check_and_wait()` method
- [ ] Add `MAX_CALLS_PER_MINUTE` config
- [ ] Add `MAX_TOKENS_PER_MINUTE` config

#### Phase 4: Stats Command
- [ ] Add `/stats` command
- [ ] Display total_tokens, api_calls, iterations, estimated_cost
- [ ] Display rate limiter status
- [ ] Persist stats to checkpoint

#### Phase 5: Verification
- [ ] Run `cargo clippy --quiet` - 0 warnings
- [ ] Run `cargo test` - all pass
- [ ] Test `/stats` output format
- [ ] Verify rate limiting kicks in at threshold

---

## 5. Parallel Tool Execution

**Problem:** Currently tools are executed one at a time in the ReAct loop. This is slow and inefficient for tasks that could run concurrently.

**Priority:** Medium | **Difficulty:** Hard

### Checkpoints

#### Phase 1: ParallelActions Enum
- [ ] Add `Parallel(Vec<ToolAction>)` variant to response parser
- [ ] Update `Action` enum documentation
- [ ] Update `ResponseParser::parse_action()` to handle parallel
- [ ] Update `ResponseParser::parse_action_json()`

#### Phase 2: ParallelExecutor
- [ ] Create `src/agent/parallel_executor.rs`
- [ ] Implement `execute_parallel()` with `futures_util::join_all`
- [ ] Add `max_parallel` config (default: 3)
- [ ] Handle partial failures gracefully

#### Phase 3: Dependency Analysis
- [ ] Add `analyze_dependencies()` function
- [ ] Detect file_write → file_read dependencies
- [ ] Detect write → shell dependencies
- [ ] Auto-split parallel actions that have dependencies

#### Phase 4: Update Verify Action
- [ ] Update `verify_action_result()` signature
- [ ] Handle `Vec<ToolResult>` for parallel execution
- [ ] Update error aggregation for partial failures

#### Phase 5: LLM Prompt Update
- [ ] Update system prompt to encourage parallel tool use
- [ ] Add examples of parallel execution in prompt
- [ ] Document parallel format in tool description

#### Phase 6: Verification
- [ ] Run `cargo clippy --quiet` - 0 warnings
- [ ] Run `cargo test` - all pass
- [ ] Test parallel execution with independent tools
- [ ] Test dependency detection prevents race conditions

---

## 6. Internationalization (i18n)

**Problem:** All user-facing messages are hardcoded in Portuguese. No way to switch languages.

**Priority:** Low | **Difficulty:** Medium

### Checkpoints

#### Phase 1: Module Structure
- [ ] Create `src/i18n/` directory
- [ ] Create `src/i18n/mod.rs`
- [ ] Define `I18n` trait
- [ ] Implement `Locale` enum (en, pt_br, es)
- [ ] Add `LOCALE` environment variable support

#### Phase 2: Translation Files
- [ ] Create `src/i18n/en.rs` with all English strings
- [ ] Create `src/i18n/pt_br.rs` (extract from existing Portuguese strings)
- [ ] Create `src/i18n/es.rs` with Spanish translations
- [ ] Define `MessageKey` enum with all translation keys

#### Phase 3: String Migration
- [ ] Migrate CLI prompts to use `i18n::message()`
- [ ] Migrate error messages to use `i18n::message()`
- [ ] Migrate tool descriptions to use `i18n::message()`
- [ ] Migrate agent responses to use `i18n::message()`
- [ ] Update `Colors` to work with translated strings

#### Phase 4: Dynamic Locale Switching
- [ ] Add `/locale <lang>` command
- [ ] Add `set_locale()` runtime method
- [ ] Persist locale preference per user
- [ ] Update config to support default locale

#### Phase 5: Verification
- [ ] Run `cargo clippy --quiet` - 0 warnings
- [ ] Run `cargo test` - all pass
- [ ] Test locale switching at runtime
- [ ] Verify all strings display correctly in different languages

---

## Progress Tracking

| # | Item | Priority | Difficulty | Status |
|---|------|----------|------------|--------|
| 1 | Trust Model Consistency | High | Medium | ✅ Complete |
| 2 | Embedding Fallback | High | Medium | ✅ Complete |
| 3 | Context Compaction | High | Hard | ✅ Complete |
| 4 | Cost Tracking | Medium | Medium | ✅ Complete |
| 5 | Parallel Execution | Medium | Hard | ✅ Complete |
| 6 | Internationalization | Low | Medium | 🔄 Infrastructure Complete |

---

## Checkpoint Directory Structure

```
checkpoint_01_trust_model/
├── phase_1_audit/
├── phase_2_trust_checker/
├── phase_3_wire_trust/
├── phase_4_cli_commands/
└── phase_5_verification/

checkpoint_02_embeddings/
├── phase_1_tfidf_fallback/
├── phase_2_config_options/
├── phase_3_bm25_ranking/
├── phase_4_quality_metrics/
└── phase_5_verification/

checkpoint_03_context_compaction/
├── phase_1_token_counting/
├── phase_2_summarizer/
├── phase_3_react_integration/
├── phase_4_cli_commands/
└── phase_5_verification/

checkpoint_04_cost_tracking/
├── phase_1_cost_tracker/
├── phase_2_llm_integration/
├── phase_3_rate_limiter/
├── phase_4_stats_command/
└── phase_5_verification/

checkpoint_05_parallel_execution/
├── phase_1_parallel_enum/
├── phase_2_executor/
├── phase_3_dependency_analysis/
├── phase_4_verify_update/
├── phase_5_llm_prompt/
└── phase_6_verification/

checkpoint_06_i18n/
├── phase_1_module_structure/
├── phase_2_translations/
├── phase_3_string_migration/
├── phase_4_locale_switching/
└── phase_5_verification/
```

---

## Commit Strategy

Each phase should result in a commit with message format:
```
feat(feature): phase N - description

- What changed
- What was verified
```

Final feature commit:
```
feat(feature): complete feature name

- All phases completed
- All tests pass
- Ready for integration
```
