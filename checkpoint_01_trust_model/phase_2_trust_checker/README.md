# Phase 2: TrustChecker Middleware

**Date:** 2026-04-09
**Status:** ✅ Complete

## Deliverables

### Created Files
- `src/security/trust_checker.rs` - New TrustChecker middleware

### Modified Files
- `src/security/mod.rs` - Added `pub mod trust_checker;`
- `src/workspace_trust.rs` - Added `can_access_network()` method to TrustEvaluator

## Implementation Details

### TrustChecker Struct
```rust
pub struct TrustChecker {
    evaluator: TrustEvaluator,
    trust_file: Option<PathBuf>,
}
```

### Key Methods Added
- `check_read()` - Check ReadFile operation
- `check_write()` - Check WriteFile operation
- `check_shell()` - Check ExecuteShell operation
- `check_network()` - Check NetworkRequest operation
- `can_write()` - Quick bool check for write permission
- `can_execute_shell()` - Quick bool check for shell
- `can_access_network()` - Quick bool check for network
- `set_trust()` - Set trust level with auto-save
- `elevate_trust()` - Alias for set_trust

### require_trust! Macro
```rust
#[macro_export]
macro_rules! require_trust {
    ($checker:expr, $path:expr, $op:expr) => {{
        let decision = $checker.evaluate($path, $op.clone());
        if !decision.allowed {
            return Err(anyhow::anyhow!(
                "Trust error: operation '{}' not allowed (trust level: {:?})",
                $op,
                decision.trust_level
            ));
        }
        decision
    }};
}
```

### Tests Added
- `test_trust_checker_default()` - Tests default untrusted state
- `test_trust_evaluation()` - Tests trust elevation
- `test_operation_checks()` - Tests individual operation checks

## Verification
```bash
cargo test trust_checker --quiet  # 3 tests pass
```
