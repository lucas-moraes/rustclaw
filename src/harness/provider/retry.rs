//! Retry policy for transient provider errors (429, 5xx, network timeouts).
//!
//! The provider adapters stay pure: they return `anyhow::Error` with a
//! `ProviderErrorKind` attached. The processor/compaction wrap their
//! `stream`/`complete` calls with [`retry_with_policy`], which classifies the
//! error and retries with exponential backoff + jitter when retryable.

use anyhow::Error;
use std::time::Duration;

/// Classification of a provider error for retry decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryKind {
    /// Transient: safe to retry (429, 500, 502, 503, 504, network timeout).
    Retryable,
    /// Permanent: do not retry (400, 401, 404, 422, model errors).
    Permanent,
}

/// Attach a retry classification to an `anyhow::Error`.
///
/// (Kept as a trait for future extension; the concrete helpers
/// [`error_retry_kind`] and [`error_retry_after`] are the current API.)
#[allow(dead_code)]
pub trait ProviderErrorExt {
    fn retry_kind(&self) -> RetryKind;
}

/// Marker type stored in the error chain to carry the retry classification.
#[derive(Debug)]
pub struct ProviderError {
    pub kind: RetryKind,
    /// HTTP status code, when the error came from an HTTP response.
    #[allow(dead_code)]
    pub status: Option<u16>,
    /// Value of the `Retry-After` header, in seconds, when present.
    pub retry_after: Option<u64>,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "provider error (kind={:?})", self.kind)
    }
}

impl std::error::Error for ProviderError {}

/// Builds an `anyhow::Error` carrying a retry classification.
pub fn provider_error(
    kind: RetryKind,
    status: Option<u16>,
    retry_after: Option<u64>,
    msg: impl Into<String>,
) -> Error {
    anyhow::Error::new(ProviderError {
        kind,
        status,
        retry_after,
    })
    .context(msg.into())
}

/// Classifies an HTTP status code.
pub fn classify_status(status: u16) -> RetryKind {
    match status {
        429 | 500 | 502 | 503 | 504 => RetryKind::Retryable,
        _ => RetryKind::Permanent,
    }
}

/// Extracts the retry classification from an error chain.
pub fn error_retry_kind(err: &Error) -> RetryKind {
    for cause in err.chain() {
        if let Some(pe) = cause.downcast_ref::<ProviderError>() {
            return pe.kind;
        }
        // reqwest network errors: timeouts and connection failures are
        // transient; everything else (DNS, body decode, request build) is not.
        if let Some(re) = cause.downcast_ref::<reqwest::Error>() {
            if re.is_timeout() || re.is_connect() {
                return RetryKind::Retryable;
            }
            return RetryKind::Permanent;
        }
        // io::ErrorKind::TimedOut (reqwest wraps these as timeouts).
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            if io.kind() == std::io::ErrorKind::TimedOut {
                return RetryKind::Retryable;
            }
            return RetryKind::Permanent;
        }
    }
    // Default: unknown errors (local parse bugs, etc.) are NOT retried —
    // retrying would mask the bug and burn quota.
    RetryKind::Permanent
}

/// Extracts the `Retry-After` value (seconds) from an error chain, if present.
pub fn error_retry_after(err: &Error) -> Option<u64> {
    for cause in err.chain() {
        if let Some(pe) = cause.downcast_ref::<ProviderError>() {
            return pe.retry_after;
        }
    }
    None
}

/// Retry policy configuration.
#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    /// Maximum number of attempts (including the first).
    pub max_attempts: usize,
    /// Base delay for the first retry, in milliseconds.
    pub base_delay_ms: u64,
    /// Maximum delay between retries, in milliseconds.
    pub max_delay_ms: u64,
    /// Whether to add random jitter to the delay.
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 500,
            max_delay_ms: 8_000,
            jitter: true,
        }
    }
}

/// Computes the delay for retry `attempt` (0-based, 0 = first retry).
pub fn backoff_delay(policy: &RetryPolicy, attempt: usize) -> Duration {
    let exp = 2u32.saturating_pow(attempt as u32);
    let base = policy.base_delay_ms.saturating_mul(exp as u64);
    let capped = base.min(policy.max_delay_ms);
    if policy.jitter {
        // Add up to 25% jitter.
        let jitter = (capped / 4).max(1);
        let offset = (rand::random::<u64>() % (jitter + 1)) as i64;
        let delay = capped as i64 + offset;
        Duration::from_millis(delay.max(1) as u64)
    } else {
        Duration::from_millis(capped)
    }
}

/// Runs `f` with retry on transient errors, honoring `Retry-After` when present.
///
/// `should_abort` is polled between attempts; when it returns `true`, the
/// current error is returned immediately (no further retries).
pub async fn retry_with_policy<T, F, Fut, A>(
    policy: &RetryPolicy,
    mut f: F,
    should_abort: A,
) -> Result<T, Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, Error>>,
    A: Fn() -> bool,
{
    let mut attempt = 0usize;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if should_abort() {
                    return Err(e);
                }
                let kind = error_retry_kind(&e);
                if kind == RetryKind::Permanent {
                    return Err(e);
                }
                attempt += 1;
                if attempt >= policy.max_attempts {
                    return Err(e);
                }
                // Honor Retry-After when present (429/503).
                let delay = match error_retry_after(&e) {
                    Some(secs) => Duration::from_secs(secs.min(60)),
                    None => backoff_delay(policy, attempt - 1),
                };
                tracing::warn!(
                    "provider error (attempt {}/{}), retrying in {:?}: {}",
                    attempt,
                    policy.max_attempts,
                    delay,
                    e
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_classify_status() {
        assert_eq!(classify_status(429), RetryKind::Retryable);
        assert_eq!(classify_status(500), RetryKind::Retryable);
        assert_eq!(classify_status(502), RetryKind::Retryable);
        assert_eq!(classify_status(503), RetryKind::Retryable);
        assert_eq!(classify_status(504), RetryKind::Retryable);
        assert_eq!(classify_status(400), RetryKind::Permanent);
        assert_eq!(classify_status(401), RetryKind::Permanent);
        assert_eq!(classify_status(404), RetryKind::Permanent);
        assert_eq!(classify_status(422), RetryKind::Permanent);
    }

    #[test]
    fn test_backoff_delay_increases() {
        let policy = RetryPolicy {
            jitter: false,
            ..Default::default()
        };
        let d0 = backoff_delay(&policy, 0);
        let d1 = backoff_delay(&policy, 1);
        let d2 = backoff_delay(&policy, 2);
        assert!(d1 > d0);
        assert!(d2 > d1);
        // Capped at max_delay_ms.
        let d10 = backoff_delay(&policy, 10);
        assert!(d10 <= Duration::from_millis(policy.max_delay_ms));
    }

    #[test]
    fn test_error_retry_kind_roundtrip() {
        let e = provider_error(RetryKind::Retryable, Some(429), Some(2), "rate limited");
        assert_eq!(error_retry_kind(&e), RetryKind::Retryable);
        assert_eq!(error_retry_after(&e), Some(2));

        let e2 = provider_error(RetryKind::Permanent, Some(400), None, "bad request");
        assert_eq!(error_retry_kind(&e2), RetryKind::Permanent);
        assert_eq!(error_retry_after(&e2), None);
    }

    #[tokio::test]
    async fn test_unknown_error_is_not_retried() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let policy = RetryPolicy {
            max_attempts: 3,
            base_delay_ms: 1,
            max_delay_ms: 5,
            jitter: false,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = calls.clone();
        let result = retry_with_policy(
            &policy,
            move || {
                let calls = calls2.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(anyhow::anyhow!("local parse bug"))
                }
            },
            || false,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1, "unknown errors: 0 retries");
    }

    #[test]
    fn test_reqwest_timeout_is_retryable() {
        // reqwest treats an io::ErrorKind::TimedOut source as a timeout
        // (see reqwest::Error::is_timeout); classification must be Retryable.
        let io = std::io::Error::new(std::io::ErrorKind::TimedOut, "simulated timeout");
        let e = anyhow::Error::new(io);
        assert_eq!(error_retry_kind(&e), RetryKind::Retryable);
        // A plain io error (not a timeout) is permanent.
        let io2 = std::io::Error::other("disk error");
        assert_eq!(
            error_retry_kind(&anyhow::Error::new(io2)),
            RetryKind::Permanent
        );
    }

    #[tokio::test]
    async fn test_retry_succeeds_after_transient_failures() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let policy = RetryPolicy {
            max_attempts: 3,
            base_delay_ms: 1,
            max_delay_ms: 5,
            jitter: false,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = calls.clone();
        let result = retry_with_policy(
            &policy,
            move || {
                let calls = calls2.clone();
                async move {
                    let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                    if n < 3 {
                        Err(provider_error(
                            RetryKind::Retryable,
                            Some(429),
                            None,
                            "rate limited",
                        ))
                    } else {
                        Ok(42)
                    }
                }
            },
            || false,
        )
        .await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_gives_up_after_max_attempts() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let policy = RetryPolicy {
            max_attempts: 2,
            base_delay_ms: 1,
            max_delay_ms: 5,
            jitter: false,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = calls.clone();
        let result = retry_with_policy(
            &policy,
            move || {
                let calls = calls2.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(provider_error(
                        RetryKind::Retryable,
                        Some(503),
                        None,
                        "unavailable",
                    ))
                }
            },
            || false,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_retry_does_not_retry_permanent_errors() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let policy = RetryPolicy {
            max_attempts: 3,
            base_delay_ms: 1,
            max_delay_ms: 5,
            jitter: false,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = calls.clone();
        let result = retry_with_policy(
            &policy,
            move || {
                let calls = calls2.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(provider_error(
                        RetryKind::Permanent,
                        Some(400),
                        None,
                        "bad request",
                    ))
                }
            },
            || false,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_aborts_on_user_cancel() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let policy = RetryPolicy {
            max_attempts: 5,
            base_delay_ms: 1,
            max_delay_ms: 5,
            jitter: false,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = calls.clone();
        let result = retry_with_policy(
            &policy,
            move || {
                let calls = calls2.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(provider_error(
                        RetryKind::Retryable,
                        Some(429),
                        None,
                        "rate limited",
                    ))
                }
            },
            {
                let calls = calls.clone();
                move || calls.load(Ordering::SeqCst) >= 2
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_retry_honors_retry_after() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let policy = RetryPolicy {
            max_attempts: 3,
            base_delay_ms: 1000,
            max_delay_ms: 5000,
            jitter: false,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = calls.clone();
        let result = retry_with_policy(
            &policy,
            move || {
                let calls = calls2.clone();
                async move {
                    let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                    if n < 2 {
                        Err(provider_error(
                            RetryKind::Retryable,
                            Some(429),
                            Some(0), // 0 seconds -> immediate retry
                            "rate limited",
                        ))
                    } else {
                        Ok("ok")
                    }
                }
            },
            || false,
        )
        .await;
        assert_eq!(result.unwrap(), "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
