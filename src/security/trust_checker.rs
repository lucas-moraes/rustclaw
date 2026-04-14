use crate::workspace_trust::{Operation, TrustDecision, TrustEvaluator, TrustLevel};
use std::path::Path;

pub struct TrustChecker {
    evaluator: TrustEvaluator,
    trust_file: Option<std::path::PathBuf>,
}

impl TrustChecker {
    pub fn new() -> Self {
        Self {
            evaluator: TrustEvaluator::new(),
            trust_file: None,
        }
    }

    pub fn with_evaluator(evaluator: TrustEvaluator) -> Self {
        Self {
            evaluator,
            trust_file: None,
        }
    }

    pub fn with_trust_file(mut self, path: std::path::PathBuf) -> Self {
        self.trust_file = Some(path);
        self
    }

    pub fn load(&mut self) -> Result<(), String> {
        if let Some(ref path) = self.trust_file {
            if path.exists() {
                self.evaluator.get_store_mut().load(path)?;
            }
        }
        Ok(())
    }

    pub fn save(&self) -> Result<(), String> {
        if let Some(ref path) = self.trust_file {
            self.evaluator.get_store().save(path)?;
        }
        Ok(())
    }

    pub fn evaluate(&self, path: &Path, operation: Operation) -> TrustDecision {
        self.evaluator.evaluate(path, &operation)
    }

    pub fn check_read(&self, path: &Path) -> TrustDecision {
        self.evaluate(path, Operation::ReadFile)
    }

    pub fn check_write(&self, path: &Path) -> TrustDecision {
        self.evaluate(path, Operation::WriteFile)
    }

    pub fn check_shell(&self, path: &Path) -> TrustDecision {
        self.evaluate(path, Operation::ExecuteShell)
    }

    pub fn check_network(&self, path: &Path) -> TrustDecision {
        self.evaluate(path, Operation::NetworkRequest)
    }

    pub fn check_install(&self, path: &Path) -> TrustDecision {
        self.evaluate(path, Operation::InstallPackage)
    }

    pub fn check_sensitive(&self, path: &Path) -> TrustDecision {
        self.evaluate(path, Operation::ReadSensitive)
    }

    pub fn can_write(&self, path: &Path) -> bool {
        self.check_write(path).allowed
    }

    pub fn can_execute_shell(&self, path: &Path) -> bool {
        self.check_shell(path).allowed
    }

    pub fn can_access_network(&self, path: &Path) -> bool {
        self.check_network(path).allowed
    }

    pub fn get_trust_level(&self, path: &Path) -> TrustLevel {
        self.evaluator.get_store().get_trust(path)
    }

    pub fn set_trust(&mut self, path: &Path, level: TrustLevel) {
        self.evaluator.set_trust(path, level);
        if let Err(e) = self.save() {
            tracing::warn!("Failed to save trust after set_trust: {}", e);
        }
    }

    pub fn elevate_trust(&mut self, path: &Path, level: TrustLevel) {
        self.set_trust(path, level);
    }

    pub fn list_workspaces(&self) -> Vec<(&std::path::PathBuf, TrustLevel)> {
        self.evaluator.get_store().list_workspaces()
    }

    pub fn get_store(&self) -> &crate::workspace_trust::WorkspaceTrustStore {
        self.evaluator.get_store()
    }

    pub fn get_store_mut(&mut self) -> &mut crate::workspace_trust::WorkspaceTrustStore {
        self.evaluator.get_store_mut()
    }
}

impl Default for TrustChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl From<TrustEvaluator> for TrustChecker {
    fn from(evaluator: TrustEvaluator) -> Self {
        Self::with_evaluator(evaluator)
    }
}

#[macro_export]
macro_rules! require_trust {
    ($checker:expr, $path:expr, $op:expr) => {{
        let decision = $checker.evaluate($path, $op.clone());
        if !decision.allowed {
            return Err(TrustError::OperationBlocked(format!(
                "operation '{}' not allowed (trust level: {:?})",
                $op, decision.trust_level
            )));
        }
        decision
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn test_trust_checker_default() {
        let checker = TrustChecker::new();
        let path = temp_dir();

        // Default trust is Untrusted
        assert!(!checker.can_execute_shell(&path));
        assert!(!checker.can_write(&path));
        // Untrusted cannot access network
        assert!(!checker.can_access_network(&path));
    }

    #[test]
    fn test_trust_evaluation() {
        let mut checker = TrustChecker::new();
        let path = temp_dir();

        // Elevate trust
        checker.set_trust(&path, TrustLevel::Trusted);

        assert!(checker.can_execute_shell(&path));
        assert!(checker.can_write(&path));
        assert!(checker.can_access_network(&path));
    }

    #[test]
    fn test_operation_checks() {
        let checker = TrustChecker::new();
        let path = temp_dir();

        let write_check = checker.check_write(&path);
        assert!(!write_check.allowed);

        let shell_check = checker.check_shell(&path);
        assert!(!shell_check.allowed);

        let network_check = checker.check_network(&path);
        assert!(!network_check.allowed); // Untrusted cannot access network
    }
}
