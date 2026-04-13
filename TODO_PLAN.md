# TODO Plan: Project Directory Sandbox

**Created:** April 13, 2026  
**Status:** ✅ Complete

---

## Overview

Implement a sandbox system that restricts ALL file/shell operations to a specified project directory when `/desenvolver <path>` is called. Without a project directory set, operations work normally.

---

## Requirements (All Implemented ✅)

1. ✅ When `/desenvolver <path>` sets project directory:
   - ALL file operations restricted to project_dir and subdirectories ONLY
   - file_read, file_write, file_edit, file_list restricted
   - shell commands with cd/paths restricted
2. ✅ When no project_dir set: normal unrestricted operations
3. ✅ Resume from checkpoint: sandbox automatically reactivated
4. ✅ Error messages in Portuguese

---

## Implementation Checklist

### Phase 1: Core Sandbox Module ✅

- [x] Create `src/security/project_sandbox.rs`
  - [x] Define `ProjectSandbox` struct with `allowed_dir: Option<PathBuf>`
  - [x] Implement `new()` constructor
  - [x] Implement `set_project_dir(path: PathBuf)` - activates sandbox
  - [x] Implement `clear()` - deactivates sandbox
  - [x] Implement `is_active() -> bool`
  - [x] Implement `allowed_dir() -> Option<&PathBuf>`
  - [x] Implement `validate_path(path: &Path) -> Result<PathBuf, String>`
    - [x] Canonicalize input path
    - [x] Check if path starts with allowed_dir
    - [x] Return canonical path on success
    - [x] Return error message on failure (in Portuguese)
  - [x] Add unit tests

- [x] Add `project_sandbox` module to `src/security/mod.rs`

### Phase 2: Agent Integration ✅

- [x] Add `project_sandbox: ProjectSandbox` field to Agent struct in `src/agent/mod.rs`

- [x] Modify `/desenvolver` handler in Agent.prompt()
  - [x] When dev_dir is set: call `self.project_sandbox.set_project_dir()`
  - [x] When session starts with checkpoint: reactivate sandbox from checkpoint.project_dir

- [x] Modify `execute_tool()` function
  - [x] Add sandbox validation before tool execution
  - [x] For file_read, file_write, file_edit, file_list: validate path parameter
  - [x] For shell: validate working_dir parameter and cd commands
  - [x] Return error message when blocked

### Phase 3: Shell Tool Enhancement ✅

- [x] Shell validation integrated in `execute_tool()` function
  - [x] Validate working_dir parameter against sandbox
  - [x] Extract and validate cd targets from commands
  - [x] Block commands that try to cd outside sandbox

### Phase 4: Testing ✅

- [x] Add integration tests for sandbox
  - [x] Test path validation with subdirectories
  - [x] Test path validation with symlinks
  - [x] Test path validation with parent directories (blocked)
  - [x] Test sandbox activation/deactivation
  - [x] Test checkpoint resume with sandbox

- [x] Run full test suite
  - [x] `cargo test` passes (167 tests)
  - [x] Build successful

---

## Files Created

| File | Purpose |
|------|---------|
| `src/security/project_sandbox.rs` | Core sandbox implementation |

## Files Modified

| File | Changes |
|------|---------|
| `src/security/mod.rs` | Added `pub mod project_sandbox;` |
| `src/agent/mod.rs` | Added field, modified /desenvolver, modified execute_tool |

---

## Error Messages (Portuguese) ✅

| Scenario | Message |
|----------|---------|
| Path outside sandbox | `🔒 Acesso negado: '{path}' está fora do diretório do projeto\nDiretório do projeto: {allowed_dir}` |
| Shell cd outside | `🔒 Acesso negado: cd para '{dir}' não permitido\nDiretório do projeto: {allowed_dir}` |
| Shell working_dir outside | `🔒 Acesso negado: {error}\nDiretório do projeto: {allowed_dir}` |

---

## Completion Criteria ✅

- [x] All checklist items completed
- [x] Tests pass (167 tests)
- [x] Sandbox prevents ALL file operations outside project directory
- [x] Sandbox allows ALL file operations inside project directory
- [x] Resume from checkpoint reactivates sandbox

---

## Implementation Details

### Sandbox Activation Flow

```
/desenvolver /path/to/project
         │
         ▼
┌─────────────────────────────────┐
│ Extract path from command       │
└─────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────┐
│ checkpoint.project_dir = path   │
│ agent.project_sandbox.set()      │
└─────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────┐
│ execute_tool() validates:       │
│   - file_read path              │
│   - file_write path             │
│   - file_edit path              │
│   - file_list path              │
│   - shell working_dir           │
│   - shell cd commands           │
└─────────────────────────────────┘
```

### Key Features

1. **Path Canonicalization**: Resolves `..` and symlinks before validation
2. **Parent Directory Blocking**: Prevents escaping via `../other_file.txt`
3. **Shell Command Validation**: Blocks `cd` commands outside sandbox
4. **Resume Support**: Sandbox automatically reactivated from checkpoint
