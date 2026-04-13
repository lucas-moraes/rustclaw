//! Hierarchical memory system with automatic tier management.
//!
//! Memory tiers: Working -> ShortTerm -> LongTerm
//! Automatic promotion based on usage and importance.

use crate::memory::MemoryEntry;
use crate::memory::MemoryType;
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTier {
    Working,
    ShortTerm,
    LongTerm,
}

impl MemoryTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryTier::Working => "working",
            MemoryTier::ShortTerm => "short_term",
            MemoryTier::LongTerm => "long_term",
        }
    }
}

pub struct HierarchicalMemory {
    working: Vec<MemoryEntry>,
    short_term: Vec<MemoryEntry>,
    long_term: Vec<MemoryEntry>,
    access_counts: HashMap<String, usize>,
}

impl HierarchicalMemory {
    pub fn new() -> Self {
        Self {
            working: Vec::new(),
            short_term: Vec::new(),
            long_term: Vec::new(),
            access_counts: HashMap::new(),
        }
    }

    pub fn add(&mut self, entry: MemoryEntry, tier: MemoryTier) {
        let id = entry.id.clone();
        match tier {
            MemoryTier::Working => self.working.push(entry),
            MemoryTier::ShortTerm => self.short_term.push(entry),
            MemoryTier::LongTerm => self.long_term.push(entry),
        }
        self.access_counts.insert(id, 0);
    }

    pub fn get(&self, id: &str) -> Option<&MemoryEntry> {
        self.working
            .iter()
            .find(|e| e.id == id)
            .or_else(|| self.short_term.iter().find(|e| e.id == id))
            .or_else(|| self.long_term.iter().find(|e| e.id == id))
    }

    pub fn get_all(&self) -> Vec<&MemoryEntry> {
        let mut all = Vec::new();
        all.extend(self.working.iter());
        all.extend(self.short_term.iter());
        all.extend(self.long_term.iter());
        all
    }

    pub fn touch(&mut self, id: &str) {
        *self.access_counts.entry(id.to_string()).or_insert(0) += 1;
    }

    pub fn promote_if_needed(&mut self, id: &str) {
        if let Some(pos) = self.working.iter().position(|e| e.id == id) {
            if self.access_counts.get(id).unwrap_or(&0) > &5 {
                let entry = self.working.remove(pos);
                self.short_term.push(entry);
            }
        }
    }

    pub fn demote_old_short_term(&mut self) {
        let cutoff = Utc::now() - Duration::days(7);
        self.short_term.retain(|e| e.timestamp > cutoff);
    }

    pub fn consolidate_to_long_term(&mut self, min_importance: f32) {
        let mut remaining = Vec::new();
        for entry in self.short_term.drain(..) {
            if entry.importance >= min_importance {
                self.long_term.push(entry);
            } else {
                remaining.push(entry);
            }
        }
        self.short_term = remaining;
    }

    pub fn tier_counts(&self) -> (usize, usize, usize) {
        (
            self.working.len(),
            self.short_term.len(),
            self.long_term.len(),
        )
    }

    pub fn clear_working(&mut self) {
        self.working.clear();
    }

    pub fn clear_all(&mut self) {
        self.working.clear();
        self.short_term.clear();
        self.long_term.clear();
        self.access_counts.clear();
    }
}

impl Default for HierarchicalMemory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_entry(id: &str, importance: f32) -> MemoryEntry {
        let mut entry = MemoryEntry::new(
            format!("Content for {}", id),
            vec![0.0; 384],
            MemoryType::Fact,
            importance,
        );
        entry.id = id.to_string();
        entry
    }

    #[test]
    fn test_tier_assignment() {
        let mut mem = HierarchicalMemory::new();

        mem.add(create_entry("w1", 0.5), MemoryTier::Working);
        mem.add(create_entry("s1", 0.7), MemoryTier::ShortTerm);
        mem.add(create_entry("l1", 0.9), MemoryTier::LongTerm);

        assert_eq!(mem.tier_counts(), (1, 1, 1));
    }

    #[test]
    fn test_access_tracking() {
        let mut mem = HierarchicalMemory::new();

        mem.add(create_entry("test", 0.5), MemoryTier::Working);
        mem.touch("test");
        mem.touch("test");

        assert_eq!(mem.access_counts.get("test"), Some(&2));
    }

    #[test]
    fn test_promotion() {
        let mut mem = HierarchicalMemory::new();

        mem.add(create_entry("w_test", 0.5), MemoryTier::Working);
        assert_eq!(mem.tier_counts(), (1, 0, 0));

        for _ in 0..6 {
            mem.touch("w_test");
        }
        mem.promote_if_needed("w_test");

        assert_eq!(mem.tier_counts(), (0, 1, 0));
    }

    #[test]
    fn test_consolidation() {
        let mut mem = HierarchicalMemory::new();

        mem.add(create_entry("s_low", 0.2), MemoryTier::ShortTerm);
        mem.add(create_entry("s_high", 0.8), MemoryTier::ShortTerm);

        mem.consolidate_to_long_term(0.5);

        assert_eq!(mem.tier_counts(), (0, 1, 1));
    }
}
