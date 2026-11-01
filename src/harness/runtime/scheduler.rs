//! Global subagent scheduler.
//!
//! `task` already caps concurrency *within* one batch call, but nothing caps
//! the total: two batches running at the same time (or a subagent spawning its
//! own children) could put 8+ agents on the provider at once. The scheduler
//! owns a single process-wide semaphore so every subagent — from any batch, at
//! any depth — queues behind the same limit.
//!
//! Queueing is fair (tokio's semaphore is FIFO), so a batch of 4 does not
//! starve a single task submitted later.

use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Default cap on subagents running at the same time, process-wide.
pub const DEFAULT_MAX_CONCURRENT_SUBAGENTS: usize = 4;

/// Process-wide subagent concurrency limiter.
#[derive(Clone)]
pub struct SubagentScheduler {
    semaphore: Arc<Semaphore>,
    limit: usize,
}

impl Default for SubagentScheduler {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CONCURRENT_SUBAGENTS)
    }
}

impl SubagentScheduler {
    /// Creates a scheduler allowing `limit` concurrent subagents (min 1).
    pub fn new(limit: usize) -> Self {
        let limit = limit.max(1);
        Self {
            semaphore: Arc::new(Semaphore::new(limit)),
            limit,
        }
    }

    /// Configured concurrency limit.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Subagents currently running.
    pub fn running(&self) -> usize {
        self.limit - self.semaphore.available_permits()
    }

    /// Subagents waiting for a slot.
    pub fn queued(&self) -> usize {
        self.semaphore
            .available_permits()
            .saturating_sub(self.limit)
    }

    /// Waits for a free slot. The permit is released when dropped, so callers
    /// must hold it for the whole duration of the subagent run.
    pub async fn acquire(&self) -> OwnedSemaphorePermit {
        self.semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("subagent scheduler semaphore is never closed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn limit_is_at_least_one() {
        assert_eq!(SubagentScheduler::new(0).limit(), 1);
        assert_eq!(SubagentScheduler::new(3).limit(), 3);
    }

    #[tokio::test]
    async fn never_exceeds_the_limit() {
        let sched = SubagentScheduler::new(2);
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let mut set = tokio::task::JoinSet::new();
        for _ in 0..6 {
            let sched = sched.clone();
            let running = running.clone();
            let peak = peak.clone();
            set.spawn(async move {
                let _permit = sched.acquire().await;
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                running.fetch_sub(1, Ordering::SeqCst);
            });
        }
        while set.join_next().await.is_some() {}
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn permit_release_frees_a_slot() {
        let sched = SubagentScheduler::new(1);
        let permit = sched.acquire().await;
        assert_eq!(sched.running(), 1);
        drop(permit);
        assert_eq!(sched.running(), 0);
        // A second acquire succeeds immediately once the slot is free.
        let _again = tokio::time::timeout(Duration::from_millis(50), sched.acquire())
            .await
            .expect("slot should be free");
    }

    #[tokio::test]
    async fn queued_tasks_wait_for_a_slot() {
        let sched = SubagentScheduler::new(1);
        let held = sched.acquire().await;
        let sched2 = sched.clone();
        let waiter = tokio::spawn(async move {
            let _p = sched2.acquire().await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished(), "second task must be queued");
        drop(held);
        tokio::time::timeout(Duration::from_millis(200), waiter)
            .await
            .expect("queued task should run after release")
            .unwrap();
    }
}
