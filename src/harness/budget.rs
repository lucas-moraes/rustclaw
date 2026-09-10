//! Daily USD budget tracking (P3).
//!
//! A [`BudgetTracker`] accumulates estimated cost per local day
//! (`YYYY-MM-DD`) and persists it to `usage-YYYY-MM.json` in the data dir so
//! the total survives restarts. Usage is persisted even when no budget is
//! configured — it is cheap (one small JSON file per month, written once per
//! turn) and powers the "today" line of `/usage`.
//!
//! Warn thresholds are computed by the pure [`budget_status`] function:
//! a warning at 80% of the configured limit and one when the limit is
//! exceeded. Each warning fires at most once per day per process (flags live
//! in the tracker, not on disk).

use std::collections::HashMap;
use std::path::PathBuf;

use crate::harness::provider::catalog::estimate_cost_cached;
use crate::harness::provider::Usage;

/// Warn level produced by [`budget_status`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BudgetWarn {
    /// Spent reached 80% of the daily limit.
    Eighty,
    /// Spent exceeded the daily limit.
    Exceeded,
}

impl BudgetWarn {
    /// Human-readable system-line message for the warn level.
    pub fn message(self, spent: f64, limit: f64) -> String {
        match self {
            BudgetWarn::Eighty => format!("⚠ daily budget at 80%: ${:.2} / ${:.2}", spent, limit),
            BudgetWarn::Exceeded => {
                format!("⚠ daily budget exceeded: ${:.2} / ${:.2}", spent, limit)
            }
        }
    }
}

/// Pure threshold decision: `None` when there is no limit or nothing was
/// crossed, `Some(Eighty)` at ≥80%, `Some(Exceeded)` at ≥100%.
pub fn budget_status(spent: f64, limit: f64) -> Option<BudgetWarn> {
    if limit <= 0.0 {
        return None;
    }
    if spent >= limit {
        Some(BudgetWarn::Exceeded)
    } else if spent >= limit * 0.8 {
        Some(BudgetWarn::Eighty)
    } else {
        None
    }
}

/// Local day key (`YYYY-MM-DD`) used as the accumulator key.
pub fn today_key() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Month file key (`YYYY-MM`) for the per-month usage file.
fn month_key() -> String {
    chrono::Local::now().format("%Y-%m").to_string()
}

/// Accumulates USD cost per local day, persisted to
/// `<data_local_dir>/rustclaw/usage-YYYY-MM.json` (atomic write: temp + rename).
pub struct BudgetTracker {
    /// Explicit path root for the usage files (`None` = default data dir).
    /// When `None` and no data dir is available, tracking is in-memory only.
    root: Option<PathBuf>,
    days: HashMap<String, f64>,
    warned: HashMap<String, BudgetWarn>,
}

impl Default for BudgetTracker {
    fn default() -> Self {
        let root = dirs::data_local_dir().map(|d| d.join("rustclaw"));
        Self::with_root(root)
    }
}

impl BudgetTracker {
    /// Testable constructor with an explicit root directory (usage files are
    /// written as `<root>/usage-YYYY-MM.json`).
    pub fn with_root(root: Option<PathBuf>) -> Self {
        let mut t = Self {
            root,
            days: HashMap::new(),
            warned: HashMap::new(),
        };
        t.load_today();
        t
    }

    fn usage_path(&self) -> Option<PathBuf> {
        self.root
            .as_ref()
            .map(|r| r.join(format!("usage-{}.json", month_key())))
    }

    /// Loads the current month's file (missing file = empty). Older months
    /// are simply not loaded; each month starts a fresh file.
    fn load_today(&mut self) {
        let Some(path) = self.usage_path() else {
            return;
        };
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return;
        };
        if let Ok(map) = serde_json::from_str::<HashMap<String, f64>>(&raw) {
            self.days = map;
        }
    }

    /// Adds `cost` to today's total, persists the file and returns the warn
    /// (if any) for the new total given `daily_limit` (0 = no limit). Each
    /// warn level fires at most once per day.
    pub fn record(&mut self, cost: f64, daily_limit: f64) -> Option<BudgetWarn> {
        if cost <= 0.0 {
            return None;
        }
        let day = today_key();
        *self.days.entry(day.clone()).or_insert(0.0) += cost;
        self.save();
        self.warn_for(&day, daily_limit)
    }

    /// Today's accumulated spend (loaded file + this process).
    pub fn spent_today(&self) -> f64 {
        self.days.get(&today_key()).copied().unwrap_or(0.0)
    }

    /// Warn decision for a day, honoring the once-per-day flags.
    fn warn_for(&mut self, day: &str, daily_limit: f64) -> Option<BudgetWarn> {
        let spent = self.days.get(day).copied().unwrap_or(0.0);
        let level = budget_status(spent, daily_limit)?;
        let already = self.warned.get(day);
        let skip = matches!(
            (&level, already),
            (BudgetWarn::Exceeded, Some(BudgetWarn::Exceeded)) | (BudgetWarn::Eighty, Some(_))
        );
        if skip {
            return None;
        }
        self.warned.insert(day.to_string(), level);
        Some(level)
    }

    /// Atomic persist: write `<path>.tmp` then rename over the target.
    /// Best-effort: failures are logged, never propagated (cost tracking
    /// must not break turns).
    fn save(&self) {
        let Some(path) = self.usage_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = match serde_json::to_string(&self.days) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("failed to serialize usage file: {e}");
                return;
            }
        };
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, json) {
            tracing::warn!("failed to write {}: {e}", tmp.display());
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, &path) {
            tracing::warn!("failed to rename usage file {}: {e}", path.display());
        }
    }
}

/// Convenience: estimated USD cost of a turn's usage at the runtime's
/// provider/model.
pub fn turn_cost(provider: &str, model: &str, usage: &Usage) -> f64 {
    estimate_cost_cached(provider, model, usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker(dir: &std::path::Path) -> BudgetTracker {
        BudgetTracker::with_root(Some(dir.to_path_buf()))
    }

    #[test]
    fn test_budget_status_thresholds() {
        assert_eq!(budget_status(0.5, 1.0), None);
        assert_eq!(budget_status(0.79, 1.0), None);
        assert_eq!(budget_status(0.80, 1.0), Some(BudgetWarn::Eighty));
        assert_eq!(budget_status(0.99, 1.0), Some(BudgetWarn::Eighty));
        assert_eq!(budget_status(1.0, 1.0), Some(BudgetWarn::Exceeded));
        assert_eq!(budget_status(2.0, 1.0), Some(BudgetWarn::Exceeded));
        // No budget configured → never warns.
        assert_eq!(budget_status(100.0, 0.0), None);
    }

    #[test]
    fn test_tracker_accumulates_and_persists() {
        let d = tempfile::tempdir().unwrap();
        {
            let mut t = tracker(d.path());
            assert_eq!(t.record(0.3, 0.0), None); // no limit → no warn
            assert_eq!(t.record(0.2, 0.0), None);
            assert!((t.spent_today() - 0.5).abs() < 1e-9);
        }
        // Reload from disk (simulated restart).
        let t = tracker(d.path());
        assert!((t.spent_today() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_warn_once_per_day() {
        let d = tempfile::tempdir().unwrap();
        let mut t = tracker(d.path());
        // limit 1.0: 0.85 → 85% (80%-warn); again 0.1 (95%) → suppressed;
        // cross 1.0 → exceeded; repeat → suppressed.
        assert_eq!(t.record(0.5, 1.0), None); // 50%: below the warn threshold
        assert_eq!(t.record(0.4, 1.0), Some(BudgetWarn::Eighty)); // 90%
        assert_eq!(t.record(0.2, 1.0), Some(BudgetWarn::Exceeded)); // 1.1 crosses 1.0
        assert_eq!(t.record(0.5, 1.0), None);
    }

    #[test]
    fn test_zero_cost_is_noop() {
        let d = tempfile::tempdir().unwrap();
        let mut t = tracker(d.path());
        assert_eq!(t.record(0.0, 1.0), None);
        assert_eq!(t.spent_today(), 0.0);
    }

    #[test]
    fn test_warn_message_format() {
        assert_eq!(
            BudgetWarn::Exceeded.message(1.234, 1.0),
            "⚠ daily budget exceeded: $1.23 / $1.00"
        );
        assert_eq!(
            BudgetWarn::Eighty.message(0.8, 1.0),
            "⚠ daily budget at 80%: $0.80 / $1.00"
        );
    }

    #[tokio::test]
    async fn test_concurrent_record_persists_total() {
        // Concurrent `record` calls through a shared tokio Mutex must not lose
        // updates and must persist the accumulated total to disk.
        let d = tempfile::tempdir().unwrap();
        let shared = std::sync::Arc::new(tokio::sync::Mutex::new(tracker(d.path())));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let t = shared.clone();
            handles.push(tokio::spawn(async move {
                let mut guard = t.lock().await;
                guard.record(0.1, 0.0);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        // All 8 × 0.1 accumulated.
        let total = shared.lock().await.spent_today();
        assert!((total - 0.8).abs() < 1e-9, "total: {total}");

        // Reload from disk: the persisted total survives.
        let reloaded = tracker(d.path());
        assert!((reloaded.spent_today() - 0.8).abs() < 1e-9);
    }
}
