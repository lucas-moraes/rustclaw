# Trust Model Audit Report

**Date:** 2026-04-09
**Phase:** 1 - Audit Trust Checks

## Summary

Trust checks are **partially implemented** in the ReAct loop's `execute_tool()` method. The `TrustEvaluator` and `WorkspaceTrustStore` exist and are loaded, but coverage is inconsistent.

## Current Implementation

### Files Involved
- `src/workspace_trust.rs` - Trust model core (405 lines)
- `src/agent/mod.rs` - Trust integration in `execute_tool()` method

### Existing Trust Checks

| Tool/Operation | Location | Status | Notes |
|----------------|----------|--------|-------|
| `file_write` | `agent/mod.rs:2740-2758` | ✅ Implemented | Checks `WriteFile` operation |
| `file_edit` | `agent/mod.rs:2740-2758` | ✅ Implemented | Checks `WriteFile` operation |
| `shell` | `agent/mod.rs:2760-2767` | ✅ Implemented | Checks `ExecuteShell` operation |
| `http_get` | - | ❌ Missing | No NetworkRequest check |
| `http_post` | - | ❌ Missing | No NetworkRequest check |

### Agent Structure
```rust
// src/agent/mod.rs:90
pub struct Agent {
    // ...
    workspace_trust: Option<TrustEvaluator>,
}
```

Trust is initialized in `Agent::new()` from `trust.json` if it exists.

### Trust Levels
- `Untrusted` - Default, no shell/write/network
- `UntrustedReadOnly` - Read-only operations
- `Trusted` - Shell + write allowed
- `FullyTrusted` - All operations including package install

### Operations Defined
- `ReadFile` - Always allowed
- `WriteFile` - Requires Trusted+
- `ExecuteShell` - Requires Trusted+
- `InstallPackage` - Requires FullyTrusted
- `NetworkRequest` - Requires !Untrusted
- `ReadSensitive` - Requires Trusted+

## Issues Found

### 1. Network Operations Not Checked (HIGH)
`http_get` and `http_post` tools do not call trust checks. An untrusted workspace can still make network requests.

**Fix:** Add trust check in `execute_tool()` for `http_get` and `http_post` actions.

### 2. Trust Not Checked at Tool Implementation Level
Trust is only checked in the agent's `execute_tool()` wrapper. Individual tool implementations (in `src/tools/`) do not perform trust checks, so if tools are called directly (not through agent), trust is bypassed.

**Current flow:**
```
Agent::execute_tool() → trust check → Tool::call()
```

**Problem:** If any code path calls tools directly without `execute_tool()`, trust is bypassed.

**Fix:** Consider adding trust middleware at tool level, or document this limitation.

### 3. Trust Store Not Persisted After Modification
When `set_trust()` is called, the store is not automatically saved to `trust.json`.

**Current:** Trust changes are in-memory only.

**Fix:** Auto-save after trust modifications.

### 4. No CLI Trust Management
Users cannot view or change trust levels via CLI or chat commands.

**Fix:** Add `trust` CLI command and `/trust` chat command.

## Recommendations

### Phase 2: Implement TrustChecker Middleware
1. Create `src/security/trust_checker.rs`
2. Consolidate trust logic into `TrustChecker` struct
3. Add `require_trust!` macro for cleaner checks

### Phase 3: Wire Trust Into Tools
1. Add NetworkRequest check for `http_get` / `http_post`
2. Ensure all tools go through trust checks

### Phase 4: Agent Integration & CLI
1. Add auto-save for trust modifications
2. Add `/trust` command for users
3. Add `trust` CLI command

## Verification

To verify trust checks are working:
```bash
# In untrusted workspace, try to write file
echo "test" | rustclaw --mode cli
> write a file
# Should be denied

# Elevate trust
> /trust trusted
# Try again
# Should succeed
```
