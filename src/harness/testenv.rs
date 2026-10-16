//! Global lock for tests that mutate process-wide environment variables.
//!
//! Several tests call `std::env::set_var`/`remove_var` (RUSTCLAW_HOME,
//! TAVILY_API_KEY, secret keys). Rust runs tests in parallel threads, so a
//! mutation in one test can corrupt assertions in another. Every test that
//! touches process env must hold [`lock`] for its whole body.

use std::sync::{Mutex, MutexGuard, OnceLock};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Acquire the global env lock. Hold the guard for the whole test body:
///
/// ```ignore
/// let _env = crate::harness::testenv::lock();
/// ```
///
/// A poisoned mutex is recovered with `into_inner` — a panicking test must
/// not cascade into lock failures for every other test.
pub(crate) fn lock() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}
