# Phase 3: Wire Trust Into Tools

**Date:** 2026-04-09
**Status:** ✅ Complete

## Summary

Added missing trust checks for network operations in `execute_tool()` method.

## Modified Files
- `src/agent/mod.rs` - Added NetworkRequest checks

## Changes Made

### Before (Missing Checks)
Only `file_write`, `file_edit`, and `shell` had trust checks.

### After (Complete Coverage)
All tool operations now have trust checks:

| Tool | Operation | Check |
|------|----------|-------|
| `file_write` | WriteFile | ✅ |
| `file_edit` | WriteFile | ✅ |
| `shell` | ExecuteShell | ✅ |
| `http_get` | NetworkRequest | ✅ Added |
| `http_post` | NetworkRequest | ✅ Added |

### Code Added to `execute_tool()`:
```rust
"http_get" | "http_post" => {
    if !trust.can_access_network(&current_dir) {
        return Ok(format!(
            "Acesso negado: operações de rede não permitida neste diretório (trust: {:?})",
            trust.evaluate(&current_dir, &crate::workspace_trust::Operation::NetworkRequest).trust_level
        ));
    }
}
```

## Bug Fix
Also added missing `can_access_network()` method to `TrustEvaluator`:
```rust
pub fn can_access_network(&self, path: &Path) -> bool {
    let trust = self.store.get_trust(path);
    trust.can_access_network()
}
```

## Verification
```bash
cargo build  # Compiles without errors
cargo test   # 83 tests pass
```
