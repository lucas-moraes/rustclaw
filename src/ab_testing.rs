use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Experiment {
    pub id: String,
    pub name: String,
    pub variants: Vec<Variant>,
    pub status: ExperimentStatus,
    pub start_time: i64,
    pub end_time: Option<i64>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variant {
    pub id: String,
    pub name: String,
    pub weight: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ExperimentStatus {
    Draft,
    Running,
    Paused,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentAssignment {
    pub experiment_id: String,
    pub variant_id: String,
    pub assigned_at: i64,
}

pub struct ABTestingEngine {
    experiments: HashMap<String, Experiment>,
    assignments: HashMap<String, Vec<ExperimentAssignment>>,
    user_weights: HashMap<String, f32>,
}

impl ABTestingEngine {
    pub fn new() -> Self {
        Self {
            experiments: HashMap::new(),
            assignments: HashMap::new(),
            user_weights: HashMap::new(),
        }
    }

    pub fn register_experiment(&mut self, experiment: Experiment) {
        self.experiments.insert(experiment.id.clone(), experiment);
    }

    pub fn create_experiment(
        &mut self,
        id: &str,
        name: &str,
        variants: Vec<(&str, f32)>,
    ) -> &mut Experiment {
        let experiment = Experiment {
            id: id.to_string(),
            name: name.to_string(),
            variants: variants
                .into_iter()
                .map(|(id, weight)| Variant {
                    id: id.to_string(),
                    name: id.to_string(),
                    weight,
                })
                .collect(),
            status: ExperimentStatus::Draft,
            start_time: chrono::Utc::now().timestamp(),
            end_time: None,
            metadata: HashMap::new(),
        };

        self.experiments.insert(id.to_string(), experiment);
        self.experiments.get_mut(id).unwrap()
    }

    pub fn start(&mut self, id: &str) -> Result<(), String> {
        let exp = self
            .experiments
            .get_mut(id)
            .ok_or_else(|| format!("Experiment '{}' not found", id))?;

        exp.status = ExperimentStatus::Running;
        Ok(())
    }

    pub fn pause(&mut self, id: &str) -> Result<(), String> {
        let exp = self
            .experiments
            .get_mut(id)
            .ok_or_else(|| format!("Experiment '{}' not found", id))?;

        exp.status = ExperimentStatus::Paused;
        Ok(())
    }

    pub fn complete(&mut self, id: &str) -> Result<(), String> {
        let exp = self
            .experiments
            .get_mut(id)
            .ok_or_else(|| format!("Experiment '{}' not found", id))?;

        exp.status = ExperimentStatus::Completed;
        exp.end_time = Some(chrono::Utc::now().timestamp());
        Ok(())
    }

    pub fn assign(&mut self, user_id: &str, experiment_id: &str) -> Option<String> {
        let experiment = self.experiments.get(experiment_id)?;

        if !matches!(experiment.status, ExperimentStatus::Running) {
            return None;
        }

        let weight = self
            .user_weights
            .get(user_id)
            .copied()
            .unwrap_or_else(|| self.compute_user_weight(user_id));

        let variant = self.choose_variant(&experiment.variants, weight)?;
        let variant_id = variant.id.clone();

        let assignment = ExperimentAssignment {
            experiment_id: experiment_id.to_string(),
            variant_id: variant_id.clone(),
            assigned_at: chrono::Utc::now().timestamp(),
        };

        self.assignments
            .entry(user_id.to_string())
            .or_insert_with(Vec::new)
            .push(assignment);

        Some(variant_id)
    }

    fn compute_user_weight(&self, user_id: &str) -> f32 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        user_id.hash(&mut hasher);
        (hasher.finish() as f32) / (u64::MAX as f32)
    }

    fn choose_variant(&self, variants: &[Variant], weight: f32) -> Option<&Variant> {
        let total: f32 = variants.iter().map(|v| v.weight).sum();
        if total == 0.0 {
            return variants.first();
        }

        let mut cumulative = 0.0;
        for variant in variants {
            cumulative += variant.weight / total;
            if weight <= cumulative {
                return Some(variant);
            }
        }

        variants.last()
    }

    pub fn get_assignment(&self, user_id: &str, experiment_id: &str) -> Option<&str> {
        self.assignments.get(user_id).and_then(|assignments| {
            assignments
                .iter()
                .find(|a| a.experiment_id == experiment_id)
                .map(|a| a.variant_id.as_str())
        })
    }

    pub fn get_variant(&self, user_id: &str, experiment_id: &str) -> Option<String> {
        self.get_assignment(user_id, experiment_id)
            .map(|s| s.to_string())
            .or_else(|| self.assign(user_id, experiment_id))
    }

    pub fn record_event(&self, user_id: &str, experiment_id: &str, event: &str) {
        tracing::info!(
            "A/B Event: user={} experiment={} variant={} event={}",
            user_id,
            experiment_id,
            self.get_assignment(user_id, experiment_id)
                .unwrap_or("unknown"),
            event
        );
    }

    pub fn list_experiments(&self) -> Vec<&Experiment> {
        self.experiments.values().collect()
    }

    pub fn get_experiment(&self, id: &str) -> Option<&Experiment> {
        self.experiments.get(id)
    }

    pub fn get_results(&self, id: &str) -> Option<ExperimentResults> {
        let experiment = self.experiments.get(id)?;

        let mut variant_counts: HashMap<String, usize> = HashMap::new();
        let mut variant_events: HashMap<String, HashMap<String, usize>> = HashMap::new();

        for assignments in self.assignments.values() {
            for a in assignments {
                if a.experiment_id == id {
                    *variant_counts.entry(a.variant_id.clone()).or_insert(0) += 1;
                }
            }
        }

        Some(ExperimentResults {
            experiment_id: id.to_string(),
            total_participants: variant_counts.values().sum(),
            variant_counts,
            variant_events,
        })
    }
}

impl Default for ABTestingEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentResults {
    pub experiment_id: String,
    pub total_participants: usize,
    pub variant_counts: HashMap<String, usize>,
    pub variant_events: HashMap<String, HashMap<String, usize>>,
}

pub struct FeatureFlags {
    flags: HashMap<String, bool>,
    experiments: ABTestingEngine,
}

impl FeatureFlags {
    pub fn new() -> Self {
        Self {
            flags: HashMap::new(),
            experiments: ABTestingEngine::new(),
        }
    }

    pub fn set(&mut self, key: &str, value: bool) {
        self.flags.insert(key.to_string(), value);
    }

    pub fn get(&self, key: &str) -> bool {
        self.flags.get(key).copied().unwrap_or(false)
    }

    pub fn is_enabled(&self, key: &str) -> bool {
        self.get(key)
    }

    pub fn toggle(&mut self, key: &str) {
        let current = self.get(key);
        self.set(key, !current);
    }

    pub fn get_with_experiment(&self, user_id: &str, key: &str, experiment_id: &str) -> bool {
        if self.flags.contains_key(key) {
            return self.get(key);
        }

        self.experiments
            .get_variant(user_id, experiment_id)
            .map(|v| v == "treatment" || v == "true" || v == "enabled")
            .unwrap_or(false)
    }

    pub fn add_experiment(&mut self, experiment: Experiment) {
        self.experiments.register_experiment(experiment);
    }

    pub fn list_flags(&self) -> Vec<(&str, bool)> {
        self.flags.iter().map(|(k, v)| (k.as_str(), *v)).collect()
    }
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_flags() {
        let mut flags = FeatureFlags::new();

        flags.set("feature_a", true);
        flags.set("feature_b", false);

        assert!(flags.is_enabled("feature_a"));
        assert!(!flags.is_enabled("feature_b"));

        flags.toggle("feature_a");
        assert!(!flags.is_enabled("feature_a"));
    }

    #[test]
    fn test_experiment_assignment() {
        let mut engine = ABTestingEngine::new();

        engine.create_experiment(
            "exp1",
            "Test Experiment",
            vec![("control", 50.0), ("treatment", 50.0)],
        );
        engine.start("exp1").unwrap();

        let variant = engine.assign("user1", "exp1");
        assert!(variant.is_some());
    }
}
