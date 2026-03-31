use crate::app_state::AppState;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::RwLock;

const MAX_HISTORY_SIZE: usize = 100;

pub struct TimeTravelState {
    history: VecDeque<AppState>,
    current_index: isize,
    max_size: usize,
}

impl TimeTravelState {
    pub fn new(max_size: usize) -> Self {
        Self {
            history: VecDeque::new(),
            current_index: -1,
            max_size,
        }
    }

    pub fn push(&mut self, state: AppState) {
        while self.current_index < (self.history.len() as isize - 1) {
            self.history.pop_back();
        }

        self.history.push_back(state);

        if self.history.len() > self.max_size {
            self.history.pop_front();
        } else {
            self.current_index += 1;
        }
    }

    pub fn can_undo(&self) -> bool {
        self.current_index > 0
    }

    pub fn can_redo(&self) -> bool {
        self.current_index < (self.history.len() as isize - 1)
    }

    pub fn undo(&self) -> Option<AppState> {
        if self.can_undo() {
            self.current_index -= 1;
            self.history.get(self.current_index as usize).cloned()
        } else {
            None
        }
    }

    pub fn redo(&self) -> Option<AppState> {
        if self.can_redo() {
            self.current_index += 1;
            self.history.get(self.current_index as usize).cloned()
        } else {
            None
        }
    }

    pub fn go_to(&self, index: usize) -> Option<AppState> {
        let idx = index as isize;
        if idx >= 0 && idx < self.history.len() as isize {
            self.current_index = idx;
            self.history.get(index).cloned()
        } else {
            None
        }
    }

    pub fn current(&self) -> Option<AppState> {
        self.history.get(self.current_index as usize).cloned()
    }

    pub fn len(&self) -> usize {
        self.history.len()
    }

    pub fn index(&self) -> isize {
        self.current_index
    }

    pub fn history_list(&self) -> Vec<(usize, String)> {
        self.history
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let desc = if i == self.current_index as usize {
                    format!("[CURRENT] {}", s.session.session_id)
                } else if i < self.current_index as usize {
                    format!("[PAST] {}", s.session.session_id)
                } else {
                    format!("[FUTURE] {}", s.session.session_id)
                };
                (i, desc)
            })
            .collect()
    }

    pub fn clear(&mut self) {
        self.history.clear();
        self.current_index = -1;
    }
}

impl Default for TimeTravelState {
    fn default() -> Self {
        Self::new(MAX_HISTORY_SIZE)
    }
}

pub struct TimeTravelStore {
    state: Arc<RwLock<TimeTravelState>>,
}

impl TimeTravelStore {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(TimeTravelState::default())),
        }
    }

    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            state: Arc::new(RwLock::new(TimeTravelState::new(max_size))),
        }
    }

    pub fn push(&self, app_state: AppState) {
        self.state.write().unwrap().push(app_state);
    }

    pub fn undo(&self) -> Option<AppState> {
        self.state.read().unwrap().undo()
    }

    pub fn redo(&self) -> Option<AppState> {
        self.state.read().unwrap().redo()
    }

    pub fn go_to(&self, index: usize) -> Option<AppState> {
        self.state.read().unwrap().go_to(index)
    }

    pub fn can_undo(&self) -> bool {
        self.state.read().unwrap().can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.state.read().unwrap().can_redo()
    }

    pub fn current(&self) -> Option<AppState> {
        self.state.read().unwrap().current()
    }

    pub fn history(&self) -> Vec<(usize, String)> {
        self.state.read().unwrap().history_list()
    }

    pub fn snapshot(&self) -> Vec<AppState> {
        self.state.read().unwrap().history.iter().cloned().collect()
    }

    pub fn restore(&self, snapshot: Vec<AppState>) {
        let mut state = self.state.write().unwrap();
        state.clear();
        for s in snapshot {
            state.push(s);
        }
    }
}

impl Default for TimeTravelStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_state(id: &str) -> AppState {
        let mut state = AppState::default();
        state.session.session_id = id.to_string();
        state
    }

    #[test]
    fn test_push_and_undo() {
        let store = TimeTravelStore::new();

        store.push(create_test_state("state1"));
        store.push(create_test_state("state2"));
        store.push(create_test_state("state3"));

        assert!(store.can_undo());
        assert!(!store.can_redo());

        let undone = store.undo();
        assert!(undone.is_some());
        assert_eq!(undone.unwrap().session.session_id, "state2");
    }

    #[test]
    fn test_redo() {
        let store = TimeTravelStore::new();

        store.push(create_test_state("state1"));
        store.push(create_test_state("state2"));

        let _ = store.undo();

        assert!(store.can_redo());

        let redone = store.redo();
        assert!(redone.is_some());
        assert_eq!(redone.unwrap().session.session_id, "state2");
    }

    #[test]
    fn test_history_list() {
        let store = TimeTravelStore::new();

        store.push(create_test_state("state1"));
        store.push(create_test_state("state2"));
        store.push(create_test_state("state3"));

        let _ = store.undo();

        let history = store.history();
        assert_eq!(history.len(), 3);

        assert!(history[0].1.contains("[PAST]"));
        assert!(history[1].1.contains("[CURRENT]"));
        assert!(history[2].1.contains("[FUTURE]"));
    }
}
