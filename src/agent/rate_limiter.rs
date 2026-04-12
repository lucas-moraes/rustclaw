use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RateLimiter {
    max_calls_per_minute: usize,
    max_tokens_per_minute: usize,
    window_start: Instant,
    call_count: usize,
    token_count: usize,
}

#[allow(dead_code)]
impl RateLimiter {
    pub fn new(max_calls_per_minute: usize, max_tokens_per_minute: usize) -> Self {
        Self {
            max_calls_per_minute,
            max_tokens_per_minute,
            window_start: Instant::now(),
            call_count: 0,
            token_count: 0,
        }
    }

    pub fn from_env() -> Self {
        let max_calls = std::env::var("MAX_CALLS_PER_MINUTE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        let max_tokens = std::env::var("MAX_TOKENS_PER_MINUTE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100_000);

        Self::new(max_calls, max_tokens)
    }

    pub fn check_and_wait(&mut self, tokens_for_call: usize) -> WaitResult {
        self.cleanup_if_needed();

        if self.call_count >= self.max_calls_per_minute {
            let wait_time = self.time_until_next_window();
            return WaitResult::RateLimited {
                reason: RateLimitReason::CallsLimit,
                wait_seconds: wait_time.as_secs() as usize,
            };
        }

        if self.token_count + tokens_for_call > self.max_tokens_per_minute {
            let wait_time = self.time_until_next_window();
            return WaitResult::RateLimited {
                reason: RateLimitReason::TokensLimit,
                wait_seconds: wait_time.as_secs() as usize,
            };
        }

        self.call_count += 1;
        self.token_count += tokens_for_call;
        WaitResult::Allowed
    }

    pub fn record_call(&mut self, tokens_used: usize) {
        self.cleanup_if_needed();
        self.call_count += 1;
        self.token_count += tokens_used;
    }

    fn cleanup_if_needed(&mut self) {
        let elapsed = self.window_start.elapsed();
        if elapsed >= Duration::from_secs(60) {
            self.window_start = Instant::now();
            self.call_count = 0;
            self.token_count = 0;
        }
    }

    fn time_until_next_window(&self) -> Duration {
        let elapsed = self.window_start.elapsed();
        if elapsed >= Duration::from_secs(60) {
            Duration::from_secs(0)
        } else {
            Duration::from_secs(60) - elapsed
        }
    }

    pub fn calls_remaining(&mut self) -> usize {
        self.cleanup_if_needed();
        self.max_calls_per_minute.saturating_sub(self.call_count)
    }

    pub fn tokens_remaining(&mut self) -> usize {
        self.cleanup_if_needed();
        self.max_tokens_per_minute.saturating_sub(self.token_count)
    }

    pub fn current_call_count(&mut self) -> usize {
        self.cleanup_if_needed();
        self.call_count
    }

    pub fn current_token_count(&mut self) -> usize {
        self.cleanup_if_needed();
        self.token_count
    }
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum WaitResult {
    Allowed,
    RateLimited {
        reason: RateLimitReason,
        wait_seconds: usize,
    },
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum RateLimitReason {
    CallsLimit,
    TokensLimit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_allows_calls() {
        let mut limiter = RateLimiter::new(10, 10000);
        let result = limiter.check_and_wait(100);
        assert!(matches!(result, WaitResult::Allowed));
        assert_eq!(limiter.calls_remaining(), 9);
    }

    #[test]
    fn test_rate_limiter_blocks_at_limit() {
        let mut limiter = RateLimiter::new(2, 10000);
        limiter.check_and_wait(100);
        limiter.check_and_wait(100);
        let result = limiter.check_and_wait(100);
        assert!(matches!(
            result,
            WaitResult::RateLimited {
                reason: RateLimitReason::CallsLimit,
                ..
            }
        ));
    }

    #[test]
    fn test_rate_limiter_resets_window() {
        let mut limiter = RateLimiter::new(1, 100);
        limiter.check_and_wait(50);
        assert_eq!(limiter.calls_remaining(), 0);
        limiter.record_call(50);
        assert_eq!(limiter.current_call_count(), 2);
    }
}
