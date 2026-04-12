use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CostTracker {
    pub total_tokens_used: usize,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub api_calls: usize,
    pub iterations: usize,
    pub estimated_cost_usd: f64,
    pub rate_limit_hits: usize,
    pub last_call_time: Option<Instant>,
    pub session_start: Instant,
}

#[allow(dead_code)]
impl CostTracker {
    pub fn new() -> Self {
        Self {
            total_tokens_used: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            api_calls: 0,
            iterations: 0,
            estimated_cost_usd: 0.0,
            rate_limit_hits: 0,
            last_call_time: None,
            session_start: Instant::now(),
        }
    }

    pub fn record_call(&mut self, prompt_tokens: usize, completion_tokens: usize, model: &str) {
        self.prompt_tokens += prompt_tokens;
        self.completion_tokens += completion_tokens;
        self.total_tokens_used += prompt_tokens + completion_tokens;
        self.api_calls += 1;
        self.last_call_time = Some(Instant::now());

        let cost = self.calculate_cost(prompt_tokens, completion_tokens, model);
        self.estimated_cost_usd += cost;
    }

    pub fn record_iteration(&mut self) {
        self.iterations += 1;
    }

    pub fn record_rate_limit_hit(&mut self) {
        self.rate_limit_hits += 1;
    }

    pub fn calculate_cost(
        &self,
        prompt_tokens: usize,
        completion_tokens: usize,
        model: &str,
    ) -> f64 {
        let pricing = ModelPricing::for_model(model);
        let prompt_cost = (prompt_tokens as f64 / 1_000_000.0) * pricing.prompt_price_per_million;
        let completion_cost =
            (completion_tokens as f64 / 1_000_000.0) * pricing.completion_price_per_million;
        prompt_cost + completion_cost
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn session_duration(&self) -> Duration {
        self.session_start.elapsed()
    }

    pub fn calls_per_minute(&self) -> f64 {
        let elapsed = self.session_duration().as_secs();
        if elapsed == 0 {
            return 0.0;
        }
        (self.api_calls as f64 / elapsed as f64) * 60.0
    }
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    pub prompt_price_per_million: f64,
    pub completion_price_per_million: f64,
}

impl ModelPricing {
    pub fn for_model(model: &str) -> Self {
        let model_lower = model.to_lowercase();

        if model_lower.contains("gpt-4o") {
            Self {
                prompt_price_per_million: 2.50,
                completion_price_per_million: 10.0,
            }
        } else if model_lower.contains("gpt-4-turbo") {
            Self {
                prompt_price_per_million: 5.00,
                completion_price_per_million: 15.0,
            }
        } else if model_lower.contains("gpt-3.5-turbo") {
            Self {
                prompt_price_per_million: 0.50,
                completion_price_per_million: 1.50,
            }
        } else if model_lower.contains("minimax") || model_lower.contains("m2.7") {
            Self {
                prompt_price_per_million: 0.50,
                completion_price_per_million: 1.50,
            }
        } else if model_lower.contains("claude") {
            Self {
                prompt_price_per_million: 3.00,
                completion_price_per_million: 15.0,
            }
        } else if model_lower.contains("qwen") {
            Self {
                prompt_price_per_million: 0.50,
                completion_price_per_million: 1.50,
            }
        } else {
            Self {
                prompt_price_per_million: 1.00,
                completion_price_per_million: 3.00,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_tracker_new() {
        let tracker = CostTracker::new();
        assert_eq!(tracker.total_tokens_used, 0);
        assert_eq!(tracker.api_calls, 0);
        assert_eq!(tracker.iterations, 0);
        assert_eq!(tracker.estimated_cost_usd, 0.0);
    }

    #[test]
    fn test_record_call() {
        let mut tracker = CostTracker::new();
        tracker.record_call(100, 50, "gpt-4o");
        assert_eq!(tracker.prompt_tokens, 100);
        assert_eq!(tracker.completion_tokens, 50);
        assert_eq!(tracker.total_tokens_used, 150);
        assert_eq!(tracker.api_calls, 1);
        assert!(tracker.estimated_cost_usd > 0.0);
    }

    #[test]
    fn test_record_iteration() {
        let mut tracker = CostTracker::new();
        tracker.record_iteration();
        assert_eq!(tracker.iterations, 1);
    }

    #[test]
    fn test_model_pricing() {
        let pricing = ModelPricing::for_model("gpt-4o");
        assert_eq!(pricing.prompt_price_per_million, 2.50);

        let pricing = ModelPricing::for_model("minimax-m2.7");
        assert_eq!(pricing.prompt_price_per_million, 0.50);
    }

    #[test]
    fn test_reset() {
        let mut tracker = CostTracker::new();
        tracker.record_call(100, 50, "gpt-4o");
        tracker.record_iteration();
        tracker.reset();
        assert_eq!(tracker.total_tokens_used, 0);
        assert_eq!(tracker.iterations, 0);
    }
}
