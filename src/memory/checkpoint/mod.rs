#![allow(dead_code)]

pub mod types;
pub use types::*;
pub mod events;
pub use events::*;
pub mod lifecycle;
pub use lifecycle::*;
pub mod store;
pub use store::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plan_steps_numbered() {
        let mut cp = DevelopmentCheckpoint::new("test".to_string());
        cp.set_plan_text("1) Criar diretório\n2) Escrever código\n3) Testar".to_string());
        let steps = cp.parse_plan_steps();
        assert_eq!(steps.len(), 3);
        assert!(steps[0].contains("Criar diretório"));
        assert!(steps[1].contains("Escrever código"));
        assert!(steps[2].contains("Testar"));
    }

    #[test]
    fn test_parse_plan_steps_bullet() {
        let mut cp = DevelopmentCheckpoint::new("test".to_string());
        cp.set_plan_text("- Criar arquivo\n- Editar conteúdo".to_string());
        let steps = cp.parse_plan_steps();
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn test_mark_step_done() {
        let mut cp = DevelopmentCheckpoint::new("test".to_string());
        cp.mark_step_done(0);
        cp.mark_step_done(2);
        cp.mark_step_done(0);
        assert!(cp.is_step_done(0));
        assert!(!cp.is_step_done(1));
        assert!(cp.is_step_done(2));
        assert_eq!(cp.completed_steps, vec![0, 2]);
    }

    #[test]
    fn test_is_plan_mode() {
        let mut cp = DevelopmentCheckpoint::new("test".to_string());
        assert!(!cp.is_plan_mode());
        cp.set_phase(PlanPhase::Executing);
        assert!(!cp.is_plan_mode());
        cp.set_plan_text("1) Passo 1".to_string());
        assert!(cp.is_plan_mode());
    }
}
