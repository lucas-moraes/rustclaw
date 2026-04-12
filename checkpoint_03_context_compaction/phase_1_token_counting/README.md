# Phase 1: Token Counting

## Status: ✅ Complete

## Created Files

- `src/agent/token_counter.rs` - Token counting utilities

## Modified Files

- `src/config.rs` - Added `max_context_tokens` field
- `src/agent/mod.rs` - Added `token_counter` module

## Implementation Details

### TokenCounter struct
- `count_tokens(text)` - Estimates tokens from character count
- `count_messages_tokens(messages)` - Counts tokens across conversation history
- `chars_to_tokens(chars)` - Static method for conversion
- `tokens_to_chars(tokens)` - Static method for reverse conversion
- `context_usage_ratio(messages)` - Returns ratio of context used (0.0 to 1.0)
- `should_summarize(messages, threshold)` - Checks if context exceeds threshold
- `reserve_space_for_response(messages, response_tokens)` - Calculates available space

### Constants
- `DEFAULT_MAX_CONTEXT_TOKENS: usize = 128_000`
- `TOKENS_PER_CHAR_RATIO: f64 = 0.25` (1 token ≈ 4 characters)

### Config Changes
- Added `MAX_CONTEXT_TOKENS` environment variable (default: 128,000)
- Added `max_context_tokens` field to `Config` struct

## Tests

All 5 token counter tests pass:
- `test_count_tokens`
- `test_chars_to_tokens`
- `test_tokens_to_chars`
- `test_context_usage_ratio`
- `test_should_summarize`

## Verification

- ✅ `cargo clippy --quiet` - 0 warnings (excluding expected unused warnings for yet-to-be-used code)
- ✅ `cargo test` - 96 tests pass
