# TODO Plan - RustClaw Improvements

Based on project analysis (April 2026)

## Completed Features (1-8)

1. ✅ Trust Model Consistency
2. ✅ Embedding Fallback
3. ✅ Context Compaction
4. ✅ Cost Tracking
5. ✅ Parallel Execution
6. ✅ Internationalization
7. ❌ Agent Decomposition (cancelled - too high risk)
8. ✅ Error Handling Improvements

---

## Feature 9: Testing Improvements (Pending)

### Goals
- Increase test coverage for untested modules
- Add integration tests
- Improve test isolation

### Tasks

#### 9.1 Add tests to plan_executor.rs
- [ ] Add tests for `BuildValidator::validate_build`
- [ ] Test build detection for different project types
- [ ] Test error parsing scenarios

#### 9.2 Add tests to session.rs
- [ ] Add tests for `SessionManager::list_sessions`
- [ ] Test session hierarchy building
- [ ] Test checkpoint retrieval

#### 9.3 Add tests to skills/manager.rs
- [ ] Add tests for skill loading
- [ ] Test skill validation
- [ ] Test skill listing

#### 9.4 Clean up dead code
- [ ] Remove unused `#[allow(dead_code)]` items
- [ ] Remove unused functions from memory/store.rs
- [ ] Clean up utils/tmux.rs unused code

### Priority: HIGH
### Estimated effort: 3-5 days

---

## Feature 10: Security Hardening (Pending)

### Goals
- Replace unsafe terminal code with safe library
- Improve command execution safety
- Add working directory restrictions

### Tasks

#### 10.1 Replace unsafe terminal code
- [x] Add `rustix` crate to Cargo.toml
- [ ] Replace `unsafe` code in cli.rs:458-459 with rustix (deferred - deeply embedded)
- [ ] Use `rustix` for low-level terminal operations

#### 10.2 Improve shell security
- [x] Review path validation in shell.rs (already has good protection)
- [x] Working directory restrictions in script_executor.rs (paths are skill-relative)
- [ ] Consider using `git2` crate instead of git command (deferred)

#### 10.3 Add input validation
- [x] Review defense prompts (already implemented)

### Priority: HIGH
### Estimated effort: 2-3 days

---

## Feature 11: Performance Optimizations (Pending)

### Goals
- Optimize regex compilation
- Add pagination for memory retrieval
- Improve async parallelism

### Tasks

#### 11.1 Optimize regex patterns
- [ ] Review all static regex patterns
- [ ] Move remaining runtime regex to OnceLock
- [ ] Add benchmarks for critical paths

#### 11.2 Add pagination to memory operations
- [ ] Implement cursor-based pagination in memory/store.rs
- [ ] Add limit/offset to get_all operations
- [ ] Optimize retrieve_relevant_memories

#### 11.3 Improve async parallelism
- [ ] Add tokio::spawn for I/O-bound operations
- [ ] Parallel tool execution for independent tools
- [ ] Concurrent memory embeddings

### Priority: MEDIUM
### Estimated effort: 3-4 days

---

## Feature 12: Documentation (Pending)

### Goals
- Add missing documentation
- Fix documentation language consistency
- Create API documentation

### Tasks

#### 12.1 Add doc comments
- [ ] Add `///` to tools in src/tools/
- [ ] Document response_parser.rs
- [ ] Document session.rs
- [ ] Document checkpoint types

#### 12.2 Fix language consistency
- [ ] Convert Portuguese comments to English
- [ ] Standardize all documentation to English

#### 12.3 Create architecture docs
- [ ] Update ARCHITECTURE.md
- [ ] Add module diagrams

### Priority: LOW
### Estimated effort: 2-3 days

---

## Feature 13: Agent Decomposition (Future)

### Note
Feature 7 (Agent Decomposition) was cancelled due to high risk.
This can be revisited for RustClaw v2.0.

### Tasks (for reference only)
- [ ] Move ReAct loop to agent/react_loop.rs
- [ ] Move tool execution to agent/tool_execution.rs
- [ ] Move memory integration to agent/memory_integration.rs
- [ ] Move checkpoint handling to agent/checkpoint_handling.rs
- [ ] Move output formatting to agent/output_formatter.rs

### Priority: FUTURE (v2.0)
### Estimated effort: 20+ days

---

## Progress Tracking

| Feature | Status | Priority |
|--------|--------|-----------|
| 1-6 | ✅ Complete | - |
| 7 | ❌ Cancelled | - |
| 8 | ✅ Complete | HIGH |
| 9 | 🔲 Pending | HIGH |
| 10 | 🔲 Pending | HIGH |
| 11 | 🔲 Pending | MEDIUM |
| 12 | 🔲 Pending | LOW |
| 13 | ⏳ Future | LONG |

---

Last updated: April 12, 2026