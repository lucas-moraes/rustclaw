# Phase 3: RateLimiter

## Status: ✅ Complete

## Created Files

- `src/agent/rate_limiter.rs` - Rate limiting utilities

## Modified Files

- `src/agent/mod.rs` - Added `rate_limiter` module and field

## Implementation Details

### RateLimiter struct
```rust
pub struct RateLimiter {
    max_calls_per_minute: usize,
    max_tokens_per_minute: usize,
    window_start: Instant,
    call_count: usize,
    token_count: usize,
}
```

### Configuration
- `MAX_CALLS_PER_MINUTE` env var (default: 60)
- `MAX_TOKENS_PER_MINUTE` env var (default: 100,000)

### Methods
- `new(max_calls_per_minute, max_tokens_per_minute)` - Creates new limiter
- `from_env()` - Creates limiter from environment variables
- `check_and_wait(tokens_for_call)` - Checks if call is allowed, returns WaitResult
- `record_call(tokens_used)` - Records a call (for external tracking)
- `calls_remaining()` - Returns remaining calls in current window
- `tokens_remaining()` - Returns remaining tokens in current window

### WaitResult enum
```rust
pub enum WaitResult {
    Allowed,
    RateLimited {
        reason: RateLimitReason,
        wait_seconds: usize,
    },
}
```

### RateLimitReason enum
```rust
pub enum RateLimitReason {
    CallsLimit,
    TokensLimit,
}
```

## Agent Integration

Added `rate_limiter: RateLimiter` field to Agent struct, initialized via `RateLimiter::from_env()`

## Tests

All 3 tests pass:
- `test_rate_limiter_allows_calls`
- `test_rate_limiter_blocks_at_limit`
- `test_rate_limiter_resets_window`

## Verification

- ✅ `cargo clippy --quiet` - 0 warnings
- ✅ `cargo test` - 107 tests pass
