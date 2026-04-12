# RustClaw Development Plan

## Overview

This plan addresses technical debt and missing features in RustClaw to make it a more capable software development agent.

---

## Completed Items (1-6)

| # | Item | Priority | Difficulty | Status |
|---|------|----------|------------|--------|
| 1 | Trust Model Consistency | High | Medium | ✅ Complete |
| 2 | Embedding Fallback | High | Medium | ✅ Complete |
| 3 | Context Compaction | High | Hard | ✅ Complete |
| 4 | Cost Tracking | Medium | Medium | ✅ Complete |
| 5 | Parallel Execution | Medium | Hard | ✅ Complete |
| 6 | Internationalization | Low | Medium | ✅ Complete |

---

## New Improvements Plan (7-12)

---

## 7. Agent Module Decomposition

**Problem:** `src/agent/mod.rs` has 3,562 lines handling too many responsibilities. This makes the code:
- Hard to maintain
- Difficult to test
- Prone to bugs
- Slow to compile (incremental)

**Priority:** Critical | **Difficulty:** Hard

### Checkpoints

#### Phase 1: Extract Plan Flow Handler
- [ ] Create `src/agent/plan_flow.rs`
- [ ] Move `PlanFlowHandler` struct from `mod.rs`
- [ ] Move: idea input, approval phases, directory input
- [ ] Move: `criar_plano`, `listar_planos`, `mostrar_plano` logic
- [ ] Update `mod.rs` to use new module

#### Phase 2: Extract Development Loop Handler
- [ ] Create `src/agent/development.rs`
- [ ] Move `run_development()` and `run_structured_development()`
- [ ] Move: stage/step execution logic
- [ ] Move: development checkpoint management
- [ ] Update `mod.rs` to use new module

#### Phase 3: Extract Self-Review Module
- [ ] Create `src/agent/self_review.rs`
- [ ] Move `SelfReviewer` struct
- [ ] Move: self-review loop, suggestions logic
- [ ] Move: `println!` statements that should use `tracing`
- [ ] Update `mod.rs` to use new module

#### Phase 4: Extract Tool Executor
- [ ] Create `src/agent/tool_executor.rs`
- [ ] Move `execute_tool()` method
- [ ] Move `verify_action_result()` method
- [ ] Move: trust checking, security cleaning
- [ ] Update `mod.rs` to use new module

#### Phase 5: Extract Output Manager
- [ ] Create `src/agent/output.rs`
- [ ] Move TMUX output functions
- [ ] Move: `init_tmux`, `cleanup_tmux`, `print_with_tmux`
- [ ] Consider `OutputManager` trait for testability
- [ ] Update `mod.rs` to use new module

#### Phase 6: Verify Decomposition
- [ ] Run `cargo clippy --quiet` - 0 warnings
- [ ] Run `cargo test` - all pass
- [ ] Verify all functionality still works
- [ ] Check compilation time improvement

---

## 8. Error Handling Improvements

**Problem:** Overuse of `anyhow::anyhow!` and inconsistent error types across the codebase.

**Priority:** High | **Difficulty:** Medium

### Checkpoints

#### Phase 1: Create Custom Error Types
- [x] Create `src/error.rs`
- [x] Define `AgentError` enum with variants:
  - `LLMError` (api failures, parsing)
  - `ToolError` (execution failures)
  - `TrustError` (trust violations)
  - `MemoryError` (storage issues)
  - `ConfigError` (invalid configuration)
- [x] Implement `std::error::Error` and `std::fmt::Display`

#### Phase 2: Replace anyhow in Agent
- [ ] Replace `anyhow::Result<String>` with `Result<String, AgentError>`
- [ ] Update `call_llm()` error handling
- [ ] Update `execute_tool()` error handling
- [ ] Update `prompt()` error handling

#### Phase 3: Replace anyhow in LLM Client
- [ ] Create `LLMError` variant
- [ ] Replace `anyhow::Result` with custom error type
- [ ] Add error context with `map_err(|e| LLMError::ApiCallFailed(format!("{e}")))`

#### Phase 4: Replace anyhow in Response Parser
- [ ] Create `ParseError` enum
- [ ] Replace regex `unwrap()` with proper error handling
- [ ] Add context to parse failures

#### Phase 5: Verify Error Handling
- [ ] Run `cargo clippy --quiet` - 0 warnings
- [ ] Run `cargo test` - all pass
- [ ] Verify error messages are informative

---

## 9. Testing Improvements

**Problem:** Critical code paths lack unit tests. The `Agent` module has no unit tests.

**Priority:** High | **Difficulty:** Medium

### Checkpoints

#### Phase 1: Add Agent Unit Tests
- [ ] Create `src/agent/tests.rs` or `src/agent/mod_tests.rs`
- [ ] Add tests for `TokenCounter`
- [ ] Add tests for `CostTracker`
- [ ] Add tests for `ConversationSummarizer`
- [ ] Add tests for `ResponseParser`

#### Phase 2: Add Shell Tool Security Tests
- [ ] Create `src/tools/shell_security_tests.rs`
- [ ] Add tests for `is_blocked()` function
- [ ] Add tests for `is_path_restricted()` function
- [ ] Add tests for dangerous command detection
- [ ] Add tests for path traversal prevention

#### Phase 3: Add Security Module Tests
- [ ] Add tests for `SecurityManager`
- [ ] Add tests for input sanitization edge cases
- [ ] Add tests for injection detection
- [ ] Add tests for `TrustChecker`

#### Phase 4: Add Memory Module Tests
- [ ] Add tests for `EmbeddingService`
- [ ] Add tests for BM25 ranking
- [ ] Add tests for TF-IDF fallback
- [ ] Add tests for checkpoint lifecycle

#### Phase 5: Verify Test Coverage
- [ ] Run `cargo test` - all pass
- [ ] Target >80% coverage on core modules
- [ ] Run `cargo clippy --quiet` - 0 warnings

---

## 10. Security Hardening

**Problem:** Unsafe FFI code, hardcoded security constants, potential for improvement.

**Priority:** High | **Difficulty:** Medium

### Checkpoints

#### Phase 1: Fix Unsafe FFI Code
- [ ] Replace `src/cli.rs:458-464` raw FFI with `termios` crate
- [ ] Remove `unsafe {}` blocks
- [ ] Add tests for terminal raw mode
- [ ] Verify behavior on macOS and Linux

#### Phase 2: Make Security Constants Configurable
- [ ] Create `src/security/config.rs`
- [ ] Move hardcoded limits to config:
  - `MAX_INPUT_LENGTH` (currently 10,240)
  - `MAX_SKILL_CONTEXT_SIZE` (currently 4,096)
  - `MAX_TOOL_OUTPUT_SIZE` (currently 65,536)
  - `MAX_OUTPUT_SIZE` in shell (currently 10,000)
- [ ] Add environment variable overrides
- [ ] Add validation with sensible defaults

#### Phase 3: Make Dangerous Commands Configurable
- [ ] Move `DANGEROUS_COMMANDS` list to config
- [ ] Move `SYSTEM_COMMANDS` list to config
- [ ] Add `DANGEROUS_COMMANDS_FILE` env var
- [ ] Add loading from JSON/YAML file

#### Phase 4: Add Security Audit Trail
- [ ] Add logging for blocked operations
- [ ] Add trust level changes to audit log
- [ ] Add network request blocking to audit log
- [ ] Document security events in checkpoint

#### Phase 5: Verify Security Hardening
- [ ] Run `cargo clippy --quiet` - 0 warnings
- [ ] Run `cargo test` - all pass
- [ ] Review for other potential security issues

---

## 11. Performance Optimizations

**Problem:** Unnecessary cloning and allocations in hot paths.

**Priority:** Medium | **Difficulty:** Medium

### Checkpoints

#### Phase 1: Reduce Memory Cloning
- [ ] Analyze `retrieve_relevant_memories()` - clone then filter
- [ ] Change to filter during retrieval
- [ ] Use references instead of cloning where possible
- [ ] Consider `Arc<[Value]>` for conversation_history

#### Phase 2: Optimize String Handling
- [ ] Audit hot loop string formatting in `mod.rs`
- [ ] Pre-allocate strings where size is known
- [ ] Use `String::with_capacity()` for large strings
- [ ] Replace repeated `format!()` with builder pattern

#### Phase 3: Optimize Regex Compilation
- [ ] Ensure all regex patterns use `OnceLock`
- [ ] Move regex compilation out of functions
- [ ] Add tests for regex patterns
- [ ] Consider `regex::RegexBuilder` for optimization

#### Phase 4: Database Optimizations
- [ ] Profile `MemoryStore::save()` serialization
- [ ] Add batch insert support
- [ ] Add index hints for FTS queries
- [ ] Consider connection pooling

#### Phase 5: Verify Performance
- [ ] Run benchmarks before/after
- [ ] Run `cargo clippy --quiet` - 0 warnings
- [ ] Run `cargo test` - all pass

---

## 12. Documentation Improvements

**Problem:** Missing documentation on complex functions and unclear naming.

**Priority:** Medium | **Difficulty:** Low

### Checkpoints

#### Phase 1: Add Module-Level Documentation
- [ ] Add docs to `src/agent/mod.rs`
- [ ] Add docs to `src/tools/mod.rs`
- [ ] Add docs to `src/memory/mod.rs`
- [ ] Add docs to `src/security/mod.rs`

#### Phase 2: Add Function Documentation
- [ ] Document `Agent::prompt()` (complex 1100+ line function)
- [ ] Document `Agent::run_development()`
- [ ] Document `Agent::execute_tool()`
- [ ] Document `ShellTool::is_blocked()`
- [ ] Document `ShellTool::is_path_restricted()`

#### Phase 3: Fix Naming Issues
- [ ] Rename `unsafe_indices` → `dependent_indices`
- [ ] Rename `checkpoint` variable → `checkpoint_data` (where confused with store)
- [ ] Rename `auto loop` → clearer name
- [ ] Fix `skip 5 words` magic number

#### Phase 4: Complete i18n Migration
- [ ] Migrate remaining hardcoded Portuguese strings
- [ ] Add Spanish translations
- [ ] Update `COMMAND_COMMANDS` to use i18n
- [ ] Update all CLI output to use i18n

#### Phase 5: Verify Documentation
- [ ] Run `cargo clippy --quiet` - 0 warnings
- [ ] Run `cargo doc` - no warnings
- [ ] Review doc coverage

---

## Progress Tracking

| # | Item | Priority | Difficulty | Status |
|---|------|----------|------------|--------|
| 1 | Trust Model Consistency | High | Medium | ✅ Complete |
| 2 | Embedding Fallback | High | Medium | ✅ Complete |
| 3 | Context Compaction | High | Hard | ✅ Complete |
| 4 | Cost Tracking | Medium | Medium | ✅ Complete |
| 5 | Parallel Execution | Medium | Hard | ✅ Complete |
| 6 | Internationalization | Low | Medium | ✅ Complete |
| 7 | Agent Decomposition | Critical | Hard | Pending |
| 8 | Error Handling | High | Medium | 🔄 Phase 1/5 Complete |
| 9 | Testing Improvements | High | Medium | Pending |
| 10 | Security Hardening | High | Medium | Pending |
| 11 | Performance Optimizations | Medium | Medium | Pending |
| 12 | Documentation | Medium | Low | Pending |

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

---

## Recommended Order

Based on dependency and priority:

1. **#7 Agent Decomposition** (Critical) - Do first to improve codebase maintainability
2. **#8 Error Handling** - Dependencies: #7
3. **#9 Testing** - Can do in parallel with #8
4. **#10 Security** - Can do in parallel with #8/#9
5. **#11 Performance** - After #7, benefits visible
6. **#12 Documentation** - Ongoing, can do anytime
